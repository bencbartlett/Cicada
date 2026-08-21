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
   agents) — see §Undo/redo for the last four. The transport controls
   (`transport_play` … `transport_reset`, §Animation transport) are
   intents too, but not edits: they change session state, write
   nothing, and answer with the `transport` broadcast, not a delta.
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

*(Live, v0.1 item 3b — the engine half.)* The decision is the
server's and is made **per tick, monotone within a drag**: every
`param_preview` predicts its own dirty cone (doc 12 §Cost prediction —
a hash-only dry run of that tick's keys against the memo, so warmed
values predict as the cache reads they are); a tick predicted at or
above `COMPUTE_ON_RELEASE_MS` (1 s) is **withheld** — always, whatever
earlier ticks of the drag did, so a drag that starts on a warm value
and moves onto cold ones never solves a multi-second preview live.
The session broadcasts ONE additive message per drag, on its first
withheld tick:

```json
{"v":1,"seq":N,"type":"preview_policy","payload":{
  "node":"deboss","port":"value","mode":"compute_on_release",
  "estimate_ms":3942.3,"rough":false,"pending_value":"0.875"}}
```

`port` is absent (not `null`) for a bare literal; `mode` is the only
mode ever announced (a cheap cone gets no message and previews
latest-wins as before); `estimate_ms` is the predicted wall time of a
live preview, a floor when `rough` (some node in the cone has no cost
evidence yet — render with `~`, like the ETA); `pending_value` is the
withheld tick's literal — the client tracks later ticks itself and
clears the pending state on the delta its release `set_param`
produces (or, when the pointer is released without a write, on its
own `end_drag` and the `drag_ended` it earns — below). Once a
drag has switched it never switches back: a later tick that is a pure
cache read (a value visited before, or warmed by scrub caching)
previews live — that is scrub caching's upgrade path — but any tick
that would compute stays withheld whatever its estimate, so nothing
flip-flops. The release is the one real op and the one generation,
exactly as before.

**What a drag is, server-side**: the run of ticks on one param closer
together than `DRAG_GAP_MS` (300 ms of op-clock time). A drag ends on
any write attempt (the release's `set_param`, any edit, a reload —
landed or refused), on Esc, on the client's **`end_drag`** (the
pointer released on the committed value — both sliders skip
`set_param` then, and send `{"type":"end_drag","payload":{"node":
"deboss","port":"value"}}` instead, `port: null` for a bare literal;
it needs the lease like the ticks, ends the drag when it is that
param's — an expired one included — and is a no-op otherwise, never an
error: a routine release must not raise a notice), on the writer's
departure or a lease handover, and on a pause longer than the gap; the
next tick starts a fresh drag, predicted again and **announced again**
if withheld, with its own `pending_value`. The gap rule is the
fallback for a release the server never hears of (a client that died
mid-drag); the release itself is `end_drag`, so a re-grab inside the
gap is a fresh drag too. Server-side, every write intent but the
preview tick and `end_drag` ends the drag at the dispatcher's door
(gestures, undo, redo, batch — landed or refused), and `apply_text`, a
reload and Esc end it at their own entries.

**The end of an announced drag is announced** (revised 2026-08-20 —
the review of the web half found the observer, and the writer's own
twin widget, had no signal for a drag that ended without a write: a
stale badge and a value that was neither committed nor pending stood
indefinitely): whenever a drag that `preview_policy` went out for
ends, the session broadcasts the additive

```json
{"v":1,"seq":N,"type":"drag_ended","payload":{"node":"deboss","port":"value"}}
```

(`port` absent for a bare literal) **after** whatever ended it
answered with — the delta of a landed write, the error of a refused
one (unicast: for every other client `drag_ended` is the whole news),
the snapshot of a reload — and on its own for `end_drag`, Esc, the
writer's departure and a lease handover. A drag that stayed live
throughout ends silently (nothing to take down). The one end that is
**not** announced is the gap rule's: it is decided lazily, at the next
tick, and a pause is not a release — the pointer may still be down
and the user looking at a viewport that is honestly not moving — so
the pending state stands until the release (or the re-announcement
replaces it).

**The frozen contract for the client** (the web client implements it
in `web/src/state/store.ts` — one `pending` param, replaced by every
arrival, cleared by a delta, a non-`lease` error, a `drag_ended` that
names it, a snapshot, a disconnect, or the widget itself on its own
release without a write, optimistically, as it sends `end_drag`; both
sliders render it as `pending · N s`):

1. `preview_policy` may arrive more than once for the same param —
   after a pause longer than the gap, after a release, after an Esc,
   after an undo. Each one is the current verdict with its own
   `pending_value` and `estimate_ms`: **replace** the pending state,
   never stack it.
2. Frames and statuses **can** arrive during a withheld drag: a tick
   that is a pure cache read (a value visited before, or warmed by
   scrub caching) previews live even after the drag has switched. A
   frame or a status is therefore never "the drag ended" — only the
   release's delta, `drag_ended`, the client's own release without a
   write, or a fresh `preview_policy` changes the pending state.
3. The server never sends a "back to live" message for a **standing**
   drag: a drag's policy stands until the drag ends, and a pause is
   not an end (a badge must not flicker off while the pointer is
   down). It does announce the **end** of every announced drag
   (`drag_ended`, above), the gap rule's excepted; the next drag is
   predicted afresh and is live unless announced otherwise.
4. The server relays no ticks: an observer's slider (and the twin
   widget of the one being dragged, in the same client, until it
   tracks the dragging widget's ticks) shows the policy's
   `pending_value` — the first withheld tick — with the badge, not the
   writer's live thumb. Honest, and marked pending; the release
   (`drag_ended` or the delta) brings the value.
5. `/debug/state` → `solve.previews_deferred` counts withheld ticks in
   total and `solve.drag` shows the standing drag `{node, port, mode:
   "live" | "compute_on_release", deferred, last_tick_ms}` (`null`
   between drags).

`PROTOCOL_VERSION` is unchanged (additive: two new message types an
old client ignores).

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

*(Live, v0.1 item 3b.)* A `cached` node's status carries `elements`
and `nanos` when its memo entry recorded the cost of its last compute
(node-level entries do, since 3b): the count is what the ETA's
per-node prediction multiplies by — it survives a warm reopen, where
nothing computes — and the time is the **last compute's**, which doc
12 §Progress asks the badge to show; it is never this generation's
(that was a cache read). Clients render it as "last 43.9 s" beside
`cached`, not as a bare time. Additive: both fields were already
optional on the wire, and `done` nodes carry this generation's numbers
as before.

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

*(Live, v0.1 item 4 — shipped whole 2026-08-20: the engine and the web's
play bar, `wt/transport`.)* **What the transport is.** Session state beside the
document, never in it: a playhead `t_ms` (milliseconds, unbounded,
0 at reset) read off the session clock — `t = anchor_t + (now −
anchor_clock) × speed` while playing, frozen while paused; every
control re-anchors at the current position first, so nothing jumps.
**How a frame reaches the graph.** The lowering is the injection
point: every lowering the session does — the structural graph, a
slider tick's scratch, a hypothetical, an explicit run — takes the
playhead and fills each transport-driven port (`catalog.json`
`transport_driven: "frame" | "time"`; the canvas hides them) with the
value it dictates, as a literal input: `clock.t` gets the seconds,
`cycle.frame` gets `floor(t × frames / period) mod frames` from the
node's own literal `frames` / `period` (or the spec's defaults). The
cone keys on the frame exactly as if the text said `frame=57`, and the
text never says it — a slider moved at frame 57 paints frame 57, an
edit while paused at frame 3 paints frame 3, an export writes the frame
the viewport shows. Headless, `cicada run` passes no playhead and the
ports evaluate as written (frame 0, t 0). A `cycle` whose `frames` or
`period` is wired rather than literal is the ONE red the transport adds
(`` `spin`: `frames` must be a literal in the app — the transport
quantizes the frame from the node's own frames and period ``); headless
it is an ordinary node. **Playback.** A ticker at the display rate (60 Hz) reads the
playhead and, when the driven ports' values moved since the last
hand-over, lowers the committed text at it and submits a transport job
to the one-slot latest-wins loop — the preview's policy: the in-flight
generation completes, the newest frame replaces a queued one, so the
solve bounds the rate (a slow cone skips frames, never queues them;
the skipped frames fill in on the next pass). A held slider's value
rides along in the frames while its drag is live (an announced
compute-on-release drag shows the committed state, as under its own
ticks). **The primary loop** — the `cycle` with the longest `period`
(ties: the first in the text), else cycle's defaults 120 frames / 4 s —
is what `frame`, `frames`, `period_ms` and `transport_seek` mean; other
cycles loop inside it at their own rate, each honouring its own
literals. **Control is writer-only** (the five intents are writes for
the lease's purposes and nothing else: not gestures, never batch
elements, never drag-enders, never an op, never a delta): playback is
shared session state every client sees, and the lease is the one
arbiter of shared state — two clients fighting over play/pause/seek
would make the shared viewport incoherent; observers follow, and take
the lease to drive. Esc pauses the transport along with cancelling the
generation; the last client leaving pauses it too (a session animating
for no one would be the ambient clock the ledger forbids); a reload
keeps it (the loop is re-read from the new text).

The wire shapes (additive; `PROTOCOL_VERSION` unchanged — an old
client ignores the new message and the new snapshot field). The view:

```json
{"playing": true, "speed": 1.0, "t_ms": 1250.0, "frame": 37,
 "frames": 120, "period_ms": 4000.0,
 "driven": [{"node": "spin", "port": "frame", "signal": "frame"},
            {"node": "elapsed", "port": "t", "signal": "time"}]}
```

`frame` is the primary loop's frame at `t_ms`; `driven` lists every
`cycle.frame` / `clock.t` that lowered (empty = no time params:
playback moves nothing). It rides every `snapshot` as
`payload.transport`, and it is the whole payload of

```json
{"v":1,"seq":N,"type":"transport","payload":{ …the view… }}
```

broadcast to every client after each control (refused or not,
nothing changes on a refusal), after Esc, when the last client's
departure paused playback, and when an edit or reload changed the
loop or the driven set. The view is a position at the moment of the
message: while `playing`, the client extrapolates `t_ms + elapsed ×
speed` for its own playhead display and trusts the next broadcast; the
frames themselves arrive as ordinary display frames from the transport
generations, which the statuses show like any other generation. The
intents:

| Intent | Payload | Effect |
|---|---|---|
| `transport_play` | `{}` | The playhead advances from where it stands at `speed`; the current frame paints at once. Idempotent while playing |
| `transport_pause` | `{}` | Freezes the playhead. Idempotent while paused |
| `transport_seek` | `{"frame": 57}` | Moves the playhead to the first representable playhead INSIDE the frame of the primary loop (nominally `frame × period_ms / frames`, nudged up until it quantizes back to `frame` — the nominal start rounds a few ulps short for some frames, so a bare seek would paint one frame low; `lower.rs` `Playhead::at_frame`), playing or paused — a paused seek paints the frame. `frame ≥ frames` is refused |
| `transport_speed` | `{"factor": 0.5}` | Playback rate, playhead ms per wall ms, from the current position. Not finite or `≤ 0` is refused |
| `transport_reset` | `{}` | Pause and rewind to `t_ms = 0` — frame 0, `clock` at 0, the values a headless run evaluates |

A refusal is the ordinary `error` with kind `transport` and the
intent's id (`"frame 500 is outside the loop (frames 0..120)"`,
`"speed must be a positive finite number, got 0"`); an observer's
control is kind `lease`. `/debug/state` carries `transport` (the same
view) and the transport generations have their own timing kind,
`transport`, beside `structural` / `preview` (`wait=true` is a quiet
oracle only while paused: between frames the loop is idle for an
instant, so it returns rather than hangs). Measured on the orbit
example (`examples/07-orbit.cic`, debug build, 15 nodes): the first
pass of the 120-frame loop is 120 generations, one per frame at 30 fps
(1,190 computed / 610 cached, p50 1.5 ms); the second pass is 120
generations, 0 computed / 1,800 cached, p50 0.43 ms — pure cache
playback; 0 deltas, 0 ops, the file's bytes untouched.

## HTTP surface

| Endpoint | Purpose |
|---|---|
| `GET /` | The embedded SPA |
| `GET /api/catalog` | Node-spec JSON catalog (docs/08) |
| `GET /api/project` | `{project, pipelines, scripts, default, open, git: {kind, branch, dirty_count}, engine, protocol}` — the pipeline list, the `scripts/*.py` beside them, and the git summary (`kind` = the git state's tag; `dirty_count` = `git status` entries under the project dir; an unexpected git failure is `kind: error` + `error`, never a failed route) |
| `GET /api/blob/{hash}` | Large payloads on demand (full inspector data, export previews) |
| `POST /api/run/{node}` | Effectful nodes — requires the explicit-run confirmation, streams progress over the session socket |
| `GET /api/edit/text` | `{path, text, text_hash}` — the base an agent reads before editing (§Undo/redo) |
| `POST /api/edit/apply_text` | The atomic whole-file edit for agents / MCP (JSON body = the `apply_text` intent payload; same error kinds as the socket: 409 `stale_base`, 422 `parse_error` / `path_not_allowed`, 500 `io_error`). Applies even while a human holds the writer lease — the agent acts for the user; the delta reaches every client |
| `GET /api/git/status` | `?pipeline=` → `{state, pipeline: {path, tracked, ignored, dirty, nodes: [{name, change, from?}], removed: [{name, line_in_head}]}, scope: [{path, status, in_head}], text_hash}` (doc 10 §Git integration, slice 1: working tree vs HEAD). `state` is tagged `kind`: `repo {root, prefix, branch, head_short, upstream: {name, ahead, behind}?, unborn, operation?}` \| `locked` (the SAME fields as `repo` — `index.lock` is held, by another git or by our own commit: status still answers, writes wait, the branch chip keeps its facts) \| `not_a_repo` \| `git_not_found`. `operation` ∈ `merge` / `rebase` / `cherry_pick` / `revert` when the shell left one unfinished (`MERGE_HEAD` etc.) — writes refuse `operation_in_progress` until it is done. `change` ∈ `added` / `modified` / `removed` / `renamed` (`from` = the HEAD name); markers are computed FROM `git diff -U0 HEAD -- <path>` (hunks → binding lines, one binding per line), so they cannot disagree with it; a rename pairs a removed + added line with a byte-identical right-hand side **within one hunk** (the writer's `rename` gesture rewrites one line; a deletion here and an unrelated same-literal addition elsewhere are two hunks → `removed` + `added`); the sidecar never marks a node; an untracked pipeline is every node `added`; an ignored one (`.gitignore`) is `ignored: true`, every node `added`, nothing in the scope. `scope` = the dirty files of the commit scope — this pipeline's `.cic`, its sidecar, `scripts/*.py` beside it (the `apply_text` set), project-relative, `status` ∈ `modified` / `added` / `deleted` / `untracked` / `renamed`, and `in_head` = HEAD has a version of the path — the rule `revert` restores by, published per file so no client re-derives it from `status` (they disagree: porcelain `AD`, added to the index then deleted from disk, is `deleted` with no HEAD version; everything on an unborn branch has none); ignored files are left out (git does not list them and `git add` refuses a list containing one). `text_hash` = blake3 of the working file the markers were computed against (clients dedupe on it). Reads only: every invocation carries `--no-optional-locks`, so a refresh never touches the project and never wakes the watcher — the route test asserts `.git/index` is byte-for-byte untouched across refreshes of a dirty tree (what the flag buys) and the command builder's unit test asserts the flag on every invocation — and no session is opened for it (status is a read about a file: polling it for a pipeline nobody has open must not start hydrating and solving one) |
| `POST /api/git/commit` | `{message, client?}` (writer-gated: `client` or `X-Cicada-Client` must be the lease holder of the pipeline's OPEN session — committing is a git action on the project, not a document edit, hence unlike `apply_text`; a pipeline nobody has open is 403 `lease` with the reason, never opened on the caller's behalf) → `git add -- <scope>` then `git commit --cleanup=verbatim -F - -- <scope>` (the message verbatim on stdin, written from its own thread so a git that exits early — a failing hook — still reports ITS exit code and stderr whatever the message's length; `-- <paths>` commits ONLY the scope, so whatever else the user staged in a shell stays staged; never `add -A`) → `{hash, short, summary, files}`. 422 `empty_message`, 409 `nothing_to_commit` / `not_a_repo` / `git_not_found` / `ignored` (the pipeline is matched by `.gitignore`: git refuses to add it) / `operation_in_progress` (+ `operation`), 423 `locked`, 403 `lease`, 500 `git_failed` (with `command`, `code`, `stderr`) / `git_timeout` / `internal` |
| `POST /api/git/revert` | `{paths?, client?}` (writer-gated as above) → `git checkout HEAD -- <paths>` for the dirty scope files that HAVE a HEAD version (the status's `in_head`; `paths` narrows the set — the client's confirm step lists exactly the `in_head` files and names exactly those; 422 `path_not_allowed` outside the scope) → the session reloads through the external-change path (`reload_from_disk` → ONE barrier snapshot, `reason: "git revert"`, op log cleared). Checkout and reload run under the session's **write hold** (`Session::hold_writes` → `reload_from_disk_held`): no intent, undo, `apply_text` or watcher reload can persist between the two — a slider drag arriving mid-revert applies to the REVERTED text afterwards instead of overwriting the restored file (which would have made the reload a no-op and the revert silently lost) — so `reloaded` is always `true` when the files changed and the barrier's reason is always ours; the watcher's later wake finds disk == memory and does nothing. → `{reverted, untracked, reloaded}`. Files without a HEAD version are never deleted: `untracked` lists the ones left alone; an untracked (or ignored) pipeline, or an explicit ask for one, is 409 `untracked`; 409 `nothing_to_revert` / `operation_in_progress`; 500 `reload_failed` when the files are back on disk but the session could not load them (previous state stays live). Measured (route test, debug build, Windows): POST → barrier snapshot on the socket ≤ 35 ms. Every git-route failure body is `{kind, message, …}` with `kind` the snake_case `GitErrorKind` enum in `protocol.rs`, mirrored by the client — including pipeline resolution (`protocol` 400, `no_such_pipeline` 404 with `path`) and server-side failures (`internal` 500); the one exception is the token middleware's 401, text like every route's |
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
disabled node NAMED). Shipped with v0.1 item 3b (engine half): every generation owns its
cancel handle — an explicit effectful run, the interactive loop and an
idle-class hypothetical solve (`Session::solve_hypothetical`, doc 12
§Speculative warming) each have their own, and the script bridge's
kill switches hang off the calling generation's token, so "a slider
drag never cancels an export" is true by construction; and the
compute-on-release degrade for expensive cones (§Slider drags above —
the `preview_policy` message, and both sliders' `pending · N s`
rendering of it). Shipped with v0.1 item 2 (2026-08-20, server half):
the three `/api/git/*` routes above over the git binary (`git.rs`;
`GET /api/project` gained `scripts` and `git`). Shipped with v0.1 item 4
(shipped whole): the transport — `TransportView` in every snapshot (each
driven port carrying its OWN loop for the inspector), the `transport`
broadcast, the five `transport_*` intents, the playhead injected at
lowering, playback over the preview loop, AND the web's play bar (Space,
the scrubber, hidden ports on both surfaces with the server refusing a
wire into them, the e2e — §Animation transport). Still not in:
the other git refs / graph-diff overlay / per-node history, `/api/blob`
beyond value summaries, reconnect replay (a reconnect is a fresh session
join: one hydration path — the client retries with backoff and
re-hydrates), scrub caching (its substrate — the idle class — is in; the
warmer and the buffer bar are item 5), groups (their keyboard rows
notice loudly), the sidecar's `port_order`/`color`/`collapsed` keys
(carried, not rendered), per-element frame ranges, and an auto-layout
beyond "layer by dependency depth, stack in definition order".

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
