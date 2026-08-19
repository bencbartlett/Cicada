/**
 * React Flow ⇄ view-model plumbing: the node/edge types the canvas uses,
 * the graph → React Flow builders (pure, unit-tested), and the write guard
 * (`sendWrite`) every write gesture goes through — observers get a notice,
 * never a silent no-op (docs/13 §lease, brief rule 2).
 */
import type { Edge, Node } from "@xyflow/react";
import type { ClientMessage, GraphView, NodeView, WireView } from "../protocol/messages";
import { canWrite, useCicada, writeBlockReason } from "../state/store";
import { cellToPx } from "./grid";

export interface CanvasNodeData extends Record<string, unknown> {
  view: NodeView;
}
export type CanvasNode = Node<CanvasNodeData, "cicada">;

export interface CanvasEdgeData extends Record<string, unknown> {
  wire: WireView;
}
export type CanvasEdge = Edge<CanvasEdgeData, "cicada">;

/**
 * Send a write intent if this client may write right now (`canWrite`: open
 * socket + the lease); otherwise a warning notice and `false`. Read intents
 * never go through here.
 */
export function sendWrite(message: ClientMessage): boolean {
  const state = useCicada.getState();
  if (!canWrite(state)) {
    const why = writeBlockReason(state) ?? "cannot write";
    const hint = state.connection === "open" ? "take the lease to edit" : "waiting for the connection";
    state.addNotice("warning", `${why} — ${message.type.replace(/_/g, " ")} ignored (${hint})`);
    return false;
  }
  state.send(message);
  return true;
}

/**
 * Graph → React Flow nodes. Positions come from the server's cells; a node
 * currently being dragged keeps its optimistic position until the drag ends
 * (the next delta overrides). Selection flags mirror the store.
 */
export function buildNodes(
  graph: GraphView,
  unit: number,
  previous: readonly CanvasNode[],
  selected: readonly string[],
): CanvasNode[] {
  const prev = new Map(previous.map((n) => [n.id, n]));
  const sel = new Set(selected);
  return graph.nodes.map((view) => {
    const before = prev.get(view.name);
    const position =
      before?.dragging === true ? before.position : cellToPx(view.cell, unit);
    const width = view.size[0] * unit;
    const height = view.size[1] * unit;
    return {
      id: view.name,
      type: "cicada",
      position,
      width,
      height,
      data: { view },
      selected: sel.has(view.name),
      dragging: before?.dragging === true,
      // Ghost boxes never move; everything else follows the canvas-wide
      // `nodesDraggable` (writer only) — a per-node `true` would override it.
      draggable: view.kind === "broken" ? false : undefined,
    };
  });
}

/** Graph → React Flow edges (one per wire; ids are the server's wire ids). */
export function buildEdges(graph: GraphView, selectedWire: string | null): CanvasEdge[] {
  return graph.wires.map((wire) => ({
    id: wire.id,
    type: "cicada",
    source: wire.from.node,
    sourceHandle: wire.from.port,
    target: wire.to.node,
    targetHandle: wire.to.port,
    data: { wire },
    selected: wire.id === selectedWire,
    // A wire is deleted by dragging an end off any handle, never by a
    // local remove: the server owns the graph.
    deletable: false,
    reconnectable: true,
  }));
}

/**
 * Apply a selection set to React Flow items; returns the same array when
 * nothing changed so React skips the render.
 */
export function syncSelected<T extends { id: string; selected?: boolean }>(
  items: T[],
  selected: ReadonlySet<string>,
): T[] {
  let changed = false;
  const next = items.map((item) => {
    const want = selected.has(item.id);
    if ((item.selected ?? false) === want) return item;
    changed = true;
    return { ...item, selected: want };
  });
  return changed ? next : items;
}

/** Same set of names, order-insensitive. */
export function sameNames(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  const set = new Set(a);
  return b.every((name) => set.has(name));
}
