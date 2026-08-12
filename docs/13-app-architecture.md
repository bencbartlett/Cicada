# App architecture: server, protocol, sessions, undo

Web-first delivery (docs 04, 12) means the app is a protocol between
two parties: the **engine server** (Rust: parser, checker, scheduler,
caches, git) and the **browser client** (React Flow canvas, three.js
viewport, inspectors). This document specifies that seam. The design
rule: **the engine owns all authoritative state; the browser is a
view that sends intents and renders deltas.**

Default stack (revisable, but these are the working picks): axum +
tokio on the engine; React + TypeScript + Vite, xyflow, three.js,
zustand on the client. One WebSocket per client session; HTTP for
static assets and fetch-style reads. The SPA bundle is embedded in the
engine binary (`cicada serve` is the whole app); dev mode runs Vite
with a proxy for HMR.

## Projects, pipelines, sessions

- A **project** is a directory (normally a git repo root): `*.cic`
  pipelines, `scripts/`, sidecars. `cicada serve [dir]` serves exactly
  one project.
- Each browser tab opens one pipeline = one **session**.
- **Single-writer lease** per pipeline: the first client takes the
  write lease; further clients are live read-only observers (which
  also gives presentation/tablet views for free). The lease transfers
  by explicit UI action, or automatically when the writer disconnects
  (5 s grace). No merge machinery exists in v1 — by design.
- AI agents are server-side actors (doc 11): their edits enter the
  same op pipeline as a client's, as one atomic batch op, and
  broadcast to clients the same way.

## State ownership and sync

Two planes over one WebSocket:

- **Control plane — JSON** (debuggability beats compactness at these
  sizes): versioned envelope `{v, seq, type, payload}`.
- **Data plane — binary frames** for geometry buffers only.

The edit flow is **intent → authoritative delta**:

1. The client sends a gesture-level intent matching doc 10's
   round-trip table: `place_node`, `connect`, `accept_lift`,
   `set_param`, `rename`, `delete_node`, `move_node` (layout),
   `undo`, `redo`, …
2. The engine validates against the checker, applies the edit to the
   text/sidecar (doc 10 writer discipline), assigns it a sequence
   number, and broadcasts the **delta**: graph view-model changes +
   fresh diagnostics + the dirty set.
3. Clients apply optimistically for feel (node moves, param drags)
   and reconcile on the authoritative delta — the server always wins;
   with a single writer, rollbacks are rare to nonexistent.

**Slider drags get a dedicated ephemeral path**: during the drag, the
client streams `param_preview` messages (not ops, not undoable); the
scheduler runs **latest-wins supersession with no debounce** — each
completed preview generation immediately starts the next with the
newest value, skipping stale intermediates. On release the client
sends the one real `set_param` op — the single undo step.

Drag responsiveness is **adaptive, driven by the cost model** (doc
12): cheap dirty cones solve every frame (60 fps repaint is the
target — smooth parameter tweaking is the point of a parametric
tool); mid-cost cones solve continuously at whatever rate completes,
the viewer always showing the latest complete generation; cones
predicted ≥ ~1 s switch to **compute-on-release** — the slider shows
the pending value and an honest estimate, and solves once on release.
Scrub caching (doc 12) upgrades expensive sliders back into the
smooth tier by warming their range during idle time.

## Binary frame format

Frames are typed arrays ready for GPU upload — zero parsing on the
client beyond the header. Field-level layout (byte-exact spec lands
with the implementation):

| Frame kind | Contents |
|---|---|
| `mesh_batch` | node ref, generation, element range; positions `f32×3`, normals `f32×3`, indices `u32`, pick-ID base `u32` |
| `instance_batch` | mesh content-hash ref, generation; transforms `f32 3×4` per instance, pick IDs `u32` per instance |
| `curve_batch` / `point_batch` | node ref, generation; polyline vertex runs / positions, pick IDs |

- **Generation-tagged**: the client drops any frame older than the
  newest applied generation for that node — cancelled solves can
  never paint stale geometry (doc 04's "last coherent frame").
- **Instancing is hash-driven**: identical mesh hashes across elements
  arrive once as a `mesh_batch` plus an `instance_batch` of
  transforms (doc 12 interning → doc 04 instanced draws).
- Pick IDs map viewport clicks back to (node, element index, part ID)
  — backward picking rides the same tables.
- The client **subscribes** to a display set (preview-enabled nodes +
  the currently inspected wire); the engine streams buffers
  progressively as elements complete.

## Solve streaming

Status is control-plane JSON, coalesced to ≤10 Hz: per-node state
transitions (queued / running / progress fraction / done / red with
diagnostics / blocked / cancelled), generation start/end, cost and ETA
updates, and wire-summary payloads (counts, bounds, samples) for
inspected wires. Diagnostics carry the doc 11 structure — kind, span,
expected/actual, suggested fix — so the same payload drives canvas
error chips, the text panel, and agent loops.

## Animation transport

Time params (`cycle`, `clock` — docs/08) are driven by a
server-authoritative **transport**: play / pause / speed / reset are
ordinary intents, so observers see the same animation the writer
sees. Playback emits frame values into the same preview path as
slider drags. For `cycle`, the loop is frame-quantized: after one
full pass every downstream NodeKey is warm and playback becomes cache
reads at display rate; the client may additionally retain recent
frames' display buffers (bounded), making warmed-loop playback 60 fps
with near-zero server traffic. `clock` is the unbounded escape hatch
— deterministic per value, uncached by design.

## HTTP surface

| Endpoint | Purpose |
|---|---|
| `GET /` | The embedded SPA |
| `GET /api/catalog` | Node-spec JSON catalog (docs/08) |
| `GET /api/project` | Pipelines, scripts, dirty/git status summary |
| `GET /api/blob/{hash}` | Large payloads on demand (full inspector data, export previews) |
| `POST /api/run/{node}` | Effectful nodes — requires the explicit-run confirmation, streams progress over the session socket |
| `GET /api/git/…` | Status, node-level diff, commit, per-node history (doc 10 git integration) |
| `GET /debug/state`, `GET /debug/screenshot` | The agent/dev verification loop (doc 14) |

## Undo/redo (formalizing the ledger row)

- The engine keeps a **linear op log per pipeline**: `{id, label,
  actor: human | agent(prompt), inverse edit, timestamp}`.
- Continuous gestures coalesce: a slider drag or node drag is one op,
  created on release.
- An agent inference's graph edits apply as **one atomic labeled op**
  (rebased onto current text; order-independence + single assignment
  make conflicts rare, and a conflict aborts the batch with
  diagnostics rather than half-applying).
- `undo`/`redo` are ordinary intents; the engine applies the inverse
  edit and broadcasts the delta like any other change.
- **Undo never recomputes** — the restored state's node keys are warm
  in the content-addressed cache (doc 12).
- The log is ephemeral (cleared on serve restart); git is the durable
  history.

## External changes and file watching

Humans edit through the canvas (locked decision), but files still
change on disk — chiefly via git (checkout, pull, restore), rarely via
a stray editor. The engine watches the project directory (debounced):
an external change to a `.cic`, sidecar, or script triggers reload →
re-check → **reload barrier** in the op log (undo stack cleared) →
full snapshot broadcast to clients. Honest and simple; no
three-way-merge machinery.

## Reconnect and resync

Clients reconnect with their last applied `seq`. Small gap → replay
deltas; large gap or barrier crossed → **snapshot**: the full graph
view-model (bindings, ports, wires, badges, diagnostics, statuses,
layout) is small — hundreds of KB worst case — and display buffers
re-stream from the subscription set afterward. Snapshots are also the
initial load path; there is exactly one client-hydration code path.

## Security and serving

- `cicada serve` binds **127.0.0.1** by default and prints a URL with
  an embedded session token (Jupyter-style); the WebSocket requires
  it, CORS is locked to the served origin.
- Remote deployment (the Onshape-style case) = the same binary behind
  a reverse proxy with real auth — explicitly out of scope for v0.1;
  nothing in the protocol assumes locality except the default bind.
- The engine binary embeds the SPA (`rust-embed`), so distribution is
  one file; the v0.2 desktop app wraps this same server + a webview.

## Latency targets (measured, not vibed — spike criteria feed here)

| Path | Target |
|---|---|
| Intent → ack (local) | < 5 ms |
| Optimistic apply | immediate |
| Param drag → dirty-cone solve → repaint (cheap cones) | 16 ms (60 fps) |
| Expensive cones (≥ ~1 s predicted) | compute-on-release with honest estimate |
| Warmed `cycle` loop playback | 60 fps sustained |
| Beyond budget | progress UI takes over, honestly (doc 12) |
| Status coalescing | ≤ 10 Hz |

## Deferred, explicitly

- **Real-time multi-writer collaboration** — the op-based protocol is
  shaped for it (ops + authoritative sequencing), but CRDTs/merge UX
  are a product decision for later; the write lease covers v1.
- **Remote auth story** (accounts, TLS termination) — deploys behind a
  proxy until then.
- **WebTransport / QUIC** as a WebSocket upgrade path if head-of-line
  blocking ever shows up in profiles; frame format is transport-
  agnostic on purpose.
