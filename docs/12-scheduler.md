# Scheduler internals

Doc 02 §3 states the contract: content-hash caching, minimal recompute,
parallel execution, disk persistence, cancellation everywhere, honest
progress, determinism. This document specifies the machinery that
delivers it. The one-line design: **everything is content-addressed,
so cancellation and restart are nearly free, and "minimal recompute"
is just cache lookup**.

Delivery is web-first (doc 04): the engine runs as a server —
`cicada serve` locally today, remote later — and the browser UI holds
no authoritative state. Everything below lives on the engine host.

## Values: immutable, hashed at construction, interned

- Every value (geometry, list, scalar, table) is immutable and carries
  a **blake3 content hash computed once at construction** — hashing is
  never a per-solve cost, and big buffers hash at GB/s.
- **Interning**: same hash → same `Arc`. Repeated geometry dedupes in
  memory automatically; the viewer exploits the same fact (identical
  mesh hashes across elements → one instanced draw call, doc 04).
- **Merkle structure**: a list's hash is the hash of its element
  hashes (plus axis name); an `Optional` slot hashes its presence.
  Changing one element changes one leaf, not a re-hash of the world.
- Floats hash by raw bits. **NaN is refused at value construction**
  (loud refusal with the producing node), and `-0.0` is canonicalized
  to `0.0` — no denormal ambiguity in cache keys.

## Cache keys

A node's cache key is:

```
NodeKey = blake3(
    node_id,           # e.g. std.extrude
    node_version,      # see below
    input value hashes (per port, in spec order),
    param literals,
    pairing shape      # each()-depth per port, zip/axis structure
)
```

`node_version` by tier:

- **Stdlib nodes** declare an explicit semantic version in `#[node]`,
  bumped on any behavior change — recompiling the engine does not
  invalidate the world's caches; changing a node's meaning does.
- **Script nodes**: hash of the source file + compiler/toolchain
  version.
- **Expression nodes**: hash of the normalized expression IR.
- The engine's major version salts everything (format evolution).

**Early cutoff falls out for free**: if an upstream node recomputes
but produces bytes with the same hash (slider 12.0 → 12.5 → 12.0),
every downstream key is unchanged and hits cache. No special
same-value detection needed — content addressing *is* the detection.

## Volatile nodes

*(Live, v0.1 item 3b.)* A node declared `#[node(volatile)]` (catalog:
`"volatile": true`, CATALOG.md tag `· volatile`) is **uncached by
design** — the flag `clock` wears (DECISIONS.md time row; shipped with
item 4, the one volatile node in the catalog). The
executor never reads or writes the memo for a volatile node's outputs,
at node AND element granularity: it executes in every generation whose
cone holds it, and inside an `each()` fan-out once per element, every
generation, whatever the measured per-element cost says about element
caching. Its cost samples still record (the estimator knows what a
clock's cone costs).

Downstream nodes are deliberately **ordinary**. Their keys include the
volatile output's fresh value hash like any other input, so they
recompute exactly when that value changed and hit the memo when it did
not — a clock quantized to the same second recomputes nothing below
it; a clock that moved dirties its cone by content addressing, no
special "downstream of volatile" rule exists. The precise statement:
*a volatile node is always a memo miss; everything else is keyed as
usual.* `volatile` and `effectful` are exclusive (the macro refuses the
pair): an effectful node already bypasses the memo and runs only on
explicit action; a volatile node runs in every generation — both at
once would be an exporter that fires every generation.

## The store

Two levels:

1. **Value store** — content-addressed blobs: `hash → zstd(bytes)`.
   Deduplicated across nodes, solves, and time by construction. Small
   blobs (≤ 256 KiB compressed — points, numbers, cells, parts) live in
   one append-only **pack** file indexed at open; only big blobs get a
   file of their own. Stage-6 measurement: a cold 1,500-element fan-out
   spent seconds creating one file per value (syscall-bound, NTFS);
   packed it is ~0.1 s. Same torn-tail recovery as the memo log.
2. **Memo table** — `NodeKey → {output hashes, status, warnings with
   element IDs, cost samples, display-cost samples}`. *(Live: output
   hashes, and — since v0.1 — the producing computation's cost
   `{elements, nanos}` on node-level entries whose computation executed
   every element (a fan partly served from the element cache measured
   only what ran, so its entry carries no cost rather than a number
   that is not what the node costs), so a cache hit still knows what
   it cost when it last ran and the cost model stays complete across a
   warm reopen; per-op samples ride beside the entries as their own
   records.)*

**Log format and engine age** *(live, v0.1 item 3b)*: the memo log is
an append-only record enum, so every older log replays under a newer
engine. The other direction is guarded by a marker: the store root
carries a `format` file naming the newest record kind the log may hold
(`LOG_FORMAT`; 1 = the spike's records, 2 = entries with cost, 3 = the
`Solid` blob kind — a VALUE codec variant rather than a record variant,
bumped for the same reason: an engine that cannot decode a Solid blob
treats the memo that promised it as broken, tombstones it and
recomputes — silently discarding a newer engine's valid work), and an
engine that finds a higher number there refuses loudly ("written by a
newer engine") instead of reading the records it cannot decode as
corruption and truncating the log at the first of them. Adding a
record variant bumps the constant in the same commit. One transitional
truth, stated plainly: engines from BEFORE the marker (every binary up
to main's 3899460) know nothing of it — pointing one of them at a store
a v0.1 engine has written drops that store's memo table once, with a
"corruption" notice; the blobs stay and recompute reuses them. While
several worktree binaries share a project, open it with one age of
engine, or pass each its own `--cache-dir`.

**Location**: the store lives on the engine host in the user cache
directory (e.g. `%LOCALAPPDATA%/cicada/cache/<project>`), keyed by
project — **never inside the project folder by default**. Project
directories are routinely cloud-synced (Dropbox, iCloud), and
gigabytes of cache churn must not sync; a project-local
`.cicada-cache/` remains as an opt-in override (and stays gitignored).

**Size defaults** (target hardware is a mid-to-high-end desktop,
laptop, or server — default big): in-memory value LRU = 25% of host
RAM; disk store = 32 GB per project, LRU-evicted by access time; the
memo table is kilobytes-to-megabytes and effectively unbounded. The
cache holds *alternate states* — the recent edit history's
intermediate values — so scrubbing a slider back and forth or undoing
an edit hits warm entries. Evicting never loses correctness, only
warmth. In memory, a byte-budgeted LRU of live `Arc` values fronts
the disk store; spill and reload are transparent.

**The lock file is not the cache**: it holds only the output hashes of
the *current* state — a pointer into the store, kilobytes forever.

What gets cached beyond outputs: node status (counts, warnings),
measured cost samples (feeds ETA), and **display artifacts** —
tessellations and GPU-ready buffers are themselves costed, cached
operations keyed like everything else (display is a first-class edge).

### Display cache (as shipped — v0.1 item 3 WP-B; tiers, the worker-pool warm-up and negative entries 2026-08-21)

The first display artifact with a cache of its own is the **solid
tessellation**: a `Solid` is its kernel's canonical bytes (DECISIONS.md
row 42), so drawing one means asking OCCT to mesh those bytes at the
generation's display tier (docs/03 §Display tessellation: the preview
deflection for a slider drag's generations, the fine one for structural
generations, the release, a joining client and the inspector). That
work lives in `cicada-server::display::SolidCache`, one instance per
session (`Core.solids`), NOT in the value store and NOT in the value:

- **Key**: the solid's value hash plus the TIER deflection it was meshed
  at (the deflection is a pure function of `ProjectConfig` and the tier,
  so a tolerance or unit change misses exactly as it should; the
  per-solid relative term is a function of the solid and needs no place
  in the key; the bytes never learn about display).
- **Value**: the welded display mesh sealed as a `HashedValue` (its hash
  is the wire blob's key — see Frames), the face count, and whether the
  mesh closed — one kernel reconstruction serves the frames and the
  inspector summary ("Solid, N faces, bbox", `watertight`).
- **Where it is computed**: by the solve loop, not the broadcaster. When
  a generation completes, the session reads (under a short lock hold)
  which outputs will draw and are not yet on screen at this tier, loads
  their values, and tessellates every DISTINCT solid among them on the
  scheduler's worker pool in parallel (`Scheduler::map_parallel`,
  `display::distinct_solids`) — then takes its lock to encode and
  broadcast, and every tessellation is a hit. Intents keep flowing
  through the tessellation; the next generation waits for it (the loop
  is sequential), which is the right order — a generation's display is
  part of its work. Measured (release, the review's 300-hole bar, 303
  distinct solids): a 5.26 s generation for a 0.5 s solve became 1.19 s
  for a 0.34 s solve at the fine tier (docs/17 §Item 3 has the table).
- **Tiers on screen**: the session records the tier each output was
  drawn at; an output already showing this value at this tier or a finer
  one is not re-sent, one showing it at the preview tier is redrawn by
  the next fine generation (the release), and a joining client is
  restreamed at whatever tier is on screen — all hits.
- **Bound**: bytes of mesh buffers as uploaded, default 1 GiB
  (`SOLID_CACHE_BUDGET`, resizable at run time — §Display below; it was
  256 MiB until v0.1 wave 5 D1), evicted least-recently-used — a recency index (touch stamp → key in a
  `BTreeMap`) makes every touch and every eviction O(log entries), so a
  display pass over N distinct solids costs O(N log entries) however
  full the cache is, never O(N × entries). Eviction costs a
  re-tessellation, never correctness. A tessellation larger than the
  whole budget is served but never kept (counted as `oversized`):
  keeping it would evict everything else for an entry that still does
  not fit. **Refusals are cached too** (negative entries under the same
  key, sized by their text, evicted like any entry, counted as
  `refusals`): a solid whose bytes the kernel refuses, or whose mesher
  fails after doing its work, is refused from the cache on the next
  pass instead of re-paying the kernel call — the earlier claim that "a
  refusal fails at the read and is cheap" was false for the mesher
  class, and a corrected value is a new hash that misses as it should.
  A mesh that merely did not close is NOT a refusal: it is drawn and
  reported (docs/03 §Display tessellation, closure policy).
- **Observable**: `/debug/state` → `display_cache` carries `entries`,
  `bytes`, `budget`, `hits`, `misses`, `evictions`, `oversized`,
  `refusals` (additive; asserted by the session's debug-state test and
  listed in docs/13); an output's display `stats` carry `tier`
  (`preview` / `fine`, when a solid was drawn), `warnings` (a solid
  drawn although its mesh did not close) and `errors` (a solid that
  could not be drawn, with the kernel's reason) — never silently.
- **Frames**: a tessellated solid emits the ordinary mesh frames
  (`frames.rs` unchanged); the instancing/blob key is the DISPLAY MESH's
  own value hash — content-addressed like a `Mesh` value's frames, so
  identical solids at one tier travel once as a blob, the same solid at
  another tier is another blob (the client caches blobs by hash forever
  and drops a re-sent hash: a deflection-dependent mesh under a
  deflection-independent key would have drawn stale), and a mesh-valued
  twin of the tessellation would share the blob.

It is the in-memory, per-session form of the "display is a first-class
edge" idea above; promoting tessellations into the costed, persisted
store (so a warm reopen draws without the kernel, and a superseded
tick's tessellation can be cancelled like a node) is the follow-up
named in docs/17, and the cache's key is already the one such a store
would use.

### Display (as bounded and shown — v0.1 wave 5 D1, 2026-08-25)

The display edge is bounded, watched and announced (DECISIONS.md row
2026-08-25; the evidence is docs/17 §Measurement U30 / U31: 1,000
fine-tier spheres were 8,000 triangles each, a 160 MB frame and 2.3 s of
tessellation per redraw, re-paid on every undo/redo because two value
sets did not fit a 256 MiB cache).

- **The triangle budget** — `display::DISPLAY_TRIANGLE_BUDGET` =
  1,000,000 triangles per output per generation
  (`SessionConfig::display_triangle_budget`; tests lower it). A
  structural generation asks for the fine tier; an output whose DISTINCT
  solids would exceed the budget at that tier is drawn at the preview
  tier instead, and when even the preview tier exceeds it the output is
  drawn at preview anyway and marked `over_budget`. The decision
  (`display::choose_tier` → `BudgetStats {limit, requested, drawn,
  triangles, over_budget}`, recorded in the output's display `stats.budget`
  and memoized per (value hash, requested tier) in the session) is a pure
  function of the value set, the requested tier and the limit — never of
  timing or of the order the solids were meshed in: the fine tally meshes
  the solids through the cache on the worker pool and **stops at the
  budget** — once the running total is past the limit the verdict
  "preview" is certain and the remaining solids are not meshed fine — so
  the fine work wasted before a "too many" verdict is bounded by the
  budget itself (plus one solid per worker), and the tessellations it did
  are ordinary cache entries (≈ 24–36 MB at the default budget, which a
  1 GiB cache absorbs). The preview tally then meshes every solid at
  preview, which the emit needs anyway. Per output, so a pipeline of many
  modest outputs is not penalised for their sum; a value without solids
  (meshes, curves, points) has no tier and no budget. The "already
  displayed" rule compares against the tier the budget CHOOSES for the
  output, not the tier the generation asks for, so a structural
  generation over an unchanged over-budget output — on screen at preview,
  chosen preview again — re-sends nothing and tessellates nothing (the
  verdict comes from the memo, which is bounded at `VERDICTS_KEPT` =
  65,536 verdicts in two halves — a verdict asked for since the last
  turnover survives it, what nobody asked for in half a memo's worth of
  new verdicts, a drag's per-tick hashes, is forgotten; a clear-all at
  the cap re-paid the fine tally of every unchanged over-budget output
  on the next structural generation). Projected, not measured (the
  measured runs are 140 spheres on 2–4 threads, docs/17 D1): U30's 1,000
  spheres would drop to ~866,000 triangles, ~17 MB and ~9× less
  tessellation; two value sets of them ~2 × 21 MB, fitting the cache, so
  the undo/redo thrash would go with it.
- **The cache: 1 GiB, resizable, watched.** `SOLID_CACHE_BUDGET` = 1024
  MiB (`SessionConfig::solid_cache_bytes`; `cicada serve
  --solid-cache-mib <n>`, 64..=65536, `app` passes it through). The
  writer-only intent `set_display_cache {mib}` resizes the session's
  cache live (`SolidCache::set_budget`; shrinking evicts least-recently-used
  entries at once, counted like any eviction); 64 ≤ mib ≤ 65536, else
  refused kind `invalid`; an observer's is refused kind `lease`; never an
  op, never the file — the answer is the `caches` broadcast. The app's
  settings menu offers 256 MiB · 512 MiB · 1 GiB · 2 GiB · 4 GiB and shows
  the session's current budget; the choice is a per-user setting the
  WRITER re-applies on every connect (the lease holder's preference wins;
  an observer's choice is kept for when it holds the lease).
- **The working set and the two flags.** After every display pass the
  session computes the **working set** — the display meshes every output
  on screen holds: each displayed output's distinct solids at the tier it
  was drawn at (`Displayed.solids`, from `DisplayFrames::solids`), unioned
  by (value hash, tier), sized as the cache counts them. `over_budget` =
  the working set is larger than the cache budget (the picture cannot all
  be held, so every redraw re-tessellates part of it — a solid larger
  than the whole budget is `oversized`: served, never kept). `thrash` = the
  pass evicted entries the previous complete generation displayed: after
  each pass (and after a resize) the cache is told to **watch** the
  picture's entries (`SolidCache::watch`; "used" = on screen at the end
  of that generation), and an eviction of a watched entry during the next
  pass — by the warm-up's insertions or by the emit's — is counted
  (`watched_evictions`). Watching also **touches** the picture's entries,
  so the picture on screen is the newest thing in the cache and the
  least-recently-used entries — past value sets nobody draws, the fine
  meshes of an output the budget dropped to preview — go first: without
  the touch a stable output's meshes, never looked up again once
  displayed, were the OLDEST entries and left before any garbage, and the
  flag called a picture that fit with room to spare thrash (fix round
  2026-08-25). So `thrash` is exactly "the previous picture and this one
  do not fit together". Both flags are set per pass (cleared by a pass
  whose picture fits and evicts none of the previous one) and ride the
  `caches` view. The **notice** (ONE `notice`, level warning, naming the
  numbers and the remedy "raise the display cache in settings, or draw
  fewer solids") is raised when a flag RISES in a pass, and when a
  STRUCTURAL generation redraws part of the picture while a flag stands
  (the user changed the picture; it still does not fit) — never on a
  preview or transport pass over a standing flag (the first build raised
  it on every pass whose flags were set, so a drag re-broadcast the same
  warning at the tick rate — review finding 2026-08-25), and never on a
  generation that drew nothing. A pipeline without solids has an empty
  working set and never raises either. `set_display_cache` re-judges
  `over_budget` against the new budget at once — a flag it raises rose at
  the resize (its `caches` turns the indicator; the pass after it raises
  no notice) — and re-arms the watch on the picture that survived the
  resize, so the resize's own evictions are never reported as the next
  generation's thrash; a resize that HOLDS the working set clears
  `thrash` at once (the condition it warns about is gone — the user did
  what the remedy said), otherwise `thrash` stands until the next pass.
  **Not bounded: the sum over outputs.** The budget is per output, so a
  picture of ten outputs of 900,000 fine triangles each is drawn fine —
  9 M triangles, the frame U30 measured — with nothing said unless the
  working set exceeds the cache; the per-output choice keeps a pipeline
  of many modest outputs from being penalised for their sum, and the
  profiler (P1) shows the sum. A soft per-generation ceiling is a
  follow-up if it ever bites (review note CR-3).
- **The lifecycle and latest-wins.** A generation's display pass is
  `display_begin` (the control lane, the moment the solve finished and
  BEFORE the tessellation — the spinner starts when the work does;
  `outputs` = the pre-filter's count of outputs not on screen at the
  requested tier) → the warm-up off the session lock (load each pending
  value, take its verdict — which IS the tessellation on the worker pool
  — and **pin** the drawn tier's meshes for the encode) → the encode and
  send under the lock, drawing the pinned meshes → `display_end` on the
  DISPLAY lane behind the last frame (`outputs` / `frames` / `bytes` as
  sent, `tessellate_ms` / `encode_ms`, `cancelled`, `cut_by`) → the cache
  verdict, its notice and `caches` (control-lane texts, which a real
  socket may deliver BEFORE `display_end` and the pass's frames — a
  client reads the flags from `caches` alone). **The encode never
  tessellates**: a verdict the memo already knew still fetches the
  value's meshes through the cache on the worker pool (a hash lookup each
  when warm, the kernel for an evicted one — an undo to a remembered
  value set after a shrink, or after enough other value sets went through
  the cache), and the pinned meshes are drawn from the pin however the
  cache's eviction went in between (a working set larger than the budget
  evicts the warm-up's own first entries before the encode reads them;
  the pin ends the cascade). Both were lock-held serial kernel work
  before the fix round of 2026-08-25 (review findings). Between outputs —
  in the warm-up and in the encode — the pass asks the solve loop whether
  it is **superseded** (`SolveLoop::superseded`), and the rule is the
  solve's own: a pending STRUCTURAL job — an edit — supersedes it (the
  pass stops, `display_end {cancelled: true, cut_by: "edit"}`, no further
  frame; the outputs it did not reach keep their previous frames and
  table entries, and the edit's generation, which starts the moment the
  pass returns, draws them at the newest state), Esc supersedes it
  (`cut_by: "esc"`: the generation is reported **cancelled** — the
  summary, the timing with its `cancel_to_idle_ms`, the chip's `cancelled
  gen N` — the solve finished and the memo holds its values, but its
  picture did not land: the outputs the pass did not reach keep the
  previous generation's picture until the next edit repaints them; Esc
  schedules nothing), and a pending PREVIEW or TRANSPORT tick does NOT
  supersede it — it waits for the pass as it waits for the solve
  (latest-wins over COMPLETED generations, §Solve generations below). The
  first D1 build cut a pass whenever ANY job was pending, so a drag or a
  playback whose pass outlasted a tick cut every pass before its first
  frame and painted nothing until the input stopped (review findings
  2026-08-25, the same root under three lenses). A pass cut during its
  warm-up encodes nothing (its tessellations stay in the cache for the
  next generation). The client keeps its per-output generation rule
  (`sceneStore`: a frame older than the newest applied for its output is
  dropped; a restream's older generations are not) — a global "older than
  the newest `display_begin`" rule would discard the tail of a completed
  pass that the control lane's `display_begin` overtook, or a restream
  interleaved with a live pass, and nothing would re-send them.
  `/debug/state.timings` carries `tessellate_ms` / `encode_ms` per
  generation; `/debug/state.caches` is the `caches` view.

## Solve generations

- An edit (text or canvas) reparses and **typechecks synchronously in
  milliseconds** — red nodes and their downstream cones are excluded
  from scheduling (status: blocked), everything else proceeds.
- Dirtiness is exact: the edited nodes' keys change; the dirty set is
  their downstream cone. Everything outside it is untouched by
  construction.
- Solves are **generations**: a structural edit starts one after a
  ~30 ms debounce; a newer edit cancels and supersedes the in-flight
  generation. Continuous param streams (slider drags, animation
  playback) skip the debounce entirely and run **latest-wins**: each
  completed generation immediately starts the next with the newest
  value, discarding stale intermediates (doc 13). Supersession is
  cheap because completed work landed in the cache — a slider drag is
  a stream of generations, each reusing everything the previous one
  finished.
- **Esc always cancels the current generation.** Per node, the viewer
  and inspectors keep the last *complete* value until a newer one
  exists — no torn state, and a cancelled solve leaves the last
  coherent frame (doc 04).

## Execution

- **Wavefront over the DAG** on a rayon pool sized `cores − 2` (UI and
  OS breathe). A node is ready when all inputs are resolved (cache hit
  or computed this generation).
- **Element fan-out**: an `each()` call becomes per-element tasks in
  chunks sized adaptively to ~10–50 ms of measured work — small enough
  for responsive cancellation, big enough to amortize overhead.
  Completed elements stream to the viewer incrementally
  (generation-tagged; only newer generations replace).
- **Priorities**: (1) nodes feeding visible previews and the currently
  inspected wire; (2) longest critical path by estimated cost.
  Attention first, wall-clock second.
- **Oversubscription control**: nodes flagged `parallel_internal`
  (Manifold's TBB, internally threaded kernels) claim more of the pool
  so outer × inner parallelism doesn't thrash.
- **Adaptive cache granularity**: per-element caching only where
  measured per-element cost exceeds the hash+store overhead
  (booleans: yes; `x * 2`: no — node-level only). The threshold is
  measured, not guessed.

## Cancellation

One token per generation, checked between nodes, between element
chunks, and at safe points inside long stdlib loops. *(Live, v0.1 item
3b: the token is owned by the `solve` call and handed to every node
invocation as its `NodeCtx` — a long node polls `ctx.cancel`, a host
bridge hooks it with `CancelToken::on_cancel`, which runs once when the
token is cancelled, immediately if it already is. There is no
session-global switch: an explicit effectful run, the interactive
latest-wins loop and an idle-class solve each own a token, and
cancelling one never touches another — the Python bridge mints one
kill switch per call, hooked to the calling generation's token, so
whoever cancels a generation kills exactly its in-flight worker calls
without a separate "kill the scripts" step to forget. **What lands
`cancelled` is narrow**: a node that stops because its generation was
cancelled says so — `NodeError::cancelled` from a long loop's safe
point, and the bridge's verdict for a worker the token killed — and
the executor lands that `cancelled` only under a cancelled token. Any
other error stays red, Esc or not: a genuine failure that coincides
with an Esc is never hidden behind "cancelled", and a node claiming a
cancellation under a live token is red with its message. Element
level: an element that bails this way leaves its slot unfilled and is
not cost evidence — the fan lands `cancelled` like one whose later
chunks never started, and a fan cancelled mid-way is `cancelled` even
if some of its elements had already gone red, because its verdict is
incomplete; the next generation re-runs it and shows the red.)* Script nodes
are hard-cancellable **by construction**: WASM epoch preemption (hard
interrupt, no cooperation needed) for Rust scripts, subprocess kill
for Python (the pool respawns workers). Individual kernel calls (one
boolean, one loft) are the atomic quantum — at per-element scale
they're milliseconds; calls *predicted* long route to a cancellable
kernel-worker subprocess (next section), so even a giant single
boolean is killable. Worst-case Esc latency is bounded by the routing
threshold, never by the biggest kernel call.

## Long-running nodes

The cost model is the keystone. Per node, the memo table stores
samples of *input size features → wall time* (element counts,
vertex/triangle totals), and the estimator fits a simple per-node
regression. Cold start uses per-category defaults and marks estimates
rough. One model, five consumers: chunk sizing, ETA weighting, the
threading decision, execution routing, and progress display.
Thresholds below are defaults to be tuned by measurement.

- **Threading decision**: predicted < ~10 ms → single-threaded
  in-process (parallelism overhead would lose); above → enable the
  kernel's internal parallelism and element fan-out.
- **The escape hatch**: predicted > ~1 s → the call runs in a
  **kernel-worker subprocess** (pooled, shared-memory value passing),
  so cancellation is `kill`. A boolean union of 10 elements stays
  in-process; a union of 10k is killable mid-flight.
- **Associative decomposition**: N-ary ops declared associative in
  their spec (`#[node(associative)]` — boolean union/intersection over
  many operands, mesh joins, concats) may be rewritten into a
  **balanced parallel reduction tree** when predicted cost warrants.
  The tree shape is fixed by index, so results are byte-identical
  regardless of thread timing, and every leaf and branch is a small,
  cancellable, *cacheable* quantum — the giant critical-path union
  becomes log-depth parallel work instead of one opaque call.
- **Progress bars**: any node with predicted-or-elapsed runtime > 1 s
  shows an inline progress bar. Where work is decomposed (chunks, tree
  leaves), progress is real and free — a relaxed atomic incremented
  per completed unit, polled by the UI at ~10 Hz; nanoseconds of
  overhead, no callbacks in hot loops. Opaque single kernel calls show
  **model-estimated progress** (elapsed vs predicted, styled as an
  estimate). Script nodes may report real progress through a throttled
  host API.

## Cost prediction (compute-on-release)

*(Live, v0.1 item 3b; DECISIONS.md row 39.)* The cost model's first
consumer beyond the ETA is the drag policy (doc 13 §Slider drags): a
`param_preview` whose dirty cone is predicted at ≥ 1 s
(`COMPUTE_ON_RELEASE_MS`) solves no previews; the slider shows the
pending value and the estimate, and the release solves once. What the
prediction is, precisely:

- **A hash-only dry run** of the tick's graph against the memo. The
  param's dirty cone is the downstream cone of the node holding the
  literal (for a bare literal, of every node referencing it),
  exporters excluded. Walking it in topological order, a node whose
  inputs are all known builds its `NodeKey` exactly as the executor
  would; a memo hit costs 0 and its recorded outputs feed downstream
  keys — so a value the slider has visited, or a hypothetical solve
  has warmed, predicts as the cache read it is (the upgrade path scrub
  caching rides). A miss — or a node fed by a miss, or a volatile node
  — is predicted to compute.
- **A predicted-to-compute node costs** `per-element nanos (the op's
  persisted mean) × the node's last element count ÷ min(threads,
  elements)`: the mean from the op's samples, the count from the
  node's last outcome (computed this session, or the cost its memo
  entry recorded — a warm reopen still knows), the divisor the
  fan-out's parallelism (a scalar node is serial). A node without a
  sample or a count contributes 0 and marks the estimate **rough** (a
  floor, shown with `~` like the ETA); a cone with no evidence at all
  previews live — the first drag measures it, the next one knows.
- **Decided per tick, monotone within a drag.** Every tick predicts
  its own cone; a tick predicted at or above the bar is withheld,
  always — a drag that began on a warm value (the load's, a prior
  release's) and moves onto cold ones never solves a slow preview
  live. `preview_policy` goes out once per drag, on the first withheld
  tick. Once a drag has switched, only a tick that is a pure cache
  read (no node predicted to compute) previews live — scrub caching's
  upgrade path; a tick that would compute stays withheld whatever its
  estimate (the hysteresis: a drag never flips back to solving). A
  drag is the run of ticks on one param closer together than
  `DRAG_GAP_MS` (300 ms); a write attempt (landed or refused — every
  write intent but the tick, at the dispatcher's door: gestures, undo,
  redo, batch; `apply_text` at its own entry), an Esc, a reload or a
  longer pause ends it, and the next tick starts a fresh one —
  re-predicted, re-announced if withheld (doc 13 §Slider drags has the
  client-side reading). The bar is inclusive: a cone predicted at
  exactly `COMPUTE_ON_RELEASE_MS` is withheld.
- **No second model.** A cone with no evidence at all previews live
  once; that generation records every node's sample and element count,
  and the next tick is predicted from them. There is no separate
  "last measured time of this param's cone" fallback: the per-node
  sum is complete after one generation of evidence (or after a warm
  reopen, from memo-recorded costs), so a second model would never be
  consulted.

Measured on the wall's `deboss` (22 threads): predicted 3.9 s, the
release took 3.7 s; after a warm reopen the prediction was 4.1 s from
memo-recorded costs alone (not rough), and the just-released value
previewed live at 0.2 ms. The regression estimator above replaces the
per-op mean when it lands; the dry run and the decision are unchanged
by it.

## Speculative warming (scrub caching)

Idle compute is spent making future interactions instant — always at
the lowest priority, preempted by any real work.

*(Live substrate, v0.1 item 3b: the **idle class**. `SolveLoop::
run_idle` waits until the interactive loop has nothing pending or in
flight, registers its own cancel handle under the same lock hold that
observed "idle", and solves on the caller's thread; every real
submission and every Esc cancels all idle handles, so real work
pre-empts it at the next chunk boundary. It is invisible to
`wait_idle`/`is_busy` and reports through the caller's observer only.
`Session::solve_hypothetical(node, port, value)` is the entry the
warmers use: the pipeline with one param overridden — spelled as
`param_preview` spells it — solved at idle class, writing nothing,
painting nothing (no frames, no statuses, no op; one `hypothetical`
row in `/debug/state` timings for the agent oracle), its results in the
ordinary memo so the later real solve of that value is a cache hit.
No UI rides it yet; items 4 and 5 are its consumers.)*

- **Slider ranges**: a slider with scrub caching enabled (per-param
  toggle, off by default, offered only when the step-quantized range
  has a bounded position count — 0…1 by 0.1 qualifies, by 0.01 does
  not; threshold set when the item lands, 2026-08-19) warms the dirty cone for its step-quantized range during
  idle time — nearest the current value first, walking outward (the
  values you're most likely to scrub to). Content addressing makes
  warming trivially incremental: it evaluates NodeKeys for
  hypothetical values and skips anything already stored. Warmed
  entries are ordinary cache entries — evictable, budget-bounded. The
  slider renders a **buffer bar** (video-player style) showing the
  warmed span.

  *(Live, v0.1 item 5 S1 — the engine half, 2026-08-24;
  `crates/cicada-server/src/scrub.rs` + the worker in `session.rs`;
  DECISIONS.md row 39 revised the same day.)* **Eligibility is a pure
  function of the slider's literals**: `positions = floor((max − min) /
  step) + 1` (the quotient nudged by 1e-9 before the floor — IEEE puts
  0…0.3 by 0.1 at 2.9999999999999996), eligible iff `step > 0`, `min`/
  `max`/`step` are literals and `positions ≤ SCRUB_MAX_POSITIONS` = 32
  (0…1 by 0.1 is 11, 0…10 by 0.5 is 21, 0…1 by 0.02 is 51 and refused).
  **The opt-in is the text**: `slider`'s `scrub = False` kwarg (version
  2; docs/08, docs/10) — the `set_scrub` gesture writes `scrub=True` or
  removes the kwarg, an op like any literal edit. **The positions are
  spelled as the canvas snaps them** (`min + k × step` rounded to the
  step's decimals, `web/src/canvas/grid.ts`), so a warmed literal and
  the widget's later tick build the same `NodeKey`. **The warmer** (the
  `cicada-scrub` thread) is generic over (param, ordered value list,
  visiting order): for every opted-in eligible slider it walks the
  positions nearest the committed value first, alternating sides (above
  first), ONE position at a time — a hash-only dry run of the position's
  cone first (`Core::dry_run`, the cost model's walk: a memo hit is
  recorded warm without a solve — skip-if-stored), else an idle-class
  `solve_hypothetical` — and stops at a **per-slider cap of 256 MiB**
  attributed to the warming, counted DEEP from the compressed blobs the
  position's computed outputs occupy (`DiskStore::stored_bytes`; the
  cost records carry elements and nanos, not bytes — a value shared
  between positions counts under each, so the cap is conservative). Any
  TEXT change drops every queue (the contract names the slider's own
  literals and `scrub` toggled off; every other text change changes the
  keys the warm set was verified against, so the new queue re-verifies
  — hits confirm in the dry run, nothing re-solves); a sidecar-only
  change keeps the queues. Idle time is when nothing real is happening:
  a live drag on any slider (within `DRAG_GAP_MS`) or transport playback
  blocks the worker; a position pre-empted by a real generation (its idle
  token cancelled) PARKS it until a real generation newer than the
  pre-empted solve completes — after an edit or a drag that is the next
  moment; Esc parks it OUTRIGHT, whatever it was doing — a position
  mid-solve is cut short, one decided but not yet submitted is withheld
  (next again on resume), one between positions takes nothing next —
  until a real generation newer than anything issued at the Esc
  completes: the user's next action ("stop solving" includes the
  warming). A position whose solve goes red is visited once and not
  retried within the queue's life. `warmed` means the position's
  MEMOIZABLE cone is stored: a `volatile` node (`clock`) recomputes in
  every generation by design — it is never memoized — while everything
  downstream of its unchanged value stays a hit, so a drag across warm
  positions computes the volatile node alone. Beside the slider's cone
  (feeding the same `add`) it leaves the dry run exact — the committed
  value is a hit as ever; inside the cone (`clock(speed=size)`) it is a
  miss in every dry run, so no position is confirmed without a solve and
  each solves once. Either way `slider_loop.mjs --expect warm`, which
  counts every computed node, fails on such a pipeline by construction
  (the session test `a_volatile_node_in_the_cone_keeps_the_rest_warm`
  pins both shapes). Protocol (docs/13): every
  slider's `ParamView.scrub`, the coalesced `scrub_progress` (≤ 10 Hz),
  `set_scrub` with typed refusals, `/debug/state.scrub`. Measured on
  `examples/02-solids.cic` (`size` 0.5…5.0 by 0.25 = 19 positions, the
  export `tessellate` in the cone): the
  worker started 90 ms after open (right behind the 88 ms load), walked
  the order `6, 7, 5, 8, 4, 9, 3, …` (2.0 is index 6), skipped the
  committed value as the memo hit it was, solved the other 18 positions
  in 18 idle-class generations (0 pre-empted; 2–62 ms each, 7 nodes
  computed / 4 cached per position) and was finished 0.9 s after open
  having stored 1.97 MB; a step-snapped `slider_loop.mjs --snap --expect
  warm` sweep afterwards (300 ticks over 5 s across all 19 positions)
  produced 300 preview generations — **0 nodes computed, 3,300 cached** —
  at 60.1 generations/s, server queued+elapsed p50 0.75 ms / p95 2.0 ms,
  client round-trip to the first frame of each newly painted position
  p50 6.1 ms / p95 12.3 ms (debug engine, 22 threads). The S2 web half
  renders the buffer bar and the toggle.
- **Cycle loops**: a `cycle` time param (docs/08) is a scrub over a
  fixed range by construction; its frames warm the same way,
  playhead-ahead first, so a loop becomes pure cache playback after
  at most one pass. *(Live, v0.1 item 4 — the pass itself: the
  transport injects `floor(t × frames / period) mod frames` into
  `cycle.frame` at lowering (docs/13 §Animation transport), so the loop
  IS a finite set of NodeKeys — `frames` of them per downstream node —
  and one pass of playback warms every one; the second pass is 100 %
  memo hits with an identical key set, a session test. A cone slower
  than the frame rate skips frames on the first pass and fills them in
  on the next. Playhead-ahead warming through the idle class is item
  5's generic warmer; not yet.)*
- Any other param change alters the hypothetical NodeKeys, so stale
  warm entries simply stop matching — **no invalidation bookkeeping
  exists at all**.

## Element failures

Default: the node goes **red with the offending element IDs and the
counterexample** (`carve failed on parts[412] (id C12): non-manifold
cutter`), downstream does not run — loud refusal, wall lesson 13. A
node may opt in to `on_error=skip` (an ordinary kwarg, visible in the
text forever), which converts failures into `Optional` empty slots —
slot-preserving, so indices never shift (wall lesson 2).

## Progress and ETA

- Cost samples per NodeKey (and per element for mapped nodes) persist
  in the memo table; the estimator uses them for **cost-weighted
  progress** — never node counts (wall lesson 9: "plate 15/16, part
  2/14" lies).
- ETA = critical-path estimate over the dirty set given current
  parallelism; displayed with a `~` until samples exist for the node
  kinds involved (first runs calibrate).
- Per-node badges: state (cached / queued / running / done / red /
  blocked / cancelled), last compute time, display cost beside it.

## Determinism rules

- Element results collect into **pre-sized, index-addressed slots** —
  parallel execution order can never reorder output.
- Reductions use fixed-order trees, not first-come accumulation;
  stable sorts with explicit tie-breaks everywhere.
- No ambient time, randomness, or locale anywhere in the engine;
  random nodes take explicit seeds.
- Same binary + same inputs → byte-identical outputs; the cache
  version salt handles cross-version honesty.

## Persistence and reopen

- The **lock file** records last-solve state: generation ID, per-node
  status, and output hashes. Reopening a project loads the memo table,
  verifies keys, and rehydrates the viewer from cache — a warm reopen
  computes *nothing*.
- A crash costs at most the in-flight chunks; everything completed is
  already on disk (wall lesson 8: killing Rhino lost the whole
  in-memory solve — unrepresentable here).
- Effectful nodes are excluded from auto-solve entirely; explicit runs
  record input/output hashes into the lock and stamp versions into
  manifests (doc 10 reproducibility).

## Why not salsa

Salsa (rust-analyzer's incremental framework) is the prior art for
memoization + early cutoff, and we steal those concepts. We don't
take the crate: salsa's power is *dynamically traced* dependencies,
but our dependency graph is static and explicit (the `.cic` file *is*
the graph); and our hard requirements — disk persistence,
per-element granularity, custom priorities, preemptive cancellation,
progress accounting — all live outside salsa's sweet spot. A custom
executor over content-addressed storage is less code than bending the
framework.

## Open questions (not yet locked)

- Whether the value store should ever be shared across projects
  (dedupe is tempting; provenance gets murkier). Per-project for now.
- Remote/distributed solve (a fleet of workers for farm-scale
  pipelines) — the content-addressed, client-server design permits
  it; explicitly out of scope until a real project demands it.
- Automatic fusion of hot mapped chains (compute `map(f) ∘ map(g)` as
  `map(f∘g)` without materializing intermediates) — a later
  optimization taken on profiling evidence; it must preserve wire
  inspection (fall back to unfused when inspected) and stays
  orthogonal to visual groups, which are aesthetics only (doc 16).
