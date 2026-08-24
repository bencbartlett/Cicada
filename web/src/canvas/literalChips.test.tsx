// @vitest-environment jsdom
/**
 * The typed-literal chip on both surfaces (docs/16 §Canvas conventions +
 * §Inspector contents; docs/17 wave 4 B3, finding U9 — "no way to type
 * `construct_domain`'s `end = 40.0` on a placed node"). Rendered for real
 * against a seeded store: a placed `construct_domain()` with two required,
 * untyped Number ports; a `shift_list()` with a wired list, an untyped
 * Integer and a Boolean the catalog defaults; a `text_outlines` with a
 * Text literal, a Number literal and a Text default; a slider whose own
 * `value` port belongs to its widget. Double-click (canvas) / click
 * (inspector) opens the editor; Enter and blur commit ONE `set_param`
 * spelled by the literal rule; Esc cancels; an unchanged value writes
 * nothing; an unspellable one is a notice, not a write; wired ports, list
 * ports, observers and `#off` ghosts get no chip.
 */
import { ReactFlowProvider, type NodeProps } from "@xyflow/react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClientMessage, InputView, NodeView } from "../protocol/messages";
import { useCicada } from "../state/store";
import { Inspector } from "../panels/Inspector";
import { CicadaNode } from "./CicadaNode";
import type { CanvasNode } from "./flow";

function port(name: string, base: string, extra: Partial<InputView> = {}): InputView {
  return { name, type: base, base, depth: 0, optional: false, required: true, lift: 0, ...extra };
}

function view(name: string, func: string, text: string, inputs: InputView[], extra: Partial<NodeView> = {}): NodeView {
  return {
    ref: 1,
    name,
    targets: [name],
    line: 1,
    text,
    kind: "call",
    func,
    title: func,
    category: "Maths & logic",
    inputs,
    outputs: [{ name: "out", type: "Domain", base: "Domain", displayable: false }],
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 0],
    size: [8, 3],
    manual: false,
    ...extra,
  };
}

/** `construct_domain_1 = construct_domain()` — just placed: both required ports untyped. */
const domain = view("construct_domain_1", "construct_domain", "construct_domain_1 = construct_domain()", [
  port("start", "Number"),
  port("end", "Number"),
]);

/** `shift_list_1 = shift_list(list=xs)` — a wire, an untyped Integer, a Boolean the catalog defaults to `true`. */
const shifted = view("shift_list_1", "shift_list", "shift_list_1 = shift_list(list=xs)", [
  port("list", "E", { type: "[E]", depth: 1, wired: { node: "xs", port: "out" } }),
  port("offset", "Integer"),
  port("wrap", "Boolean", { required: false, default: "true", default_value: true }),
]);

/** `t = text_outlines(text="hi", size=2.0)` — literals present, a Text default, a Plane default, an Integer default. */
const outlines = view("t", "text_outlines", 't = text_outlines(text="hi", size=2.0)', [
  port("text", "Text", { literal: '"hi"', literal_value: "hi" }),
  port("size", "Number", { literal: "2.0", literal_value: 2 }),
  port("plane", "Plane", { required: false, default: "xy_plane" }),
  port("font", "Text", { required: false, default: '"DejaVu Sans Bold"', default_value: "DejaVu Sans Bold" }),
  port("segments", "Integer", { required: false, default: "8", default_value: 8 }),
]);

/** `size = slider(value=2.0, min=0.5, max=5.0)` — `value` is the widget's; `step` is untyped. */
const slider = view(
  "size",
  "slider",
  "size = slider(value=2.0, min=0.5, max=5.0)",
  [
    port("value", "Number", { literal: "2.0", literal_value: 2 }),
    port("min", "Number", { literal: "0.5", literal_value: 0.5 }),
    port("max", "Number", { literal: "5.0", literal_value: 5 }),
    port("step", "Number", { required: false, default: "0.0", default_value: 0 }),
  ],
  { param: { kind: "slider", port: "value", value: 2, min: 0.5, max: 5, step: 0 } },
);

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
    width: 192,
    height: 72,
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
function seed(role: "writer" | "observer", selected: NodeView) {
  sent = [];
  useCicada.setState({
    connection: "open",
    role,
    catalog: null,
    graph: { nodes: [domain, shifted, outlines, slider], wires: [], diagnostics: [] },
    selection: { nodes: [selected.name], wire: null, element: null },
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

const setParam = (node: string, portName: string, value: string): ClientMessage => ({
  type: "set_param",
  payload: { node, port: portName, value },
});

describe("the typed-literal chip on the canvas node row", () => {
  beforeEach(() => seed("writer", domain));
  afterEach(cleanup);

  it("a placed node's required ports wear empty slots; double-click → type 40 → Enter is one set_param spelled `40.0`", () => {
    renderNode(domain);
    const end = screen.getByTestId("lit-construct_domain_1-end");
    expect(end.dataset.state).toBe("unset");
    expect(end.textContent).toBe("…");
    expect(end.title).toMatch(/required, nothing typed yet — double-click to type a value/);
    expect(screen.getByTestId("lit-construct_domain_1-start").dataset.state).toBe("unset");
    // A single click selects the node; it does not open the editor.
    fireEvent.click(end);
    expect(screen.queryByTestId("lit-construct_domain_1-end-input")).toBeNull();
    fireEvent.doubleClick(end);
    const input = screen.getByTestId("lit-construct_domain_1-end-input") as HTMLInputElement;
    expect(document.activeElement).toBe(input);
    expect(input.value).toBe("");
    expect(screen.queryByTestId("lit-construct_domain_1-end"), "the chip gave way to the editor").toBeNull();
    fireEvent.change(input, { target: { value: "40" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([setParam("construct_domain_1", "end", "40.0")]);
    expect(screen.queryByTestId("lit-construct_domain_1-end-input")).toBeNull();
    expect(screen.getByTestId("lit-construct_domain_1-end"), "the chip is back (the delta will fill it)").toBeTruthy();
  });

  it("Esc cancels: nothing is sent and the chip returns; an empty Enter writes nothing either", () => {
    renderNode(domain);
    fireEvent.doubleClick(screen.getByTestId("lit-construct_domain_1-start"));
    const input = screen.getByTestId("lit-construct_domain_1-start-input");
    fireEvent.change(input, { target: { value: "1" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(sent).toEqual([]);
    expect(screen.queryByTestId("lit-construct_domain_1-start-input")).toBeNull();
    expect(screen.getByTestId("lit-construct_domain_1-start").dataset.state).toBe("unset");
    fireEvent.doubleClick(screen.getByTestId("lit-construct_domain_1-start"));
    fireEvent.keyDown(screen.getByTestId("lit-construct_domain_1-start-input"), { key: "Enter" });
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices).toEqual([]);
  });

  it("leaving the field commits what was typed, once", () => {
    renderNode(domain);
    fireEvent.doubleClick(screen.getByTestId("lit-construct_domain_1-start"));
    const input = screen.getByTestId("lit-construct_domain_1-start-input");
    fireEvent.change(input, { target: { value: "0" } });
    fireEvent.blur(input);
    expect(sent).toEqual([setParam("construct_domain_1", "start", "0.0")]);
  });

  it("keys inside the editor stay there — except Ctrl+S, the commit dialog's chord", () => {
    renderNode(domain);
    fireEvent.doubleClick(screen.getByTestId("lit-construct_domain_1-end"));
    const input = screen.getByTestId("lit-construct_domain_1-end-input");
    const reached: string[] = [];
    const listener = (event: KeyboardEvent) => reached.push(event.key);
    window.addEventListener("keydown", listener);
    fireEvent.keyDown(input, { key: "d" });
    fireEvent.keyDown(input, { key: "s", ctrlKey: true });
    window.removeEventListener("keydown", listener);
    expect(reached).toEqual(["s"]);
  });

  it("an Integer port spells bare integers and refuses a fraction with a notice, never a write", () => {
    seed("writer", shifted);
    renderNode(shifted);
    expect(screen.getByTestId("lit-shift_list_1-offset").dataset.state).toBe("unset");
    fireEvent.doubleClick(screen.getByTestId("lit-shift_list_1-offset"));
    let input = screen.getByTestId("lit-shift_list_1-offset-input");
    fireEvent.change(input, { target: { value: "2.5" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.map((n) => [n.level, n.message])).toEqual([
      ["warning", 'shift_list_1.offset: "2.5" is not an integer — nothing written'],
    ]);
    fireEvent.doubleClick(screen.getByTestId("lit-shift_list_1-offset"));
    input = screen.getByTestId("lit-shift_list_1-offset-input");
    fireEvent.change(input, { target: { value: "3" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([setParam("shift_list_1", "offset", "3")]);
  });

  it("a Boolean the catalog defaults: the chip says `True` greyed; the checkbox starts checked; toggling and Enter writes `False`; an untouched Enter writes nothing", () => {
    seed("writer", shifted);
    renderNode(shifted);
    const wrap = screen.getByTestId("lit-shift_list_1-wrap");
    expect(wrap.dataset.state).toBe("default");
    expect(wrap.textContent, "the dialect's spelling, not the macro's `true`").toBe("True");
    expect(wrap.className).toMatch(/\bdefault\b/);
    fireEvent.doubleClick(wrap);
    let box = screen.getByTestId("lit-shift_list_1-wrap-input") as HTMLInputElement;
    expect(box.type).toBe("checkbox");
    expect(box.checked).toBe(true);
    fireEvent.keyDown(box, { key: "Enter" });
    expect(sent, "the default it already showed is no edit").toEqual([]);
    fireEvent.doubleClick(screen.getByTestId("lit-shift_list_1-wrap"));
    box = screen.getByTestId("lit-shift_list_1-wrap-input") as HTMLInputElement;
    fireEvent.click(box);
    expect(box.checked).toBe(false);
    fireEvent.keyDown(box, { key: "Enter" });
    expect(sent).toEqual([setParam("shift_list_1", "wrap", "False")]);
  });

  it("a wired port has no chip and no text; a list port has none either", () => {
    seed("writer", shifted);
    const { container } = renderNode(shifted);
    expect(screen.queryByTestId("lit-shift_list_1-list")).toBeNull();
    expect(container.querySelectorAll(".cn-literal")).toHaveLength(0);
  });

  it("Text ports: a literal is shown quoted and edited bare; a default starts the editor on its value; a Plane default gets no chip", () => {
    seed("writer", outlines);
    renderNode(outlines);
    const text = screen.getByTestId("lit-t-text");
    expect(text.dataset.state).toBe("literal");
    expect(text.textContent).toBe('"hi"');
    fireEvent.doubleClick(text);
    const input = screen.getByTestId("lit-t-text-input") as HTMLInputElement;
    expect(input.type).toBe("text");
    expect(input.value).toBe("hi");
    fireEvent.change(input, { target: { value: "hello world" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([setParam("t", "text", '"hello world"')]);

    const font = screen.getByTestId("lit-t-font");
    expect(font.dataset.state).toBe("default");
    expect(font.textContent).toBe('"DejaVu Sans Bold"');
    fireEvent.doubleClick(font);
    const fontInput = screen.getByTestId("lit-t-font-input") as HTMLInputElement;
    expect(fontInput.value).toBe("DejaVu Sans Bold");
    fireEvent.change(fontInput, { target: { value: "Foo" } });
    fireEvent.blur(fontInput);
    expect(sent.at(-1)).toEqual(setParam("t", "font", '"Foo"'));

    expect(screen.queryByTestId("lit-t-plane"), "a Plane takes no literal").toBeNull();
    expect(screen.getByTestId("lit-t-segments").textContent).toBe("8");
  });

  it("a Number literal: Enter on the unchanged text writes nothing; a change is spelled with the point", () => {
    seed("writer", outlines);
    renderNode(outlines);
    const size = screen.getByTestId("lit-t-size");
    expect(size.textContent).toBe("2.0");
    fireEvent.doubleClick(size);
    let input = screen.getByTestId("lit-t-size-input") as HTMLInputElement;
    expect(input.value).toBe("2.0");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([]);
    fireEvent.doubleClick(screen.getByTestId("lit-t-size"));
    input = screen.getByTestId("lit-t-size-input") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "3" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sent).toEqual([setParam("t", "size", "3.0")]);
  });

  it("the slider's own `value` port belongs to its widget; its bounds are chips", () => {
    seed("writer", slider);
    renderNode(slider);
    expect(screen.queryByTestId("lit-size-value")).toBeNull();
    expect(screen.getByTestId("lit-size-min").textContent).toBe("0.5");
    expect(screen.getByTestId("lit-size-step").dataset.state).toBe("default");
  });

  it("an observer sees the text, no chip: the literal as written, the default greyed, nothing for an untyped required port", () => {
    seed("observer", outlines);
    const { container } = renderNode(outlines);
    expect(container.querySelector("[data-testid^='lit-']")).toBeNull();
    const shown = Array.from(container.querySelectorAll(".cn-literal")).map((el) => [el.textContent, el.className]);
    expect(shown).toEqual([
      ['"hi"', "cn-literal mono"],
      ["2.0", "cn-literal mono"],
      ['"DejaVu Sans Bold"', "cn-literal mono faint"],
      ["8", "cn-literal mono faint"],
    ]);
    cleanup();
    renderNode(domain);
    expect(container.querySelectorAll(".cn-literal")).toHaveLength(0);
  });

  it("a `#off` ghost keeps its ports but takes no edits: text, no chip", () => {
    const off = { ...outlines, kind: "disabled" as const, text: `#off ${outlines.text}` };
    const { container } = renderNode(off);
    expect(container.querySelector("[data-testid^='lit-']")).toBeNull();
    expect(container.querySelectorAll(".cn-literal")).toHaveLength(4);
  });
});

describe("the typed-literal chip in the inspector's Node tab", () => {
  beforeEach(() => seed("writer", domain));
  afterEach(cleanup);
  // The inspector asks for the node's values (`inspect`, a read) on mount;
  // the writes are what these tests are about.
  const writes = () => sent.filter((m) => m.type !== "inspect");

  it("a click opens the editor; Enter commits the spelled set_param; Esc cancels", () => {
    render(<Inspector />);
    expect(screen.getByTestId("node-inspect").dataset.node).toBe("construct_domain_1");
    const end = screen.getByTestId("insp-lit-construct_domain_1-end");
    expect(end.dataset.state).toBe("unset");
    expect(end.title).toMatch(/click to type a value/);
    fireEvent.click(end);
    const input = screen.getByTestId("insp-lit-construct_domain_1-end-input") as HTMLInputElement;
    expect(document.activeElement).toBe(input);
    fireEvent.change(input, { target: { value: "40" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(writes()).toEqual([setParam("construct_domain_1", "end", "40.0")]);
    fireEvent.click(screen.getByTestId("insp-lit-construct_domain_1-start"));
    const start = screen.getByTestId("insp-lit-construct_domain_1-start-input");
    fireEvent.change(start, { target: { value: "9" } });
    fireEvent.keyDown(start, { key: "Escape" });
    expect(writes()).toHaveLength(1);
    expect(screen.getByTestId("insp-lit-construct_domain_1-start").dataset.state).toBe("unset");
  });

  it("a default and a wire: the Boolean chip says `True`; the wired port shows its source, no chip", () => {
    seed("writer", shifted);
    render(<Inspector />);
    expect(screen.getByTestId("insp-lit-shift_list_1-wrap").textContent).toBe("True");
    expect(screen.queryByTestId("insp-lit-shift_list_1-list")).toBeNull();
    expect(screen.getByTestId("in-list").textContent).toMatch(/xs\.out/);
    fireEvent.click(screen.getByTestId("insp-lit-shift_list_1-wrap"));
    const box = screen.getByTestId("insp-lit-shift_list_1-wrap-input") as HTMLInputElement;
    fireEvent.click(box);
    fireEvent.blur(box);
    expect(writes()).toEqual([setParam("shift_list_1", "wrap", "False")]);
  });

  it("an observer reads the rows: the literal, `default True` spelled as the dialect, `unset`", () => {
    seed("observer", shifted);
    render(<Inspector />);
    expect(screen.queryByTestId("insp-lit-shift_list_1-wrap")).toBeNull();
    expect(screen.getByTestId("in-wrap").textContent).toMatch(/default\s*True/);
    expect(screen.getByTestId("in-offset").textContent).toMatch(/unset/);
    cleanup();
    seed("observer", outlines);
    render(<Inspector />);
    expect(screen.getByTestId("in-text").textContent).toMatch(/"hi"/);
    expect(screen.getByTestId("in-plane").textContent, "a non-literal default keeps its rendering").toMatch(
      /default\s*xy_plane/,
    );
  });
});
