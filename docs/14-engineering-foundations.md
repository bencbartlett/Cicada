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
  cicada-geom/       # geometry types, tolerance ops, kernel seams (manifold3d, spade, curvo, lyon, cavalier_contours, ttf-parser)
  cicada-dialect/    # .cic lexer/parser/AST, minimal-edit writer, fmt; tree-sitter grammar
  cicada-check/      # kind lattice, unification, axis/shape rules, diagnostics
  cicada-stdlib/     # the node catalog (docs/08)
  cicada-sched/      # generations, stores, executor, cost models, scrub warming
  cicada-script/     # wasmtime host, Python worker pool, marshalling
  cicada-server/     # axum app: protocol, sessions, op log, transport, git
  cicada-cli/        # the `cicada` binary: serve, run, fmt, cache
web/                 # SPA: React + TypeScript + Vite + xyflow + three.js + zustand
corpus/              # wall-pipeline end-to-end corpus + golden hashes
docs/                # these documents
```

**Dependency direction is law**: `core ← {geom, dialect, check,
stdlib, sched, script} ← server ← cli`. Nothing depends on `server`
except `cli`; `stdlib` never depends on `sched` (nodes are pure
functions; the scheduler calls *them*); `web/` speaks only the
protocol. Crate boundaries double as agent work boundaries — most
tasks should touch one crate.

## Toolchain and conventions

- Rust stable, edition 2024; MSRV = current stable, updated freely
  (solo project — no legacy support burden).
- `rustfmt` defaults; `clippy --all-targets -- -D warnings` gates CI.
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
- **Solid** wraps a Manifold-validated watertight mesh (docs/08); the
  seam converts zero-copy where kernel layouts allow.
- Display buffers are derived f32 with origin-rebasing (doc 12);
  they live in the display cache, never in the value model.

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
  comparison path in geometry code. Raw `==` on floats is
  lint-banned except in hash/determinism tests.

## The script marshalling ABI

- **Python** (ecosystem fallback): persistent worker subprocesses;
  length-prefixed MessagePack frames over pipes. Geometry crosses as
  typed extensions wrapping flat `f64` binaries — `numpy.frombuffer`
  views them zero-copy. Kill = cancel; workers respawn. Impurity
  breaks caching honestly: the engine hashes declared inputs only,
  and the docs say so loudly.
- **WASM** (the Rust script default): wasmtime with epoch
  interruption (per-call deadlines), a memory cap (default 512 MB),
  and **no WASI filesystem/network** — script nodes are pure compute.
  Values cross as postcard buffers in linear memory (shared-memory
  fast path is a later optimization). A `cicada-guest` SDK crate
  provides the types and a macro that emits the ports manifest as a
  custom section; compiled artifacts cache by source hash (doc 12).

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
  workspace tests (Linux); `cargo check` on Windows + macOS; web
  `tsc` + eslint + vitest; WASM guest build check; Playwright smoke.
  Rust build caching via `rust-cache`.
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
- **Working rules**: scope to one crate where possible; run the
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

- Exact clippy lint set beyond `-D warnings` (candidate: a curated
  pedantic subset).
- Corpus asset storage: in-repo vs git LFS (size-dependent).
- Criterion baseline storage: committed JSON vs a benchmarks branch.
