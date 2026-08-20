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
    /// Elements processed (done nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elements: Option<u64>,
    /// Measured work nanoseconds (done nodes).
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
        /// `stale_base`, `parse_error`, `path_not_allowed`, `io_error`).
        kind: String,
        /// Human message.
        message: String,
        /// Kind-specific facts, flattened into the payload (additive):
        /// `current_text_hash` (`stale_base`), `diagnostics` (`parse_error`),
        /// `index` (the failing op of a batch).
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

/// Client → server intents (docs/10 round-trip table + reads).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Handshake.
    Hello {
        /// Protocol version the client speaks.
        v: u32,
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
    /// Cancel the running generation (Esc).
    Cancel {},
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
            | ClientMessage::Rename { .. }
            | ClientMessage::DeleteNode { .. }
            | ClientMessage::ToggleDisable { .. }
            | ClientMessage::MoveNode { .. }
            | ClientMessage::SetPreview { .. }
            | ClientMessage::Cancel {}
            | ClientMessage::Undo {}
            | ClientMessage::Redo {}
            | ClientMessage::Batch { .. }
            | ClientMessage::ApplyText(_)
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
