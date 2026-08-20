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
  one project (`cicada serve file.cic` serves the file's directory and
  opens that pipeline by default). Clients name pipelines by plain
  project-relative paths only — absolute, rooted, or `..` forms are
  refused, and a session can never open or write a file outside the
  project.
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
   round-trip table: `place_node`, `connect`, `disconnect`,
   `accept_lift`, `set_param`, `rename`, `delete_node`,
   `toggle_disable` (the `#off` prefix; the delta says `disable x` /
   `enable x`), `move_node` (layout), `set_preview`, `undo`, `redo`,
   `batch` (several gestures as one op), `apply_text` (whole files,
   agents) — see §Undo/redo for the last four.
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
client beyond the header. **The byte-exact spec is the module doc of
`crates/cicada-server/src/frames.rs`** (encoder + decoder + round-trip
tests; mirrored by `web/src/protocol/frames.ts`). Shape (stage 5):

| Frame kind | Contents |
|---|---|
| `mesh` / `curve` / `point` (batch) | 32-byte header (magic, version, kind, generation, node ref, output, element range) + element table (element index, pick id, vertex/index ranges) + positions `f32×3` + indices `u32` (triangles / segment pairs / none) + per-vertex pick ids `u32` |
| `mesh_blob` + `instances` | a content-addressed mesh (blake3 hash, positions, indices) sent once, then per-output instance lists (element index, pick id, `f32 3×4` transform) |
| `clear` | header only — the output draws nothing this generation |

Normals are not transmitted: the viewport shades flat via screen-space
derivatives (CAD-correct hard edges, half the mesh bandwidth); smooth
normals arrive with the display-cost work if a use case needs them
(revised 2026-08-19 with the implementation).

- **Generation-tagged**: the client drops any frame older than the
  newest applied generation for that node — cancelled solves can
  never paint stale geometry (doc 04's "last coherent frame").
- **Instancing is hash-driven**: identical mesh hashes across elements
  arrive once as a `mesh_blob` plus an `instances` frame (doc 12
  interning → doc 04 instanced draws). Spike: transforms are the
  identity — hashes decide, not rigid-copy detection (v0.1 interner
  work); instancing is per node output (cross-node dedupe later).
- Pick IDs map viewport clicks back to (node, element index, part ID)
  — backward picking rides the same tables. Ids are stable per
  `(node, output, element)` for a session's life.
- The client **subscribes** to a display set (preview-enabled nodes +
  the currently inspected wire); the engine streams buffers
  progressively as elements complete. Spike: the display set is
  server-derived (every previewed, displayable output; the sidecar
  `preview` key toggles), frames go out per whole output only for
  outputs whose value hash changed since the last broadcast, and a
  joining client receives `display_reset` plus a full re-stream.
  Element-range streaming waits on the executor's chunk-level
  persistence (the frame header already carries the range).

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
| `GET /api/edit/text` | `{path, text, text_hash}` — the base an agent reads before editing (§Undo/redo) |
| `POST /api/edit/apply_text` | The atomic whole-file edit for agents / MCP (JSON body = the `apply_text` intent payload; same error kinds as the socket: 409 `stale_base`, 422 `parse_error` / `path_not_allowed`, 500 `io_error`). Applies even while a human holds the writer lease — the agent acts for the user; the delta reaches every client |
| `GET /api/git/…` | Status, node-level diff, commit, per-node history (doc 10 git integration) |
| `GET /debug/state`, `GET /debug/screenshot` | The agent/dev verification loop (doc 14). `state` (`?pipeline=&values=&wait=`) is the authoritative JSON oracle — graph view-model, text, statuses, summary, per-output display stats with bounds/triangles, lease, and `timings` (the last 1,024 generations: kind, `queued_ms` intent-arrival → start, `elapsed_ms`, `cancelled`, computed/cached counts, frame bytes, and `cancel_to_idle_ms` on a generation Esc ended — measured server-side, poll-free; the doc-15 measurement currency); `screenshot` (`?target=viewport`) asks a connected client to render the WebGL viewport to PNG (503 when no client is connected — loud, never blank; whole-page shots are Playwright's job) |
| `GET /health` | Readiness (no token) — Playwright's `webServer` waits on it |

## Stage-5 slice, stated honestly

What the spike ships of this document (doc 15): one project directory,
sessions per pipeline created on first open, the single-writer lease
with observers and the 5 s automatic hand-off, intents → minimal text
edits through the doc-10 writer (persisted immediately) → full
view-model deltas (the "delta" carries the whole graph view — hundreds
of KB worst case at wall scale; incremental node deltas wait for a
profile that asks), the ~30 ms structural debounce and the no-debounce
latest-wins preview loop, ≤10 Hz coalesced statuses with a
cost-weighted ETA from persisted samples (rough while ops lack samples),
frames as above, the project watcher (debounced; a `.cic`, sidecar, or
`scripts/*.py` change → reload → barrier snapshot), explicit effectful
runs on the shared scheduler with their own token (a slider drag never
cancels an export), and the debug endpoints. Wire compatibility during a
drag is answered by the SERVER (`probe_wire`: the checker evaluates the
hypothetical wire on a scratch copy — no second copy of the type lattice
in TypeScript; cost is one re-check per candidate port — fine at the
wall's ~10-node scale, an incremental checker's job beyond). Persisted
writes are atomic (temp file + rename) and a refused or failed gesture
rolls the in-memory document back to what is on disk. Shipped with
v0.1 item 1 (2026-08-20): undo/redo over the snapshot op log (§Undo/redo
below), the `batch` and `apply_text` paths, and `#off` node disable
(`toggle_disable`: the view-model renders a parsed `#off` line as the
node it is — ports, literals and wires intact — with `kind: disabled`
and the `disabled (`#off`)` exclusion; its downstream is red with the
disabled node NAMED). Still not in: transport, git routes, `/api/blob`
beyond value summaries, reconnect replay (a reconnect is a fresh
session join: one hydration path — the client retries with backoff and
re-hydrates), the cost-model's compute-on-release degrade for expensive
cones and scrub caching (every drag solves latest-wins; a slow cone
simply shows progress), groups (their keyboard rows notice loudly), the
sidecar's `port_order`/`color`/`collapsed` keys (carried, not rendered),
per-element frame ranges, and an auto-layout beyond "layer by
dependency depth, stack in definition order".

## Undo/redo (formalizing the ledger row)

- The engine keeps a **linear op log per pipeline**: `{id, label,
  actor: human | agent(prompt), state snapshot, timestamp}` — the
  snapshot is the pipeline text + sidecar BEFORE the op (revised
  2026-08-19: snapshots, not per-gesture inverse edits; any change from
  any source is one op, so new gestures are undoable for free).
  Sidecar-only ops (node moves, preview toggles) are undo steps;
  effectful runs are non-undoable and say so. *(Live, v0.1:
  `session.rs` `OpLog` — `VecDeque<Op {id, label, actor, before, after,
  at}>` with a cursor, cap 200, the `before`/`after` pair so redo is a
  restore too; `actor` serializes as `{"kind":"human"}` /
  `{"kind":"agent","prompt":…}`; `at` is server-monotonic ms. Every
  `delta` and `snapshot` carries `history {can_undo, can_redo,
  undo_label, redo_label, depth}` (`depth` = the cursor: undoable
  steps); `/debug/state` adds `history` and `ops: [{id, label, actor,
  at}]`. `param_preview` and `POST /api/run/{node}` never push an op;
  neither does a write that left text and sidecar identical (a move to
  no cell on an un-moved node, a re-sent text) — it is answered with a
  delta but is not an undo step. The `prompt` key is always present on
  an agent actor (`null` when absent). **Persist discipline**: text then
  sidecar, each temp + rename; a persist that fails half-way (the text
  landed, the sidecar could not — a transient lock on a synced project
  dir) takes the text off the disk again and rolls memory back under
  the same lock hold, so a refused edit is never left anywhere, and
  `text_hash` is by construction the hash of the text in memory — the
  value `GET /api/edit/text` ships and `apply_text` checks against.)*
- Continuous gestures coalesce: a slider drag or node drag is one op,
  created on release.
- An agent inference's graph edits apply as **one atomic labeled op**
  through the `batch` operation (intent + HTTP route): the whole new
  text (+ optional sidecar and script files), a label, the actor, and
  the **base** text hash the caller read. Stale base or a text that
  does not parse → refused with diagnostics (red wires are a valid
  state); otherwise applied under the session lock as one persist
  (temp + rename), one op, one delta — never a partial state on disk
  or in any client. Multi-node canvas gestures use the same path. An
  external agent (MCP) MUST use this route; a direct disk write is the
  external-change path below (barrier, stack cleared). Rebase onto a
  moved base is a later refinement — v0.1 refuses and the agent
  re-reads. *(Live, v0.1, as two intents: **`batch {ops, label}`** for
  the canvas — a list of write gestures applied in order under the
  lock, all or nothing; its error carries the FAILING op's `kind`
  (`refused` / `writer` / `unknown` / `protocol`, what the client reacts
  to) plus a flattened `index` saying where; and
  **`apply_text {base_text_hash, files: [{path, text}], label, actor}`**
  for agents — paths are project-relative and limited to this
  pipeline's `.cic`, its `.cic.layout.json`, and `scripts/*.py` beside
  it (`path_not_allowed` otherwise); refusals are `stale_base` (carries
  `current_text_hash`), `parse_error` (carries `diagnostics`), and
  `io_error` (a later file failed to land → the earlier ones were
  restored). A scripts change reloads the catalog and hydrates clients
  with a non-barrier `snapshot` instead of a delta — the log is NOT
  cleared. The HTTP pair: **`GET /api/edit/text`** → `{path, text,
  text_hash}` is the base to read, **`POST /api/edit/apply_text`**
  (JSON body = the intent payload) applies it — and it applies even
  while a human holds the writer lease, because the agent acts FOR the
  user; the resulting delta reaches every connected client. Status
  codes: 409 `stale_base`, 422 `parse_error` / `path_not_allowed`, 500
  `io_error`, 400 for a malformed body.)*
- `undo`/`redo` are ordinary intents; the engine applies the inverse
  edit and broadcasts the delta like any other change. *(Live:
  `undo {}` / `redo {}` — lease-gated writes that restore the op's
  `before` / `after` snapshot through the normal persist + delta path,
  labelled `undo: <label>` / `redo: <label>`; an empty side refuses
  with `nothing_to_undo` / `nothing_to_redo` and says why — no edits
  yet, everything already undone, or cleared by the reload barrier.)*
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
three-way-merge machinery. The session's own writes echo back through
the watcher too; they are recognised because after a successful persist
disk == memory — text by hash, sidecar by equality, scripts by the
loaded fingerprint — and that equality is the WHOLE echo guard: the
server keeps no memory of "what I last wrote", which would mask a real
external change back to a text it once wrote (git checkout there and
back) and leave memory stale.

## Reconnect and resync

Clients reconnect with their last applied `seq`. Small gap → replay
deltas; large gap or barrier crossed → **snapshot**: the full graph
view-model (bindings, ports, wires, badges, diagnostics, statuses,
layout) is small — hundreds of KB worst case — and display buffers
re-stream from the subscription set afterward. Snapshots are also the
initial load path; there is exactly one client-hydration code path.

## Security and serving

- `cicada serve` binds **127.0.0.1** by default and prints a URL with
  an embedded session token (Jupyter-style); the WebSocket, `/api/*`,
  and `/debug/*` require it (`?token=`, `Authorization: Bearer`, or
  `X-Cicada-Token`); the SPA and its assets load without it (the page
  reads the token from its URL). Same-origin only; no CORS headers are
  emitted.
- Remote deployment (the Onshape-style case) = the same binary behind
  a reverse proxy with real auth — explicitly out of scope for v0.1;
  nothing in the protocol assumes locality except the default bind.
- The engine binary embeds the SPA (`rust-embed`) behind the server's
  `embed` build feature — the release / CI-Playwright shape, so debug
  and test builds need no `npm run build`; without it `cicada serve`
  serves the API plus a built SPA from `--web-dir <dir>`, or says so
  loudly at `/` and dev uses Vite's proxy (`cd web && npm run dev`).
  Distribution is one file; the v0.2 desktop app wraps this same server
  + a webview.

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
