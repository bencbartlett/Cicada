/**
 * The keyboard map (docs/16 §Keyboard map). One window `keydown` listener;
 * events from inputs / textareas / contenteditable / `data-no-hotkeys`
 * subtrees are ignored so typing never triggers gestures. Every handled
 * key is a gesture-level intent through the store's `send`; nothing here
 * mutates authoritative state.
 */
import { useEffect } from "react";
import { asOneOp, type GestureMessage } from "./protocol/messages";
import { canWrite, nodeByName, useCicada, writeBlockReason } from "./state/store";
import { viewportApi } from "./viewport/api";

const NOT_YET = {
  disable: "disable arrives with #off support (v0.1)",
  group: "groups arrive later",
  transport: "transport arrives with time params",
  commit: "commit dialog arrives with the git panel — every op is already saved",
} as const;

/** True when the event originates in a text-entry surface (hotkeys stay off). */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (target === null || typeof target !== "object" || !("tagName" in target)) return false;
  const el = target as HTMLElement;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (el.isContentEditable) return true;
  return typeof el.closest === "function" && el.closest("[data-no-hotkeys]") !== null;
}

/**
 * Handle one keydown against the current store. Returns true when the key
 * was consumed (the caller `preventDefault`s only then). Exported for tests.
 */
export function handleHotkey(event: KeyboardEvent): boolean {
  const state = useCicada.getState();
  const ctrl = event.ctrlKey || event.metaKey;
  const key = event.key;
  const writer = canWrite(state);
  const notice = state.addNotice;
  const selected = state.selection.nodes;

  // Every write hotkey: the lease AND an open socket (`canWrite`).
  const needsLease = (what: string): boolean => {
    if (writer) return false;
    const hint = state.connection === "open" ? "take the lease" : "wait for the connection";
    notice("warning", `${writeBlockReason(state) ?? "cannot write"} — ${hint} to ${what}`);
    return true;
  };

  if (key === "Escape") {
    if (state.summary.running) {
      if (needsLease("cancel the solve")) return true;
      state.send({ type: "cancel", payload: {} });
      return true;
    }
    if (state.search !== null) state.closeSearch();
    state.clearSelection();
    return true;
  }

  // `Del` only — Backspace does NOT delete (docs/16 keyboard map: GH
  // parity, 2026-08-19); it falls through unconsumed.
  if (key === "Delete") {
    if (state.selection.wire !== null) {
      const wire = state.graph.wires.find((w) => w.id === state.selection.wire);
      if (wire === undefined) {
        notice("error", `wire ${state.selection.wire} is no longer in the graph`);
        return true;
      }
      if (needsLease("disconnect wires")) return true;
      state.send({ type: "disconnect", payload: { to: wire.to } });
      return true;
    }
    if (selected.length === 0) return false;
    if (needsLease("delete nodes")) return true;
    // A multi-delete is ONE op (a `batch`), so one Ctrl+Z brings them all back.
    const ops: GestureMessage[] = selected.map((node) => ({ type: "delete_node", payload: { node } }));
    state.send(asOneOp(ops, `delete ${selected.length} nodes`));
    return true;
  }

  if (ctrl && !event.shiftKey && (key === "f" || key === "F")) {
    state.openSearch({
      x: window.innerWidth * 0.4,
      y: window.innerHeight * 0.35,
      cell: null,
      from: null,
    });
    return true;
  }

  if (ctrl && (key === "a" || key === "A")) {
    state.selectNodes(state.graph.nodes.map((n) => n.name));
    return true;
  }

  // Undo / redo (docs/13 op log): Ctrl+Z · Ctrl+Shift+Z / Ctrl+Y. Sent
  // even when the mirror says the side is empty — the server's refusal
  // carries the reason (no edits yet / all undone / the reload barrier).
  if (ctrl && (key === "z" || key === "Z")) {
    const redo = event.shiftKey;
    if (needsLease(redo ? "redo" : "undo")) return true;
    state.send({ type: redo ? "redo" : "undo", payload: {} });
    return true;
  }
  if (ctrl && (key === "y" || key === "Y")) {
    if (needsLease("redo")) return true;
    state.send({ type: "redo", payload: {} });
    return true;
  }
  if (ctrl && (key === "g" || key === "G")) {
    notice("info", NOT_YET.group);
    return true;
  }
  if (ctrl && (key === "s" || key === "S")) {
    notice("info", NOT_YET.commit);
    return true;
  }
  if (ctrl) return false;

  if (key === "f" || key === "F") {
    viewportApi.frameSelection();
    return true;
  }
  if (key === "Home") {
    viewportApi.frameAll();
    return true;
  }

  if (key === "p" || key === "P") {
    if (selected.length === 0) {
      notice("info", "select a node to toggle its preview");
      return true;
    }
    if (needsLease("toggle previews")) return true;
    const ops: GestureMessage[] = [];
    for (const name of selected) {
      const node = nodeByName(state.graph, name);
      if (node === undefined) continue;
      if (!node.outputs.some((o) => o.displayable)) {
        notice("info", `\`${name}\` has no displayable output`);
        continue;
      }
      ops.push({ type: "set_preview", payload: { node: name, on: !node.preview } });
    }
    if (ops.length > 0) state.send(asOneOp(ops, `preview ${ops.length} nodes`));
    return true;
  }

  if (key === "d" || key === "D") {
    notice("info", NOT_YET.disable);
    return true;
  }
  // Space reaches here only from `useKeyboard`'s keyup path (see below):
  // React Flow's Space+drag pan owns the keydown.
  if (key === " " || event.code === "Space") {
    notice("info", NOT_YET.transport);
    return true;
  }

  if (key === "ArrowLeft" || key === "ArrowRight" || key === "ArrowUp" || key === "ArrowDown") {
    if (selected.length === 0) return false;
    if (needsLease("move nodes")) return true;
    const dx = key === "ArrowLeft" ? -1 : key === "ArrowRight" ? 1 : 0;
    const dy = key === "ArrowUp" ? -1 : key === "ArrowDown" ? 1 : 0;
    const ops: GestureMessage[] = [];
    for (const name of selected) {
      const node = nodeByName(state.graph, name);
      if (node === undefined) continue;
      ops.push({
        type: "move_node",
        payload: { node: name, cell: [node.cell[0] + dx, node.cell[1] + dy] },
      });
    }
    if (ops.length > 0) state.send(asOneOp(ops, `move ${ops.length} nodes`));
    return true;
  }

  return false;
}

/** Is this the Space key (by key or by code, whichever the event carries)? */
function isSpace(event: KeyboardEvent): boolean {
  return event.key === " " || event.code === "Space";
}

/**
 * Install the window key listeners (once, from `App`).
 *
 * Space is special: React Flow's `panActivationKeyCode` ("Space") listens on
 * the window and `preventDefault`s every Space keydown for the Space+drag
 * pan, so the keydown path never sees it un-prevented. A plain tap — Space
 * down and up with no pointer press in between — is the transport hotkey
 * and is answered on keyup; a Space+drag pan stays React Flow's.
 */
export function useKeyboard(): void {
  useEffect(() => {
    let spaceHeld = false;
    let spaceUsedForPan = false;
    const onKeyDown = (event: KeyboardEvent) => {
      if (isEditableTarget(event.target)) return;
      if (event.isComposing) return;
      if (isSpace(event)) {
        if (!event.repeat) {
          spaceHeld = true;
          spaceUsedForPan = false;
        }
        return;
      }
      if (event.defaultPrevented) return;
      if (handleHotkey(event)) event.preventDefault();
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (!isSpace(event)) return;
      const tapped = spaceHeld && !spaceUsedForPan;
      spaceHeld = false;
      spaceUsedForPan = false;
      if (!tapped || isEditableTarget(event.target)) return;
      if (handleHotkey(event)) event.preventDefault();
    };
    const onPointerDown = () => {
      if (spaceHeld) spaceUsedForPan = true;
    };
    const onBlur = () => {
      spaceHeld = false;
      spaceUsedForPan = false;
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("blur", onBlur);
    };
  }, []);
}
