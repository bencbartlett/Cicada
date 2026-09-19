// @vitest-environment jsdom
/**
 * The slider's face, wave 5 N1 (docs/16 §Canvas conventions §Sliders;
 * finding U17 — "the collapse toggle should be part of the node; a
 * collapsed slider's value should be an editable text field"), rendered for
 * real against a seeded store:
 *
 *   - the collapse CHEVRON on the expanded face (centred on the bottom edge,
 *     absolutely positioned — an extra element, never a layout row) and on
 *     the collapsed row before the output handle; one click = ONE
 *     `set_collapsed`; a wired bound greys it with the mirror's reason
 *     (`data-blocked`) while the click still goes to the server; observers
 *     and `#off` ghosts see none; a node that is no slider wears none;
 *   - the collapsed row's VALUE: double-click opens the typed-literal
 *     editor in its place, Enter is ONE `set_param` spelled by the one
 *     literal rule, Esc cancels, an unspellable value is a notice and no
 *     write, the committed value typed back writes nothing; the expanded
 *     face keeps the plain label.
 */
import { ReactFlowProvider, type NodeProps } from "@xyflow/react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClientMessage, InputView, NodeView } from "../protocol/messages";
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
    size: [9, 6],
    manual: false,
    ...extra,
  };
}

const collapsed = sliderView("size", { collapsed: true, size: [9, 1] });
const expanded = sliderView("size");
/** A wired `max`: the server refuses to collapse it; the mirror says why beforehand. */
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
/** A `#off` slider: the ghost keeps its ports, takes no in-place edits. */
const off = sliderView("size", { kind: "disabled", text: "#off size = slider(value=2.0, min=0.5, max=5.0)" });
const domain: NodeView = {
  ...sliderView("span"),
  func: "construct_domain",
  title: "Construct Domain",
  text: "span = construct_domain(start=0.0, end=size)",
  param: undefined,
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

let sent: ClientMessage[];
function seed(role: "writer" | "observer", nodes: NodeView[]) {
  sent = [];
  useCicada.setState({
    connection: "open",
    role,
    catalog: null,
    graph: { nodes, wires: [], diagnostics: [] },
    selection: { nodes: [], wire: null, element: null },
    transport: null,
    statuses: {},
    nodeValues: {},
    notices: [],
    pending: null,
    hello: { clientId: 1, role, protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unitPx: 24 },
  });
  useCicada.getState().installSender((message) => {
    sent.push(message);
    return "";
  });
}

describe("the collapse chevron on the face", () => {
  beforeEach(() => seed("writer", [expanded, bound]));
  afterEach(cleanup);

  it("sits on the expanded slider's bottom edge, over the border — no layout row — and collapses with one op", () => {
    const { container } = renderNode(expanded);
    const chevron = screen.getByTestId("chevron-size");
    expect(chevron.classList.contains("expanded")).toBe(true);
    expect(chevron.classList.contains("nodrag")).toBe(true);
    expect(chevron.getAttribute("data-blocked")).toBeNull();
    expect(chevron.getAttribute("aria-label")).toBe("collapse size");
    expect(chevron.querySelector("svg")!.getAttribute("data-icon")).toBe("chevron-up");
    // The face's rows are the server's: four port rows and the widget, the chevron beside them, not among them.
    expect(container.querySelectorAll(".cn-row")).toHaveLength(4);
    expect(chevron.parentElement!.classList.contains("cn")).toBe(true);
    expect(chevron.closest(".cn-row")).toBeNull();
    fireEvent.click(chevron);
    expect(sent).toEqual([{ type: "set_collapsed", payload: { node: "size", collapsed: true } }]);
  });

  it("sits at the collapsed row's right, before the output handle, and expands with one op", () => {
    const { container } = renderNode(collapsed);
    const chevron = screen.getByTestId("chevron-size");
    expect(chevron.classList.contains("collapsed")).toBe(true);
    expect(chevron.getAttribute("aria-label")).toBe("expand size");
    expect(chevron.querySelector("svg")!.getAttribute("data-icon")).toBe("chevron-down");
    const row = container.querySelector(".cn-collapsed-row")!;
    const order = Array.from(row.children).map((el) => el.className.split(" ")[0]);
    // name · the widget (its range and value) · the tail (badges + chevron) · the handle
    expect(order.indexOf("cn-collapsed-name")).toBeLessThan(order.indexOf("cn-widget"));
    expect(order.indexOf("cn-widget")).toBeLessThan(order.indexOf("cn-collapsed-tail"));
    expect(order.indexOf("cn-collapsed-tail")).toBeLessThan(order.indexOf("react-flow__handle"));
    expect(row.querySelector(".cn-collapsed-tail .cn-chevron")).toBe(chevron);
    fireEvent.click(chevron);
    expect(sent).toEqual([{ type: "set_collapsed", payload: { node: "size", collapsed: false } }]);
  });

  it("is greyed with the server's reason for a wired bound — and the click still lets the server refuse", () => {
    renderNode(bound);
    const chevron = screen.getByTestId("chevron-bound");
    expect(chevron.classList.contains("blocked")).toBe(true);
    expect(chevron.getAttribute("data-blocked")).toBe("max is wired");
    expect(chevron.title).toMatch(/^max is wired — a slider collapses only while value, min, max and step are literals/);
    fireEvent.click(chevron);
    expect(sent).toEqual([{ type: "set_collapsed", payload: { node: "bound", collapsed: true } }]);
  });

  it("shows none to an observer, none on a #off ghost, none on a node that is no slider", () => {
    seed("observer", [expanded, collapsed]);
    renderNode(expanded);
    expect(screen.queryByTestId("chevron-size")).toBeNull();
    cleanup();
    const observed = renderNode(collapsed);
    expect(screen.queryByTestId("chevron-size")).toBeNull();
    // … and the row says so to the CSS: the track's floor subtracts the
    // chevron only where there is one (review findings L1-2 / C-9).
    expect(observed.container.querySelector(".cn-collapsed-row")!.classList.contains("has-chevron")).toBe(false);
    cleanup();
    seed("writer", [collapsed]);
    const written = renderNode(collapsed);
    expect(written.container.querySelector(".cn-collapsed-row")!.classList.contains("has-chevron")).toBe(true);
    cleanup();
    seed("writer", [off, domain]);
    renderNode(off);
    expect(screen.queryByTestId("chevron-size")).toBeNull();
    cleanup();
    renderNode(domain);
    expect(screen.queryByTestId("chevron-span")).toBeNull();
    expect(sent).toEqual([]);
  });
});

describe("the collapsed row's editable value", () => {
  beforeEach(() => seed("writer", [collapsed]));
  afterEach(cleanup);

  const open = () => {
    const label = screen.getByTestId("slider-value-size");
    expect(label.getAttribute("data-editable")).toBe("true");
    fireEvent.doubleClick(label);
    const input = screen.getByTestId("slider-value-size-input") as HTMLInputElement;
    expect(input.value, "the editor starts from the committed value as the label spells it").toBe("2.0");
    expect(input.type, "a plain text field, so every keystroke reaches the spelling rule").toBe("text");
    return input;
  };

  it("double-click opens the chip editor in the label's place; Enter is ONE set_param through the literal rule", () => {
    renderNode(collapsed);
    const input = open();
    expect(screen.queryByTestId("slider-value-size")).toBeNull();
    fireEvent.change(input, { target: { value: "3.5" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([{ type: "set_param", payload: { node: "size", port: "value", value: "3.5" } }]);
    // The label is back (the delta will bring the value).
    expect(screen.getByTestId("slider-value-size").textContent).toBe("2.0");
    expect(screen.queryByTestId("slider-value-size-input")).toBeNull();
  });

  it("spells a whole number as the slider does (`4` → `4.0`) and writes nothing for the committed value typed back", () => {
    renderNode(collapsed);
    let input = open();
    fireEvent.change(input, { target: { value: "4" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([{ type: "set_param", payload: { node: "size", port: "value", value: "4.0" } }]);
    sent.length = 0;
    input = open();
    fireEvent.change(input, { target: { value: "2" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent, "2 over 2.0 is no edit").toEqual([]);
  });

  it("Esc cancels — nothing written, nothing previewed", () => {
    renderNode(collapsed);
    const input = open();
    fireEvent.change(input, { target: { value: "9.0" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(sent).toEqual([]);
    expect(screen.getByTestId("slider-value-size").textContent).toBe("2.0");
  });

  it("an unspellable value is a warning notice and no write", () => {
    renderNode(collapsed);
    const input = open();
    fireEvent.change(input, { target: { value: "1/2" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([]);
    const notices = useCicada.getState().notices;
    expect(notices).toHaveLength(1);
    expect(notices[0]!.level).toBe("warning");
    expect(notices[0]!.message).toBe('size.value: "1/2" is not a valid number — nothing written');
  });

  it("the expanded face's label is not an editor, and an observer's collapsed value is not either", () => {
    seed("writer", [expanded]);
    renderNode(expanded);
    const label = screen.getByTestId("slider-value-size");
    expect(label.getAttribute("data-editable")).toBeNull();
    fireEvent.doubleClick(label);
    expect(screen.queryByTestId("slider-value-size-input")).toBeNull();
    cleanup();
    seed("observer", [collapsed]);
    renderNode(collapsed);
    fireEvent.doubleClick(screen.getByTestId("slider-value-size"));
    expect(screen.queryByTestId("slider-value-size-input")).toBeNull();
    expect(sent).toEqual([]);
  });
});
