import { beforeEach, describe, expect, it } from "vitest";
import { handleHotkey, isEditableTarget } from "./keyboard";
import type { ClientMessage, NodeView } from "./protocol/messages";
import { useCicada } from "./state/store";

function key(k: string, mods: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return { key: k, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...mods } as KeyboardEvent;
}

function fakeNode(name: string, extra: Partial<NodeView> = {}): NodeView {
  return {
    ref: 1,
    name,
    targets: [name],
    line: 1,
    text: `${name} = box()`,
    kind: "call",
    func: "box",
    title: "Box",
    category: "Surface & solid",
    inputs: [],
    outputs: [{ name: "out", type: "Mesh", base: "Mesh", displayable: true }],
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [3, 4],
    size: [8, 3],
    manual: false,
    ...extra,
  };
}

describe("handleHotkey", () => {
  let sent: ClientMessage[];
  beforeEach(() => {
    sent = [];
    // Global window shims for the node test environment.
    (globalThis as { window?: unknown }).window = { innerWidth: 1000, innerHeight: 800 };
    useCicada.setState({
      role: "writer",
      connection: "open",
      selection: { nodes: [], wire: null, element: null },
      notices: [],
      search: null,
      summary: { ...useCicada.getState().summary, running: false },
      graph: {
        nodes: [fakeNode("a"), fakeNode("b", { outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }] })],
        wires: [{ id: "a.out->b.x", from: { node: "a", port: "out" }, to: { node: "b", port: "x" }, lift: 0, depth: 0, red: false }],
        diagnostics: [],
      },
    });
    useCicada.getState().installSender((m) => {
      sent.push(m);
      return "id";
    });
  });

  it("Esc cancels a running solve, else clears selection + search", () => {
    useCicada.setState({ summary: { ...useCicada.getState().summary, running: true } });
    expect(handleHotkey(key("Escape"))).toBe(true);
    expect(sent).toEqual([{ type: "cancel", payload: {} }]);
    useCicada.setState({ summary: { ...useCicada.getState().summary, running: false } });
    useCicada.getState().selectNodes(["a"]);
    useCicada.getState().openSearch({ x: 1, y: 2, cell: null, from: null });
    expect(handleHotkey(key("Escape"))).toBe(true);
    expect(useCicada.getState().selection.nodes).toEqual([]);
    expect(useCicada.getState().search).toBeNull();
  });

  it("Delete removes selected nodes, disconnects a selected wire", () => {
    useCicada.getState().selectNodes(["a", "b"]);
    expect(handleHotkey(key("Delete"))).toBe(true);
    expect(sent).toEqual([
      { type: "delete_node", payload: { node: "a" } },
      { type: "delete_node", payload: { node: "b" } },
    ]);
    sent = [];
    useCicada.getState().selectWire("a.out->b.x");
    expect(handleHotkey(key("Backspace"))).toBe(true);
    expect(sent).toEqual([{ type: "disconnect", payload: { to: { node: "b", port: "x" } } }]);
  });

  it("does not consume Delete with nothing selected", () => {
    expect(handleHotkey(key("Delete"))).toBe(false);
    expect(sent).toEqual([]);
  });

  it("observers get a notice instead of a write intent", () => {
    useCicada.setState({ role: "observer" });
    useCicada.getState().selectNodes(["a"]);
    expect(handleHotkey(key("Delete"))).toBe(true);
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.at(-1)?.message).toMatch(/read-only observer/);
  });

  it("a writer on a dropped socket cannot write either (canWrite = lease AND open)", () => {
    useCicada.setState({ role: "writer", connection: "reconnecting" });
    useCicada.getState().selectNodes(["a"]);
    expect(handleHotkey(key("Delete"))).toBe(true);
    expect(handleHotkey(key("ArrowRight"))).toBe(true);
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.at(-1)?.message).toMatch(/not connected/);
  });

  it("Ctrl+F opens search at 40%/35% of the window; Ctrl+A selects all", () => {
    expect(handleHotkey(key("f", { ctrlKey: true }))).toBe(true);
    expect(useCicada.getState().search).toEqual({ x: 400, y: 280, cell: null, from: null });
    expect(handleHotkey(key("a", { ctrlKey: true }))).toBe(true);
    expect(useCicada.getState().selection.nodes).toEqual(["a", "b"]);
  });

  it("P toggles preview only on displayable outputs", () => {
    useCicada.getState().selectNodes(["a", "b"]);
    expect(handleHotkey(key("p"))).toBe(true);
    expect(sent).toEqual([{ type: "set_preview", payload: { node: "a", on: true } }]);
    expect(useCicada.getState().notices.at(-1)?.message).toMatch(/no displayable output/);
  });

  it("arrows nudge by one grid cell from the node's current cell", () => {
    useCicada.getState().selectNodes(["a"]);
    expect(handleHotkey(key("ArrowRight"))).toBe(true);
    expect(handleHotkey(key("ArrowUp"))).toBe(true);
    expect(sent).toEqual([
      { type: "move_node", payload: { node: "a", cell: [4, 4] } },
      { type: "move_node", payload: { node: "a", cell: [3, 3] } },
    ]);
  });

  it("deferred features answer with a notice and consume the key", () => {
    for (const [k, mods] of [
      ["z", { ctrlKey: true }],
      ["z", { ctrlKey: true, shiftKey: true }],
      ["g", { ctrlKey: true }],
      ["s", { ctrlKey: true }],
      ["d", {}],
      [" ", {}],
    ] as [string, Partial<KeyboardEvent>][]) {
      expect(handleHotkey(key(k, mods))).toBe(true);
    }
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.length).toBe(6);
  });

  it("leaves unknown keys alone", () => {
    expect(handleHotkey(key("q"))).toBe(false);
    expect(handleHotkey(key("x", { ctrlKey: true }))).toBe(false);
  });
});

describe("isEditableTarget", () => {
  const el = (tagName: string, extra: Record<string, unknown> = {}) =>
    ({ tagName, isContentEditable: false, closest: () => null, ...extra }) as unknown as EventTarget;
  it("is false for non-elements and plain elements", () => {
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(el("DIV"))).toBe(false);
  });
  it("is true for inputs, contenteditable, and data-no-hotkeys subtrees", () => {
    expect(isEditableTarget(el("INPUT"))).toBe(true);
    expect(isEditableTarget(el("TEXTAREA"))).toBe(true);
    expect(isEditableTarget(el("DIV", { isContentEditable: true }))).toBe(true);
    expect(isEditableTarget(el("DIV", { closest: () => ({}) }))).toBe(true);
  });
});
