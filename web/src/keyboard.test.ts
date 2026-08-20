import { beforeEach, describe, expect, it } from "vitest";
import { handleHotkey, hotkeysReach, isControlTarget, isEditableTarget } from "./keyboard";
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

  it("Delete removes ONE selected node as itself, several as one batch (one undo step)", () => {
    useCicada.getState().selectNodes(["a"]);
    expect(handleHotkey(key("Delete"))).toBe(true);
    expect(sent).toEqual([{ type: "delete_node", payload: { node: "a" } }]);
    sent = [];
    useCicada.getState().selectNodes(["a", "b"]);
    expect(handleHotkey(key("Delete"))).toBe(true);
    expect(sent).toEqual([
      {
        type: "batch",
        payload: {
          label: "delete 2 nodes",
          ops: [
            { type: "delete_node", payload: { node: "a" } },
            { type: "delete_node", payload: { node: "b" } },
          ],
        },
      },
    ]);
  });

  it("Delete disconnects a selected wire", () => {
    useCicada.getState().selectWire("a.out->b.x");
    expect(handleHotkey(key("Delete"))).toBe(true);
    expect(sent).toEqual([{ type: "disconnect", payload: { to: { node: "b", port: "x" } } }]);
  });

  it("Backspace does NOT delete — nodes or wires — and is not consumed (docs/16: Del only, GH parity)", () => {
    useCicada.getState().selectNodes(["a", "b"]);
    expect(handleHotkey(key("Backspace"))).toBe(false);
    useCicada.getState().selectWire("a.out->b.x");
    expect(handleHotkey(key("Backspace"))).toBe(false);
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices).toEqual([]);
  });

  it("does not consume Delete with nothing selected", () => {
    expect(handleHotkey(key("Delete"))).toBe(false);
    expect(sent).toEqual([]);
  });

  it("Ctrl+Z undoes; Ctrl+Shift+Z and Ctrl+Y redo; Cmd works as Ctrl", () => {
    expect(handleHotkey(key("z", { ctrlKey: true }))).toBe(true);
    expect(handleHotkey(key("Z", { ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(handleHotkey(key("y", { ctrlKey: true }))).toBe(true);
    expect(handleHotkey(key("z", { metaKey: true }))).toBe(true);
    expect(sent).toEqual([
      { type: "undo", payload: {} },
      { type: "redo", payload: {} },
      { type: "redo", payload: {} },
      { type: "undo", payload: {} },
    ]);
    expect(useCicada.getState().notices).toEqual([]);
  });

  it("undo/redo are sent even when the mirror's history is empty — the server's refusal says why", () => {
    useCicada.setState({
      history: { can_undo: false, can_redo: false, undo_label: null, redo_label: null, depth: 0 },
    });
    expect(handleHotkey(key("z", { ctrlKey: true }))).toBe(true);
    expect(handleHotkey(key("y", { ctrlKey: true }))).toBe(true);
    expect(sent).toEqual([
      { type: "undo", payload: {} },
      { type: "redo", payload: {} },
    ]);
  });

  it("a key-REPEAT past an empty side sends nothing (consumed, no notice flood); a repeat with history behind it still undoes", () => {
    useCicada.setState({
      history: { can_undo: false, can_redo: true, undo_label: null, redo_label: "move a", depth: 0 },
    });
    expect(handleHotkey(key("z", { ctrlKey: true, repeat: true }))).toBe(true);
    expect(sent).toEqual([]);
    expect(handleHotkey(key("y", { ctrlKey: true, repeat: true }))).toBe(true);
    expect(handleHotkey(key("Z", { ctrlKey: true, shiftKey: true, repeat: true }))).toBe(true);
    expect(sent).toEqual([
      { type: "redo", payload: {} },
      { type: "redo", payload: {} },
    ]);
    useCicada.setState({
      history: { can_undo: true, can_redo: false, undo_label: "move a", redo_label: null, depth: 1 },
    });
    sent = [];
    expect(handleHotkey(key("z", { ctrlKey: true, repeat: true }))).toBe(true);
    expect(handleHotkey(key("y", { ctrlKey: true, repeat: true }))).toBe(true);
    expect(sent).toEqual([{ type: "undo", payload: {} }]);
    expect(useCicada.getState().notices).toEqual([]);
  });

  it("undo/redo are writes: observers and dropped sockets get the notice, no intent", () => {
    useCicada.setState({ role: "observer" });
    expect(handleHotkey(key("z", { ctrlKey: true }))).toBe(true);
    expect(handleHotkey(key("y", { ctrlKey: true }))).toBe(true);
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.map((n) => n.message)).toEqual([
      "read-only observer — take the lease to undo",
      "read-only observer — take the lease to redo",
    ]);
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

  it("P on several displayable nodes is one batch", () => {
    useCicada.setState({
      graph: { ...useCicada.getState().graph, nodes: [fakeNode("a"), fakeNode("c", { preview: true })] },
    });
    useCicada.getState().selectNodes(["a", "c"]);
    expect(handleHotkey(key("P"))).toBe(true);
    expect(sent).toEqual([
      {
        type: "batch",
        payload: {
          label: "preview 2 nodes",
          ops: [
            { type: "set_preview", payload: { node: "a", on: true } },
            { type: "set_preview", payload: { node: "c", on: false } },
          ],
        },
      },
    ]);
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

  it("an arrow nudge of a multi-selection is one batch of move_node", () => {
    useCicada.getState().selectNodes(["a", "b"]);
    expect(handleHotkey(key("ArrowDown"))).toBe(true);
    expect(sent).toEqual([
      {
        type: "batch",
        payload: {
          label: "move 2 nodes",
          ops: [
            { type: "move_node", payload: { node: "a", cell: [3, 5] } },
            { type: "move_node", payload: { node: "b", cell: [3, 5] } },
          ],
        },
      },
    ]);
  });

  it("D toggles #off on ONE selected node as itself, on several as one batch labelled by the direction", () => {
    expect(handleHotkey(key("d"))).toBe(true);
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.at(-1)?.message).toMatch(/select a node to disable/);
    useCicada.getState().selectNodes(["a"]);
    expect(handleHotkey(key("d"))).toBe(true);
    expect(sent).toEqual([{ type: "toggle_disable", payload: { node: "a" } }]);
    sent = [];
    // Two live nodes → `disable 2 nodes`; one Ctrl+Z flips both back.
    useCicada.getState().selectNodes(["a", "b"]);
    expect(handleHotkey(key("D"))).toBe(true);
    expect(sent).toEqual([
      {
        type: "batch",
        payload: {
          label: "disable 2 nodes",
          ops: [
            { type: "toggle_disable", payload: { node: "a" } },
            { type: "toggle_disable", payload: { node: "b" } },
          ],
        },
      },
    ]);
    sent = [];
    // All ghosts → `enable N nodes`; a mix → `toggle N nodes`; a broken line
    // is skipped with a notice (there is nothing to prefix).
    useCicada.setState({
      graph: {
        ...useCicada.getState().graph,
        nodes: [
          fakeNode("a", { kind: "disabled" }),
          fakeNode("b", { kind: "disabled" }),
          fakeNode("c"),
          fakeNode("x", { kind: "broken", func: undefined, inputs: [], outputs: [] }),
        ],
      },
    });
    useCicada.getState().selectNodes(["a", "b"]);
    expect(handleHotkey(key("d"))).toBe(true);
    expect((sent.at(-1) as { payload: { label: string } }).payload.label).toBe("enable 2 nodes");
    useCicada.getState().selectNodes(["a", "c", "x"]);
    expect(handleHotkey(key("d"))).toBe(true);
    expect(sent.at(-1)).toEqual({
      type: "batch",
      payload: {
        label: "toggle 2 nodes",
        ops: [
          { type: "toggle_disable", payload: { node: "a" } },
          { type: "toggle_disable", payload: { node: "c" } },
        ],
      },
    });
    expect(useCicada.getState().notices.at(-1)?.message).toMatch(/`x` does not parse/);
    // A write: observers get the notice, no intent.
    sent = [];
    useCicada.setState({ role: "observer" });
    useCicada.getState().selectNodes(["c"]);
    expect(handleHotkey(key("d"))).toBe(true);
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.at(-1)?.message).toMatch(/read-only observer — take the lease to toggle disable/);
  });

  it("deferred features answer with a notice and consume the key", () => {
    for (const [k, mods] of [
      ["g", { ctrlKey: true }],
      ["s", { ctrlKey: true }],
      [" ", {}],
    ] as [string, Partial<KeyboardEvent>][]) {
      expect(handleHotkey(key(k, mods))).toBe(true);
    }
    expect(sent).toEqual([]);
    expect(useCicada.getState().notices.length).toBe(3);
  });

  it("leaves unknown keys alone", () => {
    expect(handleHotkey(key("q"))).toBe(false);
    expect(handleHotkey(key("x", { ctrlKey: true }))).toBe(false);
  });
});

describe("isEditableTarget / isControlTarget / hotkeysReach", () => {
  const el = (tagName: string, extra: Record<string, unknown> = {}) =>
    ({ tagName, isContentEditable: false, closest: () => null, ...extra }) as unknown as EventTarget;
  it("is false for non-elements and plain elements", () => {
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(el("DIV"))).toBe(false);
    expect(isControlTarget(null)).toBe(false);
    expect(isControlTarget(el("DIV"))).toBe(false);
  });
  it("is true for text-entry inputs, textareas, selects, contenteditable, and data-no-hotkeys subtrees", () => {
    expect(isEditableTarget(el("INPUT"))).toBe(true); // no `type` = text
    expect(isEditableTarget(el("INPUT", { type: "text" }))).toBe(true);
    expect(isEditableTarget(el("INPUT", { type: "number" }))).toBe(true);
    expect(isEditableTarget(el("INPUT", { type: "search" }))).toBe(true);
    expect(isEditableTarget(el("TEXTAREA"))).toBe(true);
    expect(isEditableTarget(el("SELECT"))).toBe(true);
    expect(isEditableTarget(el("DIV", { isContentEditable: true }))).toBe(true);
    expect(isEditableTarget(el("DIV", { closest: () => ({}) }))).toBe(true);
  });
  it("a range slider / checkbox / button is a CONTROL, not text entry (the slider kept focus after a drag and swallowed Ctrl+Z)", () => {
    for (const type of ["range", "checkbox", "radio", "button", "submit", "color"]) {
      expect(isEditableTarget(el("INPUT", { type })), type).toBe(false);
      expect(isControlTarget(el("INPUT", { type })), type).toBe(true);
    }
    expect(isControlTarget(el("BUTTON"))).toBe(true);
    expect(isEditableTarget(el("BUTTON"))).toBe(false);
    expect(isControlTarget(el("INPUT", { type: "text" }))).toBe(false);
  });
  it("hotkeysReach: everything from the canvas, nothing from text entry, only Ctrl/Cmd chords from a control", () => {
    const ev = (target: EventTarget | null, mods: { ctrlKey?: boolean; metaKey?: boolean } = {}) => ({
      target,
      ctrlKey: false,
      metaKey: false,
      ...mods,
    });
    expect(hotkeysReach(ev(el("DIV")))).toBe(true);
    expect(hotkeysReach(ev(null))).toBe(true);
    expect(hotkeysReach(ev(el("INPUT", { type: "text" }), { ctrlKey: true }))).toBe(false);
    expect(hotkeysReach(ev(el("TEXTAREA"), { metaKey: true }))).toBe(false);
    const slider = el("INPUT", { type: "range" });
    expect(hotkeysReach(ev(slider)), "arrows on a focused slider stay the slider's").toBe(false);
    expect(hotkeysReach(ev(slider, { ctrlKey: true })), "Ctrl+Z from a focused slider undoes").toBe(true);
    expect(hotkeysReach(ev(slider, { metaKey: true }))).toBe(true);
  });
});
