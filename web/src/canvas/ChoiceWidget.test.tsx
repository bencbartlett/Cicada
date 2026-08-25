// @vitest-environment jsdom
/**
 * The `choice` param's dropdown (catalog C2b, docs/16 §Canvas conventions)
 * on both surfaces — the on-canvas widget row and the params panel —
 * against a seeded store: the options in the text's order with the
 * current value selected; picking one commits ONE `set_param` spelling the
 * option as a Text literal (the one literal rule); picking the current
 * value writes nothing; a stray value (the text's value is not among the
 * options — the node is red) stays selectable, marked "(not an option)",
 * never silently replaced; wired options leave a text field; a non-writer
 * sees the select disabled.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClientMessage, GraphView, NodeView } from "../protocol/messages";
import { ParamsPanel } from "../panels/ParamsPanel";
import { useCicada } from "../state/store";
import { ParamWidget } from "./ParamWidget";

const mode = (value: string, options: string[] | undefined): NodeView => ({
  ref: 5,
  name: "mode",
  targets: ["mode"],
  line: 2,
  text: `mode = choice(value="${value}", options=["fast", "exact", "draft"])`,
  kind: "call",
  func: "choice",
  title: "Value List",
  category: "Params & input",
  inputs: [],
  outputs: [{ name: "out", type: "Text", base: "Text", displayable: false }],
  diagnostics: [],
  effectful: false,
  preview: false,
  cell: [0, 0],
  size: [8, 3],
  manual: false,
  param: { kind: "choice", port: "value", value, options },
});

const OPTIONS = ["fast", "exact", "draft"];

describe("the choice param's dropdown", () => {
  let sent: ClientMessage[];
  beforeEach(() => {
    sent = [];
    useCicada.setState({ connection: "open", role: "writer", pending: null, notices: [] });
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return "";
    });
  });
  afterEach(cleanup);

  it("on the canvas: the options in order, the value selected, one set_param per pick spelled as a Text literal", () => {
    const view = mode("exact", OPTIONS);
    render(<ParamWidget view={view} param={view.param!} writer />);
    const select = screen.getByTestId("choice-mode") as HTMLSelectElement;
    expect(select.disabled).toBe(false);
    expect([...select.options].map((o) => o.value)).toEqual(OPTIONS);
    expect(select.value).toBe("exact");
    expect(select.dataset.stray).toBeUndefined();
    fireEvent.change(select, { target: { value: "draft" } });
    expect(sent).toEqual([{ type: "set_param", payload: { node: "mode", port: "value", value: '"draft"' } }]);
    // Re-picking the committed value writes nothing.
    fireEvent.change(select, { target: { value: "exact" } });
    expect(sent).toHaveLength(1);
  });

  it("a stray value stays selectable and marked — the red node shows what the text says", () => {
    const view = mode("slow", OPTIONS);
    render(<ParamWidget view={view} param={view.param!} writer />);
    const select = screen.getByTestId("choice-mode") as HTMLSelectElement;
    expect(select.value).toBe("slow");
    expect(select.dataset.stray).toBe("true");
    expect([...select.options].map((o) => o.textContent)).toEqual(["slow (not an option)", ...OPTIONS]);
    expect(select.parentElement?.className).toMatch(/\bstray\b/);
    expect(select.parentElement?.title).toBe('"slow" is not one of the options');
    fireEvent.change(select, { target: { value: "fast" } });
    expect(sent).toEqual([{ type: "set_param", payload: { node: "mode", port: "value", value: '"fast"' } }]);
  });

  it("wired options: a text field, not a select", () => {
    const view = mode("fast", undefined);
    render(<ParamWidget view={view} param={view.param!} writer />);
    const field = screen.getByTestId("choice-mode") as HTMLInputElement;
    expect(field.tagName).toBe("INPUT");
    expect(field.value).toBe("fast");
    expect(field.parentElement?.title).toMatch(/options are wired/);
  });

  it("a non-writer sees the select disabled and nothing goes out", () => {
    const view = mode("fast", OPTIONS);
    render(<ParamWidget view={view} param={view.param!} writer={false} />);
    const select = screen.getByTestId("choice-mode") as HTMLSelectElement;
    expect(select.disabled).toBe(true);
    expect(select.value).toBe("fast");
    expect(sent).toEqual([]);
  });

  it("in the params panel: the same select, the same one set_param, the stray marked", () => {
    const view = mode("exact", OPTIONS);
    const graph: GraphView = { nodes: [view], wires: [], diagnostics: [] };
    act(() => useCicada.setState({ graph }));
    render(<ParamsPanel />);
    expect(screen.getByTestId("param-mode")).toBeTruthy();
    const select = screen.getByTestId("widget-mode") as HTMLSelectElement;
    expect([...select.options].map((o) => o.value)).toEqual(OPTIONS);
    expect(select.value).toBe("exact");
    fireEvent.change(select, { target: { value: "fast" } });
    expect(sent).toEqual([{ type: "set_param", payload: { node: "mode", port: "value", value: '"fast"' } }]);
    cleanup();
    sent.length = 0;
    const stray = mode("slow", OPTIONS);
    act(() => useCicada.setState({ graph: { nodes: [stray], wires: [], diagnostics: [] } }));
    render(<ParamsPanel />);
    const marked = screen.getByTestId("widget-mode") as HTMLSelectElement;
    expect(marked.value).toBe("slow");
    expect(marked.className).toMatch(/\bstray\b/);
    expect(marked.options.item(0)?.textContent).toBe("slow (not an option)");
    // An observer: disabled.
    cleanup();
    act(() => useCicada.setState({ role: "observer", graph }));
    render(<ParamsPanel />);
    expect((screen.getByTestId("widget-mode") as HTMLSelectElement).disabled).toBe(true);
  });
});
