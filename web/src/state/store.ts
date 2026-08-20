/**
 * The app store (zustand). Authoritative state MIRRORS the server (graph,
 * text, statuses, summary, lease) — `applyServerMessage` is the only writer
 * of those slices. Everything else is UI state (selection, hover, settings,
 * probe results, notices) or read caches (catalog, inspector values).
 *
 * Intents go out through `send` (installed by the connection module) — the
 * canvas/viewport/panels never touch the socket directly.
 */
import { create } from "zustand";
import type {
  Catalog,
  ClientMessage,
  ErrorKind,
  ErrorPayload,
  GraphView,
  HistoryView,
  LeaseView,
  NodeStatus,
  PreviewMode,
  ProbeCatalogEntry,
  ProbeVerdict,
  Role,
  ServerEnvelope,
  SolveSummary,
  ValueSummary,
  WireEnd,
} from "../protocol/messages";

/**
 * Socket lifecycle. `reconnecting` = the socket dropped without us closing
 * it and the connection module is retrying with backoff; while it lasts the
 * session identity is cleared (role observer, empty lease) so no write can
 * be attempted against a dead socket — `canWrite` gates every write.
 */
export type Connection = "idle" | "connecting" | "open" | "reconnecting" | "closed" | "error";

/** Reconnect bookkeeping for the banner: attempt count and when the next try fires (null = a try is in flight). */
export interface ReconnectState {
  attempt: number;
  nextAt: number | null;
}

export interface HelloInfo {
  clientId: number;
  role: Role;
  protocol: number;
  engine: string;
  project: string;
  pipeline: string;
  unitPx: number;
}

/** A picked geometry element (backward picking, docs/04). */
export interface ElementPick {
  pickId: number;
  /** View-model node ref. */
  nodeRef: number;
  /** Binding name (resolved from the graph when known). */
  node: string | null;
  output: number;
  element: number;
}

export interface Selection {
  nodes: string[];
  wire: string | null;
  element: ElementPick | null;
}

export interface Notice {
  id: number;
  level: "info" | "warning" | "error";
  message: string;
  at: number;
}

export type SplitPreset = "canvas" | "even" | "viewport";
export type WireMode = "spline" | "trace";
export type DisplayMode = "shaded_edges" | "shaded" | "wireframe";
export type Theme = "dark" | "light";

export interface Settings {
  theme: Theme;
  split: SplitPreset;
  swap: boolean;
  wireMode: WireMode;
  displayMode: DisplayMode;
  textPanel: boolean;
  ribbonCollapsed: boolean;
  navigation: "rhino" | "blender";
}

const SETTINGS_KEY = "cicada.settings.v1";

const DEFAULT_SETTINGS: Settings = {
  theme: "dark",
  split: "canvas",
  swap: false,
  wireMode: "spline",
  displayMode: "shaded_edges",
  textPanel: false,
  ribbonCollapsed: false,
  navigation: "rhino",
};

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw === null) return DEFAULT_SETTINGS;
    return { ...DEFAULT_SETTINGS, ...(JSON.parse(raw) as Partial<Settings>) };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export interface ProbeState {
  from: WireEnd;
  /** `node.port` → verdict. */
  targets: Record<string, ProbeVerdict>;
  catalog: ProbeCatalogEntry[];
  intentId: string | null;
}

export interface NodeValues {
  generation: number;
  outputs: [string, ValueSummary | null][];
}

export interface WireValues {
  from: WireEnd;
  to: WireEnd;
  summary: ValueSummary | null;
  pairing: string;
}

/**
 * The param whose drag is under compute-on-release (docs/13 §Slider drags,
 * DECISIONS.md row 39): the server withheld this drag's previews because
 * the cone is predicted at ≥ 1 s, and `preview_policy` said so — the
 * slider shows `value` as pending with the estimate; the viewport is NOT
 * expected to move until release. `value` starts as the message's
 * `pending_value` (the first withheld tick) and follows the dragging
 * widget's later ticks (`trackPendingValue`); observers keep the last
 * value they heard. The server holds ONE drag at a time, so there is one
 * pending param at a time: every `preview_policy` REPLACES it (for the
 * same param or another — the other's drag has ended). Cleared by the
 * release's delta (any write ends the drag server-side — a refused one
 * too: an `error`), by `drag_ended` (the server announcing the end of an
 * announced drag: after a release that wrote nothing, an Esc, a refused
 * write, the writer's departure or a lease handover — the signal observers
 * and the non-dragging twin widget have for those ends), by a snapshot (a
 * reload barrier ends it), a disconnect, or the widget itself on its own
 * release that writes nothing (`endDrag`: optimistic, ahead of the
 * `drag_ended` its `end_drag` intent earns).
 */
export interface PendingParam {
  node: string;
  /** The kwarg; `null` for a bare literal. */
  port: string | null;
  mode: PreviewMode;
  /** The dialect literal the release will solve. */
  value: string;
  /** Predicted wall ms of a live preview — a floor when `rough`. */
  estimateMs: number;
  rough: boolean;
  /**
   * The session `seq` current when the `preview_policy` arrived — the
   * write counter, NOT unique per message (two policies between writes
   * share it). Informational; nothing decides on it.
   */
  seq: number;
}

/** Does this pending entry belong to `node`/`port` (`null` = a bare literal)? */
function pendingIs(pending: PendingParam | null, node: string, port: string | null | undefined): boolean {
  return pending !== null && pending.node === node && pending.port === (port ?? null);
}

const EMPTY_GRAPH: GraphView = { nodes: [], wires: [], diagnostics: [] };
/** No ops yet — what the server reports before any edit (and after a reload barrier). */
export const EMPTY_HISTORY: HistoryView = {
  can_undo: false,
  can_redo: false,
  undo_label: null,
  redo_label: null,
  depth: 0,
};

/** The last refused intent: the `error` payload minus its wire casing. */
export interface LastError {
  intentId?: string;
  kind: ErrorKind;
  message: string;
  /** `stale_base`: the hash to rebase on. */
  currentTextHash?: string;
  /** `parse_error`: the doc-11 diagnostics. */
  diagnostics?: ErrorPayload["diagnostics"];
  /** A failed `batch`: the 0-based index of the op that failed. */
  index?: number;
}
const EMPTY_SUMMARY: SolveSummary = {
  generation: 0,
  running: false,
  cancelled: false,
  computed: 0,
  cached: 0,
  pending: 0,
  red: 0,
  blocked: 0,
  elapsed_ms: 0,
  eta_rough: false,
};

export interface CicadaState {
  // ---- connection / identity
  connection: Connection;
  connectionMessage: string;
  reconnect: ReconnectState | null;
  hello: HelloInfo | null;
  role: Role;
  lease: LeaseView;
  token: string;
  pipeline: string;

  // ---- authoritative mirror
  seq: number;
  graph: GraphView;
  text: string;
  statuses: Record<string, NodeStatus>;
  summary: SolveSummary;
  /** Bindings the last delta named dirty (for a brief flash). */
  dirty: string[];
  lastDeltaLabel: string;
  /**
   * Undo/redo state as of the last `delta` / `snapshot` (docs/13 §Undo/redo).
   * Display + affordance gating only: the server is the authority, and
   * `undo` / `redo` go out as intents whose refusal says why.
   */
  history: HistoryView;
  lastError: LastError | null;
  /** Number of snapshots received (barrier reloads bump it). */
  snapshots: number;
  displayGeneration: number;
  /** Count of `display_reset` messages (a re-stream can repeat a generation). */
  displayResets: number;

  // ---- read caches
  catalog: Catalog | null;
  nodeValues: Record<string, NodeValues>;
  wireValues: Record<string, WireValues>;
  probe: ProbeState | null;

  // ---- ephemeral drag state
  /**
   * The param whose current drag is compute-on-release, or null
   * (`preview_policy` sets it — the latest arrival replaces it; every
   * delta / error / snapshot / disconnect clears it: a write attempt or a
   * reload ends the drag server-side; `drag_ended` clears it when it names
   * it; the widget's own `endDrag` clears it ahead of that). Null for
   * cheap cones, which never hear of the policy.
   */
  pending: PendingParam | null;

  // ---- ui
  selection: Selection;
  hoverPick: ElementPick | null;
  notices: Notice[];
  settings: Settings;
  /** Search-to-place box: null = closed; else its anchor + optional wire source filter. */
  search: { x: number; y: number; cell: [number, number] | null; from: WireEnd | null } | null;
  runNotice: { node: string; ok: boolean; message: string } | null;

  // ---- actions
  /** Installed by the connection module. */
  send: (message: ClientMessage) => string;
  installSender: (send: (message: ClientMessage) => string) => void;
  setConnection: (connection: Connection, message?: string) => void;
  /**
   * The socket died under us (close/error not initiated by the client):
   * identity is cleared so nothing writes until the next `hello`; the
   * mirror (graph, text, statuses) stays for display and is replaced by
   * the re-hydration snapshot.
   */
  markDisconnected: (message: string, reconnect: ReconnectState) => void;
  setReconnect: (reconnect: ReconnectState | null) => void;
  setIdentity: (token: string, pipeline: string) => void;
  applyServerMessage: (envelope: ServerEnvelope) => void;
  setCatalog: (catalog: Catalog) => void;
  setDisplayGeneration: (generation: number) => void;
  selectNodes: (nodes: string[], additive?: boolean) => void;
  selectWire: (wire: string | null) => void;
  selectElement: (pick: ElementPick | null) => void;
  clearSelection: () => void;
  setHoverPick: (pick: ElementPick | null) => void;
  clearProbe: () => void;
  /**
   * The dragging widget reports each later tick's literal so the pending
   * entry (and every other view of this param) follows the thumb — the
   * message only carried the FIRST withheld tick's value. A no-op (same
   * state object) when this param is not the pending one.
   */
  trackPendingValue: (node: string, port: string | null, value: string) => void;
  /**
   * The widget released on the committed value — no `set_param` goes out
   * (both sliders skip it then), so no delta will clear this param's
   * pending entry: the widget clears it here, optimistically, and tells
   * the server the drag is over (`end_drag`) so the server's drag ends NOW
   * — a re-grab inside the 300 ms gap is a fresh drag, announced again,
   * rather than a silent continuation of one this client already took
   * down — and so every other client hears `drag_ended`. (A release that
   * writes leaves it to the delta — value and badge change in one render,
   * no snap-back.) Sent on every release that writes nothing, pending or
   * not: the server's drag exists for cheap cones too, and a stale one is
   * a no-op there. Nothing is sent when this client cannot write (the
   * lease, the socket) — its drag is not the server's then.
   */
  endDrag: (node: string, port: string | null) => void;
  addNotice: (level: Notice["level"], message: string) => void;
  dismissNotice: (id: number) => void;
  updateSettings: (patch: Partial<Settings>) => void;
  openSearch: (anchor: { x: number; y: number; cell: [number, number] | null; from: WireEnd | null }) => void;
  closeSearch: () => void;
  clearRunNotice: () => void;
}

let noticeCounter = 0;

export const useCicada = create<CicadaState>((set, get) => ({
  connection: "idle",
  connectionMessage: "",
  reconnect: null,
  hello: null,
  role: "observer",
  lease: { writer: null, clients: [] },
  token: "",
  pipeline: "",

  seq: 0,
  graph: EMPTY_GRAPH,
  text: "",
  statuses: {},
  summary: EMPTY_SUMMARY,
  dirty: [],
  lastDeltaLabel: "",
  history: EMPTY_HISTORY,
  lastError: null,
  snapshots: 0,
  displayGeneration: 0,
  displayResets: 0,

  catalog: null,
  nodeValues: {},
  wireValues: {},
  probe: null,

  pending: null,

  selection: { nodes: [], wire: null, element: null },
  hoverPick: null,
  notices: [],
  settings: loadSettings(),
  search: null,
  runNotice: null,

  send: (message) => {
    console.warn("cicada: no connection yet — dropped", message.type);
    return "";
  },
  installSender: (send) => set({ send }),
  setConnection: (connection, message = "") =>
    set((state) => ({
      connection,
      connectionMessage: message,
      reconnect: connection === "open" ? null : state.reconnect,
    })),
  markDisconnected: (message, reconnect) =>
    set({
      connection: "reconnecting",
      connectionMessage: message,
      reconnect,
      role: "observer",
      lease: { writer: null, clients: [] },
      probe: null,
      search: null,
      // The drag died with the socket: the re-hydrated session knows
      // nothing of it, and the next tick is announced afresh.
      pending: null,
    }),
  setReconnect: (reconnect) => set({ reconnect }),
  setIdentity: (token, pipeline) => set({ token, pipeline }),

  applyServerMessage: (envelope) => {
    const seq = envelope.seq;
    switch (envelope.type) {
      case "hello": {
        const p = envelope.payload;
        const rejoined = get().hello !== null;
        set({
          hello: {
            clientId: p.client_id,
            role: p.role,
            protocol: p.protocol,
            engine: p.engine,
            project: p.project,
            pipeline: p.pipeline,
            unitPx: p.unit_px,
          },
          role: p.role,
        });
        if (rejoined) {
          get().addNotice(
            "info",
            `reconnected as client #${p.client_id} — ${
              p.role === "writer" ? "you hold the write lease" : "read-only observer"
            }`,
          );
        }
        break;
      }
      case "snapshot": {
        const p = envelope.payload;
        const state = get();
        // Selection survives when the node still exists (a barrier reload
        // keeps names); dead names drop out.
        const live = new Set(p.graph.nodes.map((n) => n.name));
        set({
          seq,
          graph: p.graph,
          text: p.text,
          statuses: p.statuses,
          summary: p.summary,
          lease: p.lease,
          history: p.history,
          snapshots: state.snapshots + 1,
          selection: {
            nodes: state.selection.nodes.filter((n) => live.has(n)),
            wire: null,
            element: null,
          },
          nodeValues: {},
          wireValues: {},
          // A reload barrier (and a fresh hydration) ends the drag.
          pending: null,
        });
        if (p.barrier) {
          get().addNotice("info", `reloaded from disk (${p.reason})`);
        }
        break;
      }
      case "delta": {
        const p = envelope.payload;
        const state = get();
        const live = new Set(p.graph.nodes.map((n) => n.name));
        const before = new Set(state.graph.nodes.map((n) => n.name));
        const added = p.graph.nodes.map((n) => n.name).filter((n) => !before.has(n));
        const mine = state.hello !== null && p.source.client === state.hello.clientId;
        // Selection follows the edit: a vanished selected name with exactly
        // one new name is a rename (or a replace) → select the new one; my
        // own place → select what I placed.
        let nodes = state.selection.nodes.filter((n) => live.has(n));
        const lost = state.selection.nodes.filter((n) => !live.has(n));
        if (lost.length > 0 && added.length === 1) nodes = [added[0]!];
        else if (mine && added.length > 0 && p.source.label.startsWith("place ")) nodes = added;
        set({
          seq,
          graph: p.graph,
          text: p.text,
          dirty: p.dirty,
          lastDeltaLabel: p.source.label,
          history: p.history,
          selection: {
            ...state.selection,
            nodes,
            wire: p.graph.wires.some((w) => w.id === state.selection.wire)
              ? state.selection.wire
              : null,
          },
          // Dead bindings never linger: a deleted red node must not keep the
          // solve bar red (probe friction: "1 red · 0 diagnostics").
          statuses: pruneKeys(state.statuses, live),
          nodeValues: pruneKeys(state.nodeValues, live),
          // Any write ends the drag server-side (docs/13 §Slider drags) —
          // the release's own `set_param` above all: its delta is the
          // signal that the pending value is now the committed one.
          pending: null,
        });
        break;
      }
      case "status": {
        const p = envelope.payload;
        const state = get();
        const merged = { ...state.statuses };
        for (const [name, status] of Object.entries(p.nodes)) merged[name] = status;
        set({ statuses: merged, summary: p.summary });
        break;
      }
      case "lease": {
        const p = envelope.payload;
        const before = get().role;
        set({ lease: p.lease, role: p.role });
        const change = roleChangeNotice(before, p.role, p.lease);
        if (change !== null) get().addNotice(change.level, change.message);
        break;
      }
      case "error": {
        const p = envelope.payload;
        // A refused write (a release the writer could not apply, an undo
        // with nothing to undo) ends the drag server-side exactly like a
        // landed one (docs/13 §Slider drags: "landed or refused") — the
        // pending value is NOT going to solve, so the badge must not
        // stand. Errors are unicast answers to this client's own intents;
        // mid-drag those are writes. The one refusal the session decides
        // BEFORE the drag-ending door is the lease check, so a `lease`
        // error leaves the drag (and the badge) standing.
        set({ lastError: lastErrorOf(p), pending: p.kind === "lease" ? get().pending : null });
        // An empty undo/redo side is a routine answer to Ctrl+Z, not a
        // failure: the message still says why (including the barrier).
        get().addNotice(errorNoticeLevel(p.kind), p.message);
        break;
      }
      case "wire_probe": {
        const p = envelope.payload;
        const targets: Record<string, ProbeVerdict> = {};
        for (const t of p.targets) targets[`${t.node}.${t.port}`] = t;
        set({ probe: { from: p.from, targets, catalog: p.catalog, intentId: p.intent_id } });
        break;
      }
      case "node_values": {
        const p = envelope.payload;
        set({
          nodeValues: {
            ...get().nodeValues,
            [p.node]: { generation: p.generation, outputs: p.outputs },
          },
        });
        break;
      }
      case "wire_values": {
        const p = envelope.payload;
        const key = `${p.from.node}.${p.from.port}->${p.to.node}.${p.to.port}`;
        set({
          wireValues: {
            ...get().wireValues,
            [key]: { from: p.from, to: p.to, summary: p.summary, pairing: p.pairing },
          },
        });
        break;
      }
      case "notice": {
        const p = envelope.payload;
        get().addNotice(p.level, p.message);
        break;
      }
      case "display_reset": {
        set((state) => ({
          displayGeneration: envelope.payload.generation,
          displayResets: state.displayResets + 1,
        }));
        break;
      }
      case "run_finished": {
        const p = envelope.payload;
        set({ runNotice: { node: p.node, ok: p.ok, message: p.message } });
        get().addNotice(p.ok ? "info" : "error", p.message);
        break;
      }
      case "preview_policy": {
        // Once per server-side drag, on its first withheld tick — and again
        // for the next drag (after a release, an Esc, a pause): each arrival
        // is the current verdict and REPLACES the pending param, never
        // stacks it (the server holds one drag at a time — a policy for
        // another param means this one's drag has ended).
        const p = envelope.payload;
        set({
          pending: {
            node: p.node,
            port: p.port ?? null,
            mode: p.mode,
            value: p.pending_value,
            estimateMs: p.estimate_ms,
            rough: p.rough,
            seq,
          },
        });
        break;
      }
      case "drag_ended": {
        // The announced drag is over — after a release that wrote nothing
        // (this client's own `end_drag`, already cleared optimistically, or
        // the writer's, which is the only way an observer or the twin
        // widget hears of it), an Esc, a refused write, the writer's
        // departure or a lease handover; after a landed write it follows
        // the delta and finds nothing to do. Only the named param: a newer
        // policy for another param has already replaced this one.
        const p = envelope.payload;
        set((state) => (pendingIs(state.pending, p.node, p.port ?? null) ? { pending: null } : state));
        break;
      }
      case "screenshot_request":
        // Handled by the connection module (needs the viewport).
        break;
    }
  },

  setCatalog: (catalog) => set({ catalog }),
  setDisplayGeneration: (generation) => set({ displayGeneration: generation }),

  selectNodes: (nodes, additive = false) =>
    set((state) => ({
      selection: {
        nodes: additive ? Array.from(new Set([...state.selection.nodes, ...nodes])) : nodes,
        wire: additive ? state.selection.wire : null,
        element: null,
      },
    })),
  selectWire: (wire) => set({ selection: { nodes: [], wire, element: null } }),
  selectElement: (pick) =>
    set({
      selection: {
        nodes: pick?.node ? [pick.node] : [],
        wire: null,
        element: pick,
      },
    }),
  clearSelection: () => set({ selection: { nodes: [], wire: null, element: null } }),
  setHoverPick: (pick) => set({ hoverPick: pick }),
  clearProbe: () => set({ probe: null }),

  trackPendingValue: (node, port, value) =>
    set((state) => {
      const entry = state.pending;
      if (entry === null || !pendingIs(entry, node, port) || entry.value === value) return state;
      return { pending: { ...entry, value } };
    }),
  endDrag: (node, port) => {
    set((state) => (pendingIs(state.pending, node, port) ? { pending: null } : state));
    const state = get();
    if (canWrite(state)) state.send({ type: "end_drag", payload: { node, port } });
  },

  addNotice: (level, message) =>
    set((state) => ({
      notices: [...state.notices.slice(-19), { id: ++noticeCounter, level, message, at: Date.now() }],
    })),
  dismissNotice: (id) => set((state) => ({ notices: state.notices.filter((n) => n.id !== id) })),

  updateSettings: (patch) =>
    set((state) => {
      const settings = { ...state.settings, ...patch };
      try {
        localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
      } catch {
        // storage unavailable — settings stay in memory
      }
      return { settings };
    }),

  openSearch: (anchor) => set({ search: anchor }),
  closeSearch: () => set({ search: null }),
  clearRunNotice: () => set({ runNotice: null }),
}));

/** The store's record of an `error` payload (flattened details named). */
export function lastErrorOf(p: ErrorPayload): LastError {
  const error: LastError = { kind: p.kind, message: p.message };
  if (p.intent_id !== undefined) error.intentId = p.intent_id;
  if (p.current_text_hash !== undefined) error.currentTextHash = p.current_text_hash;
  if (p.diagnostics !== undefined) error.diagnostics = p.diagnostics;
  if (p.index !== undefined) error.index = p.index;
  return error;
}

/**
 * The notice level an `error` kind deserves: `nothing_to_undo` /
 * `nothing_to_redo` are informational (Ctrl+Z on an empty stack is not a
 * fault — and the server's reason, e.g. the reload barrier, still shows);
 * everything else is an error.
 */
export function errorNoticeLevel(kind: ErrorKind): Notice["level"] {
  return kind === "nothing_to_undo" || kind === "nothing_to_redo" ? "info" : "error";
}

/** Keep only the entries whose key is a live binding. */
export function pruneKeys<T>(record: Record<string, T>, live: Set<string>): Record<string, T> {
  let changed = false;
  const out: Record<string, T> = {};
  for (const [key, value] of Object.entries(record)) {
    if (live.has(key)) out[key] = value;
    else changed = true;
  }
  return changed ? out : record;
}

// ------------------------------------------------------------- selectors --

/** Node view by name (or undefined). */
export function nodeByName(graph: GraphView, name: string) {
  return graph.nodes.find((n) => n.name === name);
}

/** Node view by ref (frames name nodes by ref). */
export function nodeByRef(graph: GraphView, ref: number) {
  return graph.nodes.find((n) => n.ref === ref);
}

/** This param's compute-on-release entry, or undefined when it is not the pending param. */
export function pendingFor(
  state: Pick<CicadaState, "pending">,
  node: string,
  port: string | null | undefined,
): PendingParam | undefined {
  const { pending } = state;
  return pending !== null && pendingIs(pending, node, port) ? pending : undefined;
}

/** Am I the writer? (Display only — every write is gated by `canWrite`.) */
export function isWriter(state: Pick<CicadaState, "role">): boolean {
  return state.role === "writer";
}

/**
 * May this client send a write intent right now? Holding the lease is not
 * enough: the socket must be open — a dropped socket clears the role, and
 * this is the ONE predicate every write affordance (canvas gestures,
 * inspector buttons, params, ribbon placement, hotkeys) checks.
 */
export function canWrite(state: Pick<CicadaState, "role" | "connection">): boolean {
  return state.connection === "open" && state.role === "writer";
}

/** Why a write is refused right now (for the read-only notices); null when it is allowed. */
export function writeBlockReason(state: Pick<CicadaState, "role" | "connection">): string | null {
  if (state.connection !== "open") return "not connected";
  if (state.role !== "writer") return "read-only observer";
  return null;
}

/**
 * The notice a lease change deserves: losing the write lease is loud (the
 * demoted writer must know their edits stopped landing); gaining it is an
 * info line. Same role → nothing.
 */
export function roleChangeNotice(
  before: Role,
  after: Role,
  lease: LeaseView,
): { level: Notice["level"]; message: string } | null {
  if (before === after) return null;
  if (after === "observer") {
    const holder = lease.writer === null ? "nobody" : `client #${lease.writer}`;
    return { level: "warning", message: `write lease taken by ${holder} — you are read-only now` };
  }
  return { level: "info", message: "you now hold the write lease" };
}
