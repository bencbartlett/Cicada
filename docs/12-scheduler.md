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
design** — the flag `Clock` wears (DECISIONS.md time row; item 4). The
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
   `{elements, nanos}` on node-level entries, so a cache hit still
   knows what it cost when it last ran and the cost model stays
   complete across a warm reopen; per-op samples ride beside the
   entries as their own records.)*

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
without a separate "kill the scripts" step to forget.)* Script nodes
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
  `DRAG_GAP_MS` (300 ms); a write attempt (landed or refused), an Esc,
  a reload or a longer pause ends it, and the next tick starts a fresh
  one — re-predicted, re-announced if withheld (doc 13 §Slider drags
  has the client-side reading).
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
- **Cycle loops**: a `cycle` time param (docs/08) is a scrub over a
  fixed range by construction; its frames warm the same way,
  playhead-ahead first, so a loop becomes pure cache playback after
  at most one pass.
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
