/**
 * JSON control-plane shapes (docs/13). Mirrors
 * `crates/cicada-server/src/protocol.rs` and `viewmodel.rs` field for field —
 * the server is the authority; the client renders these and never invents
 * structure. Envelope: server → `{v, seq, type, payload}`, client →
 * `{v, id?, type, payload}`.
 */

export type Role = "writer" | "observer";

/** The scheduler's status vocabulary (docs/16): one vocabulary everywhere. */
export type NodeState =
  | "idle"
  | "cached"
  | "queued"
  | "running"
  | "done"
  | "red"
  | "blocked"
  | "cancelled";

export interface NodeStatus {
  state: NodeState;
  generation: number;
  elements_done?: number;
  elements?: number;
  nanos?: number;
  message?: string;
  element_ids?: number[];
}

export interface SolveSummary {
  generation: number;
  running: boolean;
  cancelled: boolean;
  computed: number;
  cached: number;
  pending: number;
  red: number;
  blocked: number;
  elapsed_ms: number;
  eta_ms?: number;
  eta_rough: boolean;
}

export interface ValueSummary {
  kind: string;
  hash: string;
  count?: number;
  absent?: number;
  axis?: string;
  bounds?: [[number, number, number], [number, number, number]];
  samples?: string[];
  facts?: Record<string, unknown>;
}

export interface LeaseView {
  writer: number | null;
  clients: [number, Role][];
}

export interface DeltaSource {
  client: number | null;
  intent_id?: string;
  label: string;
}

/**
 * Who made an edit (docs/13 §Undo/redo; `protocol::Actor`). The server
 * always writes the `prompt` key on an agent (`null` when it has none).
 */
export type Actor = { kind: "human" } | { kind: "agent"; prompt: string | null };

/**
 * The undo/redo state carried on every `delta` and `snapshot`
 * (`protocol::HistoryView`): `depth` is the op cursor (undoable steps); the
 * labels are those of the op `undo` / `redo` would apply next.
 */
export interface HistoryView {
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
  depth: number;
}

/** One file of an `apply_text` edit (`protocol::FileText`). */
export interface FileText {
  /** Project-relative, `/`-separated: this pipeline, its sidecar, or `scripts/*.py` beside it. */
  path: string;
  text: string;
}

/**
 * The atomic whole-text edit (`protocol::ApplyTextRequest`): refused on a
 * stale `base_text_hash` (`GET /api/edit/text` → `text_hash`) or a text that
 * does not parse; else one persist, one op, one delta.
 */
export interface ApplyTextRequest {
  base_text_hash: string;
  files: FileText[];
  label: string;
  actor: Actor;
}

/**
 * The machine `kind` of an `error` message (`IntentError::kind`) — a string
 * on the wire; these are the documented values (docs/13). A failed `batch`
 * carries the FAILING op's kind plus `index`.
 */
export type ErrorKind =
  | "writer"
  | "lease"
  | "protocol"
  | "unknown"
  | "refused"
  | "persist"
  | "nothing_to_undo"
  | "nothing_to_redo"
  | "stale_base"
  | "parse_error"
  | "path_not_allowed"
  | "io_error"
  | "encode"
  | (string & {});

/**
 * The `error` payload: `kind` + `message`, plus the kind-specific facts the
 * server flattens in (`current_text_hash` on `stale_base`, `diagnostics` on
 * `parse_error`, `index` = the failing op of a `batch`).
 */
export interface ErrorPayload {
  intent_id?: string;
  kind: ErrorKind;
  message: string;
  current_text_hash?: string;
  diagnostics?: Diagnostic[];
  index?: number;
}

/** doc-11 diagnostic (cicada-lang `Diagnostic`). `span.line` is 1-based. */
export interface Diagnostic {
  kind: string;
  node?: string | null;
  span: { line: number; col_start: number; col_end: number };
  message: string;
  expected?: string | null;
  actual?: string | null;
  fix?: { label: string; replacement?: string | null } | null;
}

// ------------------------------------------------------------ view-model --

/**
 * What a node renders as. `disabled` = a `#off` ghost: when its body parses
 * the view keeps `func`, `inputs`, `outputs`, `param` and its wires — only
 * the kind (and the `excluded` reason) says it is off; a body that does not
 * parse is a port-less ghost showing its text.
 */
export type NodeKind = "call" | "literal" | "expression" | "broken" | "disabled";

export interface WireEnd {
  node: string;
  port: string;
}

export interface InputView {
  name: string;
  type: string;
  base: string;
  depth: number;
  optional: boolean;
  required: boolean;
  default?: string;
  doc?: string;
  dimension?: "length" | "angle";
  wired?: WireEnd;
  literal?: string;
  literal_value?: number | boolean | string;
  lift: number;
  unknown?: boolean;
  span?: [number, number];
}

export interface OutputView {
  name: string;
  type: string;
  resolved?: string;
  base: string;
  displayable: boolean;
}

export interface ParamView {
  kind: "slider" | "toggle" | "number" | "integer" | "boolean" | "text" | "list";
  port?: string;
  value: number | boolean | string;
  min?: number;
  max?: number;
  step?: number;
}

export interface ExcludedView {
  status: "red" | "blocked";
  reason: string;
}

export interface NodeView {
  ref: number;
  name: string;
  targets: string[];
  /** 0-based line index in the file (diagnostics' `span.line` is 1-based). */
  line: number;
  text: string;
  kind: NodeKind;
  func?: string;
  title: string;
  category: string;
  description?: string;
  panics?: string;
  inputs: InputView[];
  outputs: OutputView[];
  param?: ParamView;
  comment?: string;
  diagnostics: Diagnostic[];
  excluded?: ExcludedView;
  effectful: boolean;
  preview: boolean;
  /** Grid cell [x, y] in units. */
  cell: [number, number];
  /** Size [w, h] in units. */
  size: [number, number];
  manual: boolean;
}

export interface WireView {
  id: string;
  from: WireEnd;
  to: WireEnd;
  lift: number;
  type?: string;
  depth: number;
  red: boolean;
  reason?: string;
}

export interface GraphView {
  nodes: NodeView[];
  wires: WireView[];
  diagnostics: Diagnostic[];
  dialect?: number;
}

// --------------------------------------------------------------- catalog --

/**
 * One port of a catalog entry (`crates/cicada-server/src/catalog.rs::Port`,
 * catalog format 2). `doc` is the port's one-line doc — for a node that
 * returns one bare value it is the `# Returns` line (docs/14 §node file
 * format); the server omits the key when the doc is empty.
 */
export interface CatalogPort {
  name: string;
  type: string;
  base: string;
  list_depth: number;
  optional: boolean;
  default?: string;
  doc?: string;
  dimension?: string;
}

/**
 * One node of `GET /api/catalog` (`catalog.rs::Node`, format 2). `gh` and
 * `examples` are ALWAYS written by the server (the `#[node]` attribute
 * requires `gh = "…" | none`): `gh` is the Grasshopper component this node
 * replaces — `null` for a Cicada-only node — and feeds search-to-place;
 * `examples` are runnable `.cic` snippets (no `# cicada 1` header) that CI
 * solves, empty for the project's script nodes.
 */
export interface CatalogNode {
  name: string;
  title: string;
  description: string;
  category: string;
  tier: string;
  version: number;
  pure: boolean;
  uses_tolerance: boolean;
  panics?: string;
  gh: string | null;
  examples: string[];
  inputs: CatalogPort[];
  outputs: CatalogPort[];
}

export interface Catalog {
  format: number;
  nodes: CatalogNode[];
}

// ------------------------------------------------------- server messages --

export interface ProbeVerdict {
  node: string;
  port: string;
  verdict: "ok" | "lift" | "blocked";
  reason?: string;
}

export interface ProbeCatalogEntry {
  func: string;
  ports: [string, "ok" | "lift"][];
}

export type ServerMessage =
  | {
      type: "hello";
      payload: {
        client_id: number;
        role: Role;
        protocol: number;
        engine: string;
        project: string;
        pipeline: string;
        unit_px: number;
      };
    }
  | {
      type: "snapshot";
      payload: {
        graph: GraphView;
        text: string;
        statuses: Record<string, NodeStatus>;
        summary: SolveSummary;
        lease: LeaseView;
        /** This snapshot follows an external file change — the reload barrier: the op log was cleared. */
        barrier: boolean;
        reason: string;
        history: HistoryView;
      };
    }
  | {
      type: "delta";
      payload: {
        source: DeltaSource;
        graph: GraphView;
        text: string;
        dirty: string[];
        /** The undo/redo state AFTER this op. */
        history: HistoryView;
      };
    }
  | {
      type: "status";
      payload: { generation: number; nodes: Record<string, NodeStatus>; summary: SolveSummary };
    }
  | { type: "lease"; payload: { lease: LeaseView; role: Role } }
  | { type: "error"; payload: ErrorPayload }
  | {
      type: "wire_probe";
      payload: {
        intent_id: string | null;
        from: WireEnd;
        targets: ProbeVerdict[];
        catalog: ProbeCatalogEntry[];
      };
    }
  | {
      type: "node_values";
      payload: { node: string; outputs: [string, ValueSummary | null][]; generation: number };
    }
  | {
      type: "wire_values";
      payload: { to: WireEnd; from: WireEnd; summary: ValueSummary | null; pairing: string };
    }
  | { type: "screenshot_request"; payload: { id: number; target: string } }
  | { type: "notice"; payload: { level: "info" | "warning" | "error"; message: string } }
  | { type: "display_reset"; payload: { generation: number } }
  | { type: "run_finished"; payload: { node: string; ok: boolean; message: string } };

export type ServerEnvelope = { v: number; seq: number } & ServerMessage;

// ------------------------------------------------------- client messages --

export interface ConnectSpec {
  from: WireEnd;
  to_port: string;
  lift?: boolean;
}

/**
 * A canvas write gesture — one that edits the text or sidecar in place and
 * may be an element of a `batch` (mirrors `protocol::is_gesture`). The
 * server validates this at runtime; the type narrows it at compile time so
 * a preview, cancel, undo/redo, nested batch or `apply_text` can never be
 * built into one.
 */
export type GestureMessage =
  | {
      type: "place_node";
      payload: { func: string; cell?: [number, number] | null; connect?: ConnectSpec | null };
    }
  | { type: "connect"; payload: { from: WireEnd; to: WireEnd; lift?: boolean } }
  | { type: "disconnect"; payload: { to: WireEnd } }
  | { type: "accept_lift"; payload: { node: string; port: string } }
  | { type: "set_param"; payload: { node: string; port?: string | null; value: string } }
  | { type: "rename"; payload: { node: string; new: string } }
  | { type: "delete_node"; payload: { node: string } }
  /**
   * Toggle `#off` on a node (docs/10 gesture table): a live statement
   * becomes a ghost — ports and wiring intact, skipped in solves, downstream
   * red as "disabled" — and a ghost becomes live again (usually a cache
   * hit). The server labels the delta `disable x` / `enable x`.
   */
  | { type: "toggle_disable"; payload: { node: string } }
  | { type: "move_node"; payload: { node: string; cell?: [number, number] | null } }
  | { type: "set_preview"; payload: { node: string; on?: boolean | null } };

export type ClientMessage =
  | { type: "hello"; payload: { v: number } }
  | GestureMessage
  | { type: "param_preview"; payload: { node: string; port?: string | null; value: string } }
  | { type: "cancel"; payload: Record<string, never> }
  /** Restore the last op's `before` snapshot (a write; the delta is labelled `undo: <label>`). */
  | { type: "undo"; payload: Record<string, never> }
  /** Re-apply the last undone op's `after` snapshot. */
  | { type: "redo"; payload: Record<string, never> }
  /**
   * Several gestures as ONE op: applied in order under the session lock,
   * all or nothing (the error names the failing `index`); one persist, one
   * op, one delta — so one Ctrl+Z undoes a multi-move / multi-delete /
   * reconnect.
   */
  | { type: "batch"; payload: { ops: GestureMessage[]; label: string } }
  /** Whole files atomically (agents / MCP) — same atomicity as `batch`. */
  | { type: "apply_text"; payload: ApplyTextRequest }
  | { type: "inspect"; payload: { node: string } }
  | { type: "inspect_wire"; payload: { to: WireEnd } }
  | { type: "probe_wire"; payload: { from: WireEnd } }
  | { type: "resync_display"; payload: Record<string, never> }
  | { type: "take_lease"; payload: Record<string, never> }
  | {
      type: "screenshot";
      payload: { id: number; png_base64?: string | null; error?: string | null };
    };

export type ClientEnvelope = { v: number; id?: string } & ClientMessage;

/** Intents that need the write lease (mirrors `protocol::is_write`). */
export function isWrite(message: ClientMessage): boolean {
  if (isGesture(message)) return true;
  switch (message.type) {
    case "param_preview":
    case "cancel":
    case "undo":
    case "redo":
    case "batch":
    case "apply_text":
      return true;
    default:
      return false;
  }
}

/**
 * Is this intent a canvas write gesture — one that may be an element of a
 * `batch`? (Mirrors `protocol::is_gesture`: previews, cancel, undo/redo,
 * batch itself and `apply_text` are writes but not gestures.)
 */
export function isGesture(message: ClientMessage): message is GestureMessage {
  switch (message.type) {
    case "place_node":
    case "connect":
    case "disconnect":
    case "accept_lift":
    case "set_param":
    case "rename":
    case "delete_node":
    case "toggle_disable":
    case "move_node":
    case "set_preview":
      return true;
    default:
      return false;
  }
}

/**
 * N gestures as the ONE intent that makes them one undo step: a single
 * gesture goes as itself (the server labels it — `delete x`, `move x`);
 * two or more go as a `batch` under `label`. An empty list is a programming
 * error and throws — the server would refuse it anyway, and a gesture site
 * with nothing to send must not send.
 */
export function asOneOp(ops: readonly GestureMessage[], label: string): ClientMessage {
  const first = ops[0];
  if (first === undefined) throw new Error(`asOneOp(${JSON.stringify(label)}): no gestures`);
  if (ops.length === 1) return first;
  return { type: "batch", payload: { ops: [...ops], label } };
}
