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
   fresh. It also carries each node's runtime contract ("Red when: …",
   from its rustdoc `# Panics` section) and the type-variable legend
   (`T` = kind-preserving transformable, `E` = any element kind, `Any` =
   display-sink catch-all).
3. The stage you are working in, from
   [docs/15-spike-plan.md](docs/15-spike-plan.md), and the docs it lists for
   that stage.

**Current status: stage 5 complete — the app: server, canvas, viewport.**
Live: the value model + `#[node]` registry (stage 1), the `.cic` toolchain
(stage 2: lossless parser, minimal-edit writer — place / wire / unwire /
lift / set-param / delete / rename — checker-lite with type variables
`T`/`E`), the scheduler (stage 3: content-addressed `NodeKey`s, two-level
disk store in the USER cache dir, rayon wavefront with chunked `each()`
fan-out, cancellation, latest-wins previews; effectful nodes bypass the
memo), stage 4 (geometry value kinds, `cicada-geom` with the Manifold and
spade seams, ~40 S-tier nodes with table + property + golden-hash tests,
the `# Panics`→catalog contract, the debug OBJ exporter, the Python worker
pool), and stage 5: `cicada-server` (axum: token-gated HTTP + one
WebSocket per client, JSON control plane + generation-tagged binary
frames with pick ids and hash-driven instancing, per-pipeline sessions
with the single-writer lease, intents → doc-10 writer gestures persisted
immediately → full view-model deltas, a 30 ms structural debounce and a
no-debounce latest-wins preview loop, ≤10 Hz coalesced statuses + ETA,
the project watcher with barrier snapshots, explicit effectful runs via
`POST /api/run/{node}`, `/debug/state` + `/debug/screenshot`; the
`.cic`→`SolveGraph` lowering and script discovery moved here from the
CLI — `cicada run` is a printer over them) and `web/` (React Flow canvas
with search-to-place, typed ports, server-probed live wire compatibility,
lift chips, red wires, sliders on canvas; three.js viewport with merged
draws + instancing, ID-buffer backward picking, Rhino-style navigation;
ribbon, inspector, params + read-only text panels, keyboard map;
Playwright smoke). Live subcommands: `cicada catalog`, `cicada run`
(always pass `--cache-dir` in tests; effectful bindings run only via
`--node`), `cicada serve`. `examples/` is the runnable playground — also
for the app (`cicada serve examples/02-solids.cic` — the canvas WRITES
the served files, so for throwaway experiments serve a scratch copy;
serving the committed examples is fine when you mean to change them).
Commands below marked
*(stage N)* arrive with that stage; do not reference them in code or docs
as if they work today.

## Project map

| Path | Contents |
|---|---|
| `crates/cicada-core` | Value model (blake3 hash-at-construction, interning, Merkle lists/axes/Optional, ProjectConfig) + node/port specs, registry (specs + erased invokers), marshalling traits, catalog renderer |
| `crates/cicada-macros` | `#[node]`, `#[derive(Ports)]` proc macros — zero workspace deps by design; compile-fail tests live in cicada-core (tests/ui + macro_ui.rs) |
| `crates/cicada-geom` | Geometry types, tolerance ops, rented-kernel FFI seams (stage 4) |
| `crates/cicada-lang` | `.cic` dialect: lossless parser, minimal-edit writer (place/wire/unwire/lift/set-param/delete/rename), checker-lite, doc-11 diagnostics |
| `crates/cicada-stdlib` | The node catalog — pure functions, never depends on sched |
| `crates/cicada-sched` | Scheduler-lite: solve graph + `NodeKey`s, two-level disk store (memo log + zstd blobs), rayon wavefront executor with `each()` fan-out, cancellation, latest-wins previews, cost sampling |
| `crates/cicada-script` | WASM host (v0.1) + Python worker pool (stage 4) |
| `crates/cicada-server` | The engine server (docs/13): axum app + token auth (`http.rs`), per-pipeline `Session` (intents → writer gestures → deltas, statuses, display set, lease), the latest-wins generation loop (`solve.rs`), JSON protocol (`protocol.rs`), graph view-model (`viewmodel.rs`), byte-exact binary frames (`frames.rs` IS the spec) + value→frame/summary (`display.rs`), sidecar + auto-layout, AND the hydration path shared with `cicada run`: `compile.rs` (targets, cone gate), `lower.rs`, `scripts.rs` (Python nodes + the cancel bridge). `embed` feature bakes `web/dist` in |
| `crates/cicada-cli` | The `cicada` binary: `catalog`, `run` (a printer over the server's compile/lower), `serve`; hosts the dependency-DAG test |
| `web/` | SPA: React + TypeScript + Vite; `src/protocol` (message + frame mirrors, WS client), `src/state` (zustand store, connection, frame bus), `src/canvas` (React Flow), `src/viewport` (three.js), `src/panels`, `e2e/` (Playwright smoke) |
| `corpus/` | Wall-pipeline corpus + golden hashes (stage 6) |
| `docs/` | Design docs 01–16 + `docs/generated/` |

**Dependency direction is law**: `core ← {geom, lang, stdlib, sched, script}
← server ← cli`; only `cli` may depend on `server` (it does, since stage 5:
`serve`, and `run` reuses the server's compile/lower); `stdlib` never
depends on `sched`. Within the mid layer, `stdlib → geom` is a sanctioned edge (nodes
ARE the geometry users, docs/03); no other intra-mid-layer edges exist.
Enforced by `crates/cicada-cli/tests/dependency_dag.rs`.

## Command palette

| Task | Command |
|---|---|
| Build (all) | `cargo check --workspace --all-targets` |
| Test (all) | `cargo test --workspace` |
| Test (one crate) | `cargo test -p cicada-core` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Format | `cargo fmt --all` (check: `--check`) |
| Regenerate catalog (CATALOG.md + catalog.json) | `cargo run -p cicada-cli -- catalog` |
| Catalog freshness (CI mode) | `cargo run -p cicada-cli -- catalog --check` |
| Bless macro compile-fail snapshots (PowerShell) | `$env:TRYBUILD = "overwrite"; cargo test -p cicada-core --test macro_ui; Remove-Item Env:\TRYBUILD` |
| Web checks (bash; PS 5.1 has no `&&` — use `;`) | `cd web && npm run check && npm run lint && npm test` |
| Serve the app | `cargo run -p cicada-cli -- serve <dir-or-pipeline.cic> [--port 8420] [--token …] [--cache-dir …] [--web-dir web/dist]` — prints the URL with the token; without a built SPA it is API-only and says so at `/`. Dev: `cd web && npm run dev` (Vite proxies `/api`, `/ws`, `/debug` to port 8420 — `CICADA_SERVER=` overrides) and open the Vite URL with the same `?token=…&pipeline=…`. Release shape: `cd web && npm run build` then `cargo build -p cicada-cli --features embed` |
| Playwright smoke (doc 15 DoD) | `cd web && npm run build && npm run e2e` — starts `cicada serve` from `$CARGO_TARGET_DIR/debug/cicada` (or `CICADA_BIN`) over a scratch copy of `examples/` (it prints which engine/scratch it uses — in a worktree export the PRIVATE `CARGO_TARGET_DIR` in the same shell or you smoke the main checkout's engine); first time: `npx playwright install chromium` |
| Agent verification of the running app | `GET /debug/state?token=…&pipeline=…&wait=true` (authoritative JSON: graph, statuses, per-output display bounds/triangles, generation timings; `wait=true` blocks until the debounce and any queued/in-flight generation are done — an intent sent on the socket is "in" once its `delta` arrived, so read the delta (or poll `seq`) before asking), `GET /debug/screenshot?token=…` (viewport PNG rendered by a connected client), `window.__cicada.{state,frames,scene,send,screenshot}` in the page |
| Headless run | `cargo run -p cicada-cli -- run <pipeline.cic> [--node <name>]… [--time] [--hashes] [--cache-dir <dir>] [--threads N]` — no `--node` = every leaf; `--hashes` prints stable hash lines INSTEAD of values; dialect syntax: [docs/10](docs/10-dialect-and-file-format.md); tests/CI always pass `--cache-dir` |
| Bless insta snapshots (checker diagnostics) | `cargo insta review` (cargo-insta installed 2026-08-12) — or `$env:INSTA_UPDATE = "always"; cargo test -p cicada-lang; Remove-Item Env:\INSTA_UPDATE` |
| Carve benchmark (kernel seam, release only) | `cargo run --release -p cicada-geom --example carve_bench [parts]` — see skill `perf-check` |
| Run the examples playground | `cargo run -p cicada-cli -- run examples/<file>.cic [--node dump] [--time]` |

Web work needs Node ≥ 20 (CI uses 22). The Python script host needs
Python 3 on PATH (or `CICADA_PYTHON`); worker protocol is dependency-free
— numpy etc. are only needed by scripts that import them.

### Dev machine notes (Windows)

- The repo lives in a **Dropbox-synced folder**, and Dropbox's file
  handles break builds run inside it (observed twice: cargo failing to
  finalize `target/` files, os error 32 — even with the
  `com.dropbox.ignored` NTFS stream set). The durable fix: cargo builds
  OUTSIDE the synced tree via a user-level environment variable, set on
  this machine since 2026-08-12:

  ```powershell
  [Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", "$env:LOCALAPPDATA\cargo-target", "User")
  ```

  Fresh shells inherit it; agent shells with stale environments must set
  `$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\cargo-target"` per command.
  There must be no in-repo `target/`. `web/node_modules/` stays in-repo
  (npm needs it in place) with the `com.dropbox.ignored` stream set —
  re-mark it if it is ever deleted and reinstalled:
  `Set-Content -Path web\node_modules -Stream com.dropbox.ignored -Value 1`.
  CI is unaffected (no Dropbox on runners; no committed target-dir config).
- **Git worktrees must NOT share the main checkout's `CARGO_TARGET_DIR`**
  — cargo fingerprints collide across workspaces sharing a target dir,
  and a worktree's build can silently masquerade as the main checkout's
  (observed: a worktree-built `cicada.exe` wrote the worktree's registry
  into the main repo's generated catalog). In a worktree, set a private
  dir per shell:
  `$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\cargo-target-wt\<worktree-name>"`.
- The engine cache itself never has this problem — it lives in the user
  cache directory, never the project folder (DECISIONS.md).
- **cmake for the Manifold kernel build**: `manifold-csg-sys` compiles
  upstream Manifold via cmake. cmake is not on this machine's PATH;
  prepend the VS Build Tools copy per shell:

  ```powershell
  $env:Path = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;$env:Path"
  ```

  Fresh builds git-clone Manifold v3.5.2 + oneTBB (network needed once
  per `OUT_DIR`). Two truths learned the hard way (stage-4 probe):
  1. The kernel rebuilds more often than "once per profile": different
     feature-unification contexts (`--workspace` vs `-p <crate>` vs
     `--release`) fingerprint separately — budget ~4 multi-minute kernel
     compiles on a fresh target dir; the recompiles are not errors.
  2. The clone failure once called "transient" is a DETERMINISTIC
     Windows long-path failure (oneTBB's `rfcs/` doc assets exceed
     MAX_PATH, worse under the longer worktree target dirs); a bare
     retry only "works" because git leaves the offending doc files
     deleted. The real fix, set machine-wide on 2026-08-18:
     `git config --global core.longpaths true`. In environments without
     that global (fresh CI-like shells), pass it per process:
     `$env:GIT_CONFIG_COUNT="1"; $env:GIT_CONFIG_KEY_0="core.longpaths"; $env:GIT_CONFIG_VALUE_0="true"`.
  CI runners have cmake preinstalled (and Windows images ship
  `core.longpaths` enabled); ci.yml still primes the kernel build with
  one retry as a belt.
- **Playwright** (stage 5): browsers install per machine —
  `cd web && npx playwright install chromium` (done here 2026-08-19).
  The app WRITES the served project's files: never point `cicada serve`
  or the smoke at the repo's `examples/` for experiments — copy them to a
  scratch dir first (`playwright.config.ts` does this itself). Node 22
  has a global `WebSocket`, handy for protocol probes from a script.

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
  only float-comparison path in geometry code. Exact float `==` is
  sanctioned in hash/determinism tests and in stdlib tests whose node
  contract is exact IEEE arithmetic (pure maths; ledger revision
  2026-08-12); geometry tests always use tolerance-aware asserts.
- **The cache never lives in the project folder** (project dirs are
  Dropbox-synced): user cache directory only; `.cicada-cache/` is an
  opt-in override and stays gitignored.
- **Verification is agent-operated, headless-first** (skill:
  `verify-change`). The human reviews evidence; the human is never the
  feedback loop.
- Tests are deterministic — no sleeps, no wall-clock, no network. No
  `#[ignore]` without a linked issue. One sanctioned exception: property
  tests draw fresh random inputs each run by design; a found failure
  persists as a committed `proptest-regressions/` file, which IS the
  deterministic regression test.

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
| `.claude/skills/dialect-change` | Any change to `cicada-lang` — grammar, writer, checker, diagnostics |
| `.claude/skills/perf-check` | Benchmarks against the doc-15 targets, and how to record numbers |
| `.claude/skills/protocol-change` | Any change to the server↔client protocol (messages, view-model, frames, routes) — server, client mirror, and tests together |
