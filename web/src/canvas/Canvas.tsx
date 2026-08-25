/**
 * The node canvas (docs/16 §Canvas conventions, docs/09 §Blocked wires,
 * docs/10 §Round-trip contract): a React Flow view over the server's graph
 * view-model. Every gesture becomes ONE intent (`move_node`, `connect`,
 * `disconnect`, `place_node`, `set_param`, …) — a gesture on several nodes
 * or with several edits (multi-move, reconnect) goes as one `batch`, so it
 * is one op and one Ctrl+Z; the client only keeps optimistic feel (drag
 * position, slider thumb) until the next delta.
 *
 * Mouse map: left-drag on empty canvas = marquee · middle/right-drag or
 * Space+drag = pan · wheel = zoom · right-click = context menu ·
 * double-click = search-to-place · drag from an output = wire (drop on
 * empty canvas → filtered search).
 */
import {
  applyEdgeChanges,
  applyNodeChanges,
  Background,
  BackgroundVariant,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Connection,
  type EdgeChange,
  type EdgeMouseHandler,
  type FinalConnectionState,
  type HandleType,
  type IsValidConnection,
  type NodeChange,
  type NodeMouseHandler,
  type OnConnectEnd,
  type OnConnectStart,
  type OnNodeDrag,
  type OnReconnect,
  type OnSelectionChangeFunc,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { asOneOp, type GestureMessage, type WireEnd } from "../protocol/messages";
import { canWrite, scrubProgressFor, useCicada, writeBlockReason } from "../state/store";
import "./canvas.css";
import { CicadaEdge } from "./CicadaEdge";
import { CicadaNode } from "./CicadaNode";
import { collapseHint } from "./collapse";
import { ConnectionLine } from "./ConnectionLine";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import {
  buildEdges,
  buildNodes,
  sameNames,
  sendWrite,
  syncSelected,
  type CanvasEdge,
  type CanvasNode,
} from "./flow";
import { pxToCell, showsPortValues } from "./grid";
import { useLodTier } from "./lod";
import { scrubMenuItems } from "./scrubMenu";
import { SearchBox } from "./SearchBox";
import { TraceLanesContext, useTraceLanes } from "./traceLanes";

const NODE_TYPES = { cicada: CicadaNode };
const EDGE_TYPES = { cicada: CicadaEdge };

type Menu =
  | { kind: "node"; node: string; x: number; y: number }
  | { kind: "wire"; wire: string; x: number; y: number }
  | { kind: "pane"; x: number; y: number };

/** Client coordinates of a mouse/touch event. */
function pointerXY(event: MouseEvent | TouchEvent): { x: number; y: number } {
  if ("changedTouches" in event) {
    const t = event.changedTouches[0];
    return { x: t?.clientX ?? 0, y: t?.clientY ?? 0 };
  }
  return { x: event.clientX, y: event.clientY };
}

/**
 * The probe verdict for `from → to`, when the live probe is for `from`.
 * `undefined` = no verdict (probe not arrived, or the port is absent from
 * it) — and NO verdict means NO wire: the gate fails closed (docs/09
 * §Blocked wires: the canvas never completes a wire the checker did not
 * approve).
 */
function verdictFor(from: WireEnd, to: WireEnd) {
  const probe = useCicada.getState().probe;
  if (probe === null || probe.from.node !== from.node || probe.from.port !== from.port) return undefined;
  return probe.targets[`${to.node}.${to.port}`];
}

/** Did the probe approve this wire (`ok` or `lift`)? Anything else — blocked or unknown — is a refusal. */
function approved(verdict: { verdict: "ok" | "lift" | "blocked" } | undefined): verdict is { verdict: "ok" | "lift" } {
  return verdict !== undefined && verdict.verdict !== "blocked";
}

/** The wire-gate refusal text for a drop that must not connect. */
function refusalText(to: WireEnd, verdict: { verdict: string; reason?: string } | undefined): string {
  if (verdict === undefined) {
    return `no type verdict yet for ${to.node}.${to.port} — wire not made; drop again once the probe answers`;
  }
  return `blocked: ${verdict.reason ?? "incompatible port"}`;
}

/** A write gesture refused for lack of the lease or the connection — loud, never silent. */
function readOnlyNotice(what: string): void {
  const state = useCicada.getState();
  const hint = state.connection === "open" ? "take the lease to edit" : "waiting for the connection";
  state.addNotice("warning", `${writeBlockReason(state) ?? "cannot write"} — ${what} (${hint})`);
}

export function Canvas() {
  return (
    <ReactFlowProvider>
      <CanvasInner />
    </ReactFlowProvider>
  );
}

function CanvasInner() {
  const graph = useCicada((s) => s.graph);
  const unit = useCicada((s) => s.hello?.unitPx ?? 24);
  const selectionNodes = useCicada((s) => s.selection.nodes);
  const selectionWire = useCicada((s) => s.selection.wire);
  const selectionElement = useCicada((s) => s.selection.element);
  // One predicate for every write affordance: the lease AND an open socket.
  const writer = useCicada(canWrite);
  const connection = useCicada((s) => s.connection);
  const search = useCicada((s) => s.search);
  const summaryGeneration = useCicada((s) => s.summary.generation);
  const summaryRunning = useCicada((s) => s.summary.running);
  const snapshots = useCicada((s) => s.snapshots);
  const wireMode = useCicada((s) => s.settings.wireMode);
  const rf = useReactFlow<CanvasNode, CanvasEdge>();
  const tier = useLodTier();

  const containerRef = useRef<HTMLDivElement>(null);
  const [nodes, setNodes] = useState<CanvasNode[]>([]);
  const [edges, setEdges] = useState<CanvasEdge[]>([]);
  // Trace mode's lanes (docs/16 §Canvas conventions): assigned over EVERY
  // wire at once from the graph and the live node positions, once per
  // render; the edges read theirs from the context.
  const traceLanes = useTraceLanes(graph, nodes, unit, wireMode === "trace");
  const [menu, setMenu] = useState<Menu | null>(null);
  // The open node menu's `scrub_progress` overlay (item 5 S2): the scrub
  // menu item reads the MERGED view — the graph's `param.scrub` with the
  // broadcast laid over it, as the bar does — so its warm count is the
  // bar's and moves while the menu is open. Undefined while no node menu
  // is open, so the 10 Hz broadcast re-renders nothing here.
  const menuScrub = useCicada((s) => (menu?.kind === "node" ? scrubProgressFor(s, menu.node) : undefined));
  const [moveTick, setMoveTick] = useState(0);
  const rightDrag = useRef<{ x: number; y: number } | null>(null);
  const reconnecting = useRef(false);
  const reconnectDone = useRef(false);
  const fitOnce = useRef(false);
  const inspected = useRef<Record<string, number>>({});
  // A re-hydration snapshot clears the value cache: forget what we asked.
  useEffect(() => {
    inspected.current = {};
  }, [snapshots]);

  // ---------------------------------------------------- graph → React Flow
  useEffect(() => {
    const state = useCicada.getState();
    setNodes((prev) => buildNodes(graph, unit, prev, state.selection.nodes));
    setEdges(buildEdges(graph, state.selection.wire));
  }, [graph, unit]);

  // First graph with nodes: frame everything once, readable zoom.
  useEffect(() => {
    if (fitOnce.current || nodes.length === 0) return;
    fitOnce.current = true;
    // Next frame: node dimensions are known from the view-model, but React
    // Flow measures on mount.
    requestAnimationFrame(() => {
      void rf.fitView({ padding: 0.15, maxZoom: 1, minZoom: 0.1 });
    });
  }, [nodes.length, rf]);

  // -------------------------------------------- store selection → React Flow
  useEffect(() => {
    setNodes((prev) => syncSelected(prev, new Set(selectionNodes)));
  }, [selectionNodes]);
  useEffect(() => {
    setEdges((prev) => syncSelected(prev, new Set(selectionWire === null ? [] : [selectionWire])));
  }, [selectionWire]);

  // Backward picking (viewport → canvas): highlight is the node's `selected`
  // flag; also bring the node on-screen when it is off-screen.
  useEffect(() => {
    const name = selectionElement?.node;
    if (!name) return;
    const node = rf.getInternalNode(name);
    const container = containerRef.current;
    if (node === undefined || container === null) return;
    const rect = container.getBoundingClientRect();
    const w = node.measured.width ?? node.width ?? 0;
    const h = node.measured.height ?? node.height ?? 0;
    const tl = rf.flowToScreenPosition(node.internals.positionAbsolute);
    const br = rf.flowToScreenPosition({
      x: node.internals.positionAbsolute.x + w,
      y: node.internals.positionAbsolute.y + h,
    });
    const inside = tl.x >= rect.left && tl.y >= rect.top && br.x <= rect.right && br.y <= rect.bottom;
    if (inside) return;
    void rf.setCenter(node.internals.positionAbsolute.x + w / 2, node.internals.positionAbsolute.y + h / 2, {
      zoom: rf.getZoom(),
      duration: 300,
    });
  }, [selectionElement, rf]);

  // ------------------------------------------- React Flow selection → store
  const onSelectionChange = useCallback<OnSelectionChangeFunc<CanvasNode, CanvasEdge>>(
    ({ nodes: selNodes, edges: selEdges }) => {
      const state = useCicada.getState();
      const names = selNodes.map((n) => n.id);
      const wire = selEdges[0]?.id ?? null;
      // Nodes win over edges (a marquee also grabs the wires between the
      // selected nodes); a lone wire selects the wire; nothing clears.
      if (names.length > 0) {
        if (!sameNames(names, state.selection.nodes)) state.selectNodes(names);
        return;
      }
      if (wire !== null) {
        if (wire !== state.selection.wire) state.selectWire(wire);
        return;
      }
      if (state.selection.nodes.length > 0 || state.selection.wire !== null) state.clearSelection();
    },
    [],
  );

  const onNodesChange = useCallback((changes: NodeChange<CanvasNode>[]) => {
    // Removes never come from the canvas: the server owns the graph.
    setNodes((prev) => applyNodeChanges(changes.filter((c) => c.type !== "remove"), prev));
  }, []);
  const onEdgesChange = useCallback((changes: EdgeChange<CanvasEdge>[]) => {
    setEdges((prev) => applyEdgeChanges(changes.filter((c) => c.type !== "remove"), prev));
  }, []);

  // ------------------------------------------------------------------ move
  const onNodeDragStop = useCallback<OnNodeDrag<CanvasNode>>(
    (_event, node, dragged) => {
      const moved = dragged.length > 0 ? dragged : [node];
      // One drag = one op: a multi-select move is a `batch` of `move_node`.
      const ops: GestureMessage[] = moved.map((n) => ({
        type: "move_node",
        payload: { node: n.id, cell: pxToCell(n.position.x, n.position.y, unit) },
      }));
      sendWrite(asOneOp(ops, `move ${ops.length} nodes`));
    },
    [unit],
  );

  // ----------------------------------------------------------------- wires
  // The source last probed for the current drag (dedupes probe_wire sends).
  const probedSource = useRef<string | null>(null);
  const requestProbe = useCallback((from: WireEnd) => {
    const key = `${from.node}.${from.port}`;
    if (probedSource.current === key) return;
    probedSource.current = key;
    // probe_wire is a read: observers may look, they just cannot connect.
    useCicada.getState().send({ type: "probe_wire", payload: { from } });
  }, []);

  const onConnectStart = useCallback<OnConnectStart>(
    (_event, params) => {
      probedSource.current = null;
      if (params.handleType === "source" && params.nodeId !== null && params.handleId !== null) {
        requestProbe({ node: params.nodeId, port: params.handleId });
      }
    },
    [requestProbe],
  );

  const isValidConnection = useCallback<IsValidConnection<CanvasEdge>>(
    (conn) => {
      const { source, target, sourceHandle, targetHandle } = conn;
      if (!source || !target || !sourceHandle || !targetHandle) return false;
      if (source === target) return false;
      // Expression inputs are free variables, not kwargs (docs/10 §4).
      const targetView = useCicada.getState().graph.nodes.find((n) => n.name === target);
      if (targetView?.kind === "expression") return false;
      const from = { node: source, port: sourceHandle };
      const verdict = verdictFor(from, { node: target, port: targetHandle });
      // A drag from a TARGET handle (reconnecting a wire's source end) learns
      // its source only now: probe it, and the next hover sees the verdict.
      if (verdict === undefined) requestProbe(from);
      // Fail CLOSED: no verdict (probe still in flight, or the port absent
      // from it) is invalid — the handle shows "checking…" meanwhile.
      return approved(verdict);
    },
    [requestProbe],
  );

  const onConnect = useCallback((conn: Connection) => {
    if (!conn.sourceHandle || !conn.targetHandle) return;
    const from = { node: conn.source, port: conn.sourceHandle };
    const to = { node: conn.target, port: conn.targetHandle };
    const verdict = verdictFor(from, to);
    // `isValidConnection` already refused these; this is the belt: never
    // send `connect` without an ok/lift verdict.
    if (!approved(verdict)) {
      useCicada.getState().addNotice("warning", refusalText(to, verdict));
      return;
    }
    sendWrite({ type: "connect", payload: { from, to, lift: verdict.verdict === "lift" } });
  }, []);

  const onConnectEnd = useCallback<OnConnectEnd>(
    (event, state) => {
      const store = useCicada.getState();
      if (state.isValid === true) {
        store.clearProbe();
        return;
      }
      const target = event.target instanceof Element ? event.target : null;
      // Dropped on a handle that did not connect: say why (blocked, or no
      // verdict yet — the gate fails closed, so the drop did nothing).
      if (state.toHandle !== null && state.fromHandle !== null && state.fromHandle.type === "source") {
        const to = { node: state.toHandle.nodeId, port: state.toHandle.id ?? "" };
        const verdict = verdictFor({ node: state.fromHandle.nodeId, port: state.fromHandle.id ?? "" }, to);
        const targetView = store.graph.nodes.find((n) => n.name === to.node);
        if (
          state.toHandle.type === "target" &&
          targetView?.kind !== "expression" &&
          to.node !== state.fromHandle.nodeId
        ) {
          store.addNotice("warning", refusalText(to, verdict));
        }
        store.clearProbe();
        return;
      }
      const onPane = target?.classList.contains("react-flow__pane") ?? false;
      const fromPort = state.fromHandle?.id ?? null;
      if (
        onPane &&
        !reconnecting.current &&
        state.fromHandle !== null &&
        state.fromHandle.type === "source" &&
        fromPort !== null
      ) {
        if (!writer) {
          readOnlyNotice("placing nodes ignored");
          store.clearProbe();
          return;
        }
        const { x, y } = pointerXY(event);
        const p = rf.screenToFlowPosition({ x, y });
        store.openSearch({
          x,
          y,
          cell: pxToCell(p.x, p.y, unit),
          from: { node: state.fromHandle.nodeId, port: fromPort },
        });
        // The probe stays alive for the search box's catalog filter.
        return;
      }
      store.clearProbe();
    },
    [rf, unit, writer],
  );

  const onReconnectStart = useCallback(() => {
    reconnecting.current = true;
    reconnectDone.current = false;
  }, []);

  const onReconnect = useCallback<OnReconnect<CanvasEdge>>((oldEdge, conn) => {
    reconnectDone.current = true;
    const wire = oldEdge.data?.wire;
    if (wire === undefined || !conn.sourceHandle || !conn.targetHandle) return;
    const from = { node: conn.source, port: conn.sourceHandle };
    const to = { node: conn.target, port: conn.targetHandle };
    const sameTarget = to.node === wire.to.node && to.port === wire.to.port;
    const sameSource = from.node === wire.from.node && from.port === wire.from.port;
    if (sameTarget && sameSource) return;
    const verdict = verdictFor(from, to);
    if (!approved(verdict)) {
      useCicada.getState().addNotice("warning", refusalText(to, verdict));
      return;
    }
    // A reconnect to another target is wire + unwire as ONE op (a `batch`,
    // all or nothing): the old kwarg never lingers half-moved, and one
    // Ctrl+Z puts the wire back where it was.
    const ops: GestureMessage[] = [{ type: "connect", payload: { from, to, lift: verdict.verdict === "lift" } }];
    if (!sameTarget) ops.push({ type: "disconnect", payload: { to: wire.to } });
    sendWrite(asOneOp(ops, `rewire ${wire.to.node}.${wire.to.port} → ${to.node}.${to.port}`));
  }, []);

  const onReconnectEnd = useCallback(
    (
      _event: MouseEvent | TouchEvent,
      edge: CanvasEdge,
      _handleType: HandleType,
      connectionState: FinalConnectionState,
    ) => {
      reconnecting.current = false;
      useCicada.getState().clearProbe();
      if (reconnectDone.current) return;
      // Dropped off any handle: the wire is deleted (one kwarg removed). A
      // drop on a blocked/incompatible handle keeps the wire (the notice from
      // onConnectEnd already said why).
      if (connectionState.toHandle !== null) return;
      const wire = edge.data?.wire;
      if (wire !== undefined) sendWrite({ type: "disconnect", payload: { to: wire.to } });
    },
    [],
  );

  const onEdgesDelete = useCallback((deleted: CanvasEdge[]) => {
    const ops: GestureMessage[] = [];
    for (const edge of deleted) {
      const wire = edge.data?.wire;
      if (wire !== undefined) ops.push({ type: "disconnect", payload: { to: wire.to } });
    }
    if (ops.length > 0) sendWrite(asOneOp(ops, `unwire ${ops.length} wires`));
  }, []);

  // ------------------------------------------------------ clicks / menus
  const dismissSearch = useCallback(() => {
    const state = useCicada.getState();
    if (state.search === null) return;
    if (state.search.from !== null) state.clearProbe();
    state.closeSearch();
  }, []);

  const onPaneClick = useCallback(() => {
    useCicada.getState().clearSelection();
    setMenu(null);
    dismissSearch();
  }, [dismissSearch]);

  const onNodeClick = useCallback<NodeMouseHandler<CanvasNode>>(() => {
    // Selection itself flows through React Flow → onSelectionChange.
    setMenu(null);
    dismissSearch();
  }, [dismissSearch]);

  const onEdgeClick = useCallback<EdgeMouseHandler<CanvasEdge>>((_event, edge) => {
    const state = useCicada.getState();
    state.selectWire(edge.id);
    const wire = edge.data?.wire;
    if (wire !== undefined) state.send({ type: "inspect_wire", payload: { to: wire.to } });
  }, []);

  const onNodeContextMenu = useCallback<NodeMouseHandler<CanvasNode>>((event, node) => {
    event.preventDefault();
    const state = useCicada.getState();
    if (!state.selection.nodes.includes(node.id)) state.selectNodes([node.id]);
    setMenu({ kind: "node", node: node.id, x: event.clientX, y: event.clientY });
  }, []);

  const onEdgeContextMenu = useCallback<EdgeMouseHandler<CanvasEdge>>((event, edge) => {
    event.preventDefault();
    setMenu({ kind: "wire", wire: edge.id, x: event.clientX, y: event.clientY });
  }, []);

  // React Flow swallows the pane's own contextmenu when right-drag pans, so
  // the canvas menu is ours: right-press without movement on the pane.
  const onPointerDownCapture = (event: React.PointerEvent) => {
    if (event.button === 2) rightDrag.current = { x: event.clientX, y: event.clientY };
  };
  const onContextMenu = (event: React.MouseEvent) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target === null) return;
    event.preventDefault();
    if (!target.classList.contains("react-flow__pane")) return;
    const start = rightDrag.current;
    rightDrag.current = null;
    if (start !== null && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 4) return;
    setMenu({ kind: "pane", x: event.clientX, y: event.clientY });
  };

  const onDoubleClick = (event: React.MouseEvent) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target === null || !target.classList.contains("react-flow__pane")) return;
    if (!writer) {
      readOnlyNotice("placing nodes ignored");
      return;
    }
    const p = rf.screenToFlowPosition({ x: event.clientX, y: event.clientY });
    useCicada.getState().openSearch({
      x: event.clientX,
      y: event.clientY,
      cell: pxToCell(p.x, p.y, unit),
      from: null,
    });
  };

  // ------------------------------------ value previews (near tier and up)
  useEffect(() => {
    if (!showsPortValues(tier)) return;
    const container = containerRef.current;
    if (container === null) return;
    const state = useCicada.getState();
    const rect = container.getBoundingClientRect();
    for (const node of rf.getNodes()) {
      const status = state.statuses[node.id];
      if (status === undefined || (status.state !== "done" && status.state !== "cached")) continue;
      const w = node.measured?.width ?? node.width ?? 0;
      const h = node.measured?.height ?? node.height ?? 0;
      const tl = rf.flowToScreenPosition(node.position);
      const br = rf.flowToScreenPosition({ x: node.position.x + w, y: node.position.y + h });
      const visible = br.x >= rect.left && tl.x <= rect.right && br.y >= rect.top && tl.y <= rect.bottom;
      if (!visible) continue;
      const have = state.nodeValues[node.id]?.generation ?? -1;
      if (have >= status.generation || inspected.current[node.id] === status.generation) continue;
      inspected.current[node.id] = status.generation;
      state.send({ type: "inspect", payload: { node: node.id } });
    }
  }, [tier, moveTick, summaryGeneration, summaryRunning, rf, nodes]);

  // ---------------------------------------------------------- menu items
  const menuItems = useMemo<{ title: string; items: MenuItem[] } | null>(() => {
    if (menu === null) return null;
    const state = useCicada.getState();
    const notice = (what: string) => () => state.addNotice("info", what);
    if (menu.kind === "node") {
      const view = state.graph.nodes.find((n) => n.name === menu.node);
      if (view === undefined) return null;
      const displayable = view.outputs.some((o) => o.displayable);
      return {
        title: menu.node,
        items: [
          {
            label: view.preview ? "hide preview" : "show preview",
            disabled: !displayable,
            hint: displayable ? "P" : "no displayable output",
            onClick: () => sendWrite({ type: "set_preview", payload: { node: view.name, on: !view.preview } }),
          },
          // A slider collapses to one row (wave 4 B4; docs/16). The server
          // decides and refuses — a wired value / min / max / step is a
          // notice — and the hint mirrors its reason, so the user reads why
          // first. Offered for every slider (the called node), widget or not.
          ...(view.func === "slider"
            ? [
                {
                  label: view.collapsed === true ? "expand" : "collapse",
                  hint: view.collapsed === true ? "full node" : (collapseHint(view) ?? "one row"),
                  onClick: () =>
                    sendWrite({
                      type: "set_collapsed",
                      payload: { node: view.name, collapsed: view.collapsed !== true },
                    }),
                },
              ]
            : []),
          // Scrub caching (item 5 S2; docs/16 §Sliders): the toggle as a
          // menu item — `scrub-cache this slider` / `stop scrub-caching`,
          // greyed with the SERVER's reason (`param.scrub.ineligible`)
          // while the slider is off and cannot; the client computes nothing.
          // Built by `scrubMenu.ts` off the merged view (`menuScrub`), where
          // `scrubMenu.test.tsx` pins the label, the greying and the hint.
          ...scrubMenuItems(view, menuScrub, sendWrite),
          {
            label: "rename…",
            onClick: () => {
              const next = window.prompt(`rename ${view.name} to`, view.name);
              if (next === null || next.trim() === "" || next === view.name) return;
              sendWrite({ type: "rename", payload: { node: view.name, new: next.trim() } });
            },
          },
          {
            label: "show in text",
            onClick: () => {
              state.selectNodes([view.name]);
              state.updateSettings({ textPanel: true });
            },
          },
          {
            label: view.kind === "disabled" ? "enable" : "disable",
            separator: true,
            hint: "D",
            disabled: view.kind === "broken",
            onClick: () => sendWrite({ type: "toggle_disable", payload: { node: view.name } }),
          },
          { label: "isolate", onClick: notice("isolate arrives later") },
          { label: "history", onClick: notice("per-node history arrives with the git panel (doc 10)") },
          {
            label: "delete",
            danger: true,
            separator: true,
            hint: "Del",
            onClick: () => sendWrite({ type: "delete_node", payload: { node: view.name } }),
          },
        ],
      };
    }
    if (menu.kind === "wire") {
      const wire = state.graph.wires.find((w) => w.id === menu.wire);
      if (wire === undefined) return null;
      return {
        title: wire.id,
        items: [
          {
            label: "inspect",
            onClick: () => {
              state.selectWire(wire.id);
              state.send({ type: "inspect_wire", payload: { to: wire.to } });
            },
          },
          { label: "insert node here", onClick: notice("insert node arrives with insert_between (v0.1)") },
          {
            label: "disconnect",
            danger: true,
            separator: true,
            onClick: () => sendWrite({ type: "disconnect", payload: { to: wire.to } }),
          },
        ],
      };
    }
    return {
      title: "canvas",
      items: [
        {
          label: "search here…",
          hint: "dbl-click",
          onClick: () => {
            if (!writer) {
              readOnlyNotice("placing nodes ignored");
              return;
            }
            const p = rf.screenToFlowPosition({ x: menu.x, y: menu.y });
            state.openSearch({ x: menu.x, y: menu.y, cell: pxToCell(p.x, p.y, unit), from: null });
          },
        },
        { label: "fit view", onClick: () => void rf.fitView({ padding: 0.15, duration: 300 }) },
        { label: "paste", separator: true, onClick: notice("paste arrives later") },
        { label: "group selection", onClick: notice("groups arrive later") },
      ],
    };
  }, [menu, rf, unit, writer, menuScrub]);

  // Overlay anchors: client → pane coordinates, kept inside the pane.
  const paneRect = containerRef.current?.getBoundingClientRect();
  const toLocal = (x: number, y: number, w: number, h: number) => {
    const clamp = (v: number, hi: number) => Math.max(0, Math.min(v, hi));
    return {
      left: clamp(x - (paneRect?.left ?? 0), (paneRect?.width ?? Infinity) - w),
      top: clamp(y - (paneRect?.top ?? 0), (paneRect?.height ?? Infinity) - h),
    };
  };

  const closeMenu = useCallback(() => setMenu(null), []);

  return (
    <div
      ref={containerRef}
      className={`cicada-canvas${writer ? "" : " readonly"}`}
      data-lod={tier}
      // Trace mode's one routing fallback, never silent: how many wires
      // share a lane because a column ran out of lines (docs/16; 0 on
      // every committed example).
      data-trace-collapsed={wireMode === "trace" ? traceLanes.collapsed.length : undefined}
      data-testid="canvas"
      onPointerDownCapture={onPointerDownCapture}
      onContextMenu={onContextMenu}
      onDoubleClick={onDoubleClick}
    >
      <TraceLanesContext.Provider value={traceLanes.routes}>
        <ReactFlow<CanvasNode, CanvasEdge>
          nodes={nodes}
          edges={edges}
          nodeTypes={NODE_TYPES}
          edgeTypes={EDGE_TYPES}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onSelectionChange={onSelectionChange}
          onNodeDragStop={onNodeDragStop}
          onConnectStart={onConnectStart}
          onConnect={onConnect}
          onConnectEnd={onConnectEnd}
          onReconnectStart={onReconnectStart}
          onReconnect={onReconnect}
          onReconnectEnd={onReconnectEnd}
          onEdgesDelete={onEdgesDelete}
          isValidConnection={isValidConnection}
          connectionLineComponent={ConnectionLine}
          connectionRadius={14}
          onPaneClick={onPaneClick}
          onNodeClick={onNodeClick}
          onEdgeClick={onEdgeClick}
          onNodeContextMenu={onNodeContextMenu}
          onEdgeContextMenu={onEdgeContextMenu}
          onMoveEnd={() => setMoveTick((t) => t + 1)}
          onMoveStart={() => setMenu(null)}
          nodesDraggable={writer}
          nodesConnectable={writer}
          edgesReconnectable={writer}
          elementsSelectable
          selectionOnDrag
          panOnDrag={[1, 2]}
          panOnScroll={false}
          zoomOnScroll
          zoomOnDoubleClick={false}
          zoomOnPinch
          snapToGrid
          snapGrid={[unit, unit]}
          minZoom={0.1}
          maxZoom={2.5}
          // Del belongs to the keyboard map (docs/16), and React Flow's own
          // arrow-key nudge would move nodes with no `move_node` behind it.
          deleteKeyCode={null}
          disableKeyboardA11y
          multiSelectionKeyCode={["Shift", "Control", "Meta"]}
          nodeDragThreshold={2}
        >
          <Background variant={BackgroundVariant.Lines} gap={unit} lineWidth={1} />
          <Controls showInteractive={false} position="bottom-left" />
        </ReactFlow>
      </TraceLanesContext.Provider>
      {!writer && (
        <div
          className="cv-readonly badge"
          title={
            connection === "open"
              ? "another client holds the write lease — take the lease to edit"
              : "not connected — edits are disabled until the session is back"
          }
        >
          {connection === "open" ? "read-only observer" : "read-only — not connected"}
        </div>
      )}
      {connection !== "open" && graph.nodes.length === 0 && (
        <div className="cv-empty dim">{connection === "connecting" ? "connecting…" : `canvas: ${connection}`}</div>
      )}
      {search !== null && <SearchBox key={`${search.x},${search.y}`} {...toLocal(search.x, search.y, 320, 340)} />}
      {menu !== null && menuItems !== null && (
        <ContextMenu
          {...toLocal(menu.x, menu.y, 200, 40 + 26 * menuItems.items.length)}
          title={menuItems.title}
          items={menuItems.items}
          onClose={closeMenu}
        />
      )}
    </div>
  );
}
