# Cicada — agent operating manual

Cicada is a code-first parametric design tool: the pipeline is a typed
program (`.cic` dialect), the node graph is a generated view, the engine is
Rust. Design docs 01–17 in `docs/` plus `DECISIONS.md` fully specify the
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
3. The item you are working in, from
   [docs/17-v01-plan.md](docs/17-v01-plan.md) (the v0.1 work order —
   items, work packages, definitions of done, status), and the docs it
   lists for that item. docs/15 is the closed spike record.

**Current status: v0.1 is UNDERWAY (work order: docs/17; order decided with Ben 2026-08-19 — DECISIONS.md row of that date): (0) fold `corpus/` into `examples/wall/` + `tools/` — DONE 2026-08-20; (1) undo/redo with the atomic `batch`/`apply_text` path + `#off` — DONE 2026-08-20; (2) git panel slice 1 — DONE 2026-08-20 (`wt/git-panel`: status markers, commit, revert; the chip, Git tab, badges, `Ctrl+S`); OCCT probe — DONE 2026-08-20, GREEN on Windows (docs/probes/occt-2026-08.md), then (3) OCCT-backed Solid as the main geometry track — WP-A (seam, build, CI), WP-B (the `Solid` kind end to end, the sharing model, display) and WP-C + WP-D (the OCCT-backed node set under the bare names, the mesh tier as `mesh_*`, `occt` ON by default, `examples/07-simple-cad.cic`) DONE 2026-08-20, WP-B's second review closed 2026-08-21 (tiered display off the session lock, the stale-pcurve transform fix, typed kernel refusals — docs/17 §Item 3) — follow-ups open (docs/17 §Follow-ups: the costed cancellable display edge, the doc-12 kernel worker, 02-solids' export node in the slider cone); (3b) scheduler foundations + compute-on-release — DONE 2026-08-20 (`wt/sched`: engine half and the sliders' `preview_policy` rendering); (4) time transport — DONE 2026-08-20 (`wt/transport`: engine + the web's play bar — the scrubber, `Space`, hidden transport-driven ports, the e2e); (5) scrub caching; (6) WASM host last; track C (catalog: one-node-per-file restructure + node-format conformance — C0 DONE 2026-08-20; C1, the first docs/08 S+1 tranche — DONE 2026-08-20; `cicada mcp` — DONE 2026-08-20; Grasshopper names in search-to-place — DONE 2026-08-20; next C2+) runs in parallel throughout. docs/17 §Scope carries the live status table — update it when an item moves.**
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
Playwright smoke), and stage 6: the wall corpus (`examples/wall/wall.cic` — the 1,200-part production wall on the engine, reproducing the shipped 3MF/DXF modulo declared noise), the ported Python script nodes (`examples/wall/scripts/`; the script host now marshals Mesh/Plane/Curve with msgpack bin, multi-output dict returns, and effectful `-> None` exporters), the new nodes `loft` / `text_outlines` / `text_solids` (bundled DejaVu Sans Bold) / `area` / `flatten` / `partition` / `chunk` / `concat` / `cull` / `construct_plane`, the measurement harness (`tools/measure/`) and normalizer (`tools/normalize.py`) — all five doc-15 criteria PASSED (docs/15 §Stage-6 results: cold carve 6.5 s, cheap slider 0.5 ms p50, Esc 170 ms, canvas round-trip ~100 ms). v0.1 so far (2026-08-20): **undo/redo** — a snapshot op log per session (`undo`/`redo` intents, `Ctrl+Z`/`Ctrl+Y`, history on every delta/snapshot, cleared by the reload barrier), the atomic **`batch`** intent (multi-node canvas gestures = one op) and **`apply_text`** (whole-file edits for agents: base text hash + files, refused when stale or unparsable — `POST /api/edit/apply_text`, `GET /api/edit/text`), a failed persist restores the disk; **`#off`** is native (parsed ghosts with ports intact, `writer::toggle_disable`, `D`, the node menu, the inspector); Backspace no longer deletes (`Del` only). **Git panel slice 1** (docs/13 `/api/git/*`, docs/16 Git panel bullet): `GET /api/git/status` — typed states (not a repo, no git, unborn, detached, `index.lock`, an unfinished merge/rebase/cherry-pick/revert), per-node markers computed FROM `git diff -U0 HEAD`, the commit scope (`.cic` + sidecar + `scripts/*.py`) with `in_head` per file; writer-gated `POST /api/git/commit` (message verbatim, exactly the scope) and `POST /api/git/revert` (to HEAD through the reload barrier, under the session's write hold); in the app the top-bar git chip, the Git inspector tab (markers → select the node, files to commit, commit form, Revert-to-HEAD behind a binding confirm list), canvas badges, `Ctrl+S` = the commit dialog; the status is re-read on connect, ≤1 s after a write, on focus — never on a timer. **Stdlib layout**: one node per file under `crates/cicada-stdlib/src/<category>/` (categories = ribbon tabs), every node carries `gh = "…"` (its Grasshopper equivalent, or `none`), a `# Returns` doc for the bare `out` port and a runnable `# Examples` snippet; `tests/conformance.rs` enforces the format and the three tests per file, `cicada-cli/tests/node_examples.rs` solves every example; the catalog sorts by name within a category and `catalog.json` carries `gh` + `examples`; the app reads them (`web/src/protocol` mirrors `/api/catalog` format 2): search-to-place — the double-click box and the wire-dropped-on-canvas box — matches name, title AND `gh` (ranked name exact > gh exact > title exact > prefix > substring; a `GH <name>` hint on a row whose GH name differs from its title), and port hovers on the canvas and in the inspector read `name: type — doc` (output docs looked up in the catalog by func; `web/e2e/search.spec.ts`). **Catalog C1** (48 nodes): the list nodes `compact` (`[E?]` in, present `[E]` + IndexMap out — an `E?` port keeps the wired `?` on the port, so the checker's `compact` advice is satisfiable), `duplicate` (`count=1` is the idiomatic singleton list), `reverse`, `sort`, `dispatch`, `group_by`, `shift_list`, `split_list`, `nest`, `transpose`, `weave`, `insert_items`, the strict-zip adapters `pad_last` / `repeat` / `truncate` (the cyclic policy is `repeat` — `cycle` is the time param; DECISIONS.md GH-tree row revised 2026-08-20), the maths tail (`negative` … `atan2`, `radians`/`degrees`, comparisons, gates, `pick`, `mass_addition`/`average`/`bounds`), `range` and `jitter`; `crates/cicada-cli/tests/diagnostic_vocabulary.rs` keeps every user-facing string (checker, scheduler, server, stdlib panics) naming only registered nodes; port docs are whole paragraphs; `examples/06-lists.cic` is the consumer. **OCCT probe** GREEN on Windows (prebuilt conda-forge 7.8.1 via `DEP_OCCT_ROOT`, one rename patch, byte-deterministic B-rep bytes, ~3 ms per boolean) — memo + reproduction in `docs/probes/occt-2026-08.md` and `tools/probes/occt-2026-08/`. **The `Solid` kind (item 3 WP-B)** — `core::Solid` IS its OCCT canonical bytes (`Arc<[u8]>`, header-checked, KindTag 20 over the length-prefixed bytes; goldens for the probe box/prism through the new path equal WP-A's); it rides `T` ports and display sinks (`TRANSFORMABLE_KINDS` / `GEOMETRY_KINDS`; a Solid moves through the kernel since WP-C — `Similarity::apply`'s Solid arm calls `cicada_geom::solid::transform`), the store (`StoredValue::Solid`, `LOG_FORMAT` 3, a committed pre-Solid pack proves older blobs still load), and the Python boundary refuses it typed (`ScriptError::Unmarshallable`). `cicada_geom::solid` is the value-level API with the same signatures in every build (`GeomError::KernelUnavailable` without the `occt` feature — never a mesh-tier fallback) over op-local, LINEAR `occt::Handle`s: the sharing model (docs/03) — a `BinTools` read is the deep copy, kernel operations consume their handles, results go back to bytes, no two live handles share a `TShape`, so the process-wide kernel lock is gone (8-thread rayon test against goldens); the handle cache was measured (a re-read is 1.3 % of a boolean, 5 % of the chain; `examples/solid_bench.rs`) and NOT built. Display (as closed by the second review, 2026-08-21): the session's `display::SolidCache` (value hash + TIER deflection → the welded display mesh sealed as a value + face count + `watertight`; refusals cached as negative entries; 256 MiB LRU; `/debug/state` → `display_cache` counters incl. `refusals`; `DisplayStats.solids`/`.tier`/`.warnings`/`.errors` additive), two tiers — `Deflection::display` = `max(0.02 mm / unit, tol)` / `max(0.1 rad, tol_angle)` for structural generations, `Deflection::preview` = 0.1 mm / 0.3 rad for a drag's generations (the release redraws a preview-tier value fine; the tier is recorded per output) — plus a relative term (1/1000 of the solid's largest extent), warmed on the solve loop's worker pool BEFORE the session lock (`Scheduler::map_parallel` over the generation's distinct solids), mesh frames keyed by the DISPLAY MESH's hash (two tiers → two blobs), a mesh the kernel could not close drawn anyway with `watertight: false` (closure is the `tessellate` NODE's contract: `solid::tessellate` requires it, `solid::tessellate_display` reports it), summary "Solid, N faces, bbox", web hue `--kind-solid`. Kernel refusals are typed: `GeomError::NotOneSolid { operation, found }` ("cut left 2 solids — a Solid is one body; …"), glue prefixes stripped, `diagnostic_vocabulary.rs` rejecting `cicada_`/"shape type"; `transform` drops the stale source-surface pcurves `BRepBuilderAPI_Transform(Copy = true)` leaves on degenerate edges (the moved-sphere-minus-cylinder mesh that did not close; docs/03); `solid::is_valid` = `BRepCheck_Analyzer`. **The OCCT-backed node set (item 3 WP-C + WP-D, 2026-08-20)** — B-rep is the DEFAULT working mode (DECISIONS.md row 42): `box` / `sphere` / `cylinder` / `cone` / `extrude` (exact edges, no `segments`) / `extrude_to_point` / `loft` (`profiles: [Closed<Curve>]`, `ruled = true`) / `revolve` / `sweep` / `pipe` / `solid_union` (n-ary) / `solid_difference` (solid + cutter list) / `solid_intersection` / `volume` / `bounding_box` / `deconstruct_solid` (edges, vertices, `face_count` — no `Surface` kind yet) / `section` (circular loops exact) / `tessellate` (weld + watertight + Manifold acceptance) / `export_step` (effectful; header and product numbering fixed → byte-deterministic) / `import_step` (the first `volatile` node); a `Solid` is always ONE body (disjoint unions, splitting cuts, empty intersections are red); the six transform nodes take a `Solid` as `T` through the kernel transform (`mirror(geometry: T, plane = xy_plane)` — the plane reflection, `Similarity::reflection` — added 2026-08-21); the spike's mesh-backed four continue as `mesh_box` / `mesh_sphere` / `mesh_extrude` / `mesh_loft` (Mesh & field; the wall and 03/04 stay on them; outputs byte-identical, the carve hash unchanged). The glue for these ops is cicada-geom's own (`src/occt/glue.hxx` + `glue.rs`, built by `build.rs`), the fork keeps the binding patches and the first glue; `occt` is ON by default and every cargo command needs the fetch script's env (palette). Goldens: transcendental-free solids only, through `solids/support.rs::platform_golden` (win-64 blessed; a second OS adds its arm). **Review closure (2026-08-21)**: `box`/`sphere`/`extrude`/`loft` are at `version = 2` — the flip changed their output KIND under the spike's names and the memo key never looks at kinds, so a pre-flip cache served a Mesh to the Solid-typed `box` (`cicada-server/tests/stale_memo.rs` is the regression) — and the rule is now a test: the conformance suite holds every `(name, version)` to its recorded signature + flags in `crates/cicada-stdlib/tests/signatures.tsv` (new rows via `CICADA_BLESS_SIGNATURES=1 cargo test -p cicada-stdlib --test conformance`, never by hand); `tessellate` (version 2, no longer `uses_tolerance`) refuses, BEFORE the mesher, a request finer than its budget for the part — 1000 facets per full turn at the solid's largest extent (`TESSELLATE_MAX_FACETS_PER_TURN`; a unit sphere at the kernel floor 1e-7 was 23 GB of mesher that never finished) — typed `GeomError::TessellationBudget` naming the floors; `section` (version 2) tells a tangent contact from a loop (probes either side of the edge in the plane; contacts bound no region and yield no loop, an open chain with the solid on one side is a typed kernel failure); the stdlib has a real kernel-free world (`cargo test -p cicada-stdlib --no-default-features`, its own forwarding `occt` feature; `support::fixture` / `expect_red`). `examples/07-simple-cad.cic` is the consumer (the bracket; `web/e2e/simple_cad.spec.ts`), `02-solids.cic` the B-rep primer. Measured 2026-08-20: wall cold carve 3.7 s (unchanged tier; 3.75 s and the same hash on 2026-08-21); the 02-solids slider on the OCCT cone 42 / 135 ms — FAILED 16/33 because the display tessellation of a curved Solid at 0.02 mm cost 23 ms per new value; 2026-08-21 with tiered display off the lock: 5.2 / 9.1 ms server, 5.5 / 9.4 ms client on the display cone (PASS), 39.9 / 54.9 ms on the committed example whose export `tessellate(deflection=0.01)` node costs 34 ms per tick in the cone (a follow-up for the example); the 300-hole bar's generation 5.26 s → 1.19 s for a 0.34 s solve; Esc in a 4,000-element B-rep chain 31 ms p95, inside ONE 1,000-tool boolean 1.8 s — the kernel-worker follow-up in docs/17. **`cicada mcp`** — the docs/11 read tools for agents as a Model Context Protocol server over stdio (`rmcp`, the official SDK, without its macros): `catalog_search` (ranked over name / title / `gh` / ports / description), `node_doc` (the `/api/catalog` node object + `signature` + `effectful`), `list_categories`, `check` (`text` or `path` → the doc-11 diagnostics through `cicada_server::compile::check_source`, THE checker — `cicada run` and the session call the same function — then the session's `lower_partial_with_playhead` dry at the playhead at rest, so `excluded` lists every binding the app would not solve with its red/blocked `reason` from the one renderer (`Exclusion::reason()`), and `ok` is false for what only lowering refuses — an integer literal at 2^53, a `cycle` whose loop port is wired (the transport's one red; headless it solves, and the reason says so)); `node_doc`'s output schema is the real node shape, held to the renderer by a test; `--project <dir-or-pipeline>` adds the project's script nodes (re-discovered whenever `scripts/*.py` change; kept, never run) and anchors relative `check` paths (a file outside the project, or in a scripted subdirectory, is checked against its own `scripts/` as `cicada run` would); refusals are structured tool errors (`{error, message, did_you_mean…}`), stdout is protocol-only; `.mcp.json.example` registers the built binary for Claude Code; `crates/cicada-cli/tests/mcp.rs` drives the binary with real JSON-RPC framing. **Scheduler foundations (3b)**: every solve owns a `CancelToken`, handed to each node as its `NodeCtx` (`NodeFn` = `Fn(&NodeCtx, &[inputs])`; `NodeError::cancelled` is the sanctioned bail-out at a safe point — any other error stays red, Esc or not; the Python bridge kills exactly the calling generation's worker); `#[node(volatile)]` → `NodeSpec`/`NodeDecl` (the memo is never read or written for it, per element inside `each()`, `"volatile"` in catalog.json); the idle-class `Session::solve_hypothetical` / `SolveLoop::run_idle` (paints nothing, pre-empted by any real generation or Esc, invisible to `wait_idle`, fills the ordinary memo); compute-on-release (`preview_policy`, decided per tick from a hash-only dry run of the cone against the memo, `COMPUTE_ON_RELEASE_MS` = 1 s inclusive, drags end on any write / Esc / a 300 ms gap — docs/13 §Slider drags is the frozen client contract; `tools/measure/slider_loop.mjs --expect compute_on_release` reads it; both sliders render it as a `pending · N s` chip on the thumb-following value — the store's one `pending` param, replaced by every arrival and cleared by the release's delta, by `drag_ended` (the server announces the end of every announced drag; a release that writes nothing is the `end_drag` intent) or by the widget's own no-write release — with `web/e2e/compute_on_release.spec.ts` dragging the wall's `deboss`, an observer page watching, as the evidence; the two sliders have jsdom + Testing Library component tests, `npm test` runs them); memo entries record their cost (`cached` statuses carry the LAST compute's `elements`/`nanos`) and the store root carries a `format` marker (`LOG_FORMAT`). **Time transport (item 4, engine + web)**: the params `cycle(period = 4, frames = 120, frame = 0)` (`(frame mod frames) / frames`, frame-quantized so one pass of the loop warms every key) and `clock(speed = 1, t = 0)` (`t × speed`, the one `volatile` node); `#[port(transport_driven = frame | time)]` marks the port the session's transport owns — `catalog.json` `"transport_driven"`, the web hides it; the LOWERING is the injection point (`lower_with_playhead` / `lower_partial_with_playhead` fill the port from the session's `Playhead { t_ms }`: `clock.t` = seconds, `cycle.frame` = `floor(t × frames / period) mod frames` from the node's literal loop — a wired `frames`/`period` is the one red the transport adds; every session lowering passes the playhead, `cicada run` passes none and the ports evaluate as written: frame 0, t 0); per-session anchored transport state on the injectable op clock; the five writer-only intents `transport_play` / `transport_pause` / `transport_seek {frame}` / `transport_speed {factor}` / `transport_reset` (never an op, never a delta, never the file; refusals are kind `transport`), `TransportView {playing, speed, t_ms, frame, frames, period_ms, driven}` in every snapshot and as the `transport` broadcast on every change, `/debug/state.transport`; playback = a 60 Hz ticker (an absolute grid walked with the high-resolution `thread::sleep` — a `Condvar::wait_timeout` per tick ran at 33 Hz on Windows; `tools/measure/transport_loop.mjs --expect warm` asserts a 60 fps loop plays at 60 generations/s with its second pass 0 computed) submitting `JobKind::Transport` jobs to the one-slot latest-wins loop only when the driven values moved (the solve bounds the rate; a live drag's thumb rides along); Esc (pause first, then cancel, under the lock the ticker submits under) and the last client leaving pause it; `examples/08-orbit.cic` is the consumer (orbit measured: second pass 0 computed / 1,800 cached, p50 0.43 ms); `transport_seek` lands the first representable playhead INSIDE the frame it names (`Playhead::at_frame` — the nominal start rounds a few ulps short for some frames), and each `DrivenView` carries its OWN loop (`loop {frames, period_ms}`) so a non-primary `cycle`'s inspector row shows its own frame, not the primary loop's. The web half: the play bar (play/pause, the frame scrubber = a stream of `transport_seek`s, speed, reset), `Space` toggles when no text field is focused, the transport-driven ports hidden on the canvas and in the inspector (no connectable handle, no literal editor; a hand-written kwarg/wire stays as the headless value/source — a wire keeps a drawn, removable edge — the SERVER owns the rule: `probe_wire`/`connect` AND `set_param`/`param_preview` into such a port refuse with one reason; `transport_speed` above 64× is refused; a frame whose lowering excludes a binding the canvas shows green — the playhead beyond the exact frame range — raises a `notice` once per change), observers read-only, the playhead extrapolated between broadcasts (`web/e2e/transport.spec.ts`: the orbit + a two-cycle+clock pipeline). Live subcommands: `cicada catalog`, `cicada run` (always pass `--cache-dir` in tests; effectful bindings run only via `--node`; `CICADA_TRACE=1` prints per-node phase timings), `cicada serve`, `cicada mcp`. `examples/wall/` is the wall — ONE copy that is at once the full-size example, the app playground (open it, edit freely; `git checkout -- examples/wall` reverts the tracked files, `git clean -f examples/wall` drops the untracked layout sidecar the canvas writes once you move a node), and the nightly regression corpus (DECISIONS.md corpus row, revised 2026-08-19: the corpus = every example with committed golden outputs; the wall is the first). `examples/` is the runnable playground — also
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
| `crates/cicada-stdlib` | The node catalog — pure functions, never depends on sched; one file per node under `src/<category>/` (skill `add-stdlib-node` has the format) |
| `crates/cicada-sched` | Scheduler-lite: solve graph + `NodeKey`s, two-level disk store (memo log + zstd blobs), rayon wavefront executor with `each()` fan-out, cancellation, latest-wins previews, cost sampling |
| `crates/cicada-script` | WASM host (v0.1) + Python worker pool (stage 4) |
| `crates/cicada-server` | The engine server (docs/13): axum app + token auth (`http.rs`), per-pipeline `Session` (intents → writer gestures → deltas, statuses, display set, lease), the latest-wins generation loop (`solve.rs`), JSON protocol (`protocol.rs`), graph view-model (`viewmodel.rs`), byte-exact binary frames (`frames.rs` IS the spec) + value→frame/summary (`display.rs`), sidecar + auto-layout, AND the hydration path shared with `cicada run`: `compile.rs` (targets, cone gate), `lower.rs`, `scripts.rs` (Python nodes + the cancel bridge). `embed` feature bakes `web/dist` in |
| `crates/cicada-cli` | The `cicada` binary: `catalog`, `run` (a printer over the server's compile/lower), `serve`; hosts the dependency-DAG test |
| `web/` | SPA: React + TypeScript + Vite; `src/protocol` (message + frame mirrors, WS client), `src/state` (zustand store, connection, frame bus), `src/canvas` (React Flow), `src/viewport` (three.js), `src/panels`, `e2e/` (Playwright smoke) |
| `examples/` | The runnable playground; `examples/wall/` is the wall project (pipeline, `scripts/`, `inputs/`, `golden/production/`, wall-only `tools/`) — example, playground, and the first member of the nightly regression corpus |
| `tools/` | Engine-wide dev tooling, never pipeline code: `normalize.py` (exporter-output normalizer + comparer), the offline `test_*.py` (wall scripts + normalizer + the launcher; `_cicada_stub.py`), `measure/` (the docs/15 measurement harness), `fetch_occt.py` (the prebuilt kernel + `--bundle`), `launch/` (the double-clickable dev launchers `Cicada.cmd` / `Cicada.command` over `launch.py`, and `bundle.py` — the redistributable folder; docs/17 L3) |
| `docs/` | Design docs 01–17 + `docs/generated/` |

**Dependency direction is law**: `core ← {geom, lang, stdlib, sched, script}
← server ← cli`; only `cli` may depend on `server` (it does, since stage 5:
`serve`, and `run` reuses the server's compile/lower); `stdlib` never
depends on `sched`. Within the mid layer, `stdlib → geom` is a sanctioned edge (nodes
ARE the geometry users, docs/03); no other intra-mid-layer edges exist.
Enforced by `crates/cicada-cli/tests/dependency_dag.rs`.

## Command palette

**Every cargo command needs the OCCT env first** (since 2026-08-20, WP-C:
`cicada-geom`'s `occt` feature is ON by default — the product's
`box`/`extrude`/… nodes are OCCT-backed). Once per shell — Bash:
`eval "$(python tools/fetch_occt.py --print-env bash --quiet)"`; PowerShell:
`python tools/fetch_occt.py --print-env powershell | Invoke-Expression` —
plus cmake and a C++ toolchain on PATH (Dev machine notes below). It exports
`DEP_OCCT_ROOT` (build) and the loader path (run); without it the build
stops at cicada-geom's `build.rs` with the message that says so, and a
binary built elsewhere fails to load `TK*.dll`/`libTK*.so`. **The loader
path is needed wherever the `cicada` binary is LAUNCHED from, not only in
cargo's shell**: Claude Code starting `cicada mcp` from `.mcp.json`,
Playwright starting `cicada serve`, a double-click — without the prebuilt's
library dir on `PATH` / `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH` the process
dies before `main` with NO message (exit 127 / `STATUS_DLL_NOT_FOUND` as a
shell sees it). `python tools/fetch_occt.py --print-env mcp > .mcp.json`
writes the MCP registration with this machine's absolute loader dir in its
`env`. **The env is for BUILDING and for dev shells; a bundled binary needs
none** (since 2026-08-24, wave 4 L2): `python tools/fetch_occt.py --bundle
<dir>` puts the kernel's run-time closure beside the `cicada` binary in
`<dir>` — Windows searches the executable's own directory first, a macOS
binary's rpath is rewritten to `@executable_path/lib` — and that binary
starts from any shell, launcher or double-click without a loader path
(proved 2026-08-24: the unbundled release `cicada.exe` exits 127 without the
env; the bundled one answers `--help` and solves `02-solids.cic`). The
kernel-free build is `--no-default-features` (same signatures, every kernel
call a typed `KernelUnavailable`).

| Task | Command |
|---|---|
| Build (all) | `cargo check --workspace --all-targets` |
| Test (all) | `cargo test --workspace` (the kernel world — the product's); the kernel-free world is `cargo test -p cicada-geom --no-default-features` and `cargo test -p cicada-stdlib --no-default-features` (every Solid call / node a typed refusal; CI's Linux job runs both) |
| Test (one crate) | `cargo test -p cicada-core` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Format | `cargo fmt --all` (check: `--check`) |
| Regenerate catalog (CATALOG.md + catalog.json) | `cargo run -p cicada-cli -- catalog` |
| Catalog freshness (CI mode) | `cargo run -p cicada-cli -- catalog --check` |
| Bless macro compile-fail snapshots (PowerShell) | `$env:TRYBUILD = "overwrite"; cargo test -p cicada-core --test macro_ui; Remove-Item Env:\TRYBUILD` |
| Web checks (bash; PS 5.1 has no `&&` — use `;`) | `cd web && npm run check && npm run lint && npm test` |
| Serve the app | `cargo run -p cicada-cli -- serve <dir-or-pipeline.cic> [--port 8420] [--token …] [--cache-dir …] [--web-dir web/dist]` — prints the URL with the token; without a built SPA it is API-only and says so at `/`. Dev: `cd web && npm run dev` (Vite proxies `/api`, `/ws`, `/debug` to port 8420 — `CICADA_SERVER=` overrides) and open the Vite URL with the same `?token=…&pipeline=…`. Release shape: `cd web && npm run build` then `cargo build -p cicada-cli --features embed` |
| Run the app (serve + its window) | `cargo run -p cicada-cli -- app [<dir-or-pipeline.cic>] --web-dir web/dist [serve's flags] [--no-browser]` (v0.1 wave 4 L1) — `serve` with exactly its arguments, resolved by the same function, then the window: a Chromium-based browser in `--app=<url>` mode when one is installed (Windows: Edge then Chrome via the registry's App Paths or the usual Program Files dirs; macOS: `open -na "Google Chrome" --args --app=<url>`, then Edge; Linux: `xdg-open`), else the default browser on the plain URL; `--no-browser` prints the URL and opens nothing; the URL is printed either way and the terminal is the server console; Ctrl-C stops the server. **The window needs a SPA**: `--web-dir web/dist` (dev — `cd web && npm run build` first) or the `embed` build (release: `cargo build -p cicada-cli --features embed`); with neither, or a `--web-dir` without an `index.html`, `app` refuses BEFORE binding and names both ways out — an app window onto `serve`'s "API only" page is the one thing it never opens (review finding 2026-08-24; `serve` is the API-only shape). Discovery is `cicada_cli::app::choose`, a pure function over `probe()`'s `Environment`, unit-tested per OS, and the SPA rule `cicada_cli::app::spa_source`; `crates/cicada-cli/tests/app.rs` drives the binary with `--no-browser` (a kill-on-drop guard and bounded line reads — a misbehaving server is a red test, never a stalled `cargo test`) |
| Playwright smoke (doc 15 DoD) | `cd web && npm run build && npm run e2e` — starts `cicada serve` from `$CARGO_TARGET_DIR/debug/cicada` (or `CICADA_BIN`) over a scratch copy of `examples/` (it prints which engine/scratch it uses — in a worktree export the PRIVATE `CARGO_TARGET_DIR` in the same shell or you smoke the main checkout's engine; the shell must also carry the OCCT loader path from `tools/fetch_occt.py --print-env`, or the engine dies before `main` without a word); first time: `npx playwright install chromium` |
| Agent verification of the running app | `GET /debug/state?token=…&pipeline=…&wait=true` (authoritative JSON: graph, statuses, per-output display bounds/triangles, generation timings; `wait=true` blocks until the debounce and any queued/in-flight generation are done — an intent sent on the socket is "in" once its `delta` arrived, so read the delta (or poll `seq`) before asking), `GET /debug/screenshot?token=…` (viewport PNG rendered by a connected client), `window.__cicada.{state,frames,scene,send,screenshot}` in the page |
| MCP server for agents | `$CARGO_TARGET_DIR/debug/cicada mcp [--project <dir-or-pipeline.cic>]` (build first: `cargo build -p cicada-cli`) — stdio JSON-RPC (stdout is the protocol; notes on stderr); tools `catalog_search`, `node_doc`, `list_categories`, `check` (checker + dry lowering: diagnostics AND `excluded` bindings with the canvas's red/blocked reasons); `--project` adds that project's `scripts/*.py` to the catalog and anchors relative `check` paths. Register via `.mcp.json.example` (copy to the gitignored `.mcp.json`; it names `${CARGO_TARGET_DIR:-target}/debug/cicada`, which Claude Code expands) — or, on any OS, `python tools/fetch_occt.py --print-env mcp > .mcp.json`, which writes the same registration with this machine's absolute OCCT library dir in the server's `env` (the example carries the Windows layout via `${LOCALAPPDATA}`). The `env` block is not optional: the binary links OCCT and Claude Code launches it from its own environment, so without the loader path the server dies silently before `main` (review finding, 2026-08-21). `cargo run -q -p cicada-cli -- mcp` works as the command only on a warm target dir with cmake on PATH — cold, the `-p cicada-cli` context rebuilds the kernel for minutes and the client times out |
| Headless run | `cargo run -p cicada-cli -- run <pipeline.cic> [--node <name>]… [--time] [--hashes] [--cache-dir <dir>] [--threads N]` — no `--node` = every leaf; `--hashes` prints stable hash lines INSTEAD of values; dialect syntax: [docs/10](docs/10-dialect-and-file-format.md); tests/CI always pass `--cache-dir` |
| Bless insta snapshots (checker diagnostics) | `cargo insta review` (cargo-insta installed 2026-08-12) — or `$env:INSTA_UPDATE = "always"; cargo test -p cicada-lang; Remove-Item Env:\INSTA_UPDATE` |
| Fetch the prebuilt OCCT (once per machine; `occt` feature) | `python tools/fetch_occt.py [--check-closure]` — sha256-pinned conda-forge OCCT 7.8.1 + its run-time closure into the user cache dir (`%LOCALAPPDATA%\cicada-occt`, `~/.cache/cicada-occt`; never a repo), prints `DEP_OCCT_ROOT` + the loader path; idempotent (the warm path re-verifies every shared library's presence and size, ~0.2 s; `--check-closure` also reads the import tables). Needs a zstd decoder (`pip install zstandard` on Python < 3.14). `--print-env bash\|powershell` emits the three lines a shell needs, `--print-env mcp` a `.mcp.json` with the loader path in the server's `env`; `--manifest-hash` is the CI cache key; `regenerate-manifest` is maintainer-only (network, re-pins everything) |
| Bundle the runtime beside a binary (no loader path at launch) | `python tools/fetch_occt.py --bundle <dir>` (v0.1 wave 4 L2) — `<dir>` holds a built `cicada` / `cicada.exe` (`cargo build --release -p cicada-cli [--features embed]`, copied there); the script fetches/verifies the prefix, refuses an open import closure, a binary whose own imports the bundle cannot satisfy or (macOS) a machine without `install_name_tool` / `codesign` — all before copying anything — then copies the closure: Windows BESIDE the exe (the loader searches the executable's directory first), macOS into `<dir>/lib` and rewrites the binary's rpath to `@executable_path/lib` with `install_name_tool` (+ an ad-hoc `codesign`; the prefix rpath the build env set is removed). Idempotent and verified by size like the prefix (a second run copies nothing; a truncated library is copied again; libraries a previous bundle recorded that the prefix dropped are removed — nothing else in `<dir>` is touched); writes `<dir>/.cicada-occt-bundle.json` (names + sizes). Linux is refused loudly (an rpath is set at link time, `$ORIGIN/lib`, not here). The bundled binary runs from a shell WITHOUT the env; the VC++ runtime (`msvcp140.dll` …) stays a machine-level requirement on Windows, as the probe memo records, and so does Python 3 everywhere (on PATH or `CICADA_PYTHON`): `run` and `serve` start the script host at launch, script nodes or not — the bundle removes the loader path, not that. Tests: `tools/test_fetch_occt.py::BundleTest` (the macOS tools mocked) |
| Launch the app by double-click; the redistributable bundle | `tools/launch/Cicada.cmd` (Windows) / `tools/launch/Cicada.command` (macOS) — v0.1 wave 4 L3. Each opens a visible terminal, finds Python 3.9+ and runs `tools/launch/launch.py` (one core for both OSes; OS differences are data), which: names any missing tool with what to install (npm, cargo, cmake — PATH, then the VS Build Tools / Homebrew dirs); runs `fetch_occt.py`'s fetch (the prefix; first use downloads); builds `cicada` in RELEASE with the SPA embedded when missing or stale (`npm ci` if `web/node_modules` is missing, `npm run build` when `web/dist` is older than a web source, `cargo build --release -p cicada-cli --features embed` when the binary is missing, older than an engine source or `web/dist`, or carries no matching `cicada.launch-stamp.json` — a binary anyone else built is rebuilt; after a build the binary is TOUCHED before it is stamped, so a file cargo does not call an input — `Cargo.lock` after a checkout, tests under `crates/` — cannot leave the rule "stale" for good with a no-op cargo build on every launch; on Windows the build's git gets `core.longpaths=true` per process through `GIT_CONFIG_COUNT`/`KEY_0`/`VALUE_0`, the oneTBB clone's MAX_PATH trap below); bundles the run-time libraries INTO `target/release/` (`fetch_occt.bundle`, idempotent); runs `cicada app` with the launcher's own arguments (none on a double-click) under an env with the loader path REMOVED and `CICADA_PYTHON` = the launcher's interpreter — in the directory the launcher was started from when it was given arguments (a relative path means what `cicada app` typed there would mean), in the repository when given none. Every failure is an `error:` line and the window stays open. `--plan` prints the plan and stops; `--no-run` builds + bundles without starting; `--launcher-help`. **`python tools/launch/bundle.py --out dist/`** makes the redistributable folder from the EXISTING release build (`--binary PATH` names another; `--cache-root` = `fetch_occt.py --dest`): `cicada.exe` + the DLLs + `Cicada.cmd` + `README.txt` on Windows; `Cicada.app/Contents/{Info.plist, MacOS/cicada, MacOS/lib/, MacOS/Cicada.command}` + `README.txt` (ASCII) on macOS (everything inside the `.app` — app translocation); idempotent (a bundled copy whose size stopped matching the recorded source is copied again). A binary that embeds no SPA — a plain `cargo build --release`, whose bundle would die at the first double-click — is REFUSED before anything is written (a static probe for two lines of `web/index.html` an `embed` build carries) unless `--allow-no-spa` asks for an engine-only bundle: its README says ENGINE ONLY and the stamp records `"spa": false`. `bundle.py --check dist/` verifies it (launcher files; the L2 stamp's libraries at their sizes; the binary's imports resolve statically; the macOS rpath; `Info.plist` naming `Cicada.command`; the binary and the stamp agreeing about the SPA; `--help` from inside the bundle under a MINIMAL env — Windows PATH = System32 alone) and `--check dist/ --smoke` adds the process-level proof (its `cicada app --no-browser` prints the URL, `/health` answers `ok`, `/` is the SPA; refused up front on an engine-only bundle). CI's Windows + macOS jobs (`ci.yml` test-cross, `nightly.yml` test-matrix) bundle their debug binary with `--allow-no-spa` and run `--check`. The bundle still needs Python 3 on the machine (the README in it says so). Tests: `tools/test_launch.py` (the smoke's assertions over an injected process and HTTP GET) |
| The OCCT seam (`cicada-geom` feature `occt`, ON by default since WP-C) | Under the env above, the seam's tests are part of `cargo test -p cicada-geom` (kernel level `src/occt/tests.rs` + the node set `src/occt/node_set_tests.rs`; the stdlib's Solid nodes and the server's display tests run in the kernel world too). The kernel-free world belongs to two crates and CI tests both: `cargo test -p cicada-geom --no-default-features` (the seam, 81 tests) and `cargo test -p cicada-stdlib --no-default-features` (the nodes: the stdlib takes cicada-geom with `default-features = false` and forwards through its own default-on `occt` feature since the WP-C review closure of 2026-08-21; every Solid node is red with the typed refusal there and `solids/support.rs`'s `with_kernel` / `expect_red` assert it — the node is reached with a pseudo solid, so the refusal is the node's own, never a fixture's). The server and the CLI take the defaults through their edges, so a workspace-level `--no-default-features` still links the kernel (`cargo tree -e features` shows it) and cicada-server's tests have no kernel-free arm (each asserts the kernel is present rather than passing vacuously). Nothing may run `--all-features`. CI: every building job fetches the prebuilt first (ci.yml `rust`/`test-cross`/`playwright-smoke`, nightly `test-matrix`/`wall-corpus`/`playwright-heavy`). Golden hashes of the canonical bytes bless via run-once (`CICADA_OCCT_DUMP=<dir>` writes the bytes). The seam's cost table: `cargo run --release -p cicada-geom --example solid_bench [parts…]` (docs/03 quotes it). The node-set glue is cicada-geom's own (`src/occt/glue.hxx` + `glue.rs`, compiled by `build.rs` with cxx-build against `DEP_OCCT_ROOT`); the fork carries the binding patches and the first glue |
| Carve benchmark (kernel seam, release only) | `cargo run --release -p cicada-geom --example carve_bench [parts]` — see skill `perf-check` |
| Wall carve (stage 6, release) | `cargo run --release -p cicada-cli -- run examples/wall/wall.cic --node carved --time --cache-dir <fresh>` (cold < 10 s; MEASURED 6.5 s). Exporters: `--node bambu --node dxf` (write to `examples/wall/out/`, gitignored) |
| Offline tests (wall scripts + normalizer) | `python -m unittest discover -s tools -p "test_*.py"` (production cross-checks skip without the wall repo) |
| Compare wall outputs to production | `python tools/normalize.py all --ours examples/wall/out --ref examples/wall/golden/production --report examples/wall/out/report.md` (verdict = exit code) |
| Regenerate the frozen wall layout | `python examples/wall/tools/extract_layout.py` then `python examples/wall/tools/recover_seeds.py` then `extract_layout.py` again (reads the wall repo; numpy for seed recovery) |
| Measurement harness (stage 6; `transport_loop.mjs` since v0.1 item 4) | `tools/measure/{carve.sh,slider_loop.mjs,esc.mjs,transport_loop.mjs}` — serve a SCRATCH copy on a private port; `CICADA_TRACE=1` on run/serve for per-node phase timings |
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
- **Mixed-age engines on one project** (since item 3b, commit 3c87387):
  the store now writes a memo-log record older engines cannot decode,
  and an engine from BEFORE that commit opened on the same project path
  drops the memo log once — it reports an "undecodable record at byte
  0" (not corruption: the newer format). Engines from that commit on
  refuse a newer store instead (`<root>/format`; `LOG_FORMAT` is 3 since
  item 3 WP-B added `StoredValue::Solid` — a value blob an older engine
  would otherwise quarantine as corruption). Stores are keyed by
  project path in the ONE user cache dir, whatever worktree the engine
  came from — serve scratch copies for experiments, as always.
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
- **Rust stable moves under CI.** `rust-toolchain.toml` says `stable`, and CI's
  `dtolnay/rust-toolchain@stable` picks a new release the day it ships —
  Rust 1.98.0 (2026-08-18) turned CI red with a new clippy lint while local
  `stable` was still 1.97.1 (fixed 2026-08-20 — `chunks_exact(N)` with a
  constant → `as_chunks::<N>()`). When CI's Linux fmt·clippy job goes red on
  a lint you do not see locally, first `rustup update stable` (or install the
  new version side by side: `rustup toolchain install 1.98.0 --profile
  minimal --component clippy --component rustfmt` and `rustup run 1.98.0
  cargo clippy …`) before touching code.
- **PowerShell execution policy blocks `npm`/`npx`** in an interactive PS
  5.1 shell (`npm.ps1 cannot be loaded because running scripts is
  disabled`). Use `npm.cmd` / `npx.cmd`, or once per user:
  `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`. Agent shells are
  unaffected (Bash), which is why this only shows up in Ben's terminal.
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
- **A worktree may hold another agent's uncommitted work.** Never clean
  by pattern — no `git clean`, no `git ls-files --others | xargs rm`, no
  `git checkout -- .` / `git stash` over files you did not touch. Delete
  only the exact artifact paths you created (a mutation run's
  `proptest-regressions/<category>/<node>.txt`, your scratch files), and
  keep everything else in the session's scratchpad directory. Learned the
  hard way (2026-08-20): a review's pattern cleanup on `wt/catalog`
  deleted the in-progress `cicada mcp` sources of a sibling package and
  they had to be replayed from that agent's transcript.
- **Fail loudly and immediately.** No silent fallbacks; a wrong answer is
  worse than a loud refusal. `unwrap`/`expect` are lint-denied in library
  code (tests exempt); `overflow-checks` stay on in release; errors are
  `thiserror` enums in libraries, `anyhow` only in `cicada-cli`;
  user-facing problems are typed diagnostics, never bare strings.
- **`unsafe` only inside FFI seam modules**, each block with a
  `// SAFETY:` comment.
- **The repository is PUBLIC (since 2026-08-24). Nothing that identifies
  this machine or its owner, and nothing secret, goes into a commit.** No
  absolute paths from a home directory — write `%LOCALAPPDATA%`,
  `%USERPROFILE%`, `$HOME`, `<repo>`, never the real value; no usernames,
  hostnames, e-mail addresses, IP addresses, tokens, API keys, cookies,
  `.mcp.json`, shell histories, screenshots of other windows, file
  listings or contents from outside the repo, and no paths into other
  projects on the machine (the wall repo's location is an environment
  variable, not a default). Probe memos, measurement logs and commit
  messages are scrubbed before they are committed (`git grep -n -i -E
  "C:\\Users|/Users/[a-z]+|@gmail|ghp_|sk-"` over the tracked files must
  stay empty); the `.gitignore`d files stay ignored. Ask Ben before
  committing anything you are unsure about — a leaked value lives in the
  history for good.
- **Never leave fmt/clippy red.** CI runs `-D warnings`; so should you.
  The per-PR clippy runs on Linux only; the 3-OS matrix that lints
  everything is the Nightly. So OS-specific behaviour is **data, not
  `cfg`-gated code** — a `#[cfg(target_os)]` table the one shared code
  path reads (`solids/support.rs::platform_golden`), never a body only
  one OS compiles: the first per-OS golden, written as a one-armed
  `match` under `cfg(target_os = "macos")`, was `clippy::single_match`
  on the Nightly for three nights (2026-08-22..24) while every per-PR
  job stayed green.
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
