# Cicada

**A code-first parametric design tool: the pipeline is a typed program, the
node graph is a generated view, and an AI collaborator works on the whole
program — not islands of nodes.**

Cicada is a successor-in-spirit to Grasshopper for procedural geometry and
fabrication work, born from a full production run of a 1,500-part
multi-machine 3D-printed wall piece built in Grasshopper — and from every
hour that project lost to Grasshopper's failure modes.

## Thesis in five sentences

1. **Code is the source of truth; the graph is a view.** The pipeline file's
   top layer is a restricted dataflow dialect — one binding per node — that
   *is* the graph; node bodies are ordinary code the canvas never renders.
2. **Shape-typed wires make Grasshopper's worst bug class unrepresentable.**
   `Curve`, `[Curve]`, and `parts: Curve` (named axes) are distinct types;
   broadcasting is always explicit (`map`, `zip`, `flatten`), never implicit
   longest-list guessing.
3. **Feel-fast is a scheduler property, not a language property.**
   Content-hash caching, minimal recompute, parallel execution, and
   cancellation everywhere — a solve you can always Esc out of.
4. **Geometry kernels are rented, never built.** Manifold for meshes, OCCT
   for B-rep + STEP, fidget for implicits, behind one typed seam.
5. **The AI writes and refactors typed functions with stored prompts and
   attached property tests** — vibe-coded, but continuously verified.

## Documents

| Doc | Contents |
|---|---|
| [DECISIONS.md](DECISIONS.md) | The living ledger of locked design decisions |
| [docs/01-use-cases.md](docs/01-use-cases.md) | Who this is for and what it must do |
| [docs/02-architecture.md](docs/02-architecture.md) | File format, type system, scheduler, script nodes, interaction model |
| [docs/03-geometry-stack.md](docs/03-geometry-stack.md) | Build vs. rent: kernels, libraries, and the seam between them |
| [docs/04-rendering-and-interop.md](docs/04-rendering-and-interop.md) | Real-time viewer, Blender bridge, exchange formats, .gh import |
| [docs/05-roadmap.md](docs/05-roadmap.md) | The vertical-slice spike, v0.1/v0.2 scope, explicit deferrals |
| [docs/06-prior-art.md](docs/06-prior-art.md) | What to steal and what to avoid, project by project |
| [docs/07-lessons-from-the-wall.md](docs/07-lessons-from-the-wall.md) | Every Grasshopper failure class from the wall project, mapped to the Cicada mechanism that kills it |
| [docs/08-standard-library.md](docs/08-standard-library.md) | The stdlib: design rules, data model, node registry, and the full v1 catalog |
| [docs/09-lists-and-iteration.md](docs/09-lists-and-iteration.md) | Lists instead of trees: pain-point research, combinators, pairing rules, iteration badges |
| [docs/10-dialect-and-file-format.md](docs/10-dialect-and-file-format.md) | The dialect grammar, canvas round-trip contract, layout sidecar, and git integration |
| [docs/11-ai-integration.md](docs/11-ai-integration.md) | The AI editing loop: read tools, checker-driven iteration, refactor primitives, permission tiers |
| [docs/12-scheduler.md](docs/12-scheduler.md) | Scheduler internals: values and hashing, cache keys, solve generations, execution, cancellation, persistence |
| [docs/13-app-architecture.md](docs/13-app-architecture.md) | Engine server ⇄ browser protocol: sessions, sync, binary frames, undo, security |
| [docs/14-engineering-foundations.md](docs/14-engineering-foundations.md) | Workspace, representations, tolerances, script ABI, testing, CI, agent operating standards |
| [docs/15-spike-plan.md](docs/15-spike-plan.md) | The vertical-slice spike: stages, definitions of done, measurement protocol, kill criteria |
| [docs/16-ui.md](docs/16-ui.md) | UI contracts: layout, canvas/viewport conventions, inspector, keyboard map |

## Status

Pre-v0. **The vertical-slice spike is complete and the empirical gate
passed.** The wall pipeline's hardest stretch — the 1,200-part
magnetic-field pyramid wall: field solve → Voronoi → frusta → debossed
labels → pin-hole carve → pack → Bambu 3MF + DXF — runs end to end on the
Rust engine + web UI stack ([DECISIONS.md](DECISIONS.md) web-first row) and
reproduces the shipped fabrication files modulo declared noise. All five
[measurement criteria](docs/15-spike-plan.md#stage-6-results-measured) were
met on the dev machine (i7-13700KF): the full labeled carve in **6.5 s**
cold (the wall's Rhino carve was ~30 min), a cheap-cone slider at
**0.5 ms** p50, **Esc to idle in 170 ms**, **file-edit → canvas in ~100 ms**,
and byte-comparable 3MF/DXF exports. The corpus and its measurement harness
live in [`corpus/`](corpus/README.md). Next is v0.1 (OCCT-backed Solid, the
full catalog, WASM script host, undo, git panel — [docs/15](docs/15-spike-plan.md)
§After the spike); the working status lives in [AGENTS.md](AGENTS.md).

License: all rights reserved while private; a public license will be
chosen at first public release.
