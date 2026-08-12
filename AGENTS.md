# Cicada — agent operating manual

Cicada is a code-first parametric design tool: the pipeline is a typed
program (`.cic` dialect), the node graph is a generated view, the engine is
Rust. Design docs 01–16 in `docs/` plus `DECISIONS.md` fully specify the
system; the current work order is the vertical-slice spike
([docs/15-spike-plan.md](docs/15-spike-plan.md)).

## Read this first

1. **`DECISIONS.md` before designing anything.** It is the binding ledger.
   Never contradict a row. If implementation genuinely forces a change,
   revise the row explicitly and update the affected doc in the same commit —
   never silently.
2. **`docs/generated/CATALOG.md` for node signatures** instead of grepping
   crates — context is expensive; the catalog is a few KB and CI-checked
   fresh.
3. The stage you are working in, from
   [docs/15-spike-plan.md](docs/15-spike-plan.md), and the docs it lists for
   that stage.

**Current status: stage 0 complete — scaffold only.** The parser, scheduler,
geometry, server, and web app do not exist yet. `cicada catalog` is the only
live subcommand. Commands below marked *(stage N)* arrive with that stage;
do not reference them in code or docs as if they work today.

## Project map

| Path | Contents |
|---|---|
| `crates/cicada-core` | Node/port specs + catalog renderer; value model — hashing, interning, axes, Optional slots, ProjectConfig — lands stage 1 |
| `crates/cicada-macros` | `#[node]`, `#[derive(Ports)]` proc macros (stage 1) |
| `crates/cicada-geom` | Geometry types, tolerance ops, rented-kernel FFI seams (stage 4) |
| `crates/cicada-lang` | `.cic` lexer/parser/AST, minimal-edit writer, checker, diagnostics (stage 2) |
| `crates/cicada-stdlib` | The node catalog — pure functions, never depends on sched |
| `crates/cicada-sched` | Generations, stores, executor, cost models (stage 3) |
| `crates/cicada-script` | WASM host (v0.1) + Python worker pool (stage 4) |
| `crates/cicada-server` | axum app: protocol, sessions, op log (stage 5) |
| `crates/cicada-cli` | The `cicada` binary; also hosts the dependency-DAG test |
| `web/` | SPA: React + TypeScript + Vite (canvas/viewport land in stage 5) |
| `corpus/` | Wall-pipeline corpus + golden hashes (stage 6) |
| `docs/` | Design docs 01–16 + `docs/generated/` |

**Dependency direction is law**: `core ← {geom, lang, stdlib, sched, script}
← server ← cli`; only `cli` may depend on `server`; `stdlib` never depends on
`sched`. Enforced by `crates/cicada-cli/tests/dependency_dag.rs`.

## Command palette

| Task | Command |
|---|---|
| Build (all) | `cargo check --workspace --all-targets` |
| Test (all) | `cargo test --workspace` |
| Test (one crate) | `cargo test -p cicada-core` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Format | `cargo fmt --all` (check: `--check`) |
| Regenerate catalog | `cargo run -p cicada-cli -- catalog` |
| Catalog freshness (CI mode) | `cargo run -p cicada-cli -- catalog --check` |
| Web checks | `cd web && npm run check && npm run lint && npm test` |
| Serve *(stage 5)* | `cicada serve` |
| Headless run *(stage 3+)* | `cicada run <pipeline> --node <name> --time` |
| Bless goldens *(insta arrives stage 2)* | `cargo insta review` |

Web work needs Node ≥ 20 (CI uses 22).

### Dev machine notes (Windows)

- The repo lives in a **Dropbox-synced folder**. Build dirs must be excluded
  from sync or Dropbox's file handles break builds (observed: cargo failing
  to finalize `target/incremental`). `target/` and `web/node_modules/` are
  marked with the `com.dropbox.ignored` NTFS stream; after deleting and
  recreating either directory, re-mark it:
  `Set-Content -Path <dir> -Stream com.dropbox.ignored -Value 1`.
- The engine cache itself never has this problem — it lives in the user
  cache directory, never the project folder (DECISIONS.md).

## Working rules

- **Scope to one crate where possible.** Crate boundaries are agent work
  boundaries. Run the touched crate's tests plus
  `cargo check --workspace` before declaring done.
- **Fail loudly and immediately.** No silent fallbacks; a wrong answer is
  worse than a loud refusal. `unwrap`/`expect` are lint-denied in library
  code (tests exempt); `overflow-checks` stay on in release; errors are
  `thiserror` enums in libraries, `anyhow` only in `cicada-cli`;
  user-facing problems are typed diagnostics, never bare strings.
- **`unsafe` only inside FFI seam modules**, each block with a
  `// SAFETY:` comment.
- **Never leave fmt/clippy red.** CI runs `-D warnings`; so should you.
- **Determinism is a unit test.** Golden hashes update only through the
  blessed path, never by hand, and the diff gets explained in the commit.
- **Tolerance is explicit state** — the sanctioned comparison API is the
  only float-comparison path in geometry code; raw float `==` only in
  hash/determinism tests.
- **The cache never lives in the project folder** (project dirs are
  Dropbox-synced): user cache directory only; `.cicada-cache/` is an
  opt-in override and stays gitignored.
- **Verification is agent-operated, headless-first** (skill:
  `verify-change`). The human reviews evidence; the human is never the
  feedback loop.
- Tests are deterministic — no sleeps, no wall-clock, no network. No
  `#[ignore]` without a linked issue.

## Definition of done

A change is done when: fmt/clippy/tests are green locally; new behavior has
tests at the right layer (see the table in
[docs/14-engineering-foundations.md](docs/14-engineering-foundations.md)
§Testing standards); determinism hashes updated through the blessed path
with the diff explained; docs and `DECISIONS.md` updated if
design-relevant; evidence attached (test output, hashes, or screenshots)
for anything user-visible.

## Commit conventions

- Imperative subject line; **the body states *why***, the diff states what.
- **Doc-update rule**: a behavior change that contradicts a design doc
  updates the doc *and* the ledger row in the same commit.
- **Bug fixes land with the regression test that would have caught them,
  in the same commit.**
- Every stdlib node change regenerates `docs/generated/CATALOG.md` in the
  same commit (CI diffs it).
- Commit at sensible stage boundaries; **push only when Ben says so.**

## Skills

| Skill | Use for |
|---|---|
| `.claude/skills/verify-change` | The evidence loop before declaring any change done |
| `.claude/skills/add-stdlib-node` | Adding or modifying a node in `cicada-stdlib`, end to end |

More arrive with their workflows (doc 14): `dialect-change` (stage 2),
`protocol-change` (stage 5), `perf-check` (first benchmarks).
