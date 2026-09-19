// @vitest-environment jsdom
/**
 * Input values on the node face and in the inspector (docs/16 §Canvas
 * conventions LOD table; wave 5 N1 — finding U23: "nodes should show the
 * values of their inputs as well as their outputs"). The `inspect` answer
 * carries `inputs` beside `outputs`; rendered for real against a seeded
 * store: a WIRED input shows its source output's summary after the port
 * label, compacted like an output's; a literal input shows nothing but its
 * chip (the answer is `null` for it, and `null` is not `—` there); a wired
 * input whose source has no value yet reads `—`; the inspector's Node tab
 * lists the wired input's value in full.
 */
import { ReactFlowProvider, type NodeProps } from "@xyflow/react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { InputView, NodeView, ValueSummary } from "../protocol/messages";
import { Inspector } from "../panels/Inspector";
import { useCicada } from "../state/store";
import { CicadaNode } from "./CicadaNode";
import type { CanvasNode } from "./flow";

const number: ValueSummary = { kind: "Number", hash: "ab".repeat(32), samples: ["2.5"] };
const domain: ValueSummary = { kind: "Domain", hash: "cd".repeat(32), samples: ["0 … 2.5"] };
const points: ValueSummary = { kind: "Point", hash: "ef".repeat(32), count: 3, samples: ["(0.123456, 1, 2)"] };

function input(name: string, extra: Partial<InputView>): InputView {
  return { name, type: "Number", base: "Number", depth: 0, optional: false, required: true, lift: 0, ...extra };
}

const size: NodeView = {
  ref: 1,
  name: "size",
  targets: ["size"],
  line: 1,
  text: "size = slider(value=2.5, min=0.5, max=5.0)",
  kind: "call",
  func: "slider",
  title: "Number Slider",
  category: "Params & input",
  inputs: [input("value", { literal: "2.5", literal_value: 2.5 })],
  outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
  param: { kind: "slider", port: "value", value: 2.5, min: 0.5, max: 5, step: 0 },
  diagnostics: [],
  effectful: false,
  preview: false,
  cell: [0, 0],
  size: [9, 3],
  manual: false,
};

/** `span = construct_domain(start=0.0, end=size)`: a literal and a wire. */
const span: NodeView = {
  ...size,
  ref: 2,
  name: "span",
  targets: ["span"],
  line: 2,
  text: "span = construct_domain(start=0.0, end=size)",
  func: "construct_domain",
  title: "Construct Domain",
  category: "Maths",
  inputs: [
    input("start", { literal: "0.0", literal_value: 0 }),
    input("end", { wired: { node: "size", port: "out" } }),
  ],
  outputs: [{ name: "out", type: "Domain", base: "Domain", displayable: false }],
  param: undefined,
  cell: [0, 4],
};

/** `moved = move(geometry=pts, vector=dir)`: two wires, one source still without a value. */
const moved: NodeView = {
  ...span,
  ref: 3,
  name: "moved",
  targets: ["moved"],
  line: 3,
  text: "moved = move(geometry=pts, vector=dir)",
  func: "move",
  title: "Move",
  category: "Transform",
  inputs: [
    input("geometry", { type: "[Point]", base: "Point", depth: 1, wired: { node: "pts", port: "out" } }),
    input("vector", { type: "Vector", base: "Vector", wired: { node: "dir", port: "out" } }),
  ],
  outputs: [{ name: "out", type: "[Point]", base: "Point", displayable: true }],
};

/** `dbl = size * 2.0`: an expression whose free variable is the wire. */
const dbl: NodeView = {
  ...span,
  ref: 4,
  name: "dbl",
  targets: ["dbl"],
  line: 4,
  text: "dbl = size * 2.0",
  kind: "expression",
  func: undefined,
  title: "Expression",
  category: "Maths & logic",
  description: "size * 2.0",
  inputs: [input("size", { wired: { node: "size", port: "out" }, doc: "" })],
  outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
};

function propsFor(v: NodeView) {
  return {
    id: v.name,
    type: "cicada",
    data: { view: v },
    selected: false,
    isConnectable: true,
    zIndex: 0,
    positionAbsoluteX: 0,
    positionAbsoluteY: 0,
    dragging: false,
    draggable: true,
    selectable: true,
    deletable: true,
    width: v.size[0] * 24,
    height: v.size[1] * 24,
  } satisfies NodeProps<CanvasNode>;
}

function renderNode(v: NodeView) {
  return render(
    <ReactFlowProvider>
      <CicadaNode {...propsFor(v)} />
    </ReactFlowProvider>,
  );
}

function seed(selected: string) {
  useCicada.setState({
    connection: "open",
    role: "writer",
    catalog: null,
    graph: { nodes: [size, span, moved, dbl], wires: [], diagnostics: [] },
    selection: { nodes: [selected], wire: null, element: null },
    transport: null,
    statuses: { size: { state: "done", generation: 4 }, span: { state: "done", generation: 4 } },
    nodeValues: {
      size: { generation: 4, outputs: [["out", number]], inputs: [["value", null]] },
      span: { generation: 4, outputs: [["out", domain]], inputs: [["start", null], ["end", number]] },
      moved: { generation: 4, outputs: [["out", null]], inputs: [["geometry", points], ["vector", null]] },
      dbl: { generation: 4, outputs: [["out", { ...number, samples: ["5"] }]], inputs: [["size", number]] },
    },
    notices: [],
    hello: { clientId: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unitPx: 24 },
  });
  useCicada.getState().installSender(() => "");
}

describe("input values on the node face (near tier — the provider's default zoom is 1)", () => {
  beforeEach(() => seed("span"));
  afterEach(cleanup);

  it("a wired input shows its source output's summary after the label; a literal shows its chip alone", () => {
    const { container } = renderNode(span);
    // `end` ← size.out: the value, compacted, in the port-value style.
    const end = screen.getByTestId("in-value-span-end");
    expect(end.textContent).toBe("2.5");
    expect(end.classList.contains("cn-port-value")).toBe(true);
    const endRow = end.closest(".cn-port.cn-in")!;
    expect(endRow.classList.contains("with-value")).toBe(true);
    expect(endRow.getAttribute("title")).toContain("← 2.5");
    // `start = 0.0`: the chip is the value; no wire value, no `—`.
    expect(screen.queryByTestId("in-value-span-start")).toBeNull();
    expect(screen.getByTestId("lit-span-start").textContent).toBe("0.0");
    const startRow = screen.getByTestId("lit-span-start").closest(".cn-port.cn-in")!;
    expect(startRow.classList.contains("with-value")).toBe(false);
    // The output keeps its own summary beside.
    expect(container.querySelector(".cn-port.cn-out .cn-port-value")!.textContent).toBe("0 … 2.5");
  });

  it("compacts the numbers to four significant figures and reads `—` while the source has no value", () => {
    renderNode(moved);
    expect(screen.getByTestId("in-value-moved-geometry").textContent).toBe("Point ×3 · (0.1235, 1, 2)");
    expect(screen.getByTestId("in-value-moved-vector").textContent).toBe("—");
  });

  it("an expression's free variable shows the value too, and its hover keeps both the rule and the value (review L5-3)", () => {
    renderNode(dbl);
    const shown = screen.getByTestId("in-value-dbl-size");
    expect(shown.textContent).toBe("2.5");
    const title = shown.closest(".cn-port.cn-in")!.getAttribute("title")!;
    expect(title).toContain("size: free variable of the expression — edit the text to change it");
    expect(title).toContain("\n← 2.5");
  });

  it("shows no input value until the answer for the node is in", () => {
    useCicada.setState({ nodeValues: {} });
    const { container } = renderNode(span);
    expect(screen.queryByTestId("in-value-span-end")).toBeNull();
    expect(container.querySelectorAll(".cn-port.cn-in.with-value")).toHaveLength(0);
  });
});

describe("input values in the inspector's Node tab", () => {
  beforeEach(() => seed("span"));
  afterEach(cleanup);

  it("lists the wired input's value in full under its row, and nothing under a literal's", () => {
    render(<Inspector />);
    const end = screen.getByTestId("in-end");
    const box = end.querySelector("[data-testid='value-box']");
    expect(box).not.toBeNull();
    expect(box!.getAttribute("data-kind")).toBe("Number");
    expect(box!.textContent).toContain("2.5");
    // The source stays selectable beside the value.
    expect(end.textContent).toContain("size.out");
    expect(screen.getByTestId("in-start").querySelector("[data-testid='value-box']")).toBeNull();
  });

  it("marks a previous generation's input value stale like an output's", () => {
    useCicada.setState({ summary: { ...useCicada.getState().summary, generation: 9 } });
    render(<Inspector />);
    const box = screen.getByTestId("in-end").querySelector("[data-testid='value-box']")!;
    expect(box.classList.contains("stale")).toBe(true);
  });
});
