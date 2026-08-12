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

## The store

Two levels:

1. **Value store** — content-addressed blobs: `hash → zstd(bytes)`.
   Deduplicated across nodes, solves, and time by construction.
2. **Memo table** — `NodeKey → {output hashes, status, warnings with
   element IDs, cost samples, display-cost samples}`.

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
- Solves are **generations**: an edit starts one after a ~30 ms
  debounce; a newer edit cancels and supersedes the in-flight
  generation. Supersession is cheap because completed work landed in
  the cache — a slider drag is a stream of generations, each reusing
  everything the previous one finished.
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
chunks, and at safe points inside long stdlib loops. Script nodes are
hard-cancellable **by construction**: WASM epoch preemption (hard
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
