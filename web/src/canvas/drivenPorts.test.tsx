// @vitest-environment jsdom
/**
 * The hidden-port rule (docs/13 §Animation transport; docs/17 item 4): a
 * transport-driven input (`cycle.frame`, `clock.t` — the catalog's
 * `transport_driven`) is the session's, not the user's. On the canvas node
 * it has NO handle (nothing to wire into, nothing to drop on) and no
 * literal editor — its row shows the transport instead, lit while the port
 * is in the current graph's driven set; in the inspector it is absent from
 * the inputs list and shown under `transport` with the value the transport
 * feeds it — the frame of THIS port's own loop (a second `cycle` loops
 * inside the primary at its own rate; the primary loop's frame would be a
 * lie), or the playhead in seconds. The node's other ports keep their
 * handles and editors. What the text says is never hidden: a hand-written
 * kwarg is the headless value; a hand-written WIRE keeps a target handle
 * (not connectable — React Flow draws an edge only between two handles)
 * and is named as the headless source on both surfaces. Both surfaces are
 * rendered for real against a seeded store: the catalog says which port,
 * the transport view says whether it is driving and on what loop.
 */
import { ReactFlowProvider, type NodeProps } from "@xyflow/react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Catalog, NodeView, TransportView } from "../protocol/messages";
import { useCicada } from "../state/store";
import { Inspector } from "../panels/Inspector";
import { CicadaNode } from "./CicadaNode";
import type { CanvasNode } from "./flow";

const catalog: Catalog = {
  format: 2,
  nodes: [
    {
      name: "cycle",
      title: "Cycle",
      description: "looping time 0 → 1",
      category: "Params & input",
      tier: "1",
      version: 1,
      pure: true,
      uses_tolerance: false,
      gh: null,
      examples: [],
      inputs: [
        { name: "period", type: "Number", base: "Number", list_depth: 0, optional: false, default: "4.0", doc: "Seconds per loop." },
        { name: "frames", type: "Integer", base: "Integer", list_depth: 0, optional: false, default: "120", doc: "Frames per loop." },
        {
          name: "frame",
          type: "Integer",
          base: "Integer",
          list_depth: 0,
          optional: false,
          default: "0",
          doc: "The current frame.",
          transport_driven: "frame",
        },
      ],
      outputs: [{ name: "out", type: "Number", base: "Number", list_depth: 0, optional: false, doc: "The loop position." }],
    },
    {
      name: "clock",
      title: "Clock",
      description: "unbounded time in seconds",
      category: "Params & input",
      tier: "1",
      version: 1,
      pure: true,
      uses_tolerance: false,
      gh: null,
      examples: [],
      inputs: [
        { name: "speed", type: "Number", base: "Number", list_depth: 0, optional: false, default: "1.0", doc: "Rate." },
        { name: "t", type: "Number", base: "Number", list_depth: 0, optional: false, default: "0.0", doc: "Seconds.", transport_driven: "time" },
      ],
      outputs: [{ name: "out", type: "Number", base: "Number", list_depth: 0, optional: false, doc: "Seconds × speed." }],
    },
  ],
};

/** A `cycle` node view: `name = cycle(period=…, frames=…[, frame=…])`. */
function cycleNode(name: string, line: number, period: number, frames: number, frame?: { literal?: string; wired?: { node: string; port: string } }): NodeView {
  const text = `${name} = cycle(period=${period.toFixed(1)}, frames=${frames}${frame?.literal !== undefined ? `, frame=${frame.literal}` : frame?.wired !== undefined ? `, frame=${frame.wired.node}` : ""})`;
  return {
    ref: line,
    name,
    targets: [name],
    line,
    text,
    kind: "call",
    func: "cycle",
    title: "Cycle",
    category: "Params & input",
    inputs: [
      { name: "period", type: "Number", base: "Number", depth: 0, optional: false, required: false, default: "4.0", literal: period.toFixed(1), literal_value: period, lift: 0 },
      { name: "frames", type: "Integer", base: "Integer", depth: 0, optional: false, required: false, default: "120", literal: String(frames), literal_value: frames, lift: 0 },
      {
        name: "frame",
        type: "Integer",
        base: "Integer",
        depth: 0,
        optional: false,
        required: false,
        default: "0",
        lift: 0,
        ...(frame?.literal !== undefined ? { literal: frame.literal, literal_value: Number(frame.literal) } : {}),
        ...(frame?.wired !== undefined ? { wired: frame.wired } : {}),
      },
    ],
    outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, line * 4],
    size: [8, 4],
    manual: false,
  };
}

/** `spin = cycle(period=4.0, frames=120, frame=5)` — the `frame=5` written by hand is the headless value. */
const spin: NodeView = {
  ref: 1,
  name: "spin",
  targets: ["spin"],
  line: 2,
  text: "spin = cycle(period=4.0, frames=120, frame=5)",
  kind: "call",
  func: "cycle",
  title: "Cycle",
  category: "Params & input",
  inputs: [
    { name: "period", type: "Number", base: "Number", depth: 0, optional: false, required: false, default: "4.0", literal: "4.0", literal_value: 4, lift: 0 },
    { name: "frames", type: "Integer", base: "Integer", depth: 0, optional: false, required: false, default: "120", literal: "120", literal_value: 120, lift: 0 },
    { name: "frame", type: "Integer", base: "Integer", depth: 0, optional: false, required: false, default: "0", literal: "5", literal_value: 5, lift: 0 },
  ],
  outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
  diagnostics: [],
  effectful: false,
  preview: false,
  cell: [0, 0],
  size: [8, 4],
  manual: false,
};

const driving: TransportView = {
  playing: false,
  speed: 1,
  t_ms: 1233.4,
  frame: 37,
  frames: 120,
  period_ms: 4000,
  driven: [{ node: "spin", port: "frame", signal: "frame", loop: { frames: 120, period_ms: 4000 } }],
};

/** The React Flow node props the face reads (`data.view`, `selected`); the rest is what the canvas would pass. */
function propsFor(view: NodeView) {
  return {
    id: view.name,
    type: "cicada",
    data: { view },
    selected: false,
    isConnectable: true,
    zIndex: 0,
    positionAbsoluteX: 0,
    positionAbsoluteY: 0,
    dragging: false,
    draggable: true,
    selectable: true,
    deletable: true,
    width: 192,
    height: 96,
  } satisfies NodeProps<CanvasNode>;
}

function renderNode(view: NodeView = spin) {
  return render(
    <ReactFlowProvider>
      <CicadaNode {...propsFor(view)} />
    </ReactFlowProvider>,
  );
}

describe("transport-driven ports are hidden", () => {
  beforeEach(() => {
    useCicada.setState({
      connection: "open",
      role: "writer",
      catalog,
      graph: { nodes: [spin], wires: [], diagnostics: [] },
      selection: { nodes: ["spin"], wire: null, element: null },
      transport: { view: driving, receivedAt: 0 },
      statuses: {},
      nodeValues: {},
      hello: { clientId: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "07-orbit.cic", unitPx: 24 },
    });
    useCicada.getState().installSender(() => "");
  });
  afterEach(cleanup);

  it("on the canvas: no handle and no literal editor for `frame`; the transport in its row; the other ports untouched", () => {
    const { container } = renderNode();
    expect(container.querySelector('[data-port="spin.frame"]'), "no wire target").toBeNull();
    expect(screen.queryByTestId("lit-spin-frame"), "no literal editor — the hand-written 5 is not edited here").toBeNull();
    const row = screen.getByTestId("driven-spin-frame");
    expect(row.dataset.signal).toBe("frame");
    expect(row.dataset.driven).toBe("true");
    expect(row.textContent).toMatch(/frame.*transport/);
    expect(row.title).toMatch(/driven by the transport \(the loop frame\)/);
    expect(row.title, "the text's kwarg is named as the headless value").toMatch(/`frame=5` is the headless value/);
    expect(row.querySelector(".react-flow__handle"), "the row has no handle at all").toBeNull();
    // `period` and `frames` are ordinary: a handle each and an inline editor.
    expect(container.querySelector('[data-port="spin.period"]')).not.toBeNull();
    expect(container.querySelector('[data-port="spin.frames"]')).not.toBeNull();
    expect(screen.getByTestId("lit-spin-period")).toBeTruthy();
    expect(screen.getByTestId("lit-spin-frames")).toBeTruthy();
    // The server sized the node for three input rows; the driven row keeps the third, so nothing shifts.
    expect(container.querySelectorAll(".cn-row")).toHaveLength(3);
  });

  it("on the canvas the row is unlit when the transport is not driving this port (the node is red or the view is gone)", () => {
    useCicada.setState({ transport: { view: { ...driving, driven: [] }, receivedAt: 0 } });
    renderNode();
    const row = screen.getByTestId("driven-spin-frame");
    expect(row.dataset.driven).toBe("false");
    expect(row.title).toMatch(/not driving while this node is not solvable/);
    expect(screen.queryByTestId("lit-spin-frame")).toBeNull();
  });

  it("in the inspector: `frame` is absent from the inputs and listed under transport with the frame it is fed", () => {
    render(<Inspector />);
    const inspect = screen.getByTestId("node-inspect");
    expect(inspect.dataset.node).toBe("spin");
    expect(screen.queryByTestId("in-frame"), "not an input row").toBeNull();
    expect(screen.queryByTestId("insp-lit-spin-frame"), "no editor").toBeNull();
    expect(screen.getByTestId("in-period")).toBeTruthy();
    expect(screen.getByTestId("in-frames")).toBeTruthy();
    const section = screen.getByTestId("node-transport");
    expect(section.textContent).toMatch(/^transport/);
    const row = screen.getByTestId("driven-frame");
    expect(row.dataset.driven).toBe("true");
    expect(row.textContent).toMatch(/frame.*Integer.*← transport.*frame 37 of 120/);
    expect(row.textContent, "the hand-written kwarg is the headless value").toMatch(/headless\s*5/);
  });

  it("in the inspector a port the transport is not driving says so instead of showing a frame", () => {
    useCicada.setState({ transport: { view: { ...driving, driven: [] }, receivedAt: 0 } });
    render(<Inspector />);
    const row = screen.getByTestId("driven-frame");
    expect(row.dataset.driven).toBe("false");
    expect(row.textContent).toMatch(/← transport \(not driving\)/);
    expect(row.textContent).not.toMatch(/frame 37/);
  });

  // The catalog is fetched over HTTP beside the socket's snapshot, so the
  // first paint can come before it. The snapshot's own driven set names the
  // ports the transport is feeding, and both surfaces read it until the
  // catalog arrives: a driven port never flashes as an ordinary input —
  // a handle and a literal editor — for the catalog's latency (review
  // 2026-08-21).
  it("before the catalog arrives, a port in the snapshot's driven set is already hidden on the canvas; the catalog then takes over", () => {
    useCicada.setState({ catalog: null });
    const { container } = renderNode();
    expect(container.querySelector('[data-port="spin.frame"]'), "no wire target, catalog or not").toBeNull();
    expect(screen.queryByTestId("lit-spin-frame"), "no literal editor").toBeNull();
    expect(screen.getByTestId("driven-spin-frame").dataset.driven).toBe("true");
    // The ordinary ports are ordinary (handles; literal editors) without the catalog too.
    expect(container.querySelector('[data-port="spin.period"]')).not.toBeNull();
    expect(screen.getByTestId("lit-spin-frames")).toBeTruthy();
    // The catalog arrives: the same row, now from the flag.
    act(() => useCicada.getState().setCatalog(catalog));
    expect(container.querySelector('[data-port="spin.frame"]')).toBeNull();
    expect(screen.getByTestId("driven-spin-frame").dataset.signal).toBe("frame");
    // With the catalog here, the driven set no longer decides the port's
    // nature: a red `cycle` (out of the driven set) keeps its hidden port.
    act(() => useCicada.setState({ transport: { view: { ...driving, driven: [] }, receivedAt: 0 } }));
    expect(container.querySelector('[data-port="spin.frame"]')).toBeNull();
    expect(screen.getByTestId("driven-spin-frame").dataset.driven).toBe("false");
  });

  it("before the catalog arrives, the inspector lists a port in the snapshot's driven set under transport, not inputs", () => {
    useCicada.setState({ catalog: null });
    render(<Inspector />);
    expect(screen.queryByTestId("in-frame"), "not an input row").toBeNull();
    expect(screen.queryByTestId("insp-lit-spin-frame"), "no editor").toBeNull();
    expect(screen.getByTestId("in-period")).toBeTruthy();
    expect(screen.getByTestId("driven-frame").textContent).toMatch(/← transport.*frame 37 of 120/);
    act(() => useCicada.getState().setCatalog(catalog));
    expect(screen.queryByTestId("in-frame")).toBeNull();
    expect(screen.getByTestId("driven-frame").textContent).toMatch(/frame 37 of 120/);
  });
});

// Two cycles and a clock: `slow = cycle(period=8.0, frames=40)` is the
// primary loop (the longest period — the scrubber's and the view's
// `frame` / `frames`); `fast = cycle(period=2.0, frames=60)` loops inside
// it four times; `tick = clock()`. Each inspector row shows the value ITS
// port is fed — `fast` at the primary's frame 10 (2 s) is at frame 0 of
// 60, not "frame 10 of 40" (the review of the web half caught the
// inspector saying exactly that, 2026-08-20).
describe("each transport-driven port shows the value it is fed, on its own loop", () => {
  const slow = cycleNode("slow", 1, 8, 40);
  const fast = cycleNode("fast", 2, 2, 60);
  const tick: NodeView = {
    ref: 3,
    name: "tick",
    targets: ["tick"],
    line: 3,
    text: "tick = clock(speed=1.0)",
    kind: "call",
    func: "clock",
    title: "Clock",
    category: "Params & input",
    inputs: [
      { name: "speed", type: "Number", base: "Number", depth: 0, optional: false, required: false, default: "1.0", literal: "1.0", literal_value: 1, lift: 0 },
      { name: "t", type: "Number", base: "Number", depth: 0, optional: false, required: false, default: "0.0", lift: 0 },
    ],
    outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 12],
    size: [8, 3],
    manual: false,
  };
  /** Paused at the primary loop's frame 10 = 2 s in. */
  const atTwoSeconds: TransportView = {
    playing: false,
    speed: 1,
    t_ms: 2000,
    frame: 10,
    frames: 40,
    period_ms: 8000,
    driven: [
      { node: "slow", port: "frame", signal: "frame", loop: { frames: 40, period_ms: 8000 } },
      { node: "fast", port: "frame", signal: "frame", loop: { frames: 60, period_ms: 2000 } },
      { node: "tick", port: "t", signal: "time" },
    ],
  };

  beforeEach(() => {
    useCicada.setState({
      connection: "open",
      role: "writer",
      catalog,
      graph: { nodes: [slow, fast, tick], wires: [], diagnostics: [] },
      selection: { nodes: ["fast"], wire: null, element: null },
      transport: { view: atTwoSeconds, receivedAt: 0 },
      statuses: {},
      nodeValues: {},
      hello: { clientId: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "loops.cic", unitPx: 24 },
    });
    useCicada.getState().installSender(() => "");
  });
  afterEach(cleanup);

  it("the non-primary cycle's row is its own loop's frame 0 of 60, never the primary's frame 10 of 40", () => {
    render(<Inspector />);
    expect(screen.getByTestId("node-inspect").dataset.node).toBe("fast");
    const row = screen.getByTestId("driven-frame");
    expect(row.dataset.driven).toBe("true");
    expect(row.textContent).toMatch(/← transport.*frame 0 of 60/);
    expect(row.textContent).not.toMatch(/of 40/);
    expect(row.textContent, "nothing written by hand: no headless note").not.toMatch(/headless/);
  });

  it("the primary cycle's row is frame 10 of 40, and the clock's is the playhead in seconds", () => {
    act(() => useCicada.getState().selectNodes(["slow"]));
    const { unmount } = render(<Inspector />);
    expect(screen.getByTestId("driven-frame").textContent).toMatch(/frame 10 of 40/);
    unmount();
    act(() => useCicada.getState().selectNodes(["tick"]));
    render(<Inspector />);
    const t = screen.getByTestId("driven-t");
    expect(t.dataset.signal).toBe("time");
    expect(t.dataset.driven).toBe("true");
    expect(t.textContent).toMatch(/← transport.*2\.00 s/);
    expect(t.textContent).not.toMatch(/frame/);
  });

  it("a later view moves every row on its own loop (2.1 s: fast at frame 3 of 60, slow still at 10 of 40)", () => {
    render(<Inspector />);
    act(() => useCicada.setState({ transport: { view: { ...atTwoSeconds, t_ms: 2100 }, receivedAt: 0 } }));
    expect(screen.getByTestId("driven-frame").textContent).toMatch(/frame 3 of 60/);
    act(() => useCicada.getState().selectNodes(["slow"]));
    expect(screen.getByTestId("driven-frame").textContent).toMatch(/frame 10 of 40/);
  });
});

// `spin = cycle(period=4.0, frames=120, frame=n)`: a wire a human wrote
// into the transport's port. The text carries it and `cicada run`
// evaluates it (the headless source); the app overrides it with the
// playhead. It must never be hidden: the canvas row keeps a target handle
// so React Flow draws the edge (an edge needs two handles — without one
// the wire vanished silently, review 2026-08-20), not connectable (the
// server refuses wires into the port anyway); the inspector names the
// source and offers to select it; nothing says "not wired".
describe("a hand-wired transport-driven port keeps its wire visible", () => {
  const n: NodeView = {
    ref: 1,
    name: "n",
    targets: ["n"],
    line: 1,
    text: "n = 7",
    kind: "literal",
    title: "7",
    category: "",
    inputs: [],
    outputs: [{ name: "out", type: "Integer", base: "Integer", displayable: false }],
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 0],
    size: [4, 2],
    manual: false,
  };
  const wiredSpin = cycleNode("spin", 2, 4, 120, { wired: { node: "n", port: "out" } });
  const view: TransportView = { ...driving, driven: [{ node: "spin", port: "frame", signal: "frame", loop: { frames: 120, period_ms: 4000 } }] };

  beforeEach(() => {
    useCicada.setState({
      connection: "open",
      role: "writer",
      catalog,
      graph: {
        nodes: [n, wiredSpin],
        wires: [{ id: "n.out->spin.frame", from: { node: "n", port: "out" }, to: { node: "spin", port: "frame" }, lift: 0, type: "Integer", depth: 0, red: false }],
        diagnostics: [],
      },
      selection: { nodes: ["spin"], wire: null, element: null },
      transport: { view, receivedAt: 0 },
      statuses: {},
      nodeValues: {},
      hello: { clientId: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "wired.cic", unitPx: 24 },
    });
    useCicada.getState().installSender(() => "");
  });
  afterEach(cleanup);

  it("on the canvas the row has a target handle for the edge to land on — not connectable — and names the source", () => {
    const { container } = renderNode(wiredSpin);
    const row = screen.getByTestId("driven-spin-frame");
    expect(row.dataset.wired).toBe("n.out");
    expect(row.dataset.driven).toBe("true");
    const handle = container.querySelector('[data-port="spin.frame"]');
    expect(handle, "a handle, so React Flow draws the text's wire").not.toBeNull();
    expect(handle!.classList.contains("react-flow__handle")).toBe(true);
    expect(handle!.classList.contains("target")).toBe(true);
    expect(handle!.classList.contains("connectable"), "never a drop target").toBe(false);
    expect(handle!.classList.contains("connectableend")).toBe(false);
    expect(row.title).toMatch(/driven by the transport \(the loop frame\)/);
    expect(row.title).toMatch(/The text wires `frame=n` — the headless source/);
    expect(row.title).not.toMatch(/not wired/);
    expect(screen.queryByTestId("lit-spin-frame"), "still no literal editor").toBeNull();
    // An unwired driven port still has no handle at all.
    cleanup();
    const plain = renderNode(spin);
    expect(plain.container.querySelector('[data-port="spin.frame"]')).toBeNull();
  });

  it("in the inspector the transport row names the headless source and selects it; it never says 'not wired'", () => {
    render(<Inspector />);
    const row = screen.getByTestId("driven-frame");
    expect(row.dataset.wired).toBe("n.out");
    expect(row.textContent).toMatch(/← transport.*frame 37 of 120/);
    expect(row.textContent).toMatch(/headless ←\s*n\.out/);
    expect(row.textContent).not.toMatch(/not wired/);
    expect(row.title).toMatch(/The text wires `frame=n`/);
    fireEvent.click(screen.getByTestId("driven-frame-wired").querySelector("button")!);
    expect(useCicada.getState().selection.nodes).toEqual(["n"]);
  });
});
