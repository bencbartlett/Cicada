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
  GraphView,
  LeaseView,
  NodeStatus,
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

const EMPTY_GRAPH: GraphView = { nodes: [], wires: [], diagnostics: [] };
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
  lastError: { intentId?: string; kind: string; message: string } | null;
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
  lastError: null,
  snapshots: 0,
  displayGeneration: 0,
  displayResets: 0,

  catalog: null,
  nodeValues: {},
  wireValues: {},
  probe: null,

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
          snapshots: state.snapshots + 1,
          selection: {
            nodes: state.selection.nodes.filter((n) => live.has(n)),
            wire: null,
            element: null,
          },
          nodeValues: {},
          wireValues: {},
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
        set({
          seq,
          graph: p.graph,
          text: p.text,
          dirty: p.dirty,
          lastDeltaLabel: p.source.label,
          selection: {
            ...state.selection,
            nodes: state.selection.nodes.filter((n) => live.has(n)),
            wire: p.graph.wires.some((w) => w.id === state.selection.wire)
              ? state.selection.wire
              : null,
          },
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
        set({ lastError: { intentId: p.intent_id, kind: p.kind, message: p.message } });
        get().addNotice("error", p.message);
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

// ------------------------------------------------------------- selectors --

/** Node view by name (or undefined). */
export function nodeByName(graph: GraphView, name: string) {
  return graph.nodes.find((n) => n.name === name);
}

/** Node view by ref (frames name nodes by ref). */
export function nodeByRef(graph: GraphView, ref: number) {
  return graph.nodes.find((n) => n.ref === ref);
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
