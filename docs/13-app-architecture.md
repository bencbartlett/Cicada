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

- `cicada serve [path]` serves exactly one **root** directory (v0.1 wave
  4 O1, docs/17): a **project** directory — normally a git repo root with
  `*.cic` pipelines, `scripts/`, sidecars — when one is named; a
  pipeline's directory when a `.cic` file is named (that pipeline opens
  by default); the user's **home directory** when nothing is named, and
  then nothing opens — the app's picker lists the root through `GET
  /api/files`, one directory at a time, and a pipeline opens on the
  first `?pipeline=`. Clients name pipelines by plain root-relative paths
  only — absolute, rooted, or `..` forms are refused, a path whose
  canonical form leaves the root (a symlink out) is refused, and a
  session can never open or write a file outside the root. Inside the
  server the root keeps its stage-5 name, the project
  (`ServeConfig::project_dir`): `apply_text`'s paths, the git handle's
  cwd and `/api/project`'s bounded walk are relative to it; scripts are
  discovered beside each pipeline, whatever the root.
- Each browser tab opens one pipeline = one **session**. A tab that
  switches pipelines (File → Open / Recent, the Back button — v0.1 wave
  4 O2, docs/16 §Application layout) closes its socket and joins the
  other pipeline's session afresh: the `?pipeline=` parameter changes
  (`history.pushState`), the client's mirror is cleared, and the join's
  `hello` + `snapshot` + restream hydrate it — the one hydration path,
  nothing pipeline-specific survives on the client. The sessions are the
  server's and outlive the visit (the one left behind keeps solving for
  whoever else has it open; with nobody left it pauses its transport and
  stays warm). **A pipeline the server cannot open is refused INSIDE the
  socket's handshake, never at the upgrade** (wave 4 O2 review,
  2026-08-24): the token middleware gates the upgrade, and after the
  version verdict the server answers the `hello` with one `error` of
  kind `pipeline` — `reason` ∈ `unnamed` / `path_not_allowed` /
  `not_found` / `open_failed` (`protocol::JoinRefusal`, the ONE
  classification the HTTP routes map to 400 / 400 / 404 / 422) and
  `pipeline` as the client sent it — then Close; no lease, no hydration,
  no session opened for a reference a route would refuse. A refused
  upgrade reaches a browser as a bare close code (1006) with no body,
  which the app could only read as a network drop and retry forever; the
  typed refusal is terminal for the client: no reconnect, the reason as
  a notice, a `not_found` file dropped from Recent, the tab back on the
  picker with the dead URL replaced (docs/16 §Application layout).
- **Single-writer lease** per pipeline: the first client takes the
  write lease; further clients are live read-only observers (which
  also gives presentation/tablet views for free). The lease transfers
  by explicit UI action, or automatically when the writer disconnects
  (5 s grace). No merge machinery exists in v1 — by design.
  **The join hint** (v0.1 wave 4 O3, additive — `PROTOCOL_VERSION`
  unchanged): the client's `hello` may carry `role: "observer"` —
  `{"type":"hello","payload":{"v":1,"role":"observer"}}` — and the
  socket then joins as a **declared observer** that never holds the
  lease: it is not made the writer at its join even when the lease is
  free, it is never promoted after the writer's departure (the grace
  period's transfer skips it; with only declared observers connected
  the lease stays free and the writer's reconnect takes it back at its
  join), and its `take_lease` is refused with kind `lease` and the
  reason. It reads everything an observer reads — the same snapshot,
  deltas, statuses and display set. The pop-out viewport
  (`?view=viewport`, docs/16 §Viewport conventions) joins this way, so a
  second window of the same person can never steal the first one's
  lease, a reconnect included. No `role` (an older client) or `role:
  "writer"` is the rule above.
- AI agents are server-side actors (doc 11): their edits enter the
  same op pipeline as a client's, as one atomic batch op, and
  broadcast to clients the same way.

## State ownership and sync

Two planes over one WebSocket:

- **Control plane — JSON** (debuggability beats compactness at these
  sizes): versioned envelope `{v, seq, type, payload}`.
- **Data plane — binary frames** for geometry buffers only.

One socket, but not one queue: each plane has its own lane to every
client and the control lane drains first (§Two lanes, one socket).

The edit flow is **intent → authoritative delta**:

1. The client sends a gesture-level intent matching doc 10's
   round-trip table: `place_node`, `connect`, `disconnect`,
   `accept_lift`, `set_param`, `rename`, `delete_node`,
   `toggle_disable` (the `#off` prefix; the delta says `disable x` /
   `enable x`), `move_node` (layout), `set_preview`, `set_collapsed`
   (layout too — the collapsed slider, wave 4 B4), `set_scrub` (the
   scrub-caching kwarg, v0.1 item 5), `undo`, `redo`,
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

`set_param` is the one intent for every literal edit — a slider's
release, the param widgets, and (wave 4 B3, 2026-08-24) the
typed-literal chips on any node's unconnected literal-typed ports
(docs/16 §Canvas conventions): it names `{node, port, value}` with the
value already spelled as a dialect literal, the server refuses anything
that is not one literal token, rewrites the kwarg's literal in place
(inside its `each(…)` when the text lifts it — the lift stays), or
**adds it at its spec-order position when the call lacks it** (a placed
node's port — docs/10 round-trip table). The view-model's `InputView` carries
what such an editor needs: `literal` / `literal_value` (the kwarg as
written and parsed), `default` (the catalog rendering) and, since B3,
`default_value` — that rendering parsed in the port's kind by the
server (`viewmodel.rs::default_json`; the macro spells a Boolean
default `true`, the chip says `True`), so no client re-derives the
catalog's spelling. Additive; the protocol version is unchanged.

Two additive pieces for the sliders (wave 4 B4, 2026-08-24; docs/16
§Canvas conventions — the GH slider shortcut and the collapsed slider):

- **`place_node` carries `params`** — `[{port, value}]`, absent = none:
  literals the placement writes into the new node as it lands, each
  applied exactly as a `set_param` on that port (one literal token, the
  spec-order insertion, transport-driven ports refused) BEFORE any
  `connect`, so the search's `1<20` is ONE op — `place slider` — and one
  undo removes the slider whole. The `batch` path could not do this: the
  server assigns the auto-name (`slider_1`) that the later `set_param`s
  would have to name. A param naming a port the node lacks, or a value
  that is not one literal, refuses the whole placement (`unknown` /
  `refused`; no node, no cell, no op); so does a port named by BOTH a
  param and the `connect` (`protocol` — a contradiction the client wrote,
  not a tie the server breaks).
- **`set_collapsed {node, collapsed}`** writes the sidecar's existing
  `collapsed` override as an op like a move (`collapse x` / `expand x`;
  sidecar only, no solve; `false` clears the override, so an expanded
  slider has no entry). The view-model carries it as `NodeView.collapsed`
  (omitted when false) with `size` already `[w, 1]`. THE rule is the
  view-model's `collapse_refusal`, read off the DOCUMENT — the binding's
  call (`func == "slider"`; a kwarg whose unlifted value is a reference
  is wired), never off the graph view: inside a `batch` the view lags
  the document (it is rebuilt at the commit), so a bound wired, unwired
  or a slider placed by an earlier op of the same batch is seen by the
  `set_collapsed` that follows — `batch[connect n.out → size.max,
  set_collapsed size]` is refused whole (the failing op named, nothing
  lands), `batch[disconnect bound.max, set_collapsed bound]` and
  `batch[place_node slider, set_collapsed slider_1]` land (the first cut
  read the view and let the flag land silently on a slider the batch had
  just wired — review finding, 2026-08-24). Only a slider, and only while
  `value`, `min`, `max` and `step` are literals (the collapsed row is
  name · track · value · output — the track IS `value`): the server
  refuses (`refused`) a non-slider and a slider with a wired port of that
  row ("`bound`: max is wired — a slider collapses only while value, min,
  max and step are literals (the collapsed row has no port for a wire)";
  the ports named in spec order); the client only MIRRORS the reason as a
  hint (`collapseHint`, keyed on `func` like the rule — the notice carries
  the hint verbatim, and `slider.spec.ts` asserts it does). A slider
  whose bound a later text edit wires is drawn expanded while the wire
  stands — the flag stays in the sidecar and takes effect again when the
  wire goes.

**Scrub caching** (v0.1 item 5 S1, 2026-08-24; docs/12 §Speculative
warming; DECISIONS.md row 39) — additive, `PROTOCOL_VERSION` unchanged:

- Every slider's `ParamView` carries `scrub: {on, positions, warmed,
  warming, bytes, capped?, ineligible?}` (`protocol::ScrubView`): `on` is
  what the TEXT says (`scrub=True`), `positions` the step-quantized count
  (0 when ineligible), `warmed` the position indices verified warm
  (ascending), `warming` whether work remains (the bar's pulse), `bytes`
  what the warming stored for this slider, `capped` (omitted when false)
  that the 256 MiB cap stopped it, and `ineligible` (absent when
  eligible) the server's reason — `too many positions (51 > 32)`, `max is
  wired — the positions are a function of literal min, max and step`,
  `step is 0 — a continuous slider has no positions to warm` — which the
  toggle is greyed with; the client computes nothing. The session
  overlays its queue's state on the view at every rebuild and every
  position, so each `snapshot`/`delta` carries the current warm set.
- `scrub_progress {node, port, warmed, warming, bytes, capped?}` is
  broadcast coalesced at the statuses' cadence (≤ 10 Hz, one per slider
  however many positions landed in between) while a queue moves; it
  updates that slider's `param.scrub` in place and is never sent for a
  slider without a queue. (The web client keeps it as an overlay BESIDE
  the graph — `store.scrubProgress`, by slider, merged on read by
  `state/scrub.ts::mergeScrub`, cleared by every snapshot / delta since
  their views carry the warm sets — so a 10 Hz broadcast never rebuilds
  the React Flow nodes or re-routes the trace wires; S2, 2026-08-24. The
  merged view feeds the buffer bar and the toggle, docs/16 §Sliders.)
- `set_scrub {node, on}` is a write gesture (a `batch` element, an op
  labelled `scrub x on` / `scrub x off`, undoable; the delta carries the
  new view and the warming starts from it): `on` writes `scrub=True` into
  the call at its spec-order position, `off` removes the kwarg. Refused
  (`refused`) for a non-slider ("`x` is not a slider — only sliders
  scrub-cache") and, when `on`, for an ineligible slider with the reason
  above ("`x`: too many positions (51 > 32)"; a slider whose `value` is
  wired has no widget and says so — "`x`: value is wired — a wired
  slider has no widget to scrub"); `unknown` for a name nobody bound;
  the rule is `scrub::eligibility`, read off the DOCUMENT so a batch
  sees what it wired earlier. Turning an ineligible slider's
  hand-written `scrub=True` off is always allowed.
- `/debug/state.scrub` = `{state: idle | working | blocked | parked,
  parked_until, byte_cap, max_positions, queues: [{node, port, id,
  positions, values, order, warmed, visited, in_flight, next, bytes,
  capped, warming}]}` — every warming queue with its visiting order (the
  harness and the tests read it; `wait=true` does not wait for the
  warming, which is invisible to `wait_idle`).

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
  work); instancing is per node output (cross-node dedupe later). The
  blob's hash IS its content: the client keeps blobs by hash for the
  page's life and drops a re-sent hash, so a `Solid` drawn through
  tessellation (v0.1 item 3) is keyed by its DISPLAY MESH's value hash,
  never by the Solid's — the same solid at another deflection tier is
  another blob (2026-08-21; docs/12 §Display cache).
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

## Two lanes, one socket

*(Live, v0.1 hardening, 2026-08-20.)* Both planes share the WebSocket,
not a queue. The server reaches every client through two lanes — the
**control lane** (the JSON texts) and the **display lane** (the binary
frames) — and the socket's write task drains control first (a `biased`
select), the display lane in its own FIFO otherwise. Why: a page that
joins receives the whole display set — the wall: ~350 MB of frames —
and with one queue per client every text queued behind it:
`preview_policy` reached a freshly joined observer ~26 s after the
writer's drag (measured 2026-08-20), and a delta, a status or a lease
change would have waited as long. With the lanes a text leaves the
server behind at most the one frame already handed to the socket; the
restream resumes behind it. Control texts are small and ≤ 10 Hz
(statuses are coalesced), so the display lane is never starved in
practice. Nothing on the wire changed — no message, no frame byte,
`PROTOCOL_VERSION` — only the interleaving of two planes the client
already keeps apart.

**The join-time half** (review 2026-08-20). The lanes alone did not make
a join fast: the socket's write task used to start only after the
restream was built, and `restream_display` built it — the wall: ~370 MB
of frames, ~3 s of store reads and encoding on a debug engine — under
the session lock. So a joiner saw nothing for those seconds (measured:
socket open → `hello` 3,031 ms), and every other client's intents and
every broadcast waited on the lock with it (a tick sent 50 ms into a
join was answered after 3,202 ms instead of 1–2 ms). Now
`attach_client` (`http.rs`) registers and hydrates the
client under one lock hold (`Session::join`: `hello`, `snapshot` on the
control lane), starts the write task, and only then starts the restream
on the blocking pool; and `restream_display` holds the lock three times
briefly — to read its plan (the display table as of the `display_reset`
it enqueues), per output to hand the encoded frames over, and for the
notices — loading and encoding outside it, with the pick table behind
its own mutex (lock order `inner` → `picks`, never the reverse). The
per-output hand-over is a **compare-and-send**: if the table no longer
says what the plan read (hash and generation), the live path already
broadcast the newer frames — or the `clear` — to this client, which was
in the roster before the plan was read, and the plan's frames are
dropped; sent, they would undo a clear on the client (a cleared output
has no generation left to compare with) or be dropped by its per-output
rule. Measured after (`tools/measure/lanes.mjs`, debug engine, wall
cached): socket open → `hello` 6–7 ms (was ~3.0 s), → `display_reset`
and the first frame 8 ms, → the last frame 3.1 s (the frames now stream
as they are encoded — the small outputs first, the plan is sorted by
node ref — instead of landing together after the build); a tick sent
50 ms into the join is answered in 1.3 ms, the observer's copy behind
the 20 small frames encoded so far (3.4 MB) — it was 3.2 s, behind all
26. The restream's build outside the lock takes the pick table's mutex
for ONE ask per output — `display::PickIds`: every drawn element's id,
allocated up front, before the tessellation and the encode — never
across the encode (review 2026-08-21: it used to hold the table for
the whole encode, up to the 94 MB output's most-of-a-second, and the
live path takes the table while holding the session lock, so every
client's intents could stall for one encode during any join). A client
that leaves mid-restream stops costing at the next output boundary —
the leave check precedes the load and the encode (`/debug/state`'s
`picks.encodes` counts encodes; the test reads it).

**What stays ordered with the frames.** Two texts are display-plane by
meaning and ride the display lane, FIFO with the frames:
`display_reset`, the header of the restream it announces (the lane's
order is what puts it ahead of them), and `screenshot_request`, which
renders what the frames before it painted — `GET /debug/screenshot`
after `/debug/state?wait=true` keeps meaning "the completed
generation".

**Why control overtaking display is harmless — by construction, not by
luck.**

1. The client keeps the planes apart. The store renders the graph,
   statuses, history, lease and pending state from texts; the
   viewport's ledger (`web/src/viewport/sceneStore.ts`, fed by the
   frame bus) renders frames keyed by `(node ref, output)`. No code path
   waits for a frame to apply a text, or for a text to apply a frame;
   the one text the ledger reacts to is `display_reset`, which is
   display-lane traffic.
2. The display lane is FIFO and per-output generations are monotone,
   so the ledger converges on the server's display table whatever texts
   land between frames: a frame is replaced by the newer one behind it,
   a vanished output is cleared by the `clear` behind it, a restream
   re-applies idempotently (`web/src/state/frameBus.test.ts` drives the
   interleavings, including a reset that overtook the frames queued
   before it — the order a second socket would produce). The argument
   needs the ledger to EMPTY on every `display_reset`, so the client
   counts resets (`displayResets`) rather than watching the reset's
   generation change: that generation is the MAX of the server's table,
   and an output that vanished meanwhile — its `clear` lost with a
   dropped socket — can leave the max unchanged, so a reconnect's or a
   `resync_display`'s reset repeats it; keyed to the generation, the
   ledger kept the vanished output painted (found and fixed
   2026-08-21, the test pins it).
3. What a user can see: a `delta` that removes a node may arrive before
   the node's `clear` frame, so the viewport draws its last geometry a
   moment longer — the same pixels it drew a moment before; a pick on
   it resolves to no node and selects nothing. Statuses,
   `preview_policy`, `drag_ended`, `lease`, `error` read nothing from
   the scene.
4. What would NOT be safe, recorded so nobody adds it: dropping frames
   "older than the reset's generation". A restream carries each output
   at the generation that last drew it and the reset's generation is
   the newest of those, so unchanged outputs legitimately arrive below
   it. Staleness is per output (§Binary frame format), never relative
   to the reset.

**What the socket promises — and what it does not.** The promise is
structural: a control text goes out behind at most ONE display message,
whatever its size, and the display lane stays FIFO behind it. It is not
a latency promise. The wall's restream is 26 frames, 368 MB, the
largest 94.4 MB; the one frame in flight may be that one. The 13.5 MB/s
figure from the original measurement (350 MB in 26 s) is the PAGE's
consumption rate — decode, GPU upload, render — not the wire's (the
whole restream reaches a fast Node client over loopback in ~0.3 s,
> 1 GB/s), so the status cadence (≤ 10 Hz) is NOT reached for the
wall's frame sizes on the page: a text lands behind every frame the
browser had already buffered ahead of a busy handler, and each of the
large ones costs seconds there (measured end to end below). Reaching
the cadence at wall scale is the display plane's work, not the
socket's: chunked / element-range frames so the unit in flight is
small (the frame header already carries the range; waits on the
executor's chunk-level persistence), frame decoding off the main
thread, rAF-coalesced renders — and the per-output latest-wins queue
under §Still one socket.

**Tests** (`http.rs`, a recording sink paced by permits — no sleeps, no
wall clock; a 60 s deadline exists only to turn a deadlock into a loud
failure). The pump's priority and FIFO, and its `biased` select pinned
by 64 pre-queued messages per lane (an unbiased select passes the
two-message test 1 run in 12; this one with probability 2⁻⁶⁴). A
wall-sized synthetic restream — the 94.4 MB frame first, then 319
distinct 1 MiB frames — queued on the display lane of a client attached
through `attach_client` (the channels and lane wiring `client_loop`
serves — review 2026-08-21: with its own channels this test passed with
both lanes merged into one there), then a slider tick: exactly the
frame in flight precedes the tick's status, every further text precedes
the rest of the restream, the restream resumes FIFO, the tick's repaint
follows it. The join-time half, with a restream parked on
`SessionConfig::restream_hold` (a test seam in the shape of
`op_clock`): the joiner's `hello`, `snapshot`, `display_reset` are on
the wire while the restream builds; a `wire_probe` plugs the in-flight
slot; an intent lands and is answered meanwhile (the lock is free); a
second `wire_probe` asked for AFTER the tick's repaint was queued still
precedes it on the wire — the served wiring is two lanes, not one; when
the restream resumes it does not resend the output the intent
superseded (the client keeps the live frame) and does send the
unchanged one. A client that disconnects while its restream is parked:
the restream resumes, encodes nothing (`picks.encodes` unchanged),
sends nothing, ends. The lane assignment of `display_reset` and
`screenshot_request`: frames already queued to a client precede a
resync's reset, the restream's frames precede the screenshot ask, a
control text enqueued last overtakes them all (both texts could move
to the control lane with every other test green). `tests/http_e2e.rs`:
the join's wire order (`hello`, `snapshot`, then `display_reset` before
any frame). Mutations that must fail, and do: no `biased`; either
display-plane text on the control lane; the compare-and-send removed;
the restream built before the pump; the lanes swapped or MERGED in
`attach_client`; the leave check moved back behind the encode.

**Measured** (the wall, Ben's machine, debug engines, 2026-08-20/21).
The lanes' evidence is the WIRE (`tools/measure/lanes.mjs`, no
browser; scratch copies of `examples/`, scratch caches, the wall solved
first). The "before" engine is `24d558b`'s (sha256
`39b1c29f…433d94`, built 2026-08-20 18:21 from a tree whose `git diff
24d558b -- crates` was empty; the first lanes commit is 18:43 — the
binary's age is what vouches for it, no other record survived). One
queue per client: socket open → `hello` 2,938–3,074 ms (the restream
built under the lock before the joiner's texts); a tick sent the
moment the observer has its snapshot reaches it after ALL 26 frames /
368 MB (278–331 ms after the tick — the wire's time for 368 MB, the
head-of-line signature); a tick 50 ms into the join 3,160–3,348 ms
after it, behind all 26. The lanes (the final shape, with the pick
table held for the id ask only; sha256 `cca36d25…df26e`): open →
`hello` 5.6–6.2 ms; the at-snapshot tick answered 1.3–1.4 ms after it
behind NO frame (the `display_reset` is out, the first output is still
being encoded); the 50 ms tick 1.3 ms after it behind the 19–20 small
frames encoded so far (2.3–3.4 MB) — mid-restream; a cheap slider's
`status` 2 ms. Two runs each, 2026-08-21; the review's own runs of
both engines the same day agree, and its hostile states — a
`resync_display` during the join's restream (two concurrent restreams
to one client: the ledger converges, 0 mismatches against the server's
table), a joiner leaving 80 ms in and a fresh one after it (full
368 MB, no errors), three simultaneous joiners and a tick (each
answered in 1.1–1.2 ms) — were all clean.

The app-side number is NOT a before/after of the lanes. The heavy spec
(`web/e2e/compute_on_release.spec.ts` under `CICADA_E2E_HEAVY=1`,
headless Chromium) logs the observer's `preview_policy` latency after
the writer's grab; it is set by where the PAGE's frame handling stands
at the grab, which the spec does not control — the observer's setup
(a tab click, a disabled-slider expect) takes about as long as a debug
engine writes the 368 MB (~3 s) — so whether any frames remain
unhandled at the grab is luck, and the one-queue engine can post the
BETTER number: reproduced 2026-08-21, the `24d558b` engine 192 ms with
26 of 26 frames already handled at the grab, this branch 7,284 ms with
23 of 26 in (the earlier paired runs, 21.0 s → 5.9/11.7 s, carried the
same confound: 14 vs 24/21 frames in at the grab). The residual IS the
page's queue, and the lanes cannot touch it: the browser takes the
whole restream into its message queue faster than it handles the
frames (368 MB reach a Node client in ~0.3 s; the page needs seconds
per 27–94 MB frame), so a text sent once the server has written its
frames waits behind every frame the page has not handled yet — on the
wire it is, legitimately, last. The lanes reorder what is still queued
at the SERVER: the text sent during the build (the debug engine's
~3 s) or while the socket is backpressured. Where the page's seconds
per frame go — headless Chromium rendering the wall's ~13 M triangles
in software (SwiftShader) once per arriving frame, ~1.5–2 s per render
on this machine, or decoding and GPU-uploading a 94 MB frame on the
main thread — is a hypothesis from an uncommitted long-task trace
(eight back-to-back tasks of 1.0–2.2 s, `renders: 12`), not separated,
not measured on a GPU browser. The spec therefore MEASURES the
observer's hint (logged, attached, annotated "mid-restream" or not)
under a 60 s sanity bound, as a diagnostic of the page; the socket's
order is pinned by the server tests and `lanes.mjs`. An end-to-end
discriminator would have to make the restream slow ON THE WIRE for the
observer (CDP `Network.emulateNetworkConditions`, or a throttling
proxy) so the text's place among the frames reflects the socket order
— not built. The page-side fix is the display plane's: handle frames
off the main thread (decode in a worker, hand the scene typed arrays),
so the queue drains at memcpy speed and a text behind it lands in
milliseconds; chunked / element-range frames keep the unit of work
small once they exist.

**Still one socket, still one lock for the live path.** A generation
that completes while a client's restream is in flight queues its frames
behind the rest of the restream (display-lane FIFO) — display-vs-display
head-of-line blocking is not this fix's subject. If it shows up, the
next step is a per-output latest-wins display queue (drop queued frames
a newer generation has already superseded) before any transport change;
WebTransport stays deferred (below). And the LIVE frame emission
(`emit_frames`, the completion hook) still loads and encodes under the
session lock — only the outputs whose hash changed, so a slider tick on
the wall's `deboss` is one ~94 MB carve, a fraction of a second, not the
whole set; the restream's shape (plan under the lock, encode outside,
compare-and-send) is the fix if a cold open's full emission ever shows
up as an intent stall.

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
the skipped frames fill in on the next pass). The ticks lie on an
absolute grid — tick N at `anchor + N / 60 s`, the anchor being the
instant of the last control — walked with a high-resolution sleep, not
a timed condition-variable wait (whose 15.6 ms quantum on Windows made
the ticker run at ~33 Hz, so a 60 fps loop never warmed in one pass;
review 2026-08-21), and re-anchored by every control so a seek on a
60 fps loop keeps the ticks in phase with the frame boundaries.
Measured with `tools/measure/transport_loop.mjs` (debug build,
Windows): a 240-frame / 4 s loop plays at 60.0 generations/s, start
gaps p50 16.66 ms / max 20 ms, every frame visited on the first pass
(239 computed generations) and the second pass 240 generations with 0
computed. A held slider's value
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
generation — paused first, cancelled under the same document-lock hold
the ticker submits under, so no frame slips in behind the Esc; the
last client leaving pauses it too (a session animating
for no one would be the ambient clock the ledger forbids); a reload
keeps it (the loop is re-read from the new text).

The wire shapes (additive; `PROTOCOL_VERSION` unchanged — an old
client ignores the new message and the new snapshot field). The view:

```json
{"playing": true, "speed": 1.0, "t_ms": 1250.0, "frame": 37,
 "frames": 120, "period_ms": 4000.0,
 "driven": [{"node": "spin", "port": "frame", "signal": "frame",
             "loop": {"frames": 120, "period_ms": 4000.0}},
            {"node": "elapsed", "port": "t", "signal": "time"}]}
```

`frame` is the primary loop's frame at `t_ms`; `driven` lists every
`cycle.frame` / `clock.t` that lowered (empty = no time params:
playback moves nothing), each `frame` port carrying its OWN `loop` —
the `frames` / `period_ms` the lowering quantized its frame from (the
node's literals, or `cycle`'s defaults) — so a client showing what a
non-primary `cycle` is fed computes `floor(t_ms × frames / period_ms)
mod frames` on that loop, never on the primary's; a `time` port has no
`loop`. It rides every `snapshot` as
`payload.transport`, and it is the whole payload of

```json
{"v":1,"seq":N,"type":"transport","payload":{ …the view… }}
```

broadcast to every client after each ACCEPTED control (a refused one
changes nothing and broadcasts nothing — the refusing client gets its
`error`, and for everyone else there is no news), after Esc, when the
last client's departure paused playback, and when an edit or reload
changed the loop or the driven set. The view is a position at the moment of the
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
| `transport_speed` | `{"factor": 0.5}` | Playback rate, playhead ms per wall ms, from the current position. Not finite, `≤ 0` or above `64` (`MAX_SPEED` — sixteen times the play bar's fastest; unbounded, `1e300` put the playhead beyond the exact frame range within a frame) is refused |
| `transport_reset` | `{}` | Pause and rewind to `t_ms = 0` — frame 0, `clock` at 0, the values a headless run evaluates |

A refusal is the ordinary `error` with kind `transport` and the
intent's id (`"frame 500 is outside the loop (frames 0..120)"`,
`"speed must be a positive finite number, got 0"`, `"speed must be at
most 64×, got 1e300"`); an observer's control is kind `lease`. The
transport-driven ports themselves are never written from the app:
`set_param` and `param_preview` into `cycle.frame` / `clock.t` are
refused (kind `refused`) with the same reason the wire probe and
`connect` give — the session fills the port from the playhead whatever
the text says, so the kwarg would be dead; `apply_text` may still carry
it by hand as the headless value. A frame whose lowering excludes a
binding the structural graph (the canvas) does not — the playhead
beyond the exact frame range (a `frames` literal at the edge of 2^53),
a held slider's literal refused — is announced with a `notice`
(warning) naming the binding, its reason and how many bindings it
feeds, once per change of that set: the cone is missing from the
transport's frames and nothing else would say so. `/debug/state` carries `transport` (the same
view) and the transport generations have their own timing kind,
`transport`, beside `structural` / `preview` (`wait=true` is a quiet
oracle only while paused: between frames the loop is idle for an
instant, so it returns rather than hangs). Measured on the orbit
example (`examples/08-orbit.cic`, debug build, 15 nodes): the first
pass of the 120-frame loop is 120 generations, one per frame at 30 fps
(1,190 computed / 610 cached, p50 1.5 ms); the second pass is 120
generations, 0 computed / 1,800 cached, p50 0.43 ms — pure cache
playback; 0 deltas, 0 ops, the file's bytes untouched.

## HTTP surface

| Endpoint | Purpose |
|---|---|
| `GET /` | The embedded SPA |
| `GET /api/catalog` | Node-spec JSON catalog (docs/08) |
| `GET /api/project` | `{project, pipelines, scripts, default, open, git: {kind, branch, dirty_count}, engine, protocol}` — the pipeline list (a bounded walk of the root, depth 4, skipping exactly the directories `GET /api/files` leaves unlisted — one predicate, `files::skipped_directory` — and collecting what it calls a pipeline, the extension case-insensitive; over a home root the walk is still the project-sized tool and the picker uses `/api/files`), the `scripts/*.py` beside them, and the git summary (`kind` = the git state's tag; `dirty_count` = `git status` entries under the project dir; an unexpected git failure is `kind: error` + `error`, never a failed route) |
| `GET /api/files` | `?dir=<root-relative>` → `{root, dir, parent, entries: [{name, kind, modified_ms}]}` — ONE directory of the served root (v0.1 wave 4 O1: the root may be the user's home directory, so nothing walks it whole and no listing ever names anything above it). `root` is the root directory's own NAME (its path only for a file-system root, which has none); `dir` is the request normalised (`a//b/`, `./a/b` → `a/b`; `""` = the root); `parent` is `dir`'s parent in the same form (`null` at the root). `entries` = the directories — minus dot-directories, `node_modules`, `target`, and the ones the OS marks hidden (Windows' profile junctions `Application Data`, `Cookies`, …, which Explorer hides and nobody can enter) — and the `*.cic` files (extension case-insensitive, like `?pipeline=` accepts it), directories first, each group in case-insensitive name order; `kind` ∈ `dir` / `pipeline`, `modified_ms` = signed milliseconds since the Unix epoch. A symlink or junction is an entry only when the server can follow it to a place under the root (what the list shows must be enterable): one that leaves the root, dangles, or cannot be resolved is no entry, nor is a name that is not valid Unicode (it could never be named in a request); a hidden directory or hidden link is dropped before anything follows it. Unlisted is not unenterable: the skip list is a convention about what the picker shows, the ROOT is the boundary — a dot-directory, `node_modules` or a hidden directory named in `dir` lists like any other directory under the root. Refusals are `{kind, message, path}` (`path` = the `dir` as sent; `FilesErrorKind` in `protocol.rs`, mirrored by the client): 400 `path_not_allowed` — `..`, a leading `/` (absolute, or `//host/share`), a `:` (a drive or a stream), a backslash, a NUL byte — all refused lexically BEFORE the file system is touched — and a `dir` whose canonical path leaves the root; 404 `not_found` — nothing is there: no such directory, a file, a path through a file, or a name the file system cannot hold (Windows: `a?b`, `a*b`, os error 123); 403 `io_error` — the directory exists under the root but could not be read (or the root itself could not be resolved). The route opens no session |
| `GET /ws?token=…&pipeline=…` | The one socket per client (§Two lanes, one socket). The upgrade is accepted for every token-bearing request; the pipeline is resolved AFTER the client's `hello` — a pipeline the server no longer has is refused INSIDE the handshake with a `pipeline`-kind `error` and a close (the client shows the reason on the picker and stops reconnecting), never at the upgrade, where no reason could reach the app. `hello.role = "observer"` declares a client that never takes the writer lease (the pop-out viewport, §Projects, pipelines, sessions) |
| `GET /api/blob/{hash}` | Large payloads on demand (full inspector data, export previews) |
| `POST /api/run/{node}` | Effectful nodes — requires the explicit-run confirmation, streams progress over the session socket |
| `GET /api/edit/text` | `{path, text, text_hash}` — the base an agent reads before editing (§Undo/redo) |
| `POST /api/edit/apply_text` | The atomic whole-file edit for agents / MCP (JSON body = the `apply_text` intent payload; same error kinds as the socket: 409 `stale_base`, 422 `parse_error` / `path_not_allowed`, 500 `io_error`). Applies even while a human holds the writer lease — the agent acts for the user; the delta reaches every client |
| `GET /api/git/status` | `?pipeline=` → `{state, pipeline: {path, tracked, ignored, dirty, nodes: [{name, change, from?}], removed: [{name, line_in_head}]}, scope: [{path, status, in_head}], text_hash}` (doc 10 §Git integration, slice 1: working tree vs HEAD). `state` is tagged `kind`: `repo {root, prefix, branch, head_short, upstream: {name, ahead, behind}?, unborn, operation?}` \| `locked` (the SAME fields as `repo` — `index.lock` is held, by another git or by our own commit: status still answers, writes wait, the branch chip keeps its facts) \| `not_a_repo` \| `git_not_found`. `operation` ∈ `merge` / `rebase` / `cherry_pick` / `revert` when the shell left one unfinished (`MERGE_HEAD` etc.) — writes refuse `operation_in_progress` until it is done. `change` ∈ `added` / `modified` / `removed` / `renamed` (`from` = the HEAD name); markers are computed FROM `git diff -U0 HEAD -- <path>` (hunks → binding lines, one binding per line), so they cannot disagree with it; a rename pairs a removed + added line with a byte-identical right-hand side **within one hunk** (the writer's `rename` gesture rewrites one line; a deletion here and an unrelated same-literal addition elsewhere are two hunks → `removed` + `added`); the sidecar never marks a node; an untracked pipeline is every node `added`; an ignored one (`.gitignore`) is `ignored: true`, every node `added`, nothing in the scope. `scope` = the dirty files of the commit scope — this pipeline's `.cic`, its sidecar, `scripts/*.py` beside it (the `apply_text` set), project-relative, `status` ∈ `modified` / `added` / `deleted` / `untracked` / `renamed`, and `in_head` = HEAD has a version of the path — the rule `revert` restores by, published per file so no client re-derives it from `status` (they disagree: porcelain `AD`, added to the index then deleted from disk, is `deleted` with no HEAD version; everything on an unborn branch has none); ignored files are left out (git does not list them and `git add` refuses a list containing one). `text_hash` = blake3 of the working file the markers were computed against (clients dedupe on it). Reads only: every invocation carries `--no-optional-locks`, so a refresh never touches the project and never wakes the watcher — the route test asserts `.git/index` is byte-for-byte untouched across refreshes of a dirty tree (what the flag buys) and the command builder's unit test asserts the flag on every invocation — and no session is opened for it (status is a read about a file: polling it for a pipeline nobody has open must not start hydrating and solving one) |
| `POST /api/git/commit` | `{message, client?}` (writer-gated: `client` or `X-Cicada-Client` must be the lease holder of the pipeline's OPEN session — committing is a git action on the project, not a document edit, hence unlike `apply_text`; a pipeline nobody has open is 403 `lease` with the reason, never opened on the caller's behalf) → `git add -- <scope>` then `git commit --cleanup=verbatim -F - -- <scope>` (the message verbatim on stdin, written from its own thread so a git that exits early — a failing hook — still reports ITS exit code and stderr whatever the message's length; `-- <paths>` commits ONLY the scope, so whatever else the user staged in a shell stays staged; never `add -A`) → `{hash, short, summary, files}`. 422 `empty_message`, 409 `nothing_to_commit` / `not_a_repo` / `git_not_found` / `ignored` (the pipeline is matched by `.gitignore`: git refuses to add it) / `operation_in_progress` (+ `operation`), 423 `locked`, 403 `lease`, 500 `git_failed` (with `command`, `code`, `stderr`) / `git_timeout` / `internal` |
| `POST /api/git/revert` | `{paths?, client?}` (writer-gated as above) → `git checkout HEAD -- <paths>` for the dirty scope files that HAVE a HEAD version (the status's `in_head`; `paths` narrows the set — the client's confirm step lists exactly the `in_head` files and names exactly those; 422 `path_not_allowed` outside the scope) → the session reloads through the external-change path (`reload_from_disk` → ONE barrier snapshot, `reason: "git revert"`, op log cleared). Checkout and reload run under the session's **write hold** (`Session::hold_writes` → `reload_from_disk_held`): no intent, undo, `apply_text` or watcher reload can persist between the two — a slider drag arriving mid-revert applies to the REVERTED text afterwards instead of overwriting the restored file (which would have made the reload a no-op and the revert silently lost) — so `reloaded` is always `true` when the files changed and the barrier's reason is always ours; the watcher's later wake finds disk == memory and does nothing. → `{reverted, untracked, reloaded}`. Files without a HEAD version are never deleted: `untracked` lists the ones left alone; an untracked (or ignored) pipeline, or an explicit ask for one, is 409 `untracked`; 409 `nothing_to_revert` / `operation_in_progress`; 500 `reload_failed` when the files are back on disk but the session could not load them (previous state stays live). Measured (route test, debug build, Windows): POST → barrier snapshot on the socket ≤ 35 ms. Every git-route failure body is `{kind, message, …}` with `kind` the snake_case `GitErrorKind` enum in `protocol.rs`, mirrored by the client — including pipeline resolution (`protocol` 400, `no_such_pipeline` 404 with `path`) and server-side failures (`internal` 500); the one exception is the token middleware's 401, text like every route's |
| `GET /debug/state`, `GET /debug/screenshot` | The agent/dev verification loop (doc 14). `state` (`?pipeline=&values=&wait=`) is the authoritative JSON oracle — graph view-model, text, statuses, summary, per-output display stats with bounds/triangles (plus, additive since v0.1 item 3 WP-B, `stats.solids` = solids drawn through tessellation, `stats.tier` = `preview` / `fine`, the deflection tier those solids were meshed at (a drag's generations draw coarse; the release redraws fine — docs/03 §Display tessellation), `stats.warnings` = per-element caveats for solids drawn although the kernel's mesh did not close, and `stats.errors` = per-element reasons for what could not be drawn; all omitted when zero/empty), `display_cache` = the session's solid tessellation cache counters (`entries`, `bytes`, `budget`, `hits`, `misses`, `evictions`, `oversized`, `refusals` = cached negative entries; docs/12 §Display cache), `watched` = the SERVER's watched directories (§External changes: every open pipeline's directory and its `scripts/`, root-relative, `/`-separated, sorted, `""` = the root — so an agent or a test can tell before an external edit whether the watcher will see it), lease, and `timings` (the last 1,024 generations: kind, `queued_ms` intent-arrival → start, `elapsed_ms`, `cancelled`, computed/cached counts, frame bytes, and `cancel_to_idle_ms` on a generation Esc ended — measured server-side, poll-free; the doc-15 measurement currency); `screenshot` (`?target=viewport`) asks a connected client to render the WebGL viewport to PNG (503 when no client is connected — loud, never blank; whole-page shots are Playwright's job) |
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
a stray editor. The engine watches (debounced) each OPEN pipeline's
directory and the `scripts/` beside it, non-recursively — never the
root as a tree: since v0.1 wave 4 (O1) the root may be the user's whole
home directory, and a recursive watch over it is unbounded (inotify's
watch limit refuses it outright; every backend floods the coalescing
thread with events about nothing the watcher acts on). The watched set
is exactly what the watcher reacts to — the `.cic`, its sidecar (same
directory), `scripts/*.py` — and a `scripts/` that appears after its
pipeline opened is put under watch on its arrival, which itself
rescans (a directory checked out together with its files is not
missed; `http.rs`'s `classify_change` is the decision, unit-tested, and
`tests/root_and_files.rs` drives the real watcher over a pipeline in a
subdirectory of the root — a `scripts/` present at open, proved by the
watched set at the moment of the join, and one arriving later, proved by
a content-only rewrite inside it — which only a watch ON the directory
can see — after an ordered barrier has quiesced the watcher). The
watched set is published:
`/debug/state` carries `watched` (root-relative, sorted), and a `scripts/`
the watcher could not put under watch is a `warning` notice to the
session's clients — the session stays open on its `.cic`, and says that
script edits there go unseen until it is reopened. The re-watch is
idempotent on purpose: Windows reports `scripts/` modified on the
parent's watch without any entry change (NTFS flushes a file's
directory-entry info when the file is next opened and closed — the
session's own read of a script does that), so the reaction runs
spuriously now and then and costs one fingerprint; a batch of changed
paths is always the union of its parts, never a count. An external
change to a `.cic`, sidecar, or script triggers reload →
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
- **`cicada app [path]`** (v0.1 wave 4, docs/17 L1) is `serve` plus the
  window: exactly `serve`'s arguments, resolved by the same function (the
  path rule is `serve`'s, whatever it becomes), then a Chromium-based
  browser in `--app=<url>` mode when the machine has one — a dedicated
  window without tabs or an address bar (Windows: Edge, then Chrome, found
  through the registry's `App Paths` or the usual Program Files dirs;
  macOS: `open -na "Google Chrome" --args --app=<url>`, then Edge; Linux:
  `xdg-open`) — else the default browser on the plain URL; `--no-browser`
  opens nothing. The URL is printed either way, the terminal is the server
  console, Ctrl-C stops the server. The window needs a SPA to load —
  `--web-dir` (with its `index.html`) first, else the embedded build's, the
  server's own preference order — and with neither `app` refuses BEFORE
  the server binds, naming both ways out: `cicada serve` is the API-only
  shape, and the "API only" page above is never what an app window opens
  onto (`cicada_cli::app::spa_source`, a pure function of the arguments,
  the build and the disk; review finding 2026-08-24). The decision is a
  pure function over a probed environment (`cicada_cli::app::choose`),
  unit-tested per OS. A
  browser that fails to START is reported on stderr and the server keeps
  running with its URL on screen — the server is the product, never a dead
  server because a window failed. The windows the browser opens are its
  own: stopping the server leaves them showing a disconnected app, as
  closing a tab would.
- **No loader path at launch** (L2): a built `cicada` needs the kernel's
  shared libraries; `tools/fetch_occt.py --bundle <dir>` copies the
  run-time closure beside the binary (Windows) or into `<dir>/lib` with the
  binary's rpath rewritten to `@executable_path/lib` (macOS), so the binary
  starts from any shell, launcher or double-click without the env the build
  needs (AGENTS.md palette).
- **Launchers and the bundle** (L3): `tools/launch/Cicada.cmd` /
  `Cicada.command` open a terminal (the server console) that runs
  `tools/launch/launch.py` — builds the release binary with the SPA embedded
  when it is missing or stale, bundles the runtime beside it and runs
  `cicada app` under an environment from which the loader path has been
  REMOVED, so the bundle is what makes it start. `tools/launch/bundle.py
  --out DIR` makes the redistributable folder from an existing release
  build — on macOS everything inside `Cicada.app/Contents/MacOS`, because
  Gatekeeper's app translocation runs a downloaded app from a random
  read-only copy and anything outside the bundle would be gone; the
  launcher script there is `Cicada.command`, never `Cicada`, the binary's
  name on a case-insensitive disk. A binary that embeds no SPA is refused
  before anything is written (a plain `cargo build --release` would die at
  the first double-click with `cicada app`'s refusal above) unless
  `--allow-no-spa` asks for an engine-only bundle whose README says so.
  `--check` verifies a bundle from a minimal environment (Windows: PATH =
  System32 alone), holding the binary and the bundle's stamp to agree
  about the SPA, and `--smoke` runs its `cicada app --no-browser` to
  `/health` and `/`. The bundle removes the loader path, not the engine's
  Python: `cicada app` starts the script host at launch, and the bundle's
  README says so.

## Latency targets (measured, not vibed — spike criteria feed here)

| Path | Target |
|---|---|
| Intent → ack (local) | < 5 ms |
| Optimistic apply | immediate |
| Param drag → dirty-cone solve → repaint (cheap cones) | 16 ms (60 fps) |
| Expensive cones (≥ ~1 s predicted) | compute-on-release with honest estimate |
| Warmed `cycle` loop playback | 60 fps sustained — MEASURED 2026-08-21 (`tools/measure/transport_loop.mjs --expect warm`, debug build, Windows): a 60 fps loop at 60.0 generations/s, gap p50 16.66 ms, second pass 0 computed |
| Beyond budget | progress UI takes over, honestly (doc 12) |
| Status coalescing | ≤ 10 Hz |

## Deferred, explicitly

- **Real-time multi-writer collaboration** — the op-based protocol is
  shaped for it (ops + authoritative sequencing), but CRDTs/merge UX
  are a product decision for later; the write lease covers v1.
- **Remote auth story** (accounts, TLS termination) — deploys behind a
  proxy until then.
- **WebTransport / QUIC** as a WebSocket upgrade path if head-of-line
  blocking shows up in profiles beyond what the two lanes answer (§Two
  lanes, one socket: control-vs-display blocking did show up, at wall
  scale, 2026-08-20, and the lanes answered it without a transport
  change); frame format is transport-agnostic on purpose.
