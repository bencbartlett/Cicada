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
  /**
   * A refused transport control: a seek outside the loop, a speed that is
   * not positive and finite or is above 64× (the server's `MAX_SPEED`). No
   * `transport` broadcast follows a refusal — this error is the whole answer.
   */
  | "transport"
  /**
   * The handshake's verdict on the pipeline the socket named (docs/13
   * §Projects, pipelines, sessions): the server cannot open it — `reason`
   * says why, `pipeline` is the reference as sent — and closes. Terminal:
   * the connection module schedules no reconnect, shows the reason, drops
   * a `not_found` file from Recent and returns the tab to the picker.
   */
  | "pipeline"
  | (string & {});

/** Why the server refused a socket's pipeline (`protocol::JoinRefusal`, snake_case; the HTTP routes' 400 / 400 / 404 / 422). */
export type JoinRefusal = "unnamed" | "path_not_allowed" | "not_found" | "open_failed";

/**
 * The `error` payload: `kind` + `message`, plus the kind-specific facts the
 * server flattens in (`current_text_hash` on `stale_base`, `diagnostics` on
 * `parse_error`, `index` = the failing op of a `batch`, `pipeline` +
 * `reason` on the handshake's `pipeline` refusal).
 */
export interface ErrorPayload {
  intent_id?: string;
  kind: ErrorKind;
  message: string;
  current_text_hash?: string;
  diagnostics?: Diagnostic[];
  index?: number;
  pipeline?: string;
  reason?: JoinRefusal;
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

// ------------------------------------------------------------- transport --
//
// The session's transport (docs/13 §Animation transport; DECISIONS.md time
// row; `protocol.rs` `TransportView` & co. — the test
// `transport_messages_have_the_documented_shapes` is the byte-level spec).
// Additive (v0.1 item 4): `PROTOCOL_VERSION` stays 1.

/**
 * The transport signal behind a driven port (`protocol::DrivenSignal`; the
 * same spelling as `catalog.json`'s `transport_driven`): a quantized loop
 * frame (`cycle.frame`) or the playhead in seconds (`clock.t`).
 */
export type DrivenSignal = "frame" | "time";

/**
 * A frame loop as the server states it (`protocol::LoopView`): `frames`
 * over `period_ms` — `period_ms` IS the lowering's `period × 1000`, so
 * `frameAt` (`state/transport.ts`) and the server's quantization agree in
 * the doubles.
 */
export interface LoopView {
  /** Frames per loop (> 0). */
  frames: number;
  /** The loop's period in milliseconds (> 0). */
  period_ms: number;
}

/**
 * One port the transport drives in the current graph (`protocol::DrivenView`).
 * A `frame` port carries its OWN `loop` — this node's `frames` / `period_ms`
 * (its literals, or `cycle`'s defaults), the numbers the lowering quantized
 * its frame from: "the frame this port is fed" is `frameAt(t_ms, loop.frames,
 * loop.period_ms)` on THIS loop, never on the primary loop's
 * (`TransportView.frames` / `period_ms` are only the scrubber's — a second
 * `cycle` loops inside the primary at its own rate). A `time` port carries
 * none: it is fed the playhead in seconds.
 */
export type DrivenView =
  | {
      /** The binding. */
      node: string;
      /** The port (`frame`). */
      port: string;
      signal: "frame";
      loop: LoopView;
    }
  | {
      node: string;
      /** The port (`t`). */
      port: string;
      signal: "time";
      loop?: undefined;
    };

/**
 * The transport as every client sees it (`protocol::TransportView`): in
 * every `snapshot` as `payload.transport` and the whole payload of the
 * `transport` broadcast after every ACCEPTED control (a refused one changes
 * nothing and broadcasts nothing — the refusing client gets its `error`,
 * kind `transport` or `lease`, and that is the whole answer), after Esc,
 * when the last client's departure paused playback, and when an edit or
 * reload changed the loop or the driven set. REPLACE the client's state
 * with it, never stack. Server-authoritative: the clock is the session's,
 * so observers see the writer's animation. The view is a position at the
 * moment of the message — while `playing` the client extrapolates
 * `t_ms + elapsed × speed` for its own playhead display (`state/transport.ts`)
 * and trusts the next broadcast; the geometry frames themselves arrive as
 * ordinary display frames from the transport's generations. Deltas carry
 * no transport.
 */
export interface TransportView {
  playing: boolean;
  /** Playhead milliseconds per wall millisecond (> 0). */
  speed: number;
  /** The playhead, milliseconds, unbounded (0 after `transport_reset`; `clock`'s `t` is this over 1000). */
  t_ms: number;
  /** The primary loop's frame at `t_ms`: `floor(t × frames / period) mod frames` — what the scrubber shows and `transport_seek` addresses. */
  frame: number;
  /** The primary loop's length — the `cycle` with the longest `period` (ties: first in the text), else cycle's defaults (120). */
  frames: number;
  /** The primary loop's period in milliseconds (default 4000). */
  period_ms: number;
  /** Every `cycle.frame` / `clock.t` that lowered. Empty = the pipeline has no time params: playback moves nothing, the bar is not shown. */
  driven: DrivenView[];
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
  /**
   * The session's transport owns this INPUT port (`cycle.frame` = `frame`,
   * `clock.t` = `time`; `PortSpec::transport_driven`): the lowering fills
   * it from the playhead, whatever the text says. The app hides it — no
   * handle, no wire target, no literal editor, on the canvas and in the
   * inspector (`transportDrivenSignal`); it never reaches the text unless
   * a human wrote the kwarg by hand, and then that is the headless value.
   * Absent on every other port.
   */
  transport_driven?: DrivenSignal;
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

// ------------------------------------------------------------------- git --
//
// The git panel's HTTP shapes (docs/13 §HTTP surface `/api/git/*`, doc 10
// §Git integration; `protocol.rs` `// --- git --`). HTTP-only — no WS
// message: the client reads `GET /api/git/status` (on connect, ≤1/s after
// writes, on window focus — `state/git.ts`) and dedupes on `text_hash`.

/** A multi-step git operation the shell left unfinished (`protocol::Operation`). */
export type GitOperation = "merge" | "rebase" | "cherry_pick" | "revert";

/** A branch's upstream and the ahead/behind counts (`protocol::Upstream`). */
export interface GitUpstream {
  name: string;
  ahead: number;
  behind: number;
}

/** The facts of the repository the project lives in (`protocol::RepoInfo`). */
export interface RepoInfo {
  /** As git reports it — informational only, never joined with paths (the server's cwd view). */
  root: string;
  /** The project dir relative to `root` (`""` when the project IS the root). */
  prefix: string;
  /** `null` = detached HEAD. */
  branch: string | null;
  /** `null` on an unborn branch. */
  head_short: string | null;
  upstream: GitUpstream | null;
  /** `git init` without a commit: everything is `added`, the first commit works. */
  unborn: boolean;
  /** Present only while one is in progress — commit and revert refuse until it is done. */
  operation?: GitOperation;
}

/**
 * Where the project stands in git (`protocol::GitState`, tagged `kind`):
 * `repo` is the normal case; `locked` is the SAME repository with
 * `index.lock` held (status still answers, writes wait — the facts stay so
 * the chip never blanks during the app's own commit); the other two are
 * typed refusals, never strings.
 */
export type GitState =
  | ({ kind: "repo" } & RepoInfo)
  | ({ kind: "locked" } & RepoInfo)
  | { kind: "not_a_repo" }
  | { kind: "git_not_found" };

/** The repository facts of `repo` and `locked` (mirrors `GitState::repo`). */
export function gitRepoInfo(state: GitState): RepoInfo | null {
  return state.kind === "repo" || state.kind === "locked" ? state : null;
}

/** How a node's binding line differs from HEAD (`protocol::ChangeKind`). */
export type ChangeKind = "added" | "modified" | "removed" | "renamed";

/** One node's change marker (`protocol::NodeChange`). */
export interface NodeChange {
  name: string;
  change: ChangeKind;
  /** `renamed`: the name the binding had in HEAD. */
  from?: string;
}

/** A binding HEAD has that the working tree lost (`protocol::RemovedNode`). */
export interface RemovedNode {
  name: string;
  /** 1-based line in `HEAD:<path>`. */
  line_in_head: number;
}

/** A file's git status in the commit scope (`protocol::FileStatus`). */
export type FileStatus = "modified" | "added" | "deleted" | "untracked" | "renamed";

/** One dirty file of the commit scope, project-relative (`protocol::ScopeFile`). */
export interface ScopeFile {
  path: string;
  status: FileStatus;
  /**
   * HEAD has a version of this path, so a revert can put it back — the
   * SERVER's rule, published per file (`git.rs` `Entry::in_head` + the
   * unborn branch). Never infer it from `status`: porcelain `AD` (added to
   * the index, then deleted from disk) is `deleted` with no HEAD version.
   * `false` → a revert leaves the file alone, and an explicit ask for it is
   * refused `untracked`.
   */
  in_head: boolean;
}

/** The pipeline file's git view (`protocol::PipelineGitStatus`). */
export interface PipelineGitStatus {
  path: string;
  /** Known to git (HEAD or index). Untracked → every node `added`. */
  tracked: boolean;
  /** Matched by `.gitignore`: git refuses to add it — commit refuses `ignored`, the scope leaves it out. */
  ignored: boolean;
  dirty: boolean;
  nodes: NodeChange[];
  removed: RemovedNode[];
}

/** `GET /api/git/status` (`protocol::GitStatusResponse`). */
export interface GitStatusResponse {
  state: GitState;
  pipeline: PipelineGitStatus;
  /** The dirty files of the commit scope — what `commit` would stage. */
  scope: ScopeFile[];
  /** blake3 hex of the working file the markers were computed against. */
  text_hash: string;
}

/** `POST /api/git/commit` body (`protocol::CommitRequest`). */
export interface CommitRequest {
  /** Verbatim (`--cleanup=verbatim`); blank → `empty_message`. */
  message: string;
  /** The writer's WS client id (or the `X-Cicada-Client` header). */
  client?: number;
}

/** `POST /api/git/commit` → the commit that landed (`protocol::CommitResponse`). */
export interface CommitResponse {
  hash: string;
  short: string;
  summary: string;
  files: string[];
}

/** `POST /api/git/revert` body (`protocol::RevertRequest`). */
export interface RevertRequest {
  /** A subset of the scope; absent = the whole dirty scope. */
  paths?: string[];
  client?: number;
}

/** `POST /api/git/revert` → what went back to HEAD (`protocol::RevertResponse`). */
export interface RevertResponse {
  reverted: string[];
  /** Dirty scope files with no HEAD version, left exactly as they were. */
  untracked: string[];
  /** The session reloaded (one barrier snapshot, `reason: "git revert"`). */
  reloaded: boolean;
}

/**
 * The `kind` of a refused git route (`protocol::GitErrorKind`, snake_case)
 * — the tag of every git-route failure body except the token middleware's
 * text 401. Status codes: docs/13 §HTTP surface.
 */
export type GitErrorKind =
  | "protocol"
  | "no_such_pipeline"
  | "lease"
  | "not_a_repo"
  | "git_not_found"
  | "locked"
  | "nothing_to_commit"
  | "nothing_to_revert"
  | "untracked"
  | "ignored"
  | "operation_in_progress"
  | "empty_message"
  | "path_not_allowed"
  | "git_failed"
  | "git_timeout"
  | "io_error"
  | "reload_failed"
  | "internal"
  /** Client-side only: no JSON body came back (the token middleware's text 401, a network failure). */
  | "transport"
  | (string & {});

/**
 * A git route's refusal body: `{kind, message}` plus the kind-specific facts
 * (`GitRefusal::body`): `path` on `no_such_pipeline` / `lease` (nobody has
 * the pipeline open) / `untracked` / `ignored` / `path_not_allowed`,
 * `operation` on `operation_in_progress`, `command` + `code` + `stderr` on
 * `git_failed`, `command` on `git_timeout`.
 */
export interface GitErrorBody {
  kind: GitErrorKind;
  message: string;
  path?: string;
  operation?: GitOperation;
  command?: string;
  /** `null` = killed by a signal. */
  code?: number | null;
  stderr?: string;
}

/** The git summary on `GET /api/project` (`protocol::ProjectGit`; `kind` = the state's tag, or `error`). */
export interface ProjectGit {
  kind: string;
  branch: string | null;
  /** `git status` entries under the project dir; 0 outside a repo. */
  dirty_count: number;
  error?: string;
}

// ------------------------------------------------- the root's file list --

/** An entry of `GET /api/files` (`protocol::FileKind`): a directory to descend into, or a `.cic` to open. */
export type FileKind = "dir" | "pipeline";

/** One entry of `GET /api/files` (`protocol::FileEntry`). */
export interface FileEntry {
  /** The entry's own name (no path). */
  name: string;
  kind: FileKind;
  /** Last modification, milliseconds since the Unix epoch (negative before it). */
  modified_ms: number;
}

/**
 * `GET /api/files?dir=<root-relative>` (`protocol::FilesResponse`; docs/13
 * §HTTP surface): ONE directory under the served root — directories first,
 * then pipelines, each group in case-insensitive name order. `root` is the
 * root directory's own name, never its path; `dir` and `parent` are
 * root-relative, `/`-separated, `""` for the root itself (`parent` is
 * `null` there). A pipeline opens as `?pipeline=<dir>/<name>`.
 */
export interface FilesResponse {
  root: string;
  dir: string;
  parent: string | null;
  entries: FileEntry[];
}

/**
 * The `kind` of a refused `GET /api/files` (`protocol::FilesErrorKind`):
 * `path_not_allowed` 400 (`..`, an absolute path, a drive or UNC prefix, a
 * backslash, a NUL byte, a symlink leaving the root), `not_found` 404 (nothing
 * is there: no such directory, a file, a path through a file, a name the file
 * system cannot hold), `io_error` 403 (the directory exists but could not be
 * read).
 */
export type FilesErrorKind = "path_not_allowed" | "not_found" | "io_error" | (string & {});

/** A refused file listing's body: `{kind, message, path}` — `path` is the `dir` as sent. */
export interface FilesErrorBody {
  kind: FilesErrorKind;
  message: string;
  path: string;
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

/**
 * How a drag's previews are handled (`protocol::PreviewMode`):
 * `compute_on_release` is the only mode the server ever announces — a
 * cheap cone gets no message at all and previews latest-wins as before.
 */
export type PreviewMode = "compute_on_release";

/**
 * The `preview_policy` payload (docs/13 §Slider drags, DECISIONS.md row
 * 39): broadcast ONCE per server-side drag, on its first withheld
 * `param_preview` tick — a cone the cost model predicts at or above
 * `COMPUTE_ON_RELEASE_MS` (1 s) is not previewed; the slider shows the
 * pending value and the estimate, and the value solves once, on release.
 * A drag ends on any write attempt, an Esc, a reload, the pointer's release
 * on the committed value (`end_drag`) or a 300 ms pause, and the next one
 * is announced AGAIN — every arrival is the current verdict: replace the
 * pending state, never stack it. An announced drag's end is announced too
 * (`drag_ended`), except the pause's.
 */
export interface PreviewPolicyPayload {
  /** The param's binding. */
  node: string;
  /** Its kwarg — ABSENT (never `null`) for a bare literal. */
  port?: string;
  mode: PreviewMode;
  /** Predicted wall ms of a live preview (rounded to 0.1); a floor when `rough`. */
  estimate_ms: number;
  /** Some node in the cone has no cost evidence yet — render with `~`, like the ETA. */
  rough: boolean;
  /** The withheld tick's literal; the client tracks later ticks itself. */
  pending_value: string;
}

/**
 * The `drag_ended` payload (docs/13 §Slider drags, contract item 3): an
 * ANNOUNCED drag — one a `preview_policy` went out for — has ended. It
 * follows whatever ended the drag answered with (the delta of a landed
 * write, the snapshot of a reload — a no-op by then), and stands alone for
 * the ends nothing else tells every client about: the pointer released on
 * the committed value (`end_drag`), Esc, a refused write (its `error` is
 * unicast), the writer's departure, a lease handover. A pause longer than
 * the drag gap is NOT announced — the pointer may still be down. Never sent
 * for a drag that stayed live. Clear the pending entry when it names it.
 */
export interface DragEndedPayload {
  /** The param's binding. */
  node: string;
  /** Its kwarg — ABSENT (never `null`) for a bare literal. */
  port?: string;
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
        /** The transport's state at the moment of the snapshot (additive, v0.1 item 4). */
        transport: TransportView;
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
  | { type: "run_finished"; payload: { node: string; ok: boolean; message: string } }
  | { type: "preview_policy"; payload: PreviewPolicyPayload }
  | { type: "drag_ended"; payload: DragEndedPayload }
  /** The transport changed (docs/13 §Animation transport): the same view every snapshot carries — replace, never stack. */
  | { type: "transport"; payload: TransportView };

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
  /**
   * The handshake. `role: "observer"` is the join hint (docs/13 §Projects,
   * pipelines, sessions; wave 4 O3): this socket joins as a DECLARED
   * observer that never holds the write lease — not at the join even on a
   * free lease, not by promotion when the writer leaves, and its
   * `take_lease` is refused (kind `lease`). The pop-out viewport
   * (`?view=viewport`) sends it; the main window sends no `role` and joins
   * by the first-client-writes rule. Additive: the field is absent unless
   * asked for.
   */
  | { type: "hello"; payload: { v: number; role?: Role } }
  | GestureMessage
  | { type: "param_preview"; payload: { node: string; port?: string | null; value: string } }
  /**
   * The pointer came up on the committed value — no `set_param` follows,
   * and this is the drag's end (docs/13 §Slider drags): the server ends its
   * drag for this param (a `drag_ended` follows if it was announced) so the
   * next tick is a fresh drag, announced afresh. Both sliders send it on
   * every release that writes nothing; a stale one (the drag already ended
   * by a write, an Esc, a reload) is a no-op server-side, never an error.
   * A write like the ticks (the lease).
   */
  | { type: "end_drag"; payload: { node: string; port: string | null } }
  /** Cancel the running generation (Esc). Also pauses the transport — a `transport` broadcast with `playing: false` follows. */
  | { type: "cancel"; payload: Record<string, never> }
  | TransportMessage
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

/**
 * The five transport controls (docs/13 §Animation transport, the intents
 * table; mirrors `protocol::is_transport`). Writer-only — playback is
 * shared session state every client sees, and the lease is the one arbiter
 * of shared state (an observer's control is refused kind `lease`) — and
 * nothing else: not a gesture (never a `batch` element), never an op
 * (nothing to undo), never a delta, never the file, never a drag-ender
 * (`dragStandsAfter`). An ACCEPTED control is answered by a `transport`
 * broadcast to every client; a refused one by the `error` to this client
 * alone — nothing is broadcast, the view stands. Play, seek and reset
 * paint the frame at once (a generation, a paused seek included).
 */
export type TransportMessage =
  /** The playhead advances from where it stands at `speed`. Idempotent while playing. */
  | { type: "transport_play"; payload: Record<string, never> }
  /** Freeze the playhead. Idempotent while paused. */
  | { type: "transport_pause"; payload: Record<string, never> }
  /** Move the playhead to a frame of the primary loop, `0 ≤ frame < frames` (beyond: refused, kind `transport`). */
  | { type: "transport_seek"; payload: { frame: number } }
  /**
   * The playback rate, playhead ms per wall ms — finite, > 0 and at most 64
   * (the server's `MAX_SPEED`; else refused, kind `transport`). The play
   * bar's menu stops at 4×.
   */
  | { type: "transport_speed"; payload: { factor: number } }
  /** Pause and rewind to `t_ms = 0`: frame 0, `clock` at 0 — the values a headless run evaluates. */
  | { type: "transport_reset"; payload: Record<string, never> };

export type ClientEnvelope = { v: number; id?: string } & ClientMessage;

/** Intents that need the write lease (mirrors `protocol::is_write`). */
export function isWrite(message: ClientMessage): boolean {
  if (isGesture(message) || isTransport(message)) return true;
  switch (message.type) {
    case "param_preview":
    case "end_drag":
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

/** Is this intent a transport control? (Mirrors `protocol::is_transport`: a write, never a gesture.) */
export function isTransport(message: ClientMessage): message is TransportMessage {
  switch (message.type) {
    case "transport_play":
    case "transport_pause":
    case "transport_seek":
    case "transport_speed":
    case "transport_reset":
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
