//! The JSON control plane (docs/13 §State ownership and sync): a versioned
//! envelope `{v, seq, type, payload}` from the server, `{v, id, type,
//! payload}` intents from the client. Debuggability beats compactness at
//! these sizes; geometry travels as binary frames ([`crate::frames`]).
//!
//! The client mirrors these shapes in `web/src/protocol/messages.ts`;
//! `PROTOCOL_VERSION` bumps together on both sides (a mismatch is refused
//! at `hello`, never guessed around).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::viewmodel::{GraphView, WireEnd};

/// Control-plane version. 1 = the stage-5 protocol.
pub const PROTOCOL_VERSION: u32 = 1;

/// A client's role on a session (single-writer lease, docs/13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Holds the write lease.
    Writer,
    /// Live read-only observer.
    Observer,
}

/// The scheduler's status vocabulary (docs/16 — one vocabulary everywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    /// Not part of any solve yet (or excluded).
    Idle,
    /// Answered by the memo table.
    Cached,
    /// In the cone of the running generation, not started.
    Queued,
    /// Executing.
    Running,
    /// Computed this generation.
    Done,
    /// Failed.
    Red,
    /// An upstream is red; did not run.
    Blocked,
    /// The generation was cancelled before/while it ran.
    Cancelled,
}

/// One node's status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeStatus {
    /// The state word.
    pub state: NodeState,
    /// Generation this status belongs to.
    pub generation: u64,
    /// Elements processed so far / total (fan-out nodes while running).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elements_done: Option<u64>,
    /// Elements processed: this generation's count for a `done` node; for
    /// a `cached` node whose memo entry recorded its cost (node-level
    /// entries since v0.1 item 3b), the count of the LAST compute — the
    /// ETA's per-node element counts survive a warm reopen through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elements: Option<u64>,
    /// Measured work nanoseconds (CPU, summed across chunks): this
    /// generation's for a `done` node; for a `cached` node, what the LAST
    /// compute of that key cost (docs/12 §Progress: the badge shows the
    /// "last compute time") — never this generation's, which paid a
    /// cache read. Render it as "last" next to `cached`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nanos: Option<u64>,
    /// Failure message (red) or reason (blocked: "fed by red `x`").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Offending element indices (red fan-outs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub element_ids: Vec<usize>,
}

impl NodeStatus {
    /// A bare status.
    #[must_use]
    pub fn new(state: NodeState, generation: u64) -> Self {
        Self {
            state,
            generation,
            elements_done: None,
            elements: None,
            nanos: None,
            message: None,
            element_ids: Vec::new(),
        }
    }
}

/// The global solve bar (docs/16 §Status and progress language).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SolveSummary {
    /// The generation these numbers describe.
    pub generation: u64,
    /// A generation is in flight.
    pub running: bool,
    /// The generation ended cancelled.
    pub cancelled: bool,
    /// Nodes computed this generation.
    pub computed: usize,
    /// Nodes served from the memo table.
    pub cached: usize,
    /// Nodes still queued/running.
    pub pending: usize,
    /// Red nodes.
    pub red: usize,
    /// Blocked nodes.
    pub blocked: usize,
    /// Wall milliseconds since the generation started (or its total).
    pub elapsed_ms: f64,
    /// Cost-weighted ETA in milliseconds from persisted samples, when the
    /// generation is running; `None` = idle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_ms: Option<f64>,
    /// The ETA is a first-run guess (some op has no samples yet — shown
    /// with a `~`, docs/12).
    pub eta_rough: bool,
}

/// A compact description of one value (inspector, wire hover, node-face
/// previews) — computed from the cached value, never a re-solve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueSummary {
    /// The value kind (`Mesh`, `List`, …).
    pub kind: String,
    /// blake3 hex — the interning key.
    pub hash: String,
    /// Element count for lists (top level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// Absent slots in the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absent: Option<usize>,
    /// Named axis, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    /// World bounds `[[minx, miny, minz], [maxx, maxy, maxz]]` for geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[[f64; 3]; 2]>,
    /// First few elements rendered compactly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<String>,
    /// Extra facts (vertex/triangle counts, curve variant, …).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facts: BTreeMap<String, serde_json::Value>,
}

/// The lease state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseView {
    /// The writer's client id, if any.
    pub writer: Option<u32>,
    /// Connected clients (id, role).
    pub clients: Vec<(u32, Role)>,
}

/// Who made an edit (docs/13 §Undo/redo: `human | agent(prompt)`).
/// Serialized as `{"kind":"human"}` / `{"kind":"agent","prompt":…}` — the
/// `prompt` key is always present on an agent (`null` when it has none),
/// so the client mirror reads `prompt: string | null`; on the way in it
/// may be omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    /// A person at the canvas.
    Human,
    /// An agent (MCP / the AI layer) — with the prompt that produced the
    /// edit, when it has one.
    Agent {
        /// The prompt, for the history view.
        #[serde(default)]
        prompt: Option<String>,
    },
}

/// Scrub caching on one slider (docs/12 §Speculative warming; DECISIONS.md
/// row 39; v0.1 item 5 S1, additive) — carried by every slider's
/// `ParamView` as `scrub`, and by the `scrub_progress` broadcast. The
/// server computes everything: eligibility is a pure function of the
/// slider's literals (`crate::scrub`), the warm set is the session's queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrubView {
    /// The text says `scrub=True` (the opt-in is the kwarg, never the
    /// sidecar). `true` on an ineligible slider is a hand-written kwarg the
    /// session warms nothing for.
    pub on: bool,
    /// Step-quantized positions, `floor((max − min) / step) + 1`; 0 when
    /// ineligible.
    pub positions: usize,
    /// Position indices verified warm (a memo hit, or solved by the
    /// warming), ascending.
    pub warmed: Vec<usize>,
    /// Work remains for this slider: not every position is warm and the
    /// cap has not stopped it — the worker will get to it (it may be
    /// waiting for the app to go idle right now).
    pub warming: bool,
    /// Bytes of memo entries the warming stored for this slider, deep.
    pub bytes: u64,
    /// The per-slider byte cap (256 MiB) stopped the warming; the
    /// positions warmed before it stay warm. Omitted when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub capped: bool,
    /// Why this slider cannot scrub-cache — the reason `set_scrub` refuses
    /// with (`too many positions (51 > 32)`, `min is wired — …`, `step is
    /// 0 — …`). Absent when eligible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ineligible: Option<String>,
}

/// The undo/redo state carried on every `delta` and `snapshot` (additive —
/// v0.1 op log, docs/13 §Undo/redo).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryView {
    /// An op can be undone.
    pub can_undo: bool,
    /// An undone op can be redone.
    pub can_redo: bool,
    /// The label of the op `undo` would revert.
    pub undo_label: Option<String>,
    /// The label of the op `redo` would re-apply.
    pub redo_label: Option<String>,
    /// Undoable steps (the cursor's position in the log).
    pub depth: usize,
}

/// One file of an `apply_text` edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileText {
    /// Project-relative path (`/`-separated): this pipeline's `.cic`, its
    /// `.cic.layout.json` sidecar, or `<pipeline dir>/scripts/<name>.py`.
    pub path: String,
    /// The whole new content.
    pub text: String,
}

/// The atomic whole-text edit (agents / MCP; docs/13 §Undo/redo): the
/// files, a label, the actor, and the base text hash the caller read
/// (`GET /api/edit/text` → `text_hash`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyTextRequest {
    /// blake3 hex of the pipeline text the caller based its edit on.
    pub base_text_hash: String,
    /// The files to replace — every one written temp + rename, or none.
    pub files: Vec<FileText>,
    /// Human label of the op (`undo: <label>` later).
    pub label: String,
    /// Who made the edit.
    pub actor: Actor,
}

/// Where a delta came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaSource {
    /// The client whose intent produced it (`None` = engine/watcher).
    pub client: Option<u32>,
    /// The intent's client-side id, echoed (the ack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_id: Option<String>,
    /// Human label of the op (`place box`, `set_param size.value`).
    pub label: String,
}

/// A wire-probe verdict for one candidate target port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeVerdict {
    /// Candidate node.
    pub node: String,
    /// Candidate port.
    pub port: String,
    /// `ok` / `lift` / `blocked`.
    pub verdict: String,
    /// The reason (checker message) when not `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A wire-probe verdict for a catalog node's ports (drag-to-empty-canvas
/// search filter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeCatalogEntry {
    /// Dialect name.
    pub func: String,
    /// Ports that would accept the wire: `(port, verdict)` for `ok`/`lift`.
    pub ports: Vec<(String, String)>,
}

// ------------------------------------------------------------- git --
//
// The git panel's HTTP shapes (docs/13 §HTTP surface `/api/git/*`, doc 10
// §Git integration, DECISIONS.md "Git integration is first-class UI").
// HTTP-only — no WS message; the client polls `status` and dedupes on
// `text_hash`. [`crate::git`] produces them over the git binary.

/// Where the project stands in git — the tagged state every git route
/// starts from. `Repo` is the normal case; `Locked` is the same
/// repository with `index.lock` held (the facts stay visible — the branch
/// chip must not blank out during the app's own commit); the other two
/// are typed refusals, never strings. Serialized with the tag `kind` and
/// the [`RepoInfo`] fields flattened beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GitState {
    /// The project lives inside a git work tree.
    Repo(RepoInfo),
    /// The project directory is not inside a git work tree.
    NotARepo,
    /// No `git` binary on PATH.
    GitNotFound,
    /// Another git process holds `index.lock` — status still answers
    /// (nothing here takes the lock), but commit / revert wait. Carries
    /// the same facts as `Repo`.
    Locked(RepoInfo),
}

impl GitState {
    /// The repository facts of `Repo` and `Locked`.
    #[must_use]
    pub fn repo(&self) -> Option<&RepoInfo> {
        match self {
            Self::Repo(info) | Self::Locked(info) => Some(info),
            Self::NotARepo | Self::GitNotFound => None,
        }
    }

    /// The `kind` tag as serialized.
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Repo(_) => "repo",
            Self::NotARepo => "not_a_repo",
            Self::GitNotFound => "git_not_found",
            Self::Locked(_) => "locked",
        }
    }
}

/// The facts of a repository the project lives in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    /// The repository root (as git reports it: absolute, `/`-separated).
    pub root: String,
    /// The project directory relative to `root` (`""` when the project
    /// IS the root; `examples/wall` otherwise — no trailing slash).
    pub prefix: String,
    /// The checked-out branch; `None` when HEAD is detached.
    pub branch: Option<String>,
    /// HEAD's abbreviated commit id; `None` on an unborn branch (the
    /// thing to show when `branch` is `None`).
    pub head_short: Option<String>,
    /// The branch's upstream and how far apart they are, when set.
    pub upstream: Option<Upstream>,
    /// HEAD points at a branch with no commits yet (`git init` without
    /// a commit): everything is `added`, and the first commit works.
    pub unborn: bool,
    /// A merge / rebase / cherry-pick / revert the user started in a
    /// shell and has not finished. Commit and revert refuse
    /// (`operation_in_progress`) — finishing it is the shell's job (doc
    /// 10: branching and merging stay in the shell for v0.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
}

/// A multi-step git operation left in progress (`.git/MERGE_HEAD` etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// `MERGE_HEAD` exists.
    Merge,
    /// `rebase-merge/` or `rebase-apply/` exists.
    Rebase,
    /// `CHERRY_PICK_HEAD` exists.
    CherryPick,
    /// `REVERT_HEAD` exists (a `git revert` of a commit, not ours).
    Revert,
}

impl Operation {
    /// The wire name (`merge`, `rebase`, `cherry_pick`, `revert`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry_pick",
            Self::Revert => "revert",
        }
    }
}

/// A branch's upstream and the ahead/behind counts against it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    /// The upstream ref (`origin/main`).
    pub name: String,
    /// Commits on the branch the upstream lacks.
    pub ahead: u32,
    /// Commits on the upstream the branch lacks.
    pub behind: u32,
}

/// How a node's binding line differs from HEAD (working tree vs HEAD —
/// slice 1 has no other ref). By construction a rendering of `git diff`:
/// a marker exists exactly when a hunk touches the binding's line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// The name is not bound in HEAD.
    Added,
    /// The line changed and the name exists in HEAD.
    Modified,
    /// The name is bound in HEAD and not in the working tree.
    Removed,
    /// A removed + added pair whose right-hand sides are byte-identical.
    Renamed,
}

/// One node's change marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeChange {
    /// The node (binding) name in the working tree.
    pub name: String,
    /// The change.
    pub change: ChangeKind,
    /// For `renamed`: the name the binding had in HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// A binding HEAD has that the working tree no longer does (not part of a
/// rename pair).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedNode {
    /// The name as bound in HEAD.
    pub name: String,
    /// Its 1-based line in `HEAD:<path>` (for "show me what was there").
    pub line_in_head: usize,
}

/// A file's git status in the commit scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// Tracked, content differs from HEAD (index or working tree).
    Modified,
    /// Added to the index, absent in HEAD.
    Added,
    /// Deleted (index or working tree).
    Deleted,
    /// Not known to git. (Ignored files never appear in the scope: git
    /// itself does not list them and `git add` refuses them — an ignored
    /// pipeline is reported through `PipelineGitStatus::ignored`.)
    Untracked,
    /// Renamed in the index.
    Renamed,
}

/// One file of the commit scope that is dirty: this pipeline's `.cic`, its
/// sidecar, or a `scripts/*.py` beside it. Paths are project-relative,
/// `/`-separated — the same currency as `GET /api/edit/text` and
/// `apply_text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeFile {
    /// Project-relative path.
    pub path: String,
    /// Its status.
    pub status: FileStatus,
    /// HEAD has a version of this path, so `revert` can put it back — THE
    /// server's rule (`git.rs` `Entry::in_head`: tracked, not an index
    /// addition, not the new side of a rename, and not on an unborn
    /// branch), published so that no client re-derives it from `status`.
    /// The two do not line up: porcelain `AD` (added to the index, then
    /// deleted from disk) is `deleted` with NO HEAD version; an unmerged
    /// `AA` is `modified` with none. `false` → a revert leaves the file
    /// alone (it never deletes) and an explicit ask for it is refused
    /// `untracked`.
    pub in_head: bool,
}

/// The pipeline file's git view: markers per node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineGitStatus {
    /// Project-relative path of the `.cic`.
    pub path: String,
    /// Known to git (in HEAD or the index). Untracked → every node `added`.
    pub tracked: bool,
    /// Matched by a `.gitignore` rule (and not tracked): git would refuse
    /// to add it, so the scope leaves it out and `commit` refuses
    /// `ignored`. Every node is `added` (there is no HEAD version).
    #[serde(default)]
    pub ignored: bool,
    /// Differs from HEAD (or untracked).
    pub dirty: bool,
    /// Markers for nodes in the working tree (added / modified / renamed).
    pub nodes: Vec<NodeChange>,
    /// Bindings HEAD has that the working tree lost.
    pub removed: Vec<RemovedNode>,
}

/// `GET /api/git/status` → the state, this pipeline's markers, and the
/// dirty files of the commit scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatusResponse {
    /// Where the project stands.
    pub state: GitState,
    /// The pipeline's markers.
    pub pipeline: PipelineGitStatus,
    /// The dirty files of the commit scope (what `commit` would stage).
    pub scope: Vec<ScopeFile>,
    /// blake3 hex of the working pipeline file's bytes — the markers were
    /// computed against exactly this text; clients dedupe on it.
    pub text_hash: String,
}

/// `POST /api/git/commit` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRequest {
    /// The commit message, verbatim (`--cleanup=verbatim`: unicode and a
    /// trailing newline survive). Empty / whitespace-only → `empty_message`.
    pub message: String,
    /// The WS client id of the lease holder (alternative to the
    /// `X-Cicada-Client` header) — committing is a git action on the
    /// project, so it needs the writer (unlike `apply_text`).
    #[serde(default)]
    pub client: Option<u32>,
}

/// `POST /api/git/commit` → the commit that landed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitResponse {
    /// Full commit id.
    pub hash: String,
    /// Abbreviated commit id.
    pub short: String,
    /// The subject line (`%s`).
    pub summary: String,
    /// The files the commit touched, project-relative.
    pub files: Vec<String>,
}

/// `POST /api/git/revert` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertRequest {
    /// The subset of the scope to revert (project-relative); `None` = the
    /// whole dirty scope. A path outside the scope → `path_not_allowed`.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// The WS client id of the lease holder (see [`CommitRequest::client`]).
    #[serde(default)]
    pub client: Option<u32>,
}

/// `POST /api/git/revert` → what was restored to HEAD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertResponse {
    /// The paths `git checkout HEAD --` restored (project-relative).
    pub reverted: Vec<String>,
    /// Dirty scope files with no HEAD version (untracked / index-only),
    /// left exactly as they were — reverting them would mean deleting.
    pub untracked: Vec<String>,
    /// The session reloaded the restored files (one barrier snapshot went
    /// out); `false` when disk already matched memory.
    pub reloaded: bool,
}

/// The `kind` of a refused git route — the `{kind, message, …}` error
/// body's tag (mirrored by the web client).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitErrorKind {
    /// Malformed request body or pipeline reference (400).
    Protocol,
    /// `?pipeline=` names no pipeline of the project (404).
    NoSuchPipeline,
    /// The caller is not the lease holder — or nobody is: no client has
    /// the pipeline open (403).
    Lease,
    /// The project is not in a git work tree (409).
    NotARepo,
    /// No `git` on PATH (409).
    GitNotFound,
    /// `index.lock` is held (423).
    Locked,
    /// Nothing in the scope is dirty (409).
    NothingToCommit,
    /// Nothing in the (requested) scope differs from HEAD (409).
    NothingToRevert,
    /// The pipeline (or a requested path) has no HEAD version (409).
    Untracked,
    /// The pipeline is matched by `.gitignore`: git refuses to add it (409).
    Ignored,
    /// A merge / rebase / cherry-pick / revert is in progress in the
    /// repository — the body carries `operation`; finish it in the shell
    /// (409).
    OperationInProgress,
    /// The commit message is empty (422).
    EmptyMessage,
    /// A requested path is outside the commit scope (422).
    PathNotAllowed,
    /// A git command failed unexpectedly — the body carries `command`,
    /// `code`, `stderr` (500).
    GitFailed,
    /// A git command did not finish within the timeout (500).
    GitTimeout,
    /// Reading the working pipeline (or its HEAD text) failed (500).
    IoError,
    /// The files were restored but the session could not reload them — the
    /// previous state stays live; the message says why (500).
    ReloadFailed,
    /// The server itself failed around the git call (a blocking task that
    /// panicked, an unresolvable project dir) (500).
    Internal,
}

/// The git summary on `GET /api/project` (additive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectGit {
    /// The state's tag (`repo` / `not_a_repo` / `git_not_found` / `locked`).
    pub kind: String,
    /// The branch, when on one.
    pub branch: Option<String>,
    /// Dirty entries under the project directory (`git status` lines,
    /// untracked directories counting once); `0` outside a repo.
    pub dirty_count: usize,
    /// When `kind` is `error`: what git said (the project route itself
    /// still answers — the pipeline list is its job).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What an entry of `GET /api/files` is (docs/13 §HTTP surface; v0.1
/// wave 4 O1): a directory to descend into, or a `.cic` pipeline to open.
/// Declared in listing order — directories sort before pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// A directory under the root (never a dot-directory, `node_modules`
    /// or `target`; never one the OS hides).
    Dir,
    /// A `*.cic` file — openable as `?pipeline=<dir>/<name>`.
    Pipeline,
}

/// One entry of `GET /api/files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// The entry's own name (no path).
    pub name: String,
    /// Directory or pipeline.
    pub kind: FileKind,
    /// Last modification, milliseconds since the Unix epoch (negative
    /// before it — the file system's clock, not ours).
    pub modified_ms: i64,
}

/// The reply of `GET /api/files?dir=<root-relative>`: ONE directory under
/// the served root — directories first, then pipelines, each group in
/// case-insensitive name order. Nothing above the root is ever named:
/// `root` is the root directory's own name (not its path), `dir` and
/// `parent` are root-relative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesResponse {
    /// The root's display name — its last path component (the full path
    /// only for a file-system root, which has none).
    pub root: String,
    /// The listed directory, normalised and root-relative, `/`-separated;
    /// `""` is the root itself.
    pub dir: String,
    /// `dir`'s parent in the same form; `None` for the root.
    pub parent: Option<String>,
    /// The entries, sorted as described.
    pub entries: Vec<FileEntry>,
}

/// The `kind` of a refused `GET /api/files` — the body is `{kind, message,
/// path}` like every git-route refusal; the status codes are docs/13's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesErrorKind {
    /// `dir` is not a plain root-relative path, or resolves outside the
    /// root (`..`, an absolute path, a drive or UNC prefix, a backslash, a
    /// NUL byte, a symlink whose canonical path leaves the root) (400).
    PathNotAllowed,
    /// No such directory under the root — missing, or a file (404).
    NotFound,
    /// The directory exists inside the root but could not be read (403).
    IoError,
}

/// Why a request's pipeline could not be opened — ONE classification for
/// every route that names one (docs/13 §Projects, pipelines, sessions). An
/// HTTP route answers with the status in parentheses; the socket answers
/// its handshake with the `error` of kind `pipeline` carrying this as
/// `reason` beside `pipeline` (the reference as the client sent it) — the
/// upgrade itself is never refused for the pipeline, because a refused
/// upgrade reaches a browser as a bare close code the app could only read
/// as a network drop (wave 4 O2 review). Terminal for the client: a retry
/// changes nothing until the file or the URL does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinRefusal {
    /// The request named no pipeline and the server has no default (400).
    Unnamed,
    /// The reference is not a plain root-relative `.cic` path, or resolves
    /// outside the root (400).
    PathNotAllowed,
    /// No such file under the root — moved, renamed or deleted (404). The
    /// client drops it from Recent.
    NotFound,
    /// The file is there but its session could not open; the message says
    /// why (422).
    OpenFailed,
}

impl ServerMessage {
    /// The handshake's refusal for the pipeline a socket named: kind
    /// `pipeline`, the reason as `reason`, the reference as `pipeline`
    /// (absent when none was named), and the text the routes would have
    /// answered with.
    #[must_use]
    pub fn join_refused(pipeline: Option<&str>, reason: JoinRefusal, message: String) -> Self {
        let mut details = serde_json::Map::new();
        if let Some(pipeline) = pipeline {
            details.insert("pipeline".to_owned(), pipeline.into());
        }
        details.insert(
            "reason".to_owned(),
            serde_json::to_value(reason).unwrap_or(serde_json::Value::Null),
        );
        Self::Error {
            intent_id: None,
            kind: "pipeline".to_owned(),
            message,
            details,
        }
    }
}

/// The session's transport (docs/13 §Animation transport; DECISIONS.md
/// time row) as every client sees it — in every `snapshot` and in the
/// `transport` broadcast after every change (play / pause / seek / speed /
/// reset, Esc, the loop changing under an edit). Server-authoritative:
/// the clock is the session's, so observers see the writer's animation.
/// The view is a position at the moment of the message: while `playing`
/// the client extrapolates `t_ms + elapsed × speed` for its own playhead
/// display; the frames themselves arrive as ordinary display frames from
/// the transport's generations. Additive (v0.1 item 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportView {
    /// The playhead is advancing.
    pub playing: bool,
    /// Playback rate: playhead milliseconds per wall millisecond (> 0).
    pub speed: f64,
    /// The playhead, milliseconds, unbounded (0 after `transport_reset`;
    /// `clock`'s `t` is this over 1000).
    pub t_ms: f64,
    /// The loop's frame at `t_ms`: `floor(t × frames / period) mod frames`
    /// of the primary loop — what a scrubber shows and `transport_seek`
    /// addresses.
    pub frame: u64,
    /// The primary loop's length in frames — the `cycle` with the longest
    /// `period` in the pipeline (ties: the first in the text), or `cycle`'s
    /// defaults (120 frames, 4 s) when no frame-driven node is solvable.
    pub frames: u64,
    /// The primary loop's period in milliseconds.
    pub period_ms: f64,
    /// The ports the transport drives in the current graph (hidden on the
    /// canvas): every `cycle.frame` and `clock.t` that lowered. Empty = the
    /// pipeline has no time params; playback then moves nothing.
    pub driven: Vec<DrivenView>,
}

/// One transport-driven port in [`TransportView::driven`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrivenView {
    /// The binding.
    pub node: String,
    /// The port (`frame`, `t`).
    pub port: String,
    /// Which signal fills it.
    pub signal: DrivenSignal,
    /// The loop a `frame` port quantizes — this node's OWN `frames` and
    /// `period_ms` (its literals, or `cycle`'s defaults), the numbers the
    /// lowering injected its frame from. A client showing "the frame this
    /// port is fed" computes `floor(t_ms × frames / period_ms) mod frames`
    /// on THIS loop, never on the primary loop's (`TransportView::frames`
    /// / `period_ms`), which is only the scrubber's: a second `cycle` loops
    /// inside the primary at its own rate. Absent for `time` ports.
    /// Additive (v0.1 item 4, the web half).
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub r#loop: Option<LoopView>,
}

/// A frame loop as a client sees it ([`DrivenView::loop`]): `frames` over
/// `period_ms` — `period_ms` IS the lowering's `period × 1000`, so the
/// client's quantization and the server's agree in the doubles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopView {
    /// Frames per loop (> 0).
    pub frames: u64,
    /// The loop's period in milliseconds (> 0).
    pub period_ms: f64,
}

/// The transport signal behind a driven port (`catalog.json`'s
/// `transport_driven` spelling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrivenSignal {
    /// A quantized loop frame (`cycle.frame`).
    Frame,
    /// The playhead in seconds (`clock.t`).
    Time,
}

/// Server → client messages.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerMessage {
    /// First message on a socket.
    Hello {
        /// This client's id.
        client_id: u32,
        /// This client's role.
        role: Role,
        /// Protocol version the server speaks.
        protocol: u32,
        /// Engine version string.
        engine: String,
        /// The project directory (display).
        project: String,
        /// The pipeline path relative to the project.
        pipeline: String,
        /// Grid unit hint (px) — the client may override.
        unit_px: u32,
    },
    /// The full authoritative state (initial load, resync, reload
    /// barrier). ONE hydration path.
    Snapshot {
        /// The graph view-model.
        graph: GraphView,
        /// The `.cic` text.
        text: String,
        /// Per-node statuses.
        statuses: BTreeMap<String, NodeStatus>,
        /// The solve bar.
        summary: SolveSummary,
        /// Lease state.
        lease: LeaseView,
        /// True when this snapshot follows an external file change (the
        /// reload barrier — docs/13: the op log was cleared).
        barrier: bool,
        /// Why (external change / initial / resync / an `apply_text` that
        /// changed scripts).
        reason: String,
        /// Undo/redo state (additive, v0.1).
        history: HistoryView,
        /// The transport's state (additive, v0.1 item 4).
        transport: TransportView,
    },
    /// After an applied op: the new graph + text (the spike sends the whole
    /// view-model — hundreds of KB worst case at wall scale — plus the
    /// dirty set; incremental node deltas arrive when a profile asks).
    Delta {
        /// Origin.
        source: DeltaSource,
        /// The graph view-model.
        graph: GraphView,
        /// The `.cic` text.
        text: String,
        /// Bindings whose text changed (the dirty set the solve pulls).
        dirty: Vec<String>,
        /// Undo/redo state after this op (additive, v0.1).
        history: HistoryView,
    },
    /// Coalesced status (≤ 10 Hz) — only changed nodes are listed.
    Status {
        /// The generation.
        generation: u64,
        /// Changed node statuses.
        nodes: BTreeMap<String, NodeStatus>,
        /// The solve bar.
        summary: SolveSummary,
    },
    /// Lease changed.
    Lease {
        /// New lease state.
        lease: LeaseView,
        /// The recipient's role now.
        role: Role,
    },
    /// An intent was refused (or a server-side problem for this client).
    Error {
        /// The intent's id, when it had one.
        #[serde(skip_serializing_if = "Option::is_none")]
        intent_id: Option<String>,
        /// Machine kind (`writer`, `lease`, `protocol`, `unknown`,
        /// `refused`, `persist`, `nothing_to_undo`, `nothing_to_redo`,
        /// `stale_base`, `parse_error`, `path_not_allowed`, `io_error`,
        /// `transport`, `pipeline`).
        kind: String,
        /// Human message.
        message: String,
        /// Kind-specific facts, flattened into the payload (additive):
        /// `current_text_hash` (`stale_base`), `diagnostics` (`parse_error`),
        /// `index` (the failing op of a batch), `pipeline` + `reason` (the
        /// handshake's `pipeline` refusal — [`ServerMessage::join_refused`]).
        #[serde(flatten)]
        details: serde_json::Map<String, serde_json::Value>,
    },
    /// Wire-drag compatibility verdicts (docs/09 blocked-wires contract).
    WireProbe {
        /// The intent's id.
        intent_id: Option<String>,
        /// The source.
        from: WireEnd,
        /// Per existing input port.
        targets: Vec<ProbeVerdict>,
        /// Per catalog node (only nodes with at least one accepting port).
        catalog: Vec<ProbeCatalogEntry>,
    },
    /// A node's current output values (inspector).
    NodeValues {
        /// The node.
        node: String,
        /// Per output port.
        outputs: Vec<(String, Option<ValueSummary>)>,
        /// The generation the values come from.
        generation: u64,
    },
    /// A tapped wire's value + pairing readout.
    WireValues {
        /// The target end.
        to: WireEnd,
        /// The source end.
        from: WireEnd,
        /// The carried value.
        summary: Option<ValueSummary>,
        /// Pairing readout (`each()` depth, counts).
        pairing: String,
    },
    /// Ask a client to render a screenshot (`/debug/screenshot`).
    ScreenshotRequest {
        /// Request id.
        id: u64,
        /// `viewport` (WebGL canvas) — the only client-renderable target.
        target: String,
    },
    /// A notice for the status bar (store recovery notes, watcher events).
    Notice {
        /// `info` / `warning` / `error`.
        level: String,
        /// Text.
        message: String,
    },
    /// The display set is about to be re-streamed to this client (frames
    /// follow) — after `snapshot`.
    DisplayReset {
        /// The generation the frames belong to.
        generation: u64,
    },
    /// Effectful node run finished (POST /api/run/{node}).
    RunFinished {
        /// The node.
        node: String,
        /// Success.
        ok: bool,
        /// Message.
        message: String,
    },
    /// The server's drag policy for one param (DECISIONS.md interactive
    /// param row; docs/13 §Slider drags; v0.1 item 3b, additive): sent ONCE
    /// per drag — on the first `param_preview` tick whose dirty cone the
    /// cost model predicts at or above the compute-on-release threshold —
    /// and never for a cheap cone (those preview live, no message). From
    /// then on the session solves no preview for that param that would
    /// compute (a pure cache read still paints): the client shows the
    /// pending value and the estimate, and the one real `set_param` on
    /// release solves as usual. A drag is the run of ticks on one param
    /// closer together than `DRAG_GAP_MS`; a write attempt, an Esc or a
    /// longer pause ends it, and the next drag is announced again — the
    /// client replaces its pending state on every arrival, never stacks it.
    PreviewPolicy {
        /// The param's binding.
        node: String,
        /// Its kwarg (`None` for a bare literal) — as the intent spelled it.
        #[serde(skip_serializing_if = "Option::is_none")]
        port: Option<String>,
        /// `compute_on_release` — the only mode that is ever announced.
        mode: PreviewMode,
        /// Predicted wall milliseconds of the dirty cone (what a live
        /// preview would cost); a lower bound when `rough`.
        estimate_ms: f64,
        /// Some node in the cone has no cost evidence yet (no sample for
        /// its op, or no element count) and contributed nothing — the
        /// estimate is a floor, shown with a `~` like the ETA.
        rough: bool,
        /// The withheld tick's literal — the value the slider will land on
        /// unless the drag moves on; the client tracks later ticks itself.
        pending_value: String,
    },
    /// An ANNOUNCED drag (one `preview_policy` went out for) has ended
    /// (docs/13 §Slider drags, contract item 3; additive). Broadcast after
    /// whatever ended it produced — the delta of a landed write, the
    /// snapshot of a reload (both already clear the client's pending state;
    /// this is a no-op there) — and on its own for the ends that produce
    /// nothing every client hears: the pointer released on the committed
    /// value (`end_drag`), Esc, a refused write (its `error` is unicast),
    /// the writer's departure or a lease handover. A pause longer than
    /// `DRAG_GAP_MS` is NOT an end the server announces: the pointer may
    /// still be down, and the pending state stands until the release. Never
    /// sent for a drag that was live throughout (nothing to take down). The
    /// client clears its pending entry when this names it.
    DragEnded {
        /// The param's binding.
        node: String,
        /// Its kwarg (`None` for a bare literal) — as the drag's ticks
        /// spelled it.
        #[serde(skip_serializing_if = "Option::is_none")]
        port: Option<String>,
    },
    /// The transport changed (docs/13 §Animation transport; additive):
    /// broadcast to every client after `transport_play` / `transport_pause`
    /// / `transport_seek` / `transport_speed` / `transport_reset`, after an
    /// Esc paused it, when the last client's departure paused it, and when
    /// an edit changed the loop or the driven set. The payload IS the
    /// [`TransportView`] — the same object every `snapshot` carries; the
    /// client replaces its transport state with it.
    Transport(TransportView),
    /// Scrub caching made progress on one slider (docs/12 §Speculative
    /// warming; v0.1 item 5; additive): broadcast coalesced at the
    /// statuses' cadence (≤ 10 Hz) while a queue warms — a position
    /// verified warm, the cap reached, the queue finished. The fields are
    /// the slider's [`ScrubView`] minus what only a text change moves
    /// (`on`, `positions`, `ineligible`), which the delta carries; the
    /// client updates that slider's `param.scrub` in place. Never sent for
    /// a slider without a queue.
    ScrubProgress {
        /// The slider's binding.
        node: String,
        /// Its kwarg (`value`).
        port: String,
        /// Position indices verified warm, ascending.
        warmed: Vec<usize>,
        /// Work remains (see [`ScrubView::warming`]).
        warming: bool,
        /// Bytes of memo entries the warming stored for this slider.
        bytes: u64,
        /// The byte cap stopped it. Omitted when false.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        capped: bool,
    },
}

/// How a drag's previews are handled (`preview_policy.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewMode {
    /// Previews are withheld; the value solves once, on release.
    ComputeOnRelease,
}

/// The wire envelope around a [`ServerMessage`].
#[derive(Debug, Serialize)]
pub struct Envelope<'a> {
    /// Protocol version.
    pub v: u32,
    /// Server sequence number (monotonic per session).
    pub seq: u64,
    /// The message.
    #[serde(flatten)]
    pub message: &'a ServerMessage,
}

/// Where a placed node also connects from (drag-wire-to-empty-canvas).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConnectSpec {
    /// Source end.
    pub from: WireEnd,
    /// The new node's input port.
    pub to_port: String,
    /// Wrap the wire in `each()` (accepted lift chip).
    #[serde(default)]
    pub lift: bool,
}

/// One literal a `place_node` writes into the new node as it lands (wave 4
/// B4 — the slider shortcut `1<20` places `slider(value=1.0, min=1.0,
/// max=20.0, step=1.0)` as ONE op; the batch path cannot, because the
/// server assigns the auto-name the later `set_param`s would have to know).
/// Applied exactly as a `set_param` on that port would be: one literal
/// token, spec-order insertion, transport-driven ports refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ParamSpec {
    /// The new node's input port.
    pub port: String,
    /// The literal's source text (`12.5`, `True`, `"x"`).
    pub value: String,
}

/// Client → server intents (docs/10 round-trip table + reads).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Handshake.
    Hello {
        /// Protocol version the client speaks.
        v: u32,
        /// The join hint (additive, v0.1 wave 4 O3 — the pop-out
        /// viewport): `observer` asks to join as a DECLARED observer, one
        /// that never holds the write lease — not at the join even when
        /// the lease is free, not by promotion when the writer leaves, and
        /// its `take_lease` is refused (kind `lease`) — so a second window
        /// of the same person can never steal the first one's lease, a
        /// reconnect included. Absent (an older client) or `writer`: the
        /// stage-5 rule, the first client takes the lease.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<Role>,
    },
    /// Place a node (search-to-place / ribbon / drag-to-empty-canvas).
    PlaceNode {
        /// Dialect name.
        func: String,
        /// Manual cell (else auto-layout).
        #[serde(default)]
        cell: Option<[i64; 2]>,
        /// Also wire it from a source.
        #[serde(default)]
        connect: Option<ConnectSpec>,
        /// Literals to write into the new node's ports in the same op
        /// (additive, wave 4 B4; absent = none). Every one is applied as
        /// a `set_param` on the new node before `connect`, and a refused
        /// one refuses the whole placement.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        params: Vec<ParamSpec>,
    },
    /// Draw a wire (rewrite one kwarg).
    Connect {
        /// Source.
        from: WireEnd,
        /// Target.
        to: WireEnd,
        /// Wrap in `each()` (accepted lift chip).
        #[serde(default)]
        lift: bool,
    },
    /// Remove a wire (remove the kwarg; a required port goes red).
    Disconnect {
        /// Target end.
        to: WireEnd,
    },
    /// Accept a lift chip on an existing kwarg.
    AcceptLift {
        /// Node.
        node: String,
        /// Port.
        port: String,
    },
    /// Set a param literal (slider release, inline edit). `port` = the
    /// kwarg on a call; `None` = a bare-literal binding.
    SetParam {
        /// Node.
        node: String,
        /// Kwarg (`None` for bare literals).
        #[serde(default)]
        port: Option<String>,
        /// The literal's source text (`12.5`, `True`, `"x"`).
        value: String,
    },
    /// Ephemeral param preview during a drag (latest-wins, no op, no
    /// undo, nothing written).
    ParamPreview {
        /// Node.
        node: String,
        /// Kwarg (`None` for bare literals).
        #[serde(default)]
        port: Option<String>,
        /// The literal's source text.
        value: String,
    },
    /// The pointer was released on the committed value: no `set_param`
    /// follows, and the drag is over (docs/13 §Slider drags). Both sliders
    /// send it on every release that writes nothing — a release that
    /// writes ends the drag with its `set_param` instead. Ends the session's
    /// drag when it is this param's (a drag that expired by the gap rule
    /// included) and broadcasts `drag_ended` if that drag was announced;
    /// when the standing drag is another param's or there is none (a
    /// reload, an Esc or a write ended it first) it is a no-op — a routine
    /// release is never an error. Needs the lease like the ticks do.
    EndDrag {
        /// Node.
        node: String,
        /// Kwarg (`None` for bare literals).
        #[serde(default)]
        port: Option<String>,
    },
    /// Rename a binding (text + references + sidecar, atomically).
    Rename {
        /// Old name.
        node: String,
        /// New name.
        new: String,
    },
    /// Delete a node's statement (downstream reds, never cascades).
    DeleteNode {
        /// Node.
        node: String,
    },
    /// Toggle `#off` on a node (docs/10 gesture table; DECISIONS.md
    /// node-disable row): a live statement becomes a ghost — ports and
    /// wiring intact, skipped in solves, downstream red as "disabled" — and
    /// a ghost becomes live again (usually a pure cache hit). The delta's
    /// label says which way it went: `disable x` / `enable x`.
    ToggleDisable {
        /// Node.
        node: String,
    },
    /// Move a node (sidecar only). `None` = snap back to auto.
    MoveNode {
        /// Node.
        node: String,
        /// Cell.
        #[serde(default)]
        cell: Option<[i64; 2]>,
    },
    /// Toggle preview (sidecar only). `None` = default.
    SetPreview {
        /// Node.
        node: String,
        /// On/off/default.
        #[serde(default)]
        on: Option<bool>,
    },
    /// Collapse or expand a slider (sidecar only — the `collapsed`
    /// override; wave 4 B4, docs/16 §Canvas conventions): a collapsed
    /// slider is one grid unit tall — name, track and value on one row.
    /// An op like a move (`collapse x` / `expand x`). Refused (kind
    /// `refused`) for a node that is not a slider, and for a slider any of
    /// whose `value` / `min` / `max` / `step` is wired — the collapsed row
    /// has no port for the wire; the server decides off the DOCUMENT
    /// (`viewmodel::collapse_refusal`, so a `batch` sees what it wired
    /// earlier), the client mirrors the reason.
    SetCollapsed {
        /// Node.
        node: String,
        /// Collapsed (`true`) or expanded (`false`, the default).
        collapsed: bool,
    },
    /// Scrub-cache a slider, or stop (docs/12 §Speculative warming; v0.1
    /// item 5): a write gesture that edits the TEXT — `on` writes
    /// `scrub=True` into the slider's call (inserted at its spec-order
    /// position, or rewritten), `off` removes the kwarg (the default says
    /// the same thing) — an op like any literal edit (`scrub x on` /
    /// `scrub x off`, undoable, a `batch` element); the delta carries the
    /// new view and the warming starts from it. Refused (kind `refused`)
    /// with the reason for a node that is not a slider, and — when `on` —
    /// for an ineligible slider: `too many positions (51 > 32)`, `min is
    /// wired — …`, `step is 0 — …` (`crate::scrub::eligibility`, read off
    /// the DOCUMENT so a `batch` sees what it wired earlier). The client
    /// computes nothing: the view's `scrub.ineligible` is the same reason.
    SetScrub {
        /// The slider's binding.
        node: String,
        /// Warm it (`true`) or stop and forget its queue (`false`).
        on: bool,
    },
    /// Cancel the running generation (Esc). Also pauses the transport.
    Cancel {},
    /// Start the transport (docs/13 §Animation transport): the playhead
    /// advances from where it stands at `speed`; each frame goes through
    /// the preview path (nothing is written). Writer-only, like every
    /// transport intent — playback is shared session state every client
    /// sees, and the lease is the one arbiter of shared state; observers
    /// follow. Idempotent while playing.
    TransportPlay {},
    /// Stop the playhead where it is. Idempotent while paused.
    TransportPause {},
    /// Move the playhead to a frame of the primary loop (`0 ..
    /// transport.frames`; beyond it is refused, kind `transport`) — playing
    /// or paused; a paused seek paints the frame.
    TransportSeek {
        /// The frame.
        frame: u64,
    },
    /// Set the playback rate (finite, > 0; else refused, kind `transport`).
    /// Takes effect from the playhead's current position.
    TransportSpeed {
        /// Playhead milliseconds per wall millisecond.
        factor: f64,
    },
    /// Pause and rewind to `t_ms = 0` (frame 0, `clock` at 0 — the values
    /// a headless run evaluates).
    TransportReset {},
    /// Undo the last op (restore its `before` snapshot; docs/13
    /// §Undo/redo). A write intent; the delta's label is `undo: <label>`.
    Undo {},
    /// Redo the last undone op (restore its `after` snapshot).
    Redo {},
    /// Several canvas gestures as ONE op (multi-move, multi-delete,
    /// reconnect): applied in order under the session lock, all or
    /// nothing — any failure rolls back to the pre-batch state and the
    /// error names the failing `index`; one persist, one op, one delta.
    Batch {
        /// The gestures — every one a write gesture (`place_node`,
        /// `connect`, `disconnect`, `accept_lift`, `set_param`, `rename`,
        /// `delete_node`, `toggle_disable`, `move_node`, `set_preview`).
        ops: Vec<ClientMessage>,
        /// The op's label.
        label: String,
    },
    /// Replace whole files atomically (agents / MCP) — the `batch`
    /// operation of the ledger row: refused on a stale base or a text that
    /// does not parse; else one persist (temp + rename per file), one op,
    /// one delta (a snapshot when scripts changed).
    ApplyText(ApplyTextRequest),
    /// Ask for a node's output values.
    Inspect {
        /// Node.
        node: String,
    },
    /// Ask for a wire's value + pairing.
    InspectWire {
        /// Target end.
        to: WireEnd,
    },
    /// Wire-drag start: which ports accept this source?
    ProbeWire {
        /// Source.
        from: WireEnd,
    },
    /// Re-stream every displayed output's frames to this client.
    ResyncDisplay {},
    /// Take the write lease (explicit UI action).
    TakeLease {},
    /// A screenshot reply.
    Screenshot {
        /// Request id.
        id: u64,
        /// PNG bytes, base64 (absent on failure).
        #[serde(default)]
        png_base64: Option<String>,
        /// Failure reason.
        #[serde(default)]
        error: Option<String>,
    },
}

/// The intent envelope: `{v, id?, type, payload}`.
#[derive(Debug, Clone, Deserialize)]
pub struct IntentEnvelope {
    /// Protocol version.
    pub v: u32,
    /// Client-side request id, echoed in the resulting delta / error.
    #[serde(default)]
    pub id: Option<String>,
    /// The intent.
    #[serde(flatten)]
    pub message: ClientMessage,
}

/// Is this intent a write (needs the lease)?
#[must_use]
pub fn is_write(message: &ClientMessage) -> bool {
    matches!(
        message,
        ClientMessage::PlaceNode { .. }
            | ClientMessage::Connect { .. }
            | ClientMessage::Disconnect { .. }
            | ClientMessage::AcceptLift { .. }
            | ClientMessage::SetParam { .. }
            | ClientMessage::ParamPreview { .. }
            | ClientMessage::EndDrag { .. }
            | ClientMessage::Rename { .. }
            | ClientMessage::DeleteNode { .. }
            | ClientMessage::ToggleDisable { .. }
            | ClientMessage::MoveNode { .. }
            | ClientMessage::SetPreview { .. }
            | ClientMessage::SetCollapsed { .. }
            | ClientMessage::SetScrub { .. }
            | ClientMessage::Cancel {}
            | ClientMessage::Undo {}
            | ClientMessage::Redo {}
            | ClientMessage::Batch { .. }
            | ClientMessage::ApplyText(_)
            | ClientMessage::TransportPlay {}
            | ClientMessage::TransportPause {}
            | ClientMessage::TransportSeek { .. }
            | ClientMessage::TransportSpeed { .. }
            | ClientMessage::TransportReset {}
    )
}

/// Is this intent a transport control (docs/13 §Animation transport)? A
/// write for the lease's purposes — shared session state — that never
/// touches the text and is neither a gesture nor a drag-ender.
#[must_use]
pub fn is_transport(message: &ClientMessage) -> bool {
    matches!(
        message,
        ClientMessage::TransportPlay {}
            | ClientMessage::TransportPause {}
            | ClientMessage::TransportSeek { .. }
            | ClientMessage::TransportSpeed { .. }
            | ClientMessage::TransportReset {}
    )
}

/// Is this intent a canvas write gesture — one that edits the text or
/// sidecar in place and may be an element of a `batch`? (Previews, cancel,
/// undo/redo, batch itself and `apply_text` are writes but not gestures.)
#[must_use]
pub fn is_gesture(message: &ClientMessage) -> bool {
    matches!(
        message,
        ClientMessage::PlaceNode { .. }
            | ClientMessage::Connect { .. }
            | ClientMessage::Disconnect { .. }
            | ClientMessage::AcceptLift { .. }
            | ClientMessage::SetParam { .. }
            | ClientMessage::Rename { .. }
            | ClientMessage::DeleteNode { .. }
            | ClientMessage::ToggleDisable { .. }
            | ClientMessage::MoveNode { .. }
            | ClientMessage::SetPreview { .. }
            | ClientMessage::SetCollapsed { .. }
            | ClientMessage::SetScrub { .. }
    )
}

/// The wire `type` tag of an intent (`set_param`, `batch`, …) — for
/// messages that name a message.
#[must_use]
pub fn type_tag(message: &ClientMessage) -> String {
    serde_json::to_value(message)
        .ok()
        .and_then(|v| v["type"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "?".to_owned())
}

/// Serialize a server message with its envelope.
#[must_use]
pub fn encode(seq: u64, message: &ServerMessage) -> String {
    serde_json::to_string(&Envelope {
        v: PROTOCOL_VERSION,
        seq,
        message,
    })
    .unwrap_or_else(|error| {
        format!(
            "{{\"v\":{PROTOCOL_VERSION},\"seq\":{seq},\"type\":\"error\",\"payload\":{{\"kind\":\"encode\",\"message\":{}}}}}",
            serde_json::Value::String(error.to_string())
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The frozen wire shape of the compute-on-release announcement (v0.1
    // item 3b; the web client mirrors exactly this in messages.ts).
    #[test]
    fn preview_policy_encodes_the_documented_shape() {
        let text = encode(
            9,
            &ServerMessage::PreviewPolicy {
                node: "deboss".into(),
                port: Some("value".into()),
                mode: PreviewMode::ComputeOnRelease,
                estimate_ms: 6512.5,
                rough: false,
                pending_value: "1.3".into(),
            },
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "v": PROTOCOL_VERSION, "seq": 9, "type": "preview_policy",
                "payload": {
                    "node": "deboss", "port": "value", "mode": "compute_on_release",
                    "estimate_ms": 6512.5, "rough": false, "pending_value": "1.3"
                }
            })
        );
        // A bare literal has no port key at all (not `null`).
        let bare = encode(
            10,
            &ServerMessage::PreviewPolicy {
                node: "x".into(),
                port: None,
                mode: PreviewMode::ComputeOnRelease,
                estimate_ms: 1000.0,
                rough: true,
                pending_value: "2.0".into(),
            },
        );
        let bare: serde_json::Value = serde_json::from_str(&bare).unwrap();
        assert!(bare["payload"].get("port").is_none(), "{bare}");
        assert_eq!(bare["payload"]["rough"], true);
    }

    // The frozen wire shape of the drag's end (docs/13 §Slider drags,
    // contract item 3) and of the release that writes nothing; the web
    // client mirrors both in messages.ts.
    #[test]
    fn drag_ended_and_end_drag_have_the_documented_shapes() {
        let text = encode(
            11,
            &ServerMessage::DragEnded {
                node: "deboss".into(),
                port: Some("value".into()),
            },
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "v": PROTOCOL_VERSION, "seq": 11, "type": "drag_ended",
                "payload": { "node": "deboss", "port": "value" }
            })
        );
        let bare = encode(
            12,
            &ServerMessage::DragEnded {
                node: "x".into(),
                port: None,
            },
        );
        let bare: serde_json::Value = serde_json::from_str(&bare).unwrap();
        assert_eq!(
            bare["payload"],
            serde_json::json!({ "node": "x" }),
            "no port key for a bare literal"
        );

        let release: IntentEnvelope = serde_json::from_str(
            r#"{"v":1,"type":"end_drag","payload":{"node":"deboss","port":"value"}}"#,
        )
        .unwrap();
        assert_eq!(
            release.message,
            ClientMessage::EndDrag {
                node: "deboss".into(),
                port: Some("value".into()),
            }
        );
        assert!(
            is_write(&release.message),
            "end_drag needs the lease like the ticks"
        );
        assert!(!is_gesture(&release.message), "not a batch element");
        let bare: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"type":"end_drag","payload":{"node":"x"}}"#).unwrap();
        assert_eq!(
            bare.message,
            ClientMessage::EndDrag {
                node: "x".into(),
                port: None,
            }
        );
    }

    // The frozen wire shapes of the transport (docs/13 §Animation
    // transport; v0.1 item 4) — the `transport` broadcast, whose payload is
    // the same `TransportView` every snapshot carries, and the five
    // intents. The web client mirrors exactly these in messages.ts.
    #[test]
    fn transport_messages_have_the_documented_shapes() {
        let view = TransportView {
            playing: true,
            speed: 1.5,
            t_ms: 1250.0,
            frame: 37,
            frames: 120,
            period_ms: 4000.0,
            driven: vec![
                DrivenView {
                    node: "spin".into(),
                    port: "frame".into(),
                    signal: DrivenSignal::Frame,
                    r#loop: Some(LoopView {
                        frames: 120,
                        period_ms: 4000.0,
                    }),
                },
                DrivenView {
                    node: "elapsed".into(),
                    port: "t".into(),
                    signal: DrivenSignal::Time,
                    r#loop: None,
                },
            ],
        };
        let text = encode(13, &ServerMessage::Transport(view.clone()));
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "v": PROTOCOL_VERSION, "seq": 13, "type": "transport",
                "payload": {
                    "playing": true, "speed": 1.5, "t_ms": 1250.0, "frame": 37,
                    "frames": 120, "period_ms": 4000.0,
                    "driven": [
                        { "node": "spin", "port": "frame", "signal": "frame",
                          "loop": { "frames": 120, "period_ms": 4000.0 } },
                        { "node": "elapsed", "port": "t", "signal": "time" }
                    ]
                }
            })
        );
        let back: TransportView = serde_json::from_value(value["payload"].clone()).unwrap();
        assert_eq!(back, view);

        let intents: Vec<(&str, ClientMessage)> = vec![
            (
                r#"{"v":1,"type":"transport_play","payload":{}}"#,
                ClientMessage::TransportPlay {},
            ),
            (
                r#"{"v":1,"type":"transport_pause","payload":{}}"#,
                ClientMessage::TransportPause {},
            ),
            (
                r#"{"v":1,"type":"transport_seek","payload":{"frame":57}}"#,
                ClientMessage::TransportSeek { frame: 57 },
            ),
            (
                r#"{"v":1,"type":"transport_speed","payload":{"factor":0.5}}"#,
                ClientMessage::TransportSpeed { factor: 0.5 },
            ),
            (
                r#"{"v":1,"type":"transport_reset","payload":{}}"#,
                ClientMessage::TransportReset {},
            ),
        ];
        for (text, expected) in intents {
            let envelope: IntentEnvelope = serde_json::from_str(text).unwrap();
            assert_eq!(envelope.message, expected, "{text}");
            assert!(
                is_write(&envelope.message),
                "transport control is writer-only: {text}"
            );
            assert!(is_transport(&envelope.message), "{text}");
            assert!(
                !is_gesture(&envelope.message),
                "never a batch element: {text}"
            );
        }
        assert!(!is_transport(&ClientMessage::Cancel {}));
    }

    #[test]
    fn intents_round_trip_and_tag_by_type() {
        let text = r#"{"v":1,"id":"7","type":"connect","payload":{"from":{"node":"a","port":"out"},"to":{"node":"b","port":"x"},"lift":true}}"#;
        let envelope: IntentEnvelope = serde_json::from_str(text).unwrap();
        assert_eq!(envelope.id.as_deref(), Some("7"));
        assert!(is_write(&envelope.message));
        assert_eq!(
            envelope.message,
            ClientMessage::Connect {
                from: WireEnd {
                    node: "a".into(),
                    port: "out".into()
                },
                to: WireEnd {
                    node: "b".into(),
                    port: "x".into()
                },
                lift: true
            }
        );
        let read: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"type":"cancel","payload":{}}"#).unwrap();
        assert!(is_write(&read.message), "cancel needs the lease");
        let read: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"type":"inspect","payload":{"node":"a"}}"#).unwrap();
        assert!(!is_write(&read.message));
    }

    // Wave 4 B4: `place_node` carries optional `params` (absent = none, so
    // a client that never heard of them still places), and `set_collapsed`
    // is a write gesture — a batch element — with the documented shape.
    #[test]
    fn place_node_params_default_empty_and_set_collapsed_is_a_gesture() {
        let bare: IntentEnvelope = serde_json::from_str(
            r#"{"v":1,"type":"place_node","payload":{"func":"slider","cell":[2,3]}}"#,
        )
        .unwrap();
        assert_eq!(
            bare.message,
            ClientMessage::PlaceNode {
                func: "slider".into(),
                cell: Some([2, 3]),
                connect: None,
                params: Vec::new(),
            }
        );
        let with_params: IntentEnvelope = serde_json::from_str(
            r#"{"v":1,"type":"place_node","payload":{"func":"slider","cell":null,
                "params":[{"port":"value","value":"1.0"},{"port":"max","value":"20.0"}]}}"#,
        )
        .unwrap();
        let ClientMessage::PlaceNode { params, .. } = &with_params.message else {
            panic!("{:?}", with_params.message);
        };
        assert_eq!(
            params,
            &[
                ParamSpec {
                    port: "value".into(),
                    value: "1.0".into()
                },
                ParamSpec {
                    port: "max".into(),
                    value: "20.0".into()
                }
            ]
        );
        assert!(is_gesture(&with_params.message));
        // Re-encoded, an empty list is omitted (the wire shape stays the
        // pre-B4 one for every placement without params).
        let encoded = serde_json::to_value(&bare.message).unwrap();
        assert!(encoded["payload"].get("params").is_none(), "{encoded}");

        let collapse: IntentEnvelope = serde_json::from_str(
            r#"{"v":1,"id":"c","type":"set_collapsed","payload":{"node":"size","collapsed":true}}"#,
        )
        .unwrap();
        assert_eq!(
            collapse.message,
            ClientMessage::SetCollapsed {
                node: "size".into(),
                collapsed: true
            }
        );
        assert!(is_write(&collapse.message));
        assert!(
            is_gesture(&collapse.message),
            "an op like a move: a batch element"
        );
        assert!(!is_transport(&collapse.message));
        assert_eq!(type_tag(&collapse.message), "set_collapsed");
    }

    // v0.1 item 5 (S1): `set_scrub` is a write gesture (a batch element,
    // an op) with the documented shape; `scrub_progress` carries the warm
    // state a delta does not — `capped` omitted when false, present when
    // true; a slider's `ScrubView` omits `capped`/`ineligible` when
    // false/absent. The web client mirrors exactly these in messages.ts.
    #[test]
    fn set_scrub_is_a_gesture_and_scrub_progress_has_the_documented_shape() {
        let on: IntentEnvelope = serde_json::from_str(
            r#"{"v":1,"id":"s","type":"set_scrub","payload":{"node":"size","on":true}}"#,
        )
        .unwrap();
        assert_eq!(
            on.message,
            ClientMessage::SetScrub {
                node: "size".into(),
                on: true
            }
        );
        assert!(is_write(&on.message));
        assert!(is_gesture(&on.message), "an op: a batch element");
        assert!(!is_transport(&on.message));
        assert_eq!(type_tag(&on.message), "set_scrub");

        let progress = encode(
            4,
            &ServerMessage::ScrubProgress {
                node: "size".into(),
                port: "value".into(),
                warmed: vec![5, 6, 7],
                warming: true,
                bytes: 4096,
                capped: false,
            },
        );
        let value: serde_json::Value = serde_json::from_str(&progress).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "v": PROTOCOL_VERSION, "seq": 4, "type": "scrub_progress",
                "payload": {"node": "size", "port": "value", "warmed": [5, 6, 7], "warming": true, "bytes": 4096}
            })
        );
        let capped = encode(
            5,
            &ServerMessage::ScrubProgress {
                node: "size".into(),
                port: "value".into(),
                warmed: vec![6],
                warming: false,
                bytes: 300_000_000,
                capped: true,
            },
        );
        let value: serde_json::Value = serde_json::from_str(&capped).unwrap();
        assert_eq!(value["payload"]["capped"], true);
        assert_eq!(value["payload"]["warming"], false);

        let view = ScrubView {
            on: true,
            positions: 19,
            warmed: vec![6],
            warming: true,
            bytes: 12,
            capped: false,
            ineligible: None,
        };
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({"on": true, "positions": 19, "warmed": [6], "warming": true, "bytes": 12})
        );
        let back: ScrubView = serde_json::from_value(serde_json::to_value(&view).unwrap()).unwrap();
        assert_eq!(back, view, "the omitted flags read back as false / absent");
    }

    // The join hint (v0.1 wave 4 O3): an old client's hello has no `role`
    // and reads as `None`; `"role":"observer"` is the declared observer;
    // the field is omitted on the wire when absent (additive — the version
    // stays).
    #[test]
    fn hello_carries_the_optional_join_role() {
        let old: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"type":"hello","payload":{"v":1}}"#).unwrap();
        assert_eq!(old.message, ClientMessage::Hello { v: 1, role: None });
        assert!(!is_write(&old.message));
        let observer: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"type":"hello","payload":{"v":1,"role":"observer"}}"#)
                .unwrap();
        assert_eq!(
            observer.message,
            ClientMessage::Hello {
                v: 1,
                role: Some(Role::Observer)
            }
        );
        let writer: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"type":"hello","payload":{"v":1,"role":"writer"}}"#)
                .unwrap();
        assert_eq!(
            writer.message,
            ClientMessage::Hello {
                v: 1,
                role: Some(Role::Writer)
            }
        );
        assert_eq!(
            serde_json::to_value(ClientMessage::Hello { v: 1, role: None }).unwrap(),
            serde_json::json!({"type": "hello", "payload": {"v": 1}})
        );
        assert_eq!(
            serde_json::to_value(ClientMessage::Hello {
                v: 1,
                role: Some(Role::Observer)
            })
            .unwrap(),
            serde_json::json!({"type": "hello", "payload": {"v": 1, "role": "observer"}})
        );
        assert!(
            serde_json::from_str::<IntentEnvelope>(
                r#"{"v":1,"type":"hello","payload":{"v":1,"role":"admin"}}"#
            )
            .is_err(),
            "an unknown role is a protocol error, never a silent default"
        );
    }

    #[test]
    fn undo_redo_batch_and_apply_text_are_writes_with_the_documented_shapes() {
        let undo: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"id":"u","type":"undo","payload":{}}"#).unwrap();
        assert_eq!(undo.message, ClientMessage::Undo {});
        assert!(is_write(&undo.message));
        assert!(!is_gesture(&undo.message), "undo is not a batch element");
        let redo: IntentEnvelope =
            serde_json::from_str(r#"{"v":1,"type":"redo","payload":{}}"#).unwrap();
        assert!(is_write(&redo.message));

        let batch: IntentEnvelope = serde_json::from_str(
            r#"{"v":1,"id":"b","type":"batch","payload":{"label":"move 2 nodes","ops":[
                {"type":"move_node","payload":{"node":"a","cell":[1,2]}},
                {"type":"move_node","payload":{"node":"b","cell":null}}]}}"#,
        )
        .unwrap();
        let ClientMessage::Batch { ops, label } = &batch.message else {
            panic!("{:?}", batch.message);
        };
        assert_eq!(label, "move 2 nodes");
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().all(is_gesture));
        assert!(is_write(&batch.message));
        assert_eq!(type_tag(&ops[0]), "move_node");
        assert_eq!(type_tag(&batch.message), "batch");

        let apply: IntentEnvelope = serde_json::from_str(
            r##"{"v":1,"type":"apply_text","payload":{"base_text_hash":"ab","label":"agent edit",
                "actor":{"kind":"agent","prompt":"add a sphere"},
                "files":[{"path":"p.cic","text":"# cicada 1\n"}]}}"##,
        )
        .unwrap();
        let ClientMessage::ApplyText(request) = &apply.message else {
            panic!("{:?}", apply.message);
        };
        assert_eq!(
            request.actor,
            Actor::Agent {
                prompt: Some("add a sphere".into())
            }
        );
        assert_eq!(request.files[0].path, "p.cic");
        assert!(is_write(&apply.message));
        assert_eq!(
            serde_json::to_value(Actor::Human).unwrap(),
            serde_json::json!({"kind": "human"})
        );
        assert_eq!(
            serde_json::to_value(Actor::Agent { prompt: None }).unwrap(),
            serde_json::json!({"kind": "agent", "prompt": null}),
            "the prompt key is always present on the wire (the mirror reads string | null)"
        );
        let bare: Actor = serde_json::from_str(r#"{"kind":"agent"}"#).unwrap();
        assert_eq!(
            bare,
            Actor::Agent { prompt: None },
            "…but may be omitted on the way in"
        );
    }

    #[test]
    fn error_details_flatten_into_the_payload() {
        let mut details = serde_json::Map::new();
        details.insert("current_text_hash".into(), "ff".into());
        let text = encode(
            1,
            &ServerMessage::Error {
                intent_id: Some("x".into()),
                kind: "stale_base".into(),
                message: "stale".into(),
                details,
            },
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["payload"]["kind"], "stale_base");
        assert_eq!(value["payload"]["current_text_hash"], "ff");
        let plain = encode(
            1,
            &ServerMessage::Error {
                intent_id: None,
                kind: "lease".into(),
                message: "m".into(),
                details: serde_json::Map::new(),
            },
        );
        let value: serde_json::Value = serde_json::from_str(&plain).unwrap();
        assert_eq!(
            value["payload"].as_object().unwrap().len(),
            2,
            "no details → just kind + message: {value}"
        );
    }

    /// The handshake's `pipeline` refusal is the client's cue to stop
    /// retrying, show the reason and (for `not_found`) drop the Recent
    /// entry — so its shape is a contract: kind `pipeline`, `reason` in
    /// `snake_case`, `pipeline` as sent, no `intent_id`.
    #[test]
    fn a_join_refusal_carries_the_pipeline_and_the_reason() {
        let text = encode(
            0,
            &ServerMessage::join_refused(
                Some("gone.cic"),
                JoinRefusal::NotFound,
                "opening gone.cic: no pipeline `gone.cic` in the project".into(),
            ),
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["seq"], 0);
        assert_eq!(
            value["payload"],
            serde_json::json!({
                "kind": "pipeline",
                "message": "opening gone.cic: no pipeline `gone.cic` in the project",
                "pipeline": "gone.cic",
                "reason": "not_found",
            }),
            "{value}"
        );
        // Every reason has a snake_case wire name the client mirrors.
        for (reason, wire) in [
            (JoinRefusal::Unnamed, "unnamed"),
            (JoinRefusal::PathNotAllowed, "path_not_allowed"),
            (JoinRefusal::NotFound, "not_found"),
            (JoinRefusal::OpenFailed, "open_failed"),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<JoinRefusal>(wire.into()).unwrap(),
                reason
            );
        }
        // No pipeline named → no `pipeline` key at all.
        let text = encode(
            0,
            &ServerMessage::join_refused(None, JoinRefusal::Unnamed, "no pipeline".into()),
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["payload"],
            serde_json::json!({"kind": "pipeline", "message": "no pipeline", "reason": "unnamed"}),
            "{value}"
        );
    }

    #[test]
    fn server_messages_carry_the_envelope() {
        let text = encode(
            42,
            &ServerMessage::Notice {
                level: "info".into(),
                message: "hi".into(),
            },
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["v"], PROTOCOL_VERSION);
        assert_eq!(value["seq"], 42);
        assert_eq!(value["type"], "notice");
        assert_eq!(value["payload"]["message"], "hi");
    }
}
