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
        barrier: boolean;
        reason: string;
      };
    }
  | {
      type: "delta";
      payload: { source: DeltaSource; graph: GraphView; text: string; dirty: string[] };
    }
  | {
      type: "status";
      payload: { generation: number; nodes: Record<string, NodeStatus>; summary: SolveSummary };
    }
  | { type: "lease"; payload: { lease: LeaseView; role: Role } }
  | { type: "error"; payload: { intent_id?: string; kind: string; message: string } }
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

export type ClientMessage =
  | { type: "hello"; payload: { v: number } }
  | {
      type: "place_node";
      payload: { func: string; cell?: [number, number] | null; connect?: ConnectSpec | null };
    }
  | { type: "connect"; payload: { from: WireEnd; to: WireEnd; lift?: boolean } }
  | { type: "disconnect"; payload: { to: WireEnd } }
  | { type: "accept_lift"; payload: { node: string; port: string } }
  | { type: "set_param"; payload: { node: string; port?: string | null; value: string } }
  | { type: "param_preview"; payload: { node: string; port?: string | null; value: string } }
  | { type: "rename"; payload: { node: string; new: string } }
  | { type: "delete_node"; payload: { node: string } }
  | { type: "move_node"; payload: { node: string; cell?: [number, number] | null } }
  | { type: "set_preview"; payload: { node: string; on?: boolean | null } }
  | { type: "cancel"; payload: Record<string, never> }
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
  switch (message.type) {
    case "place_node":
    case "connect":
    case "disconnect":
    case "accept_lift":
    case "set_param":
    case "param_preview":
    case "rename":
    case "delete_node":
    case "move_node":
    case "set_preview":
    case "cancel":
      return true;
    default:
      return false;
  }
}
