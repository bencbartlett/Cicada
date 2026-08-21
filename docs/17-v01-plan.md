# v0.1 plan — the work order after the spike

The spike (doc 15) passed its gate on 2026-08-19. This document is the
v0.1 counterpart: the items, their order, the work packages each splits
into, what "done" means for each, and the process rules that keep the
work resumable across sessions. The order and the design choices were
decided with Ben on 2026-08-19 (DECISIONS.md row of that date); the
rows it cites are binding, this file is the schedule.

## Scope

Seven items from doc 05 §v0.1, in this order, plus a catalog track that
runs in parallel from day 1:

| # | Item | Track | Size | Status |
|---|---|---|---|---|
| 0 | Fold `corpus/` into `examples/wall/` + `tools/`; this plan | foreground | hours | **done** 2026-08-20 (every nightly step passes locally from the new paths; the first Nightly run itself is pending the push) |
| 1 | Undo/redo — snapshot op log + atomic `batch`/`apply_text` path; riders `#off`, Backspace-no-delete | foreground (server/web) | days | **done** 2026-08-20 (merged) |
| 2 | Git panel slice 1 — status strip, per-node change markers, commit, revert-to-HEAD | foreground (server/web) | ~1 week | **done** 2026-08-20 (wt/git-panel): `GET /api/git/status` + writer-gated `POST /api/git/commit` / `POST /api/git/revert` (docs/13), the web chip / Git tab / canvas badges / `Ctrl+S` commit dialog (docs/16); measured, debug builds: revert POST → barrier snapshot ≤ 35 ms (route test), Revert click → reloaded text in the store 69–81 ms across runs (Playwright) |
| P | OCCT probe — prebuilt 7.8.x build/link on win/mac/linux, determinism, timings, license, CI shape | parallel worktree | hours, cap 1 day | **done** 2026-08-20 — GREEN on win-64 with one rename patch, byte-deterministic, ~3 ms/boolean; Linux/macOS measured by item 3 WP-A's CI job; memo `docs/probes/occt-2026-08.md` |
| 3 | OCCT-backed Solid — the `Solid` kind, primitives/extrude/loft/revolve/sweep, booleans, `tessellate`, STEP; `mesh_*` renames in the same commit | main geometry track from week 3 | weeks | **WP-A done** 2026-08-20, review fixes applied the same day (fork `bencbartlett/opencascade-rs@960a8bc`, `occt` feature + seam in `cicada-geom`, `tools/fetch_occt.py`, CI jobs `occt (ubuntu)` per PR and `occt (<os>)` nightly — the non-Windows jobs await their first run); **WP-B done** 2026-08-20 (`wt/solid`: the `Solid` kind end to end, the sharing model — op-local linear handles, no kernel lock — the value-level `cicada_geom::solid`, display through the session's `SolidCache`, the typed Python refusal, the store variant with a committed pre-change pack; the handle cache measured and NOT built); **WP-B second review closed** 2026-08-21 (`wt/solid`: the moved-sphere stale-pcurve root cause fixed in `transform`, display draws unclosed meshes and says so, display tiered + off the session lock on the worker pool, blobs keyed by the display mesh's hash, typed `NotOneSolid`, cached refusals, the 02-solids display cone at 5.2 ms p50 — §Item 3 has the table); **WP-C + WP-D done** 2026-08-20 (`wt/solid`: `occt` ON by default + every CI job fetches the prebuilt; the node-set glue in cicada-geom; `box`/`sphere`/`cylinder`/`cone`/`extrude`/`extrude_to_point`/`loft`/`revolve`/`sweep`/`pipe`/`solid_union`/`solid_difference`/`solid_intersection`/`volume`/`bounding_box`/`deconstruct_solid`/`section`/`tessellate`/`export_step`/`import_step`; the mesh tier as `mesh_*`, the wall's carve hash unchanged; `examples/07-simple-cad.cic` + its Playwright spec; `mirror` added 2026-08-21; numbers below — the cheap-cone slider on the OCCT example: the display cone PASSES since the second review (5.2 / 9.1 ms) while the COMMITTED 02-solids misses the 16 ms bar because its export `tessellate(deflection=0.01)` node sits in the cone (34 ms per tick — a one-line decision for Ben), and Esc inside ONE kernel call misses 250 ms: both named follow-ups) |
| 3b | Scheduler foundations — per-solve cancel handle, `volatile`, idle-class hypothetical solve — plus compute-on-release | parallel (sched/server) | ~1 week | **done** 2026-08-20 (`wt/sched`, eighteen commits after three review rounds: the engine half, then the web half — both sliders show the pending value + estimate from `preview_policy`, the release that writes nothing is `end_drag` and every announced drag's end is `drag_ended` — with a Playwright drag of the wall's `deboss`, an observer page watching, as its evidence) |
| 4 | Time transport — Cycle thin slice + orbit example; Clock via `volatile` | foreground | ~1 week | pending |
| 5 | Scrub caching — bounded-position sliders only, toggleable, buffer bar | foreground | 1–2 weeks | pending |
| 6 | WASM script host — load precompiled guests, epoch cancellation, `cicada-guest` SDK | last | weeks | pending |
| C | Catalog — one-node-per-file restructure, node-format conformance test, then the docs/08 S+1 list in tranches; `cicada mcp` | parallel worktrees, continuous | continuous | **C0 done** (2026-08-20); **C1 done** (2026-08-20: 48 nodes — lists, maths tail, sequences; the diagnostics name real nodes and a test keeps it so; `compact` satisfiable at check time; `examples/06-lists.cic`); **`cicada mcp` done** (2026-08-20: the four doc-11 read tools over stdio on `rmcp`); C2+ pending |

Out of v0.1 (unchanged from doc 05): fillets/chamfers and B-rep
maturity, the Blender bridge, fidget, the .gh importer, Tauri, the AI
layer (the `batch` path is its landing pad), the 2D sketcher.

## Process rules (how this stays resumable)

- **The repo is the memory.** This file carries status; AGENTS.md
  carries the one-paragraph "what is live"; DECISIONS.md carries the
  choices. A new session reads those three and loses nothing.
- **Work packages fit one subagent context.** The orchestrating session
  freezes a contract (types, ports, protocol shape), subagents build the
  packages in worktrees with private `CARGO_TARGET_DIR`s, the
  orchestrator merges. It holds summaries, never the work.
- **Every package ends in a commit.** Uncommitted work never spans a
  session boundary. Probes write their memo to a file as they go.
- **Review before merge**: the 5-lens + critic adversarial review with
  reproduce-or-refute verification (the pattern that shipped stage 6).
  The most valuable lens is "can a check report a false PASS".
- Commit conventions, doc-update rule, catalog regeneration, and the
  definition of done are AGENTS.md's; they apply unchanged.

## Item 0 — the corpus move (hours)

`corpus/` becomes `examples/wall/` (pipeline, scripts, inputs, golden
production references, README, the wall-only layout tools) and `tools/`
(the measurement harness, `normalize.py` and its tests — engine-wide).
Nightly, CI's per-PR `corpus-offline` job, docs, AGENTS.md, the skills
and `.gitignore` paths follow in the same commit. **Done when**: `git mv`
history preserved; the nightly corpus job runs from the new paths and
passes; `python -m unittest discover -s tools -p "test_*.py"` passes;
`examples/wall` opens in the app; no reference to `corpus/` remains
outside history.

Landed in 60656b5 + the review follow-up. The generated data's
provenance stamps (`layout.json`, the extraction / seed-recovery reports,
`plates_summary.json`) were regenerated through the blessed
extract → recover → extract path so they name the tools where they now
live — the offline suite asserts every stamp points at a file in the
repo, and that the frozen inputs are present and are the 1,200-part
production wall. The nightly job's every step passes locally from the new
paths (offline tests with and without the wall repo, the exporters,
`normalize.py all` → NOISE); the first Nightly run from the new layout
happens after the push — record its URL in `examples/wall/README.md`.

## Item 1 — undo/redo (days)

Design: DECISIONS.md rows 37 (revised 2026-08-19) and doc 13 §Undo.
- **WP-S (server)**: `OpLog` on the session (`VecDeque` of
  `{id, label, actor, before: (document, sidecar), timestamp}`, capped);
  every successful write pushes the pre-state `Session::handle` already
  clones for rollback; `undo`/`redo` intents restore and go through the
  normal persist + delta; `reload_from_disk` clears the log (barrier);
  effectful runs are not ops; lease-gated like every write.
- **WP-B (batch)**: the atomic multi-file edit for agents — shipped as
  the `apply_text` intent and `POST /api/edit/apply_text` (the name
  `batch` went to the canvas's gesture list, below):
  `{base_text_hash, files: [{path, text}], label, actor}` → refuse on
  stale base (`stale_base`, returns the current hash) or parse failure
  (diagnostics); else apply under the lock: write every file temp +
  rename, one op, one delta. `GET /api/edit/text` is the base to read.
  Multi-node canvas gestures (multi-move, multi-delete, reconnect) use
  the `batch {ops, label}` intent — a list of write gestures applied in
  order under the lock, all or nothing, one op, one delta.
- **WP-P (protocol + web)**: additive `history {can_undo, can_redo,
  undo_label, redo_label}` on Delta/Snapshot/`/debug/state`;
  `Ctrl+Z`/`Ctrl+Shift+Z`; toolbar buttons; Backspace removed from the
  delete keys; `#off` toggle on `D` (`writer::toggle_disable`,
  ports-intact ghost).
- **Done when**: session tests drive place → wire → set-param → delete →
  undo ×4 → redo ×4 and the text is byte-identical at every step; a
  `batch` with a stale base is refused and leaves disk + memory
  untouched (test asserts the file hash); a `batch` that fails mid-way
  leaves no partial file (fault-injection test); undo after a reload
  barrier is refused with the documented reason; Playwright: delete a
  node, `Ctrl+Z`, the node and its wires are back and the solved state
  is a cache hit (`/debug/state` shows `cached` for its outputs).
- **Shipped** (2026-08-20, `wt/undo`): all of the above plus the review
  riders — `Ctrl+Z` reaches the map from a focused slider (a range input
  is a control, not text entry), the one-op canvas gestures
  (multi-drag, target-anchor rewire) have Playwright coverage, a held
  `Ctrl+Z` past an empty side does not flood the notices — and `#off`:
  `writer::toggle_disable`, the `toggle_disable` intent, the ports-intact
  ghost (a `Line::Disabled` carries its parse), `D` / the node menu / the
  inspector action, `web/e2e/disable.spec.ts`.

## Item 2 — git panel slice 1 (~1 week)

Design: DECISIONS.md git row; doc 10 §Git integration; doc 13 routes.
- `git.rs` shells out to the git binary (`rev-parse`, `status
  --porcelain=v2 --no-optional-locks`, `diff -U0 HEAD`, `show
  HEAD:path`); typed states: not-a-repo, git-not-found, unborn,
  detached, `index.lock`. Project dir ≠ repo root is normal
  (`examples/wall`).
- Markers: hunks → binding lines → node names (the parser's spans);
  sidecar changes excluded from markers. `GET /api/git/status`;
  writer-gated `POST /api/git/commit` (message on stdin; scope = this
  pipeline's `.cic` + sidecar + `scripts/`) and `POST /api/git/revert`
  (to HEAD, through the existing barrier path).
- UI: TopBar branch/dirty chip; Git inspector tab with per-node markers;
  `Ctrl+S` commit dialog. HTTP-only — no WS message.
- **Done when**: markers equal `git diff` by construction (test: for a
  fixture diff, the set of marked nodes == bindings on changed lines); a
  status refresh never re-triggers itself on Linux (the 82df8a3 loop
  shape, CI test); commit from the app produces a commit `git log` shows
  with exactly the scoped paths; revert reaches the canvas within the
  measured barrier budget.
- Deferred inside the item: the canvas graph-diff overlay and `git log
  -L` per-node history — after the markers have weeks of use.
- **Shipped 2026-08-20** (wt/git-panel; docs/13 §HTTP surface rows,
  docs/16 Git panel bullet, doc 10 §Git integration): every "done when"
  holds — markers vs `git diff` (`git_routes.rs`
  `markers_are_exactly_the_bindings_on_the_diffs_changed_lines`), the
  no-self-retrigger status (`--no-optional-locks`, `.git/index`
  byte-identical across refreshes), commit = exactly the scope (route
  test + Playwright `git log`), revert → barrier ≤ 35 ms server-side and
  click → canvas 69–81 ms in the browser (debug builds). The scope carries
  `in_head` per file — the revert rule is the server's, published, not
  re-derived by the client (review finding: porcelain `AD` is `deleted`
  with no HEAD version).

## Item P — the OCCT probe (hours, capped at one day)

A throwaway worktree with a private target dir; the deliverable is
`docs/probes/occt-2026-08.md` (written incrementally) plus the ledger
rows it forces. Questions, in order:
1. **Prebuilt build/link**: `opencascade-sys` with `--no-default-features`
   and `DEP_OCCT_ROOT` pointing at conda-forge `occt 7.8.1` (win-64 first
   — Ben's machine — then linux CI; macOS via CI). Report the Windows
   verdict within the first two hours. If MSVC fails, read the open
   upstream PRs (#230, #216) as starting points; keep only reviewed,
   minimal patches in an own fork.
2. **Determinism**: build the same box / extruded rectangle / boolean
   twice in two processes, serialize (BinTools, no triangulation),
   compare bytes; repeat across OSes in CI. This decides whether `Solid`
   hashes its canonical bytes or a geometric summary.
3. **Timings**: box, extrude, boolean, tessellate at 1 / 100 / 1,000
   parts, release build; cold CI minutes per OS with prebuilt OCCT.
4. **Policy**: LGPL-2.1 + OCCT exception row; `deny.toml` license and
   git-source exceptions scoped to the OCCT crates; the CI shape
   (`occt` cargo feature in `cicada-geom`, dedicated job).
**Done when** the memo answers all four with numbers and the ledger rows
are drafted; a red verdict is a valid outcome and comes back to Ben with
the fallback options.

## Item 3 — OCCT-backed Solid (weeks, from week 3 on a green probe)

Design: DECISIONS.md rows 16 and 42 (revised 2026-08-19), doc 03, doc 08
§7–8. B-rep is the default working mode; the wall stays on the mesh tier.
- **WP-A seam + build + CI** — **done 2026-08-20**, review fixes the same
  day (docs/03 §The OCCT seam as built): feature-gated `occt` in
  `cicada-geom` over Ben's fork `github.com/bencbartlett/opencascade-rs`
  branch `cicada` @ `960a8bcb9e3dbf1916778dabb8288c1bda4c6d91` (upstream
  `d114250` + the MSVC handle aliases, the honest `BinTools`/`BRepTools`
  writers, in-memory serialization, the total exception boundary
  (`Standard_Failure` / `std::exception` / `catch (...)` → `Err`, with a
  per-clause self-test), the `cicada` glue, no OCCT source submodule — the
  git dependency costs 6 MB in `~/.cargo/git`, not 161 MB + 421 MB —
  and `LGPL-2.1-only` manifests); `tools/fetch_occt.py` +
  `tools/fetch_occt_manifest.json` (conda-forge 7.8.1 build 103 for
  win-64/linux-64/osx-64/osx-arm64 + the run-time closure, sha256-pinned,
  user cache dir, a warm path that re-verifies every shared library's
  presence and size, static closure check, typed network failures with a
  timeout, `--print-env`); deny rows for `opencascade-sys` + the fork
  source; CI jobs `occt (ubuntu)` (ci.yml, per PR) and `occt (<os>)`
  (nightly.yml, three OSes). Seam surface: `occt::Solid` with `box_at`,
  `extrude_polygon`, `difference`, `tessellate → Watertight<Mesh>`,
  `canonical_bytes`/`from_canonical_bytes`, every kernel call under one
  process-wide kernel lock; 21 tests incl. golden blake3 hashes of the
  canonical bytes, the weld refusals, the two-thread related-solids
  test and the per-clause boundary test. Measured on Windows; the
  Linux/macOS jobs are unverified until they run (the Linux job exports
  `LD_LIBRARY_PATH=<prefix>/lib` for the whole cargo step, which also
  shadows conda's libstdc++/libz/libexpat for the toolchain — probably
  fine, newer libstdc++ is backward compatible; switch to the macOS
  job's rpath approach if the first run says otherwise). Left for WP-B:
  the `Solid` value kind over `occt::Solid`, and the **sharing model**
  the kernel lock stands in for — OCCT results share `TShape`s with
  their inputs, so DISTINCT `Solid`s race in C++ when one is tessellated
  while another is serialized; choose deep copies at the seam
  (`BRepBuilderAPI_Copy`) or doc 12's kernel worker, then retire the
  lock; per-OS goldens until the nightly shows agreement (the policy is
  in DECISIONS.md rows 16 and 42, revised 2026-08-20 at the merge of
  WP-A from the probe memo's §4d drafts);
  for WP-C: the own-built OCCT with FreeType/FreeImage off (the fetch
  table takes a second source), `Message_Printer` redirection for the
  STEP writer, the patches for the static path (PR #216's system libs,
  cmake-rs `/O2`) if it is ever taken.
- **WP-B the `Solid` kind** — **done 2026-08-20** (`wt/solid`, four
  commits; docs/03 §The sharing model, §No handle cache, §Display
  tessellation; docs/12 §Display cache; docs/14 §Value and geometry
  representations; DECISIONS.md row 16 revised in place):
  `core::Solid` = the canonical bytes in an `Arc<[u8]>` and nothing else
  (header-checked; KindTag 20 over the length-prefixed bytes; goldens for
  the probe box and prism through the new path equal WP-A's raw blake3
  goldens, and their `HashedValue` hashes are blessed); `Solid` in
  `TRANSFORMABLE_KINDS` / `GEOMETRY_KINDS` (the checker admits it into
  `T` ports and display sinks; `Similarity::apply` on a Solid is a red
  node with `SOLID_TRANSFORM_DEFERRED` until WP-C); `StoredValue::Solid`
  appended with `LOG_FORMAT` 2 → 3 and `tests/fixtures/pack-24d558b.bin`
  (a pre-Solid engine's pack, every blob loaded under its golden hash);
  `ScriptError::Unmarshallable { kind: "Solid", .. }` at the Python
  boundary, bare or in a list. **The sharing model**: op-local, LINEAR
  handles — every `occt::Handle` owns its `TShape` graph (a `BinTools`
  read is the deep copy), kernel operations consume their handles
  (booleans raise input tolerances in place; the mesher keeps a finer
  triangulation), results go back to bytes; the process-wide kernel lock
  is retired (no OCCT global is written by the glue's calls;
  `Interface_Static` for STEP is WP-C's to lock); proved by an 8-thread
  rayon test over 13 related values against single-threaded goldens.
  **No handle cache**, on numbers (release, 1,000 parts): a block's
  re-read is 41 µs against a 3.1 ms boolean (1.3 %), the whole box →
  extrude → difference → tessellate chain pays 5 % for re-reading at
  every step (4.54 vs 4.30 ms per part), and on the rayon pool the same
  chain runs 6.4× the serial rate (710 µs wall per part) — the lock would
  have held it at 1×; `examples/solid_bench.rs` is the table. **Display**:
  `cicada_geom::solid` (value level, same signatures in every build —
  `GeomError::KernelUnavailable` without the feature, never a mesh-tier
  fallback); the session's `display::SolidCache` keyed by value hash +
  deflection, bounded (256 MiB), LRU, counters in `/debug/state` →
  `display_cache`; `Deflection::display` = `max(0.02 mm / unit, tol)` and
  `max(0.1 rad, tol_angle)`; the frames stay mesh frames keyed by the
  Solid's hash; `DisplayStats.solids` / `.errors` additive; the summary is
  "Solid, N faces, bbox"; web hue `--kind-solid`. `cargo test --workspace`
  passed with and without `--features occt` at the time (892 / 868
  tests; since WP-C's default flip only cicada-geom has a kernel-free
  world and the server's tests assert the kernel is present — second
  review closure below), and a local flip of `default = ["occt"]`
  checked the whole workspace.
  **Consequence WP-C acts on**: once `box` / `extrude` / … are
  OCCT-backed stdlib nodes the product build needs the kernel, so WP-C
  flips `occt` to a default feature of `cicada-geom` (and revises the
  DECISIONS.md row-16 sentence "the `occt` cargo feature is OFF by
  default and default builds never touch OCCT" in the same commit), and
  every CI job runs `tools/fetch_occt.py` first; until then the
  `occt (ubuntu)` job should add `cargo test --workspace --features occt`
  so the server's Solid display tests run in the kernel world in CI
  (today they run there only on this machine). **Review fixes (same
  day)**: a compile-time `!Sync` assertion on `occt::Handle` (the belt
  behind `canonical_bytes(&self)`); the sharing model proved under the
  SCHEDULER too — `cicada-server/tests/solid_scheduler.rs`, closures
  over `cicada_geom::solid` in a `SolveGraph` with a 48-element `each()`
  fan-out on 8 threads vs 1, hashes equal (the node-level `.cic` form
  follows WP-C's nodes); the review's UNVERIFIED determinism question
  answered with evidence — `canonical_bytes_do_not_depend_on_heap_state_or_thread`
  (a seven-cut, 58-face solid under allocator churn and on 8 threads,
  byte-identical; WP-C reruns the shape on loft/revolve before blessing
  goldens); `Deflection::new` floors at the kernel's own
  (`MIN_LINEAR_DEFLECTION` 1e-7, `MIN_ANGULAR_DEFLECTION` 1e-12 — the
  mesher throws below them; WP-C's `tessellate` node inherits the "Red
  when"); `SolidCache` touches and evictions are O(log n) (a recency
  index, asserted at 2,000 entries without the kernel) and an entry
  larger than the whole budget is served, counted `oversized`, never
  kept; `/debug/state` → `display_cache` and the omitted-when-empty
  `stats.solids`/`stats.errors` are asserted by the session test and
  listed in docs/13; the five transform nodes' `# Panics` (and the
  catalog's "Red when") name the Solid refusal, with a node-level test;
  the Python refusal names `tessellate` as ARRIVING with WP-C, not as a
  node to use. Not changed, on purpose: the handle cache stays unbuilt
  (item 4 of the package — the measured ≤ 5 % gain and the fork-glue
  conditions in docs/03 §No handle cache; an orchestrator-level
  acceptance, recorded here, and re-measured by the second review: the
  reader nodes on the 300-hole bar are 1.6 % of its boolean), and
  display tessellation still runs under the session lock (§Follow-ups;
  closed below). **Second review closure (2026-08-21, after WP-C/WP-D;
  findings 1–8 of the adversarial review):** (1) *a valid Solid could
  vanish from the viewport* — a sphere moved by the kernel transform,
  minus a cylinder through both its poles, meshed with 159 T-junctions
  and the display path refused any unclosed mesh; root cause found and
  fixed at its source: `BRepBuilderAPI_Transform(Copy = true)` leaves a
  pcurve on the SOURCE surface on the degenerate pole edges (the moved
  sphere serialized with two spherical surfaces), the cut carried it and
  the mesher discretized the intersection edge differently per face —
  `transform` now drops pcurves on surfaces no face uses (docs/03 §The
  node-set glue), after which the moved sphere serializes to its
  in-place twin's size and the cut meshes closed with the twin's
  triangle count; AND the policy is split — `tessellate` (the node)
  requires closure, `tessellate_display` reports it, display draws the
  welded mesh either way with `watertight: false` / `unclosed: N` in the
  summary and `stats.warnings` in `/debug/state`; `solid::is_valid`
  (`BRepCheck_Analyzer`) exposed for diagnosis — it called both solids
  valid. (2) *display under the session lock at a fixed 0.02 mm* — the
  display edge now runs on the solve loop's worker pool BEFORE the
  session lock (`Scheduler::map_parallel` over the generation's distinct
  solids; the broadcast only hits), at two TIERS (preview generations
  0.1 mm / 0.3 rad, structural 0.02 mm / 0.1 rad, the release redrawing
  a preview-tier value, tiers recorded per output, a joining client
  restreamed at the tier on screen) with a relative term (1/1000 of the
  solid's largest extent — OCCT's viewer convention); numbers below.
  (3) *blob frames keyed by the Solid's hash while their content
  followed the deflection* — the blob key is now the display mesh's own
  value hash (content-addressed like a `Mesh`; two tiers → two blobs,
  asserted; docs/13). (4) the Python refusal reworded to the present and
  its test now asserts the named node EXISTS (a cli test against the
  registry) and rejects milestone wording. (5) kernel refusals are
  typed: `GeomError::NotOneSolid { operation, found }` ("cut left 2
  solids — a Solid is one body; …"), glue `name: ` prefixes stripped,
  operation labels plain words, and `diagnostic_vocabulary.rs` rejects
  `cicada_` / "shape type" in every seam literal. (6) refusals are
  cached as negative entries (`display_cache.refusals`); docs/12 no
  longer claims a refusal is cheap. (7) the seam goldens route through
  `platform_golden(win64)` like the stdlib's; the three-OS verdict is
  PENDING the first CI run of this branch. (8) the dead kernel-free arms
  in cicada-server's tests are gone (the crate links the kernel
  unconditionally; each test asserts the kernel is present) and the
  Linux job now RUNS `cargo test -p cicada-geom --no-default-features`
  (78 tests) instead of checking it; the ledger's "Scheduler internals"
  row says a new `StoredValue` variant bumps `LOG_FORMAT` (revised in
  place, dated). **Measured (release, dev machine, 2026-08-21):**

  | case | before (review) | after |
  |---|---|---|
  | 02-solids `size` slider, as committed (its `tessellate(deflection=0.01)` export node in the cone) | 42 / 135 ms p50/p95 server | 39.9 / 54.9 ms — the `shell` node alone is 34 ms per tick; display is no longer the cost |
  | 02-solids `size` slider, the display part only (`shell`/`dump` removed from a scratch copy) | — | **5.2 / 9.1 ms server, 5.5 / 9.4 ms client; 300 of 300 ticks at 60 Hz — PASS against 16 / 33** |
  | 300-hole bar, structural load (303 distinct solids) | 5,263 ms generation for a 505 ms solve; `holed` 152,412 triangles | 1,195 ms for a 341 ms solve; `holed` 58,812 triangles (4 mm relative deflection), 300 drills in parallel |
  | 300-hole bar, a preview tick (`pitch` drag) | — | 1,100 ms per tick for a 330 ms solve: `holed` at the preview tier (51,612 triangles) is ~0.7 s of mesher on two 6,300-vertex planar faces — one solid, one kernel call, the remaining cost |
  | wall carve (`--node carved`, fresh cache) | `507f582b…` 3.80 s | `507f582b…` 3.75 s — unchanged |

  The bar's remaining display cost is a single `BRepMesh` call that no
  tier makes cheap; docs/12's costed, cancellable display edge is where
  it goes (§Follow-ups). Left for WP-C from WP-B:
  the kernel-backed transforms (`Similarity::apply`'s Solid arm and the
  five transform nodes' `# Panics`, which then lose the Solid sentence),
  a `tessellate` node over `solid::tessellate` (its "Red when" includes
  the deflection floor), the STEP nodes' `Interface_Static` lock, and —
  if a handle cache is ever wanted — `SetNonDestructive(true)` booleans
  plus clean-before-mesh in the fork's glue (docs/03 records the
  conditions). One ledger wording for Ben: the "Scheduler internals" row
  says `LOG_FORMAT` bumps with a new `LogRecord` variant; WP-B bumped it
  (2 → 3) for a new `StoredValue` (blob codec) variant on the same
  reasoning — an older engine tombstones and recomputes every memo whose
  Solid blob it cannot decode — and docs/12 says so; the row's sentence
  could read "any new record OR value-codec variant".
- **WP-C nodes — done 2026-08-20** (`wt/solid`, eight commits on top of
  WP-B; docs/03 §The node-set glue, docs/08 §7–§9/§11 rows as shipped,
  DECISIONS.md row 16 revised in place). **The default flip**: `occt` is
  a default feature of `cicada-geom` (the product's nodes need the
  kernel); every cargo command runs under `tools/fetch_occt.py`'s env
  (AGENTS.md palette) and every CI job that builds fetches the prebuilt
  first — the dedicated `occt` jobs folded into the standard per-PR and
  nightly matrices, macOS on an rpath, the Linux job keeping the
  kernel-free `--no-default-features` build compiling. **The glue lives
  in cicada-geom**, not the fork: `src/occt/glue.hxx` + `glue.rs`, a
  second cxx bridge compiled by the crate's own `build.rs` with cxx-build
  against the same `DEP_OCCT_ROOT` (the fork is pinned by rev and every
  patch to it is a release of a second repository; these are Cicada's
  own kernel calls and change with the node set) — same rules: every
  function `Result`, the trycatch hook repeated per translation unit, no
  OCCT global written except by the STEP translators, which run under
  `occt::STEP_LOCK`. **The nodes** (all three tests each; every Solid
  node's table test runs in BOTH worlds through `with_kernel` — without
  the kernel the node must be red with the typed refusal): `box`,
  `sphere`, `cylinder`, `cone` (BRepPrimAPI in a plane's frame; a
  world-frame `box` is byte-identical to the seam's `box_at`), `extrude`
  (exact edges for every curve kind: a circle is an exact cylinder, so
  no `segments`; a rectangle's prism equals `extrude_polygon`'s bytes),
  `extrude_to_point`, `loft` (`profiles: [Closed<Curve>]`, `ruled = true`
  — GH "Straight", the wall's and the mesh tier's behaviour; `false` =
  GH "Normal"; sections made compatible first), `revolve` (a `Line`
  curve axis in the profile's plane; touch but never cross; an angle
  domain whose start and sign are one rigid kernel transform), `sweep`
  (Sweep1: MakePipeShell, corrected Frenet, mitred corners) and `pipe`,
  `solid_union` (n-ary, ONE general-fuse pass), `solid_difference` (one
  solid, a cutter list — the `mesh_difference` shape), `solid_intersection`,
  every boolean followed by `UnifySameDomain` (two fused cubes are a
  six-face box) and every result required to be ONE body (disjoint
  unions, splitting/emptying cuts and empty intersections are red — the
  B-rep tier has no empty solid), `volume`, `bounding_box` (exact on
  points/curves/meshes, kernel bounds for solids, in any plane),
  `deconstruct_solid` (edges + vertices + `face_count` — no `Surface`
  kind yet, and the port is named so the future `faces` port does not
  collide), `section` (one closed curve per loop; a circular loop is an
  exact `Circle`), `tessellate` (weld + watertight + Manifold
  acceptance; the deflection floor inherited from WP-B as its "Red
  when"), `export_step` (effectful; AP214 in the document unit; the
  header fixed AND the products renumbered in file order — OCCT numbers
  them from a process-wide counter, measured: a second export in one
  process wrote 'parts 11' — so the same solids give the same bytes,
  proved by a write-twice test) and `import_step` (the first shipped
  `volatile` node: a file on disk is external state; the registry test
  now holds a sanctioned list of one; its example reads a committed
  fixture, `crates/cicada-stdlib/fixtures/block.step`). **The
  transforms**: `Similarity::apply`'s Solid arm goes through the kernel
  (`solid::transform`), so `move` / `rotate` / `scale` / `orient` /
  `linear_array` accept a `Solid` as `T` with no arm of their own, and
  `mirror(geometry: T, plane: Plane = xy_plane)` — docs/08 §10 tier 1,
  added 2026-08-21 — is the plane reflection (`Similarity::reflection`;
  a mirrored `Solid` is reversed by the kernel, its volume positive, its
  bounds on the far side: asserted at the node in both worlds). **The rename**:
  `mesh_box` / `mesh_sphere` / `mesh_extrude` / `mesh_loft` (Mesh &
  field; gh Mesh Box / Mesh Sphere, and honestly Extrude / Loft for the
  two GH reaches only via Extrude/Loft + Mesh Brep), outputs
  byte-identical — the mesh goldens did not move and the wall's carve is
  still `507f582b5fe4b575cacc71e34346db2f9ab9c25391ce74ff8f9550dc877fa8a2`
  (release, fresh cache: 3.7 s wall cold on the dev machine, 44.6 s CPU
  over 22 workers) — and every consumer migrated (wall.cic, 03/04/06,
  the mesh nodes' examples, three cicada-server test fixtures whose
  stories are mesh stories, the smoke's binding count).
  **Goldens**: `HashedValue` hashes of canonical bytes for
  transcendental-free inputs only (boxes, rectangle prisms, a polyline
  loft, box booleans, a box's section loop and vertex list), blessed via
  run-once on win-64 AFTER `node_set_bytes_do_not_depend_on_heap_state_or_thread`
  (the churn + 8-thread shape WP-B's review asked for) passed on that
  corpus plus the revolve and sweep; they reach the tests through
  `platform_golden`, the one place a second OS adds its arm (row 42);
  curved primitives, revolve, sweep and pipe hash nothing — run-to-run
  identity + analytic volumes (every primitive, sweep and boolean has its
  volume formula as oracle, read by the integrator AND the
  tessellation). `cargo deny check licenses bans sources` green (the
  advisory DB needs network). 507 stdlib tests, 123 in cicada-geom.
- **WP-D consumer — done 2026-08-20**: `examples/07-simple-cad.cic` —
  the bracket (a `box` plate, a `cylinder` boss, an `extrude`d gusset
  4 mm inside the boss so no tangent contact reaches a boolean, one
  `solid_union`, a through-bore and a `linear_array` of mounting holes
  removed by one `solid_difference`, `volume`, a `section` through the
  plate — the holes come back as exact circles — `bounding_box`,
  `export_step` on `--node step`; eight sliders; 55 nodes, 21 ms wall
  cold on a debug build). `web/e2e/simple_cad.spec.ts` opens it in the
  app: every binding green, the bracket's display stats count one Solid
  with triangles and the bounds [0,0,0]–[80,50,28], `bounding_box`
  agreeing, the Solid cache holding entries, frames in the viewport.
  `examples/02-solids.cic` is now the B-rep example (box, sphere,
  `solid_difference`, `volume`, one `tessellate` for the OBJ exporter).
- **Measured (release, fresh caches, 2026-08-20, the dev machine — 22
  rayon workers), against docs/15 §Stage-6 results:**
  - *Wall cold carve*: 3.7 s wall (gate < 10 s; stage 6: 6.5 s) — the
    mesh tier is untouched; faster only because the machine is.
  - *Cheap-cone slider, `02-solids` `size`* (now an OCCT box → sphere →
    `solid_difference` → `volume` → `tessellate` cone; `tools/measure/slider_loop.mjs`,
    300 sends in 5 s at 60 Hz): 122 preview generations (23.8/s, 178
    superseded), server queued+elapsed **p50 42.1 ms / p95 135.4 ms**
    (elapsed p50 25.7, queued p50 10.4), client round-trip **p50 32.6 ms
    / p95 98.0 ms**, longest silence 49 ms, 0 errors — **FAILS the 16/33
    ms bar** (stage 6, mesh tier: 0.5 / 1.4 ms). The cause, isolated:
    the kernel work is within budget — per tick `box` 0.2 ms, the cut
    2.3 ms, `volume` 1.4 ms (reconstruction from bytes is in those
    numbers; no lock exists) — and with the explicit `tessellate` node
    removed the generation still takes 25 ms, because the carved Solid's
    DISPLAY tessellation at the display deflection (0.02 mm / 0.1 rad on
    a 0.75-radius spherical cut: 4,266 triangles) costs **23 ms** in
    `BRepMesh_IncrementalMesh` (the sphere alone at that deflection: 37
    ms / 8,000 triangles; the same solid at 0.1 mm / 0.3 rad: 3 ms /
    532 triangles), and it runs inside the generation on every new value
    (a new Solid is a cache miss by construction). Generations longer
    than the 16.7 ms tick then queue (queued p50 10–24 ms). Follow-up
    named below: a preview-time display deflection (coarse while
    dragging, the 0.02 mm mesh on release), and the display edge moved
    off the session lock and onto the solve loop's workers (docs/12).
  - *Esc during a long boolean* (`tools/measure/esc.mjs`, 20 trials,
    0 missed): a **4,000-element `each()` chain** of `solid_difference`
    (29 s CPU, 1.5 s wall) — time-to-idle client **p50 6.2 ms / p95 31
    ms**, server cancel→idle p50 4.3 / p95 6.3 ms: **PASS** (the cancel
    lands between chunks; the sharing model's lock-free pool is what
    makes the chain 20× parallel). **ONE 1,000-tool `solid_difference`**
    (a 3,000 mm bar minus 1,000 cylinders, one kernel call of 1.4 s) —
    time-to-idle client **p50 1,689 ms / p95 1,790 ms**: **FAILS 250
    ms**. A single kernel call is not interruptible from Rust — the
    token is checked between nodes and chunks, never inside OCCT — so
    Esc waits for the boolean to finish. The doc-12 kernel worker (ops
    the cost model predicts above ~1 s routed to a cancellable
    subprocess, killed on Esc) is the named follow-up; until it lands a
    long B-rep boolean holds the session for its own duration.
- **Done when** (as written, with the verdicts): the wall's cold carve
  re-measured unchanged ✓; the 02-solids slider re-measured — NOT
  unchanged: it now drives the OCCT cone and misses the bar for the
  reason above (recorded, follow-up named); golden hashes for
  transcendental-free solids: committed on win-64 through
  `platform_golden`, the three-OS verdict comes with the first CI run
  of this branch (row 42's per-OS policy applies if they disagree); Esc
  during a long boolean measured and written down, the kernel worker
  named ✓; `cargo deny` green ✓; CATALOG.md regenerated ✓.

## Item 3b — scheduler foundations + compute-on-release (~1 week, parallel)

- A per-solve cancel handle through `NodeFn` (not one session-global
  switch); a `volatile` `NodeDecl` flag beside `effectful` with executor
  gates at node and element granularity (Clock's "uncached by design");
  an idle-class hypothetical solve entry that paints nothing and is
  excluded from `wait_idle`.
- **Compute-on-release** (row 39): cones predicted ≥ ~1 s from the
  persisted cost samples show the pending value and an estimate during
  the drag and solve once on release. The wall's `deboss` is the test.
- **Done when**: virtual-time scheduler tests cover both gates; the
  deboss drag produces exactly one generation per release in
  `slider_loop.mjs`; the cheap-cone numbers (p50 0.5 ms) are unchanged.
- **Shipped (engine half, 2026-08-20, `wt/sched`)**: (1) the cancel
  handle — `NodeFn` takes the generation's `NodeCtx` (its
  `CancelToken`, now with `on_cancel` hooks); the script bridge mints
  one kill switch per call hooked to the calling token, so explicit
  runs, the interactive loop and idle solves are isolated by
  construction (the old session-global switch let an Esc kill an
  export's Python call); (2) `#[node(volatile)]` → `NodeSpec`/`NodeDecl`
  with node- and element-level memo gates, downstream keyed as usual on
  the fresh hash (doc 12 §Volatile nodes), `volatile`+`effectful`
  refused by the macro (trybuild), a cfg(test) fixture node,
  `"volatile"` in catalog.json; (3) `SolveLoop::run_idle` +
  `Session::solve_hypothetical` — idle class, pre-empted by any real
  submission or Esc, invisible to `wait_idle`, paints nothing, one
  `hypothetical` timing row; (4) compute-on-release — decided PER TICK
  from a hash-only dry run of the tick's keys against the memo (warm
  values stay live; a cold tick is withheld whatever the drag's first
  tick was), monotone within a drag, announced once per drag by the
  additive `preview_policy` message (doc 13 §Slider drags has the
  frozen shape and the drag-gap rule: a drag ends on a write attempt,
  an Esc, or a 300 ms pause, and the next one is announced again),
  `COMPUTE_ON_RELEASE_MS` = 1 s; memo entries record their
  computation's cost so the model is complete after a warm reopen.
  Review fixes 2026-08-20: the warm-first-tick and no-write-drag holes
  closed (both reproduced on the wall), the Python bridge wiring pinned
  by an end-to-end Esc test. Measured: 02-solids `size` warm p50/p95
  0.22/0.82 → 0.23/0.84 ms server, 0.42/1.4 → 0.43/1.42 ms client
  (within noise); wall `deboss` 301 ticks → 0 preview generations, 1
  policy message (estimate 3.9 s), 1 generation on release (3.7 s);
  after a warm reopen the estimate is 4.1 s from memo costs alone and
  the released value previews live at 0.2 ms. `slider_loop.mjs` gained
  the compute-on-release mode and `--expect`. Re-measured after the
  review fixes (per-tick prediction on every tick): 02-solids `size`
  warm 0.24/0.84 ms server, 0.45/1.47 ms client; wall `deboss` 301
  ticks → 301 withheld, 0 preview generations, exactly one policy for
  the stream (estimate 3.9 s), one 3.5 s generation on the step-snapped
  release. Second review round (2026-08-20): the "failure under a
  cancelled token is cancelled" rule narrowed to errors the node MARKS
  as cancellation (`NodeError::cancelled`; the bridge's verdict for a
  killed worker) so a genuine red coinciding with Esc stays red; the
  drag is ended in one place (the dispatcher's door, for every write
  intent but the tick) with undo / redo / a refused batch pinned as
  drag-enders; the inclusive 1 s bar, the `÷ min(threads, elements)`
  divisor and the volatile memo-READ gate each got the test whose
  absence a mutation had exposed; a `cached` status carrying its last
  compute's `elements`/`nanos` is a decision now (doc 13 §Solve
  streaming), not an open question; the store's format marker is
  written temp + rename and an empty (torn) one heals.
- **Shipped (web half, 2026-08-20, `wt/sched`)**: both sliders (canvas
  and params panel) show the pending value — thumb and number in the
  warn color — and a `pending · N s` chip (`~` when `rough`, the ETA's
  spelling) from `preview_policy`; the store holds ONE pending param
  (the server holds one drag), every arrival replaces it, and it
  clears on the delta of the release's `set_param` (any write ends the
  drag), on a refused write's `error` (a `lease` refusal excepted — it
  is decided before the drag-ending door), on a snapshot and on a
  disconnect; a release on the committed value writes nothing, so the
  widget clears it itself AND sends `end_drag`, and the server's
  `drag_ended` takes it down everywhere else (see the review round
  below); frames and statuses never clear it (a memo-warm tick paints
  live mid-drag by design); observers render the same from the
  broadcast; `cached` statuses read `cached · last 43.9 s`. Evidence:
  store unit tests for every transition and
  `web/e2e/compute_on_release.spec.ts` — a real pointer drag of the
  wall's `deboss` on the debug engine (2 threads): cold open 31 s,
  estimate 23 s, 9 ticks withheld, 0 computing previews, the hint up
  while held, one 29.6 s generation on release (1.2 min on the dev
  machine; it is the suite's slow spec by design and carries its own
  timeout). Re-measured at the final engine: 02-solids `size` warm
  0.24/0.89 and 0.24/0.72 ms server p50/p95 (0.44/1.52, 0.43/1.33 ms
  client); wall `deboss` 301 ticks → 301 withheld, 0 preview
  generations, 1 policy for the stream (estimate 3.6 s), one 3.6 s
  generation on release; warm reopen estimate 3.9 s, release 0.22 ms
  all cached.
- **Review round on the web half (2026-08-20)**: three holes, one
  contract revision (doc 13 §Slider drags, the "end of an announced
  drag is announced" paragraph; DECISIONS.md row 39). (1) The params
  panel's slider ended its drag only from the native `change` event,
  which Chrome does not fire when a drag returns to its start value —
  the badge stood for ever after such a drag (the canvas widget, which
  releases from pointer events, did not have the bug); both sliders
  now decide the no-write release on the pointer's / key's release.
  (2) Observers (and the non-dragging twin widget, and a writer whose
  release is refused by the lease) had no signal at all for a drag
  that ended without a write: the frozen contract's "the server never
  sends back-to-live" left them a stale badge and a value that was
  neither committed nor pending. Decided: the release that writes
  nothing is an intent — `end_drag` — and the server broadcasts
  `drag_ended` whenever an ANNOUNCED drag ends (after the delta /
  error / snapshot when there is one; alone for `end_drag`, Esc, the
  writer's departure, a lease handover), the gap rule's end excepted
  because a pause is not a release and the badge must not flicker off
  under a held pointer. (3) A re-grab inside the 300 ms gap after a
  no-write release continued the server's drag un-announced while the
  client had already cleared — `end_drag` ends the server's drag at
  the release, so the re-grab is a fresh drag, announced again. Tests:
  session tests for every end and both clients (`an_announced_drags_
  end_is_announced_to_every_client`, `the_writers_departure_and_a_
  lease_handover_end_the_drag_for_the_observers`), the protocol shapes,
  store tests for `endDrag` / `drag_ended`, and — new — jsdom +
  Testing Library component tests for BOTH sliders (the chip and the
  value while pending, the pending value following the thumb, the
  drag-return release, the write release, the keyboard path, the
  observer view; they fail on the review's mutations), and the e2e
  grew a second half: an observer page, a real pointer drag away and
  back to the committed value (28 more ticks withheld, both pages'
  badges down on release, the server's drag ended, nothing written,
  nothing solved), the re-grab announced again, the canvas twin's chip
  and value asserted from the DOM (1.7 min on the dev machine). The
  `cancel` on the canvas sender drops a queued tick on a no-write
  release (sent after the `end_drag` it would be a fresh drag on the
  committed value — cold while its own carve is still running). Still
  not relayed: the writer's later ticks to observers (they see the
  policy's first withheld value with the badge until the release —
  doc 13 contract item 4).

## Item 4 — time transport, Cycle thin slice (~1 week)

Design: DECISIONS.md time row; docs 08 §1, 12, 13 §Animation transport.
`cycle(period, frames, frame: Integer = 0)` with the transport-driven
port hidden on the canvas; per-session transport state with an
injectable clock; play/pause/seek/speed intents + `TransportView` in
Snapshot (additive — PROTOCOL_VERSION stays); the frame injected through
the preview path (never the file); Esc pauses; Space toggles;
`examples/06-orbit.cic` in the same slice. Clock follows via `volatile`.
**Done when**: the second pass of a loop is 100 % `cached` with an
identical NodeKey set; "previews never write the file" holds under
playback (test); a headless run yields frame 0.

## Item 5 — scrub caching (1–2 weeks)

Design: DECISIONS.md row 39 (revised 2026-08-19), doc 12 §Speculative
warming. Opt-in per slider (`scrub=True` in the text — never the
sidecar), offered only when the step-quantized range has a bounded
position count (threshold set here and recorded in the ledger), off by
default, toggleable; a warming worker generic over param × ordered
value list (Cycle's playhead-ahead warming reuses it); nearest-first;
preempted by any real intent; a per-slider byte cap; one buffer-bar
component for both slider widgets. **Done when**: after idle every
position of the test slider is a memo hit; a step-snapped
`slider_loop.mjs` sweep reports every generation `cached` and the
client round-trip is compared against a MEASURED warm-restream floor;
the first drag tick mid-warm stays under the Esc p95.

## Item 6 — WASM script host (weeks, last)

Design: DECISIONS.md user-code row; doc 08 script nodes; doc 12
cancellation. Load precompiled `scripts/*.wasm` with a ports-manifest
custom section; postcard buffers in linear memory; wasmtime epoch
interruption wired to the per-solve cancel handle (3b); memory cap; no
WASI in the first slice; `cicada-guest` SDK + manifest macro (a tenth
crate — ledger row revised then); feature-gated `wasm`; one committed
fixture guest + `examples/08`; marshalling measured against the Python
bar. Compile-on-save is deferred explicitly. **Done when**: a guest node
solves in the app, an infinite-loop guest is killed by Esc within the
budget, and a crashing guest reds one node and nothing else.

## Track C — the catalog (continuous, parallel)

Design: DECISIONS.md stdlib row (revised 2026-08-19), doc 14 §node file
format, doc 08.
- **C0 restructure**: one node per file under
  `crates/cicada-stdlib/src/<category>/`, categories = ribbon tabs; the
  conformance test (title line, description, every port documented,
  `gh =` present, `# Examples` present and solving); `gh` attribute in
  `#[node]`; `# Examples` execution in CI. **Done when** the regenerated
  catalog is byte-identical to before the move, golden hashes unchanged.
  **Done 2026-08-20** in three commits: the catalog's within-category
  tie-break became the dialect name (so the move could be proven
  byte-identical and future moves never reorder the catalog), then the
  pure move (117 top-level items byte-identical, 58 golden constants
  unchanged, catalog byte-identical), then the format: `gh = "…" | none`
  required by the macro (trybuild cases), `# Examples` ```` ```cic ````
  fences extracted into `NodeSpec::examples` (bare fences refused —
  rustdoc would doctest them), all 57 nodes named or `none`d and given a
  runnable snippet, the conformance test
  (`crates/cicada-stdlib/tests/conformance.rs`), the runner
  (`crates/cicada-cli/tests/node_examples.rs` — parse, check with zero
  diagnostics, lower, solve with a fresh cache; exporters' inputs solve,
  the exporter itself is asserted skipped), CATALOG.md's `· GH:` tag and
  catalog.json's `gh`/`examples` fields, the `add-stdlib-node` skill
  rewritten for the layout. Honest reading of the DoD: "byte-identical"
  holds against the tie-break commit, not against the pre-C0 catalog —
  that one was ordered by source position (kinship order: add, subtract,
  multiply …), so the name tie-break permuted every category's lines
  once (a pure permutation, proven by sorted-line diff; the ribbon was
  never affected — the web client sorts each tab by title itself).
  **Review fixes (2026-08-20)**: `# Returns` (one line) is the doc of a
  bare single `out` port, required by the macro exactly when a node
  returns one bare value (three trybuild cases) — the first conformance
  test had exempted `out`, leaving 47 output ports undocumented in
  catalog.json; the "example calls the node" rule matches at an
  identifier boundary (`polyline(` no longer satisfies `line`); every
  node file now holds its own three tests (deconstruct_domain,
  deconstruct_point, flatten, mesh_difference, mesh_intersection gained
  theirs; sphere gained a transcendental-free topology golden) and the
  conformance test reads the source to enforce the layout
  (`src/<category>/<node>.rs`, one `#[node]` per file) and the three
  tests. **Web lane (2026-08-20)**: the client's `CatalogNode` mirror
  reads format 2 (`gh`, `examples`, per-port `doc`; a vitest pins it to
  the committed `catalog.json`); search-to-place — the double-click box
  and the wire-to-empty-canvas box — matches name, title AND `gh`,
  ranked name exact > gh exact > title exact > prefix > substring
  (`Addition` → `add` above `mass_addition`), and shows `GH <name>` on a
  row whose GH name differs from its title; port hovers on the canvas
  and in the inspector read `name: type — doc`, output docs looked up
  in the catalog by func (the view-model's `OutputView` still carries
  no `doc` — deliberately: the catalog is the source either way).
  `web/e2e/search.spec.ts` drives it through the real app.
- **C1**: the nodes our diagnostics already cite — `compact`,
  `pad_last`/`repeat`/`truncate` policies — plus duplicate / reverse /
  sort / dispatch / group_by and the maths tail (min/max/abs/round/
  floor/ceil/trig). A test asserts no diagnostic names an unregistered
  node. **Done 2026-08-20** (`wt/catalog`, five node commits plus the
  review-fix commits, 48 nodes, all in the C0 format with the three
  tests, goldens blessed through the run-once path, catalog regenerated
  each commit): lists — `compact`, `duplicate`, `reverse`, `sort`,
  `dispatch`, `group_by`, `shift_list`, `split_list`, `nest`,
  `transpose`, `pad_last`, `truncate`, `weave`, `insert_items`; maths —
  `negative`, `absolute`, `round`, `floor`, `ceiling`, `min`, `max`,
  `sqrt`, `ln`, `log`, `exp`, `sin`, `cos`, `tan`, `asin`, `acos`,
  `atan`, `atan2`, `radians`, `degrees`, `smaller`, `larger`, `equals`,
  `and`, `or`, `not`, `xor`, `pick`, `mass_addition`, `average`,
  `bounds`; sequences — `range`, `repeat`, `jitter` (one shared PRNG).
  Decisions recorded: the cyclic zip policy is the node `repeat` (the §1
  time param owns `cycle`; DECISIONS.md GH-tree row revised 2026-08-20,
  docs/02/09/10 updated); `compact(list: [E?]) → (values: [E], map)` is
  honored at check time — an `E?` port keeps the wired `?` on the port
  (`bind_var`), so the "`compact` removes the holes" advice is
  satisfiable (checker fixture + a `[Number?]` script-node CLI test);
  `cross` (two type variables) and `squeeze`/`flatten_all`
  (data-dependent depth) wait on checker work.
  `crates/cicada-cli/tests/diagnostic_vocabulary.rs` scans the checker,
  scheduler, server (lowering, compile, session) and every stdlib node's
  string literals against the registry, with a PHANTOMS rule for the
  words that are not nodes (`cycle`, `cross`, `squeeze`, `flatten_all`,
  …) in any spelling. Port docs are whole paragraphs (the `Ports` macro
  joins a field doc's first paragraph; the conformance test reads a doc
  ending on a bare word as truncated).
  `examples/02-solids`/`03-voronoi` use `duplicate(count=1)` as the
  singleton (02 drops from 11 to 10 nodes; the Playwright smoke's count
  assertion moved with it); `examples/06-lists.cic` is the
  consumer (the orbit example planned as 06 moves to the next free
  number).
- **C2**: mesh-tier cylinder/cone/extrude_to_point/volume/bounding_box
  and the vector/plane nodes (ports pin_cutters / tip_caps math out of
  Python in the wall).
- **C3**: the core Curve ABI landed once (Arc/Ellipse/Compound,
  `Planar<Curve>`, Color authoring) then the curve nodes.
- **C4+**: the rest of docs/08 S+1, category by category, Solid rows
  riding with item 3.
- **`cicada mcp`**: catalog search, node docs, the checker — from the
  same data as `/api/catalog`; its own package after C1 (C1 shipped
  without it). **Done 2026-08-20**
  (`wt/catalog`, one commit): `cicada mcp [--project <dir-or-pipeline>]`
  is a Model Context Protocol server over stdio on `rmcp` 3 (the
  official Rust SDK, Apache-2.0; `server` + `transport-io` features
  only — no macros, the tools are plain routed functions the workspace
  lints see; 17 new crates in the native build graph — 32 lockfile
  entries counting the platform-only ones (wasm-bindgen / js-sys,
  windows-core, core-foundation, android, haiku) — every one MIT
  and/or Apache-2.0, `cargo deny check` green, no new duplicate
  versions). Tools: `catalog_search {query, category?,
  limit?}` (per-word scoring over name / title / `gh` / ports /
  description, exact > prefix > contains, prose at word starts only;
  empty query lists), `node_doc {name}` (the `/api/catalog` node object
  from the server's own renderer — `catalog::node_value`, an additive
  accessor — plus `signature` and `effectful`; unknown names get the
  checker's did-you-mean by checking a one-binding probe), `list_categories`,
  `check {text | path}` (the doc-11 diagnostics from
  `compile::check_source`, the new shared entry `load` and the session's
  `reload_text` now call — one checker). `--project` discovers the
  project's scripts through `compile::catalog_specs_in` /
  `scripts::discover_in` (additive) and re-discovers when the bytes of
  `scripts/*.py` change; a broken `scripts/` refuses at startup.
  Refusals are structured tool errors, stdout is protocol-only, the
  server exits on EOF. Tests: `crates/cicada-cli/tests/mcp.rs` drives the
  built binary with JSON-RPC framing (handshake, `tools/list` shapes,
  every tool, the `slider` doc's ports + GH name, the did-you-mean
  diagnostic, `--project` script nodes + relative paths + live
  re-discovery, the startup refusal) plus unit tests in `mcp.rs`.
  `.mcp.json.example` registers it for Claude Code; docs 11 and 14 and
  the AGENTS.md palette carry the surface. Deliberately NOT in this
  slice: the live-graph tools (`what_feeds`, `wire_summary`, `profile`,
  `diagnostics`) — they need a running solve and belong to the app's
  server; a `volatile` flag in `node_doc` — `NodeSpec` has none until
  item 3b adds it (only `pure` / `effectful` are reported; when 3b adds
  the field to `catalog::node()`, the `node_doc_schema_matches_every_catalog_entry`
  test demands it be described in `NodeDoc` in `mcp.rs` — the value
  flows through the renderer on its own, the schema does not).
  **Review fixes 2026-08-20** (adversarial review of the package):
  `check` now runs the dry lowering (`lower_partial`, the session's
  form) after the checker and reports `excluded` bindings with the
  canvas's status + reason (`Exclusion::reason()`, one renderer shared
  with the view-model) — `ok` was true for an integer literal at 2^53
  that `cicada run` refused; `node_doc`'s `outputSchema` is the real
  node shape (`NodeDoc` / `PortDoc`, held to `catalog::node_value` by a
  test over every stdlib node) instead of an open object; the
  project's script nodes are KEPT in the catalog cache (the dry lowering
  needs their run functions to exist; nothing ever runs); tests now pin
  GH-only search matches (`Pick'n'Choose` → `pick`, `addition` → `add`
  above `mass_addition` — deleting the `gh` scoring branch once left
  every test green), the out-of-project and scripted-subdirectory
  `check {path}` branches (forcing the in-project branch once passed
  every test), an exporter's `pure` / `effectful`, and the 2^53 literal.
  `.mcp.json.example` registers the BUILT binary
  (`${CARGO_TARGET_DIR:-target}/debug/cicada`, Claude Code's variable
  expansion) — the `cargo run -p cicada-cli` form is a cold
  feature-unification context that rebuilds the Manifold kernel with
  cmake on PATH, which no MCP client's startup timeout survives; `.mcp.json`
  is gitignored (the docs say to copy the example there).
Every node: the three tests, the doc format, catalog regenerated in the
same commit (skill `add-stdlib-node`).

## Follow-ups (found by the v0.1 reviews and measurements; scheduled, not yet placed)

- **Control-plane priority over the display restream.** A client that joins
  a session receives the whole display set on the one socket (the wall:
  ~350 MB of binary frames, measured 2026-08-20) and every text frame —
  `preview_policy`, deltas, statuses — queues behind it; on a loaded dev
  machine `preview_policy` reached a fresh observer ~26 s after the drag.
  doc 13 named head-of-line blocking as the trigger for a transport change:
  the fix is a priority lane for text frames (two channels per client with
  text-first draining — the frame bus already drops stale-generation frames
  — or a second socket for binary), with a test that a text frame sent
  behind a multi-hundred-MB restream arrives within the status cadence.
  Protocol-change skill; one package. Until then
  `web/e2e/compute_on_release.spec.ts` waits for delivery, not latency.
- **A count/allocation guard for every count-taking node** (`series`,
  `range`, `duplicate`, `repeat`, …): a slider wired into `count` can ask
  for a capacity the allocator refuses, which aborts the process —
  `catch_unwind` cannot catch it. A shared `checked_count(count,
  bytes_per_slot)` that refuses loudly (red, with the number) before
  allocating; one package in stdlib + a table test per node.
- **CI solves every `examples/*.cic`.** Only `02-solids` is exercised (by
  the Playwright smoke); a `cicada-cli` test that runs each example headless
  with a fresh `--cache-dir` keeps `06-lists` and the rest solving.
- **Renumber item 4's orbit example**: `examples/06-lists.cic` took the
  number; the transport slice uses the next free one.
- **Stale catalog on the client after a scripts-change reload**: the app
  fetches `/api/catalog` once; search rows and port tooltips for script
  nodes go stale until a reload. Refetch on the catalog-reload barrier.
- **The costed, cancellable display edge** (WP-B review closure,
  2026-08-21): display tessellation now runs on the solve loop's workers
  before the session lock, tiered (preview / fine) and in parallel over
  a generation's distinct solids (docs/12 §Display cache) — but ONE
  giant solid is one `BRepMesh` call (the 300-hole bar: ~0.7 s per
  preview tick even at the preview tier, two 6,300-vertex planar faces),
  and a superseded tick's tessellation runs to completion. docs/12 names
  the rest: display as a costed, persisted edge in the store (the cache
  key is already the one it would use), so it is cancelled like a node,
  survives a warm reopen without the kernel, and can be routed to the
  kernel worker below when the cost model predicts it long.
- **Mixed-age stores, again** (WP-B): `LOG_FORMAT` is 3 — an engine from
  before `StoredValue::Solid` refuses a store this engine wrote; serve
  scratch copies across worktrees, as AGENTS.md says.
- **02-solids' export tessellation sits in the slider's cone**: the
  example's `shell = tessellate(solid=carved, deflection=0.01)` is
  recomputed on every tick (34 ms — the cone's whole cost now that
  display is 5 ms); it exists for the `--node dump` exporter. Either the
  example marks it as export-time work (`#off` until export, or a
  coarser deflection) or the docs/15 slider criterion names the scratch
  variant without it. A one-line decision for Ben; the measurement of
  both is in §Item 3.
- **The doc-12 kernel worker** (WP-C measurement): Esc inside ONE
  long OCCT boolean waits for it (1,000 tools: 1.7 s to idle). Route
  kernel calls the cost model predicts above ~1 s to a cancellable
  subprocess that Esc kills; the op-local sharing model makes this
  cheap — the call's inputs are bytes already. Until then a long B-rep
  boolean holds the session for its duration; an `each()` chain of small
  booleans does not (p95 31 ms).
- **`import_step` re-reads its file every solve** (volatile by design):
  a content-keyed memo (hash the file's bytes, key the translation on
  that) would keep the honesty and drop the read from a slider's cone
  when a large import sits in it. Measure a real import first.
- **Three cicada-server test fixtures were touched by WP-C** (outside its
  file list, to keep the rename's one commit green): `session.rs`'s
  mesh-display story and the OBJ-export story use `mesh_box`,
  `viewmodel.rs` expects `Solid` for `box`. Whoever owns the server
  crate should glance at them; nothing behavioural changed.
- **`cicada-geom/src/occt/glue.hxx` is a second glue location** (WP-C):
  the fork keeps the binding patches and the first glue, cicada-geom the
  node set's. If Ben prefers one home, the cicada-geom header moves into
  the fork's `cicada.hxx` in one patch and the bridge's `include!`
  changes; the `cicada_geom` C++ namespace keeps the two from colliding
  meanwhile. A decision for the ledger, not a blocker.

## Gates that must not regress (re-measured at each geometry or scheduler landing)

From doc 15 §Stage-6 results: cold wall carve < 10 s (6.5 s; 3.7 s on
2026-08-20 after WP-C; 3.75 s on 2026-08-21), warm < 100 ms (0.13 ms);
cheap-cone slider p50 ≤ 16 ms / p95 ≤ 33 ms (0.5 / 1.4 ms on the mesh
tier; the OCCT 02-solids cone 5.2 / 9.1 ms server, 5.5 / 9.4 ms client
on 2026-08-21 with display tiered and off the lock — measured on a
scratch copy without the example's export `tessellate` node, which
alone is 34 ms per tick and keeps the committed example at 39.9 / 54.9
ms, §Follow-ups); Esc time-to-idle p95 < 250 ms (214
ms; a 4,000-element B-rep chain 31 ms; ONE 1,000-tool boolean 1.8 s —
the kernel worker follow-up); file edit → canvas < 500 ms (~100 ms);
wall output equivalence `overall NOISE` on Windows and Linux.
