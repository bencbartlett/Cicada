// @vitest-environment jsdom
/**
 * The hidden-port rule (docs/13 §Animation transport; docs/17 item 4): a
 * transport-driven input (`cycle.frame`, `clock.t` — the catalog's
 * `transport_driven`) is the session's, not the user's. On the canvas node
 * it has NO handle (nothing to wire into, nothing to drop on) and no
 * literal editor — its row shows the transport instead, lit while the port
 * is in the current graph's driven set; in the inspector it is absent from
 * the inputs list and shown under `transport` with the value the transport
 * feeds it. The node's other ports keep their handles and editors. Both
 * surfaces are rendered for real against a seeded store: the catalog says
 * which port, the transport view says whether it is driving.
 */
import { ReactFlowProvider, type NodeProps } from "@xyflow/react";
import { cleanup, render, screen } from "@testing-library/react";
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
  ],
};

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
  driven: [{ node: "spin", port: "frame", signal: "frame" }],
};

/** The React Flow node props the face reads (`data.view`, `selected`); the rest is what the canvas would pass. */
const nodeProps = {
  id: "spin",
  type: "cicada",
  data: { view: spin },
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

function renderNode() {
  return render(
    <ReactFlowProvider>
      <CicadaNode {...nodeProps} />
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
});
