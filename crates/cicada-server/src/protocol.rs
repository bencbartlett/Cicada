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
        /// True when this snapshot follows an external file change (undo
        /// barrier — docs/13; the spike has no undo, the flag is honest
        /// anyway).
        barrier: bool,
        /// Why (external change / initial / resync).
        reason: String,
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
        /// Machine kind (`writer`, `lease`, `protocol`, `unknown_node`, …).
        kind: String,
        /// Human message.
        message: String,
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
            | ClientMessage::MoveNode { .. }
            | ClientMessage::SetPreview { .. }
            | ClientMessage::Cancel {}
    )
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
