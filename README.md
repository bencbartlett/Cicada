# Cicada

> ## ⚠️ Work in progress
>
> Cicada is pre-release software under active, daily development. Nothing
> here is stable yet: the `.cic` dialect, the node catalog, the protocol,
> the file formats and the UI all change without notice, and there are no
> releases, no installers and no support. The repository is public so the
> work can be followed, not because it is ready to use. If you build it
> anyway, expect rough edges and read `AGENTS.md` for how it is run.

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
and byte-comparable 3MF/DXF exports. The wall — example, playground, and
the first member of the nightly regression corpus — lives in
[`examples/wall/`](examples/wall/README.md); the normalizer and the
measurement harness in [`tools/`](tools/). Next is v0.1 (OCCT-backed Solid, the
full catalog, WASM script host, undo, git panel — [docs/15](docs/15-spike-plan.md)
§After the spike); the working status lives in [AGENTS.md](AGENTS.md).

## Run it

Cicada is source-only for now: you build it, and a launcher does the
building for you. You need Rust stable ([rustup](https://rustup.rs)),
Node.js 20+, Python 3.9+ (the engine's script host uses it at launch —
`CICADA_PYTHON`, else `python` / `python3` on PATH), cmake and a C++
toolchain (Windows: the Visual Studio Build Tools; macOS: the Xcode command
line tools). Then:

- **Windows**: double-click `tools\launch\Cicada.cmd`.
- **macOS**: double-click `tools/launch/Cicada.command` (the first time,
  right-click → Open).

A terminal window opens and stays — it is the server console. The launcher
fetches the pinned OpenCASCADE prebuilt on first use (`tools/fetch_occt.py`,
into your user cache dir), builds the engine in release with the app
embedded when it is missing or stale (the first build compiles the
geometry kernel and takes about ten minutes; later launches are seconds),
puts the kernel's run-time libraries beside the binary so no environment
variable is ever needed, and runs `cicada app`: the server on `127.0.0.1`
with a session token, and the app window in Edge or Chrome (app mode) or in
your default browser. Ctrl-C in the console stops it. Arguments go to
`cicada app` — `Cicada.cmd examples\02-solids.cic` opens that pipeline.

The same steps by hand, and everything else the binary does, are in
[AGENTS.md](AGENTS.md)'s command palette. `python tools/launch/bundle.py
--out dist/` turns an existing release build with the app embedded
(`--features embed`; a build without it is refused unless you ask for an
engine-only bundle with `--allow-no-spa`) into a redistributable folder
— the engine with its libraries, a double-clickable `Cicada.cmd` /
`Cicada.app` and a `README.txt` that states what the machine still needs
(Python 3) — and `bundle.py --check dist/` verifies it.

### Agents: the catalog and the checker over MCP

`cicada mcp` serves the node catalog and the `.cic` checker to any Model
Context Protocol client — the read tools of
[docs/11](docs/11-ai-integration.md): `catalog_search`, `node_doc`,
`list_categories`, `check` (typecheck + dry lowering, no geometry). Build
the binary once (`cargo build -p cicada-cli`), then for Claude Code copy
[`.mcp.json.example`](.mcp.json.example) to `.mcp.json` at the repo root
(gitignored): it starts `${CARGO_TARGET_DIR:-target}/debug/cicada mcp
--project examples` — Claude Code expands the variable, so it follows
your target dir (other clients take the literal path instead); swap
`--project` for your own project directory or `.cic` file so its
`scripts/*.py` join the catalog. `cargo run -q -p cicada-cli -- mcp` also
works as the command, but only with a warm target dir and cmake on PATH:
a cold `-p cicada-cli` context rebuilds the Manifold kernel for minutes,
longer than any MCP client waits for a server to start.

License: all rights reserved while private; a public license will be
chosen at first public release.
