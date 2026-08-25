// @vitest-environment jsdom
/**
 * The collapsed slider on both surfaces (docs/16 §Canvas conventions, wave
 * 4 B4 — finding U11: "sliders collapse to a single-unit-tall node"): the
 * node face a server-collapsed slider renders — ONE row with the name, the
 * same slider widget and the output handle, no header, no input handles —
 * against the expanded face; and the inspector's collapse / expand action,
 * whose title mirrors the server's refusal for a wired bound while the
 * click still sends the intent (the server decides and refuses). Rendered
 * for real against a seeded store.
 */
import { ReactFlowProvider, type NodeProps } from "@xyflow/react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClientMessage, InputView, NodeView } from "../protocol/messages";
import { Inspector } from "../panels/Inspector";
import { useCicada } from "../state/store";
import { CicadaNode } from "./CicadaNode";
import type { CanvasNode } from "./flow";

function port(name: string, literal: string, extra: Partial<InputView> = {}): InputView {
  return {
    name,
    type: "Number",
    base: "Number",
    depth: 0,
    optional: false,
    required: name === "value",
    lift: 0,
    literal,
    literal_value: Number(literal),
    ...extra,
  };
}

function sliderView(name: string, extra: Partial<NodeView> = {}): NodeView {
  return {
    ref: 1,
    name,
    targets: [name],
    line: 1,
    text: `${name} = slider(value=2.0, min=0.5, max=5.0)`,
    kind: "call",
    func: "slider",
    title: "Number Slider",
    category: "Params & input",
    inputs: [port("value", "2.0"), port("min", "0.5"), port("max", "5.0"), port("step", "0.0")],
    outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
    param: { kind: "slider", port: "value", value: 2, min: 0.5, max: 5, step: 0 },
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 0],
    size: [8, 6],
    manual: false,
    ...extra,
  };
}

/** `size`, collapsed by the server: one unit tall. */
const collapsed = sliderView("size", { collapsed: true, size: [8, 1] });
/** `size` expanded (the sidecar says nothing). */
const expanded = sliderView("size");
/** `driven = slider(value=size, min=0.0, max=10.0)` — a wired value: no widget (no `param`), and the server refuses to collapse it (the row IS the track). */
const driven = sliderView("driven", {
  text: "driven = slider(value=size, min=0.0, max=10.0)",
  inputs: [
    { ...port("value", ""), literal: undefined, literal_value: undefined, wired: { node: "size", port: "out" } },
    port("min", "0.0"),
    port("max", "10.0"),
    port("step", "0.0"),
  ],
  param: undefined,
});
/** `bound = slider(value=1.0, min=0.0, max=size)` — a wired max: the server refuses to collapse it. */
const bound = sliderView("bound", {
  text: "bound = slider(value=1.0, min=0.0, max=size)",
  inputs: [
    port("value", "1.0"),
    port("min", "0.0"),
    { ...port("max", ""), literal: undefined, literal_value: undefined, wired: { node: "size", port: "out" } },
    port("step", "0.0"),
  ],
  param: { kind: "slider", port: "value", value: 1, min: 0, max: 10, step: 0 },
});

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

let sent: ClientMessage[];
function seed(role: "writer" | "observer", nodes: NodeView[], selected: string) {
  sent = [];
  useCicada.setState({
    connection: "open",
    role,
    catalog: null,
    graph: { nodes, wires: [], diagnostics: [] },
    selection: { nodes: [selected], wire: null, element: null },
    transport: null,
    statuses: {},
    nodeValues: {},
    notices: [],
    hello: { clientId: 1, role, protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unitPx: 24 },
  });
  useCicada.getState().installSender((message) => {
    sent.push(message);
    return "";
  });
}

describe("the collapsed slider's face", () => {
  beforeEach(() => seed("writer", [collapsed, expanded, bound], "size"));
  afterEach(cleanup);

  it("is one row — name, the slider widget, the output handle — one unit tall, no header, no input handles", () => {
    const { container } = renderNode(collapsed);
    const face = container.querySelector(".cn")!;
    expect(face.getAttribute("data-collapsed")).toBe("true");
    expect(face.classList.contains("cn-collapsed")).toBe(true);
    expect((face as HTMLElement).style.height).toBe("24px");
    expect(container.querySelectorAll(".cn-collapsed-row")).toHaveLength(1);
    expect(screen.getByTestId("collapsed-size").textContent).toBe("size");
    expect(container.querySelector(".cn-header")).toBeNull();
    expect(container.querySelectorAll(".cn-row")).toHaveLength(0);
    expect(container.querySelectorAll(".react-flow__handle.target")).toHaveLength(0);
    const source = container.querySelectorAll(".react-flow__handle.source");
    expect(source).toHaveLength(1);
    expect(source[0]!.getAttribute("data-handleid")).toBe("out");
    // The widget is the one slider widget: its range drags the value.
    const range = screen.getByTestId("slider-size") as HTMLInputElement;
    expect(range.value).toBe("2");
    expect(screen.getByTestId("slider-value-size").textContent).toBe("2.0");
    // No problem, no state badge (the value says the rest); the state rides the attribute.
    expect(screen.queryByTestId("state-size")).toBeNull();
    expect(face.getAttribute("data-state")).toBe("idle");
  });

  it("the expanded face is the full node: header, four port rows, the widget row", () => {
    const { container } = renderNode(expanded);
    const face = container.querySelector(".cn")!;
    expect(face.getAttribute("data-collapsed")).toBeNull();
    expect(container.querySelector(".cn-header")).not.toBeNull();
    expect(container.querySelectorAll(".cn-row")).toHaveLength(4);
    expect(container.querySelectorAll(".react-flow__handle.target")).toHaveLength(4);
    expect(screen.getByTestId("slider-size")).toBeTruthy();
  });

  it("a collapsed slider that is red wears its badge", () => {
    useCicada.setState({
      statuses: { size: { state: "red", generation: 1, message: "slider: value 9 is outside 0.5..=5" } },
    });
    const { container } = renderNode(collapsed);
    expect(container.querySelector(".cn")!.getAttribute("data-state")).toBe("red");
    expect(screen.getByTestId("state-size").className).toMatch(/state-red/);
  });

  it("an observer sees the row with the widget disabled", () => {
    seed("observer", [collapsed], "size");
    renderNode(collapsed);
    expect((screen.getByTestId("slider-size") as HTMLInputElement).disabled).toBe(true);
  });
});

describe("the inspector's collapse / expand action", () => {
  afterEach(cleanup);
  // The inspector asks for the selected node's values on mount (`inspect`,
  // a read); the writes are what these tests are about.
  const writes = () => sent.filter((message) => message.type !== "inspect");

  it("collapses an expanded slider with literal bounds: one set_collapsed {collapsed: true}", () => {
    seed("writer", [expanded, bound], "size");
    render(<Inspector />);
    const button = screen.getByTestId("action-collapse");
    expect(button.textContent).toBe("collapse");
    expect(button.getAttribute("data-blocked")).toBeNull();
    fireEvent.click(button);
    expect(writes()).toEqual([{ type: "set_collapsed", payload: { node: "size", collapsed: true } }]);
  });

  it("expands a collapsed one: set_collapsed {collapsed: false}", () => {
    seed("writer", [collapsed], "size");
    render(<Inspector />);
    const button = screen.getByTestId("action-collapse");
    expect(button.textContent).toBe("expand");
    fireEvent.click(button);
    expect(writes()).toEqual([{ type: "set_collapsed", payload: { node: "size", collapsed: false } }]);
  });

  it("mirrors the server's reason for a wired bound in its title, and still lets the server refuse", () => {
    seed("writer", [expanded, bound], "bound");
    render(<Inspector />);
    const button = screen.getByTestId("action-collapse");
    expect(button.getAttribute("data-blocked")).toBe("max is wired");
    expect(button.title).toMatch(/^max is wired — a slider collapses only while value, min, max and step are literals/);
    fireEvent.click(button);
    expect(writes()).toEqual([{ type: "set_collapsed", payload: { node: "bound", collapsed: true } }]);
  });

  it("offers the action for a slider whose value is wired — no widget, still a slider — with the mirrored reason", () => {
    seed("writer", [expanded, driven], "driven");
    render(<Inspector />);
    const button = screen.getByTestId("action-collapse");
    expect(button.textContent).toBe("collapse");
    expect(button.getAttribute("data-blocked")).toBe("value is wired");
    expect(button.title).toMatch(/^value is wired — a slider collapses only while value, min, max and step are literals/);
  });

  it("has no such action for a node that is not a slider, and is disabled for an observer", () => {
    const domain: NodeView = {
      ...sliderView("span"),
      func: "construct_domain",
      text: "span = construct_domain(start=0.0, end=size)",
      param: undefined,
    };
    seed("writer", [domain], "span");
    render(<Inspector />);
    expect(screen.queryByTestId("action-collapse")).toBeNull();
    cleanup();
    seed("observer", [expanded], "size");
    render(<Inspector />);
    expect((screen.getByTestId("action-collapse") as HTMLButtonElement).disabled).toBe(true);
  });
});
