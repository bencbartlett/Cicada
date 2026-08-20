# Engineering foundations

Everything a fresh clone (or a fresh agent) needs to build correctly:
workspace shape, representations, tolerances, the script ABI, test
standards, CI, and the operating rules for a dev team of one human
plus many context-free agents.

## Workspace layout

```
Cargo.toml                # workspace root
crates/
  cicada-core/       # value model: hashing, interning, axes, Optional slots, ProjectConfig
  cicada-macros/     # #[node], #[derive(Ports)] proc macros
  cicada-geom/       # geometry types, tolerance ops, kernel seams (manifold3d, opencascade-rs, spade, curvo, lyon, cavalier_contours, ttf-parser)
  cicada-lang/       # .cic lexer/parser/AST, minimal-edit writer, fmt; kind lattice, unification, axis rules, diagnostics; tree-sitter grammar
  cicada-stdlib/     # the node catalog (docs/08)
  cicada-sched/      # generations, stores, executor, cost models, scrub warming
  cicada-script/     # wasmtime host, Python worker pool, marshalling
  cicada-server/     # axum app: protocol, sessions, op log, transport, git
  cicada-cli/        # the `cicada` binary: serve, run, fmt, docs, catalog, cache
web/                 # SPA: React + TypeScript + Vite + xyflow + three.js + zustand
examples/            # runnable examples; examples/wall/ = the wall (pipeline, scripts, inputs, golden refs) — the nightly regression corpus is every example with goldens
tools/               # engine-wide dev tooling: exporter-output normalizer, offline tests, measurement harness
docs/                # these documents (+ docs/generated/, see Documentation pipeline)
```

**Dependency direction is law**: `core ← {geom, lang, stdlib, sched,
script} ← server ← cli`. Nothing depends on `server` except `cli`;
`stdlib` never depends on `sched` (nodes are pure functions; the
scheduler calls *them*); `web/` speaks only the protocol. A CI check
asserts the dependency DAG matches this layering.

The principles that produced this shape: **the core stays tiny and
fast** (everything depends on it, so it must compile in seconds);
**heavy dependencies are quarantined** (`geom` holds kernel FFI,
`script` holds wasmtime, `server` holds axum/tokio — iterating on the
scheduler never rebuilds a kernel binding); **churn is isolated**
(`stdlib` is ~130 nodes of constant addition; node work rebuilds one
crate); `macros` is separate because Rust requires it; parser and
checker share one crate (`lang`) because they share the AST and
diagnostics types and co-evolve — splitting them would put an
interface boundary through the highest-churn seam. Crates may start
merged and split along these seams as compile times demand; **the
seams are the contract, not the crate count**. Crate boundaries
double as agent work boundaries — most tasks should touch one crate.

## Toolchain and conventions

- Rust stable, edition 2024; MSRV = current stable, updated freely
  (solo project — no legacy support burden).
- `rustfmt` defaults; lints: `clippy::all` + `clippy::pedantic` with
  a small curated allow-list for noise, `-D warnings` in CI. **Fail
  loudly and immediately**: `unwrap`/`expect` are lint-denied in
  library code (proper errors or invariant panics with messages);
  `overflow-checks = true` in all profiles including release; no
  silent fallbacks anywhere — a wrong answer is worse than a loud
  refusal (wall lesson 13).
- `unsafe` only inside FFI seam modules, each block with a
  `// SAFETY:` comment.
- Errors: `thiserror` enums in library crates; `anyhow` allowed only
  in `cicada-cli`. User-facing problems are **diagnostics** (the
  typed structure of doc 11), never bare strings.
- Serialization: **postcard** (+ leading version byte) for cache
  blobs and WASM buffers; **MessagePack** for the Python boundary;
  **JSON** for the protocol control plane and the node catalog.
- Python tooling on the script side: `uv`-managed project venv.

## Value and geometry representations

f64 is canonical (ledger); concrete layouts:

- Scalars/spatial: `glam` f64 types (`DVec3`, `DAffine3`); `Point`
  and `Vector` are distinct newtypes over `DVec3`.
- **Mesh** — SoA, flat, `Arc`'d, hash computed at construction:

```rust
struct Mesh {
    positions: Arc<[f64]>,        // xyz interleaved, len = 3·v
    triangles: Arc<[u32]>,
    normals: Option<Arc<[f64]>>,
    hash: ValueHash,              // blake3
}
```

- **Curves are analytic**, not tessellated: the `Curve` enum stores
  parameters (a Circle is a plane + radius; a Nurbs is control
  points/weights/knots/degree). Tessellation for display or meshing
  is a derived, cached, costed operation (doc 12).
- **Solid is B-rep-backed (OCCT) from v0.1** (docs/08): an opaque
  kernel shape handle wrapped with a content hash (hash of the
  canonical serialized shape — serialization stability is a
  spike-verify item). `Watertight<Mesh>` (Manifold-validated) is the
  mesh-tier solid; the seams convert zero-copy where kernel layouts
  allow.
- Display buffers are derived f32 with origin-rebasing (doc 12);
  they live in the display cache, never in the value model.
- **No dtype-generic geometry.** Values are f64-only; parameterizing
  the value model over f32/f64 would double the stdlib surface,
  marshalling, and cache-key combinatorics while fighting the
  f64-only kernels (OCCT, curvo). The f32 wins are captured where
  they pay without new value types: the display path is already f32;
  individual nodes may compute internally in f32 when accuracy
  permits; and a dedicated bulk-f32 kind (e.g. point clouds) can be
  added later if a real workload demands it.

## Tolerance and units policy

- The document unit tag defaults to **mm** (consumed by exporters;
  resolves the docs/08 open question).
- One **`ProjectConfig`** value carries `unit`, `tol` (default
  `1e-6` units — point coincidence, closure, join threshold), and
  `tol_angle` (default `1e-9` rad — parallel/planar checks). It is
  configurable per project, and it **participates in cache keys**:
  nodes that consult tolerance declare it (`#[node(uses_tolerance)]`)
  and get it hashed into their `NodeKey` — tolerance is explicit
  state, never ambient (the no-ambient-state rule survives CAD).
- No scattered epsilons: the sanctioned comparison API
  (`approx_eq`, `coincident`, `is_closed_within`) is the only float
  comparison path in geometry code. Raw `==` on floats is lint-banned in
  geometry code; exact `==` is sanctioned in hash/determinism tests and
  in stdlib tests whose node contract is exact IEEE arithmetic (pure
  maths — ledger revision 2026-08-12).
- **Units are changeable after the fact**, two ways: **relabel**
  (1 mm → 1 in; numbers untouched) or **convert** (1 mm → 0.03937 in;
  numbers rescaled). Convert rewrites the literals of params feeding
  length-dimensioned ports — port specs carry a dimension tag
  (`#[port(dimension = length)]`) so the engine knows a `radius` from
  a `count` — including slider min/max/step, as one undoable op.
  Anything it can't confidently convert (free variables in
  expressions) is flagged for review, never silently scaled. Either
  way the `ProjectConfig` hash changes, dirtying exactly the
  unit-sensitive caches.

## The script marshalling ABI

- **Python** (ecosystem fallback): persistent worker subprocesses;
  length-prefixed MessagePack frames over pipes. Geometry crosses as
  typed maps wrapping flat `f64`/`u32` binaries (msgpack `bin`) —
  `array('d').frombytes` / `numpy.frombuffer` view them without a
  per-float Python loop (stage-6 measurement: 7,200 meshes × 100
  vertices cross in ~0.1 s release, either direction). Kill = cancel;
  workers respawn. Impurity breaks caching honestly: the engine hashes
  declared inputs only, and the docs say so loudly.
  **As shipped (stage 6)**: `@cicada.node(title, description,
  effectful=False)`; inputs are kwargs with string annotations in
  catalog notation; the return annotation declares the outputs —
  `-> "T"` (port `out`), `-> {"name": "T", …}` (multi-output; the
  function returns a dict with exactly those keys, order = port order),
  `-> None` (exporters; no outputs, effectful). Kinds that cross:
  Number, Integer, Boolean, Text, Point, Vector, Domain, Plane, Mesh,
  `Watertight<Mesh>`, Curve (polyline, line, circle, rectangle),
  `Closed<Curve>`, `[…]` to any depth, `?` optionality (Python `None` =
  absent slot). Declared refinements are RE-CHECKED on the way back
  from Python (an unwatertight mesh behind a `Watertight<Mesh>`
  annotation is red with counts) and dropped to the base kind on the
  way in. The `cicada` module offers `Mesh` (flat arrays +
  `from_triangles`), `Plane`, `Polyline`, `Line`, `Circle`,
  `Rectangle`, `Vector`, `Domain`; a plain 3-tuple is a Point.
  Effectful script leaves skip in `cicada run` unless named with
  `--node`, and run from the app via `POST /api/run/{node}`.
- **WASM** (the Rust script default): wasmtime with epoch
  interruption (per-call deadlines), a memory cap (default 512 MB),
  and **no WASI filesystem/network** — script nodes are pure compute.
  Values cross as postcard buffers in linear memory (shared-memory
  fast path is a later optimization). A `cicada-guest` SDK crate
  provides the types and a macro that emits the ports manifest as a
  custom section; compiled artifacts cache by source hash (doc 12).

## Documentation pipeline

One source of truth: the doc comments on nodes and ports. The
`NodeSpec` already captures them (docs/08); two artifacts generate
from it, and nothing is hand-maintained in parallel:

- **`docs/generated/CATALOG.md`** — the condensed reference: one line
  per registered node (signature + title line), grouped by category,
  **committed to the repo and CI-checked** (regenerate + diff, like a
  lockfile). This is what agents read while building — signatures
  without grepping crates, a few KB of context instead of thousands
  of lines of source. `cicada catalog` regenerates it; inside a user
  project it merges the project's script nodes so the reference is
  always complete.
- **`cicada docs`** — the human-readable reference: per-node HTML
  pages generated from the same doc comments, with runnable dialect
  examples that CI actually executes (an example that stops solving
  fails the build).

Standards that keep this alive: every node's doc comment has a title
line, a description, and at least one example; `add-stdlib-node` and
script-authoring flows regenerate the catalog in the same commit; a
stale CATALOG.md fails CI.

**The node file format (v0.1, decided 2026-08-19).** One node per
file: `crates/cicada-stdlib/src/<category>/<node>.rs`, categories =
the ribbon tabs (docs/08), a `mod.rs` per category listing the files.
Each file holds, in this order: the input struct (`#[derive(Ports)]`,
one doc line per field — units where relevant — `#[port(default)]`
for optional ports); the node doc comment with fixed sections — line 1
`Title — one-sentence description`, an optional paragraph of
semantics, `# Panics` (rendered as "Red when"), `# Examples` (a
runnable `.cic` snippet CI solves — REQUIRED); the `#[node(category,
tier, version, gh = "Move" | none)]` function (`gh` = the Grasshopper
component it replaces, fed to search-to-place and the docs); and the
three tests (table cases, property test, golden hash). A conformance
test in the crate fails the build when any registered node lacks a
piece. One source renders every view: `CATALOG.md`, `catalog.json`,
`/api/catalog`, `cicada mcp` (catalog search + node docs + the checker
for agents, v0.1), and `cicada docs`. Node icons are generated assets in the
same spirit (doc 16): produced by the AI icon pipeline from
NodeSpecs, committed, CI-warned when missing.

## Testing standards

| Layer | Standard |
|---|---|
| Stdlib nodes | Table-driven cases + a proptest property test + a **determinism test** (golden blake3 output hashes — byte-identical is a unit test) for every node |
| Dialect | Golden round-trip corpus (parse → emit, byte-identical); a gesture-edit fixture for every op in doc 10's round-trip table |
| Checker | Positive + negative wiring cases per rule; diagnostics as `insta` snapshots |
| Scheduler | Fake nodes with virtual costs and **virtual time** — cache-hit, dirty-cone, supersession, cancellation-latency, and warming tests run in milliseconds |
| Geometry | Tolerance-aware asserts (explicit eps), golden mesh hashes; never raw float equality |
| Protocol | Server + headless client library integration tests; snapshot/replay fixtures |
| End-to-end | Playwright smoke (serve → place → wire → drag → screenshot); the wall corpus nightly with output-hash comparison |

Standing rules: every bug fix lands with the regression test that
would have caught it, in the same commit; every stdlib node ships
with tests and a doc example; no `#[ignore]` without a linked issue;
tests are deterministic — no sleeps, no wall-clock, no network.
Golden files update only through the blessed command (`cargo insta
review` / `--bless`), never by hand.

## CI pipeline (GitHub Actions)

- **Every PR / push**: `rustfmt --check`; `clippy -D warnings`;
  workspace tests (Linux); workspace tests on Windows + macOS too while
  the suite stays fast — the determinism-hash DoD is cross-platform, so
  it should hold at merge time, not nightly-after (demote to
  `cargo check` when suite runtime demands); web `tsc` + eslint +
  vitest; WASM guest build check; Playwright smoke. Rust build caching
  via `rust-cache`.
- **Nightly**: full test matrix (Linux/Windows/macOS); wall-corpus
  end-to-end with hash comparison; criterion benchmarks against
  stored baselines (fail on >10% regression); `cargo deny` +
  `cargo audit`.
- The corpus's bulky inputs live in git LFS if they exceed tens of
  MB (flagged below).

## Agent operating standards

The dev team is one human directing many context-free agents, so the
repo itself carries the operating manual:

- **Root `CLAUDE.md` / `AGENTS.md`** (created with the workspace):
  the project map (one line per crate), the command palette (build,
  test, serve, run, bless), the rule **"read `DECISIONS.md` before
  designing anything"**, the definition of done, commit conventions,
  and the doc-update rule: *a behavior change that contradicts a
  design doc updates the doc and the ledger in the same commit*.
- **Skills** (`.claude/skills/`), one per recurring workflow:
  - `verify-change` — the headless-first loop: build → unit tests →
    `cicada run corpus-slice --hashes` → drive the browser via
    Playwright only if the change is UI-facing → attach evidence.
  - `add-stdlib-node` — checklist: node fn + `Ports` structs + tests
    (table/property/determinism) + catalog check + docs/08 row.
  - `dialect-change` — grammar/writer changes with round-trip
    fixtures on both sides.
  - `protocol-change` — server + client + snapshot fixtures updated
    together.
  - `perf-check` — criterion before/after with the relevant bench.
- **Working rules**: scope to one crate where possible; **read
  `docs/generated/CATALOG.md` for node signatures instead of grepping
  crates** (context is expensive; the catalog is a few KB); run the
  touched crate's tests plus `cargo check --workspace` before
  declaring done; never leave fmt/clippy red; small PRs; the commit
  body states *why*, the diff states *what*.
- **The verification loop is agent-operated**: engine restarts are
  warm (doc 12 — a restart recomputes nothing), `cicada run`
  verifies most changes headlessly, `/debug/state` and
  `/debug/screenshot` (doc 13) plus Playwright let agents see the
  running app themselves. The human reviews evidence; the human is
  never the feedback loop.

## Definition of done

A change is done when: fmt/clippy/tests are green locally; new
behavior has tests at the right layer; determinism hashes updated
through the blessed path with the diff explained; docs and ledger
updated if design-relevant; evidence attached (test output, hashes,
or screenshots) for anything user-visible.

## License

**All rights reserved while the repo is private** (ledger,
2026-08-11). The real license gets chosen at first public release,
when product shape and any commercial intent are clear — the options
weighed then: AGPL-3.0 (copyleft; blocks closed SaaS forks),
Apache-2.0 (permissive, patent grant), BSL 1.1 (source-available,
converts to open later).

## Open questions (not yet locked)

- Criterion baseline storage: committed JSON vs a benchmarks branch.
- The exact pedantic allow-list (curate during the spike from real
  noise, not speculation).
