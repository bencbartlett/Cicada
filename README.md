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

## Status

Docs-first, pre-v0. The founding documents synthesize design conversations
from the wall-piece project (2026). The first code milestone is the
[vertical-slice spike](docs/05-roadmap.md): the wall pipeline's hardest
stretch running on the Rust + Tauri stack, measured.

License: TBD (private repository).
