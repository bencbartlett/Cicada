# The dialect and file format

The pipeline file's top layer is the graph (doc 02). This document
specifies that layer: its grammar, its round-trip contract with the
canvas, the layout sidecar, and the git integration. The governing
constraints, in order:

1. **Faithful**: every construct maps 1:1 to a graph object; nothing in
   the file is invisible on canvas, nothing on canvas is absent from the
   file (layout aside).
2. **Diffable**: git diffs are small, local, and reviewable; a slider
   drag is a one-token diff. Diffability is a product feature with UI,
   not a byproduct (see Git integration below).
3. **Editable by all three hands**: human in a text editor, canvas
   drag-and-drop, AI patch — all producing the same shapes of edit.
4. **Boring to parse**: total error recovery (a broken statement reds
   one node, not the file), no lookahead heroics.

## One language

The engine, the stdlib, and default script nodes are Rust; that is the
committed core. The dialect itself is **Cicada's own minimal grammar** —
deliberately tiny: bindings and expressions only, nothing else. Its
kwargs map 1:1 onto the node ABI's input-struct fields (docs/08): the
same names appear in the Rust source, the JSON catalog, the canvas
labels, and the text. The file is still not a Rust source file — for
semantic reasons, not syntax ones: the dialect is order-independent
(forward references are legal), `^` means power, axis annotations
exist, and — decisively — a miswired graph must produce domain-quality
errors ("`[Curve]` into `Curve` — accept a map lift?"), never rustc
trait diagnostics. A tree-sitter grammar and editor extension ship
with v0.1 — the grammar is small enough that first-party tooling beats
borrowed highlighting.

Python exists in exactly one place: an optional **Python 3 script
node** (like Grasshopper's), running full CPython in a subprocess for
ecosystem access (numpy/scipy/rhino3dm). It is an option at the edges,
never the core representation.

## File anatomy

```
wall.cic              # the pipeline: the dialect, nothing else
wall.cic.layout.json  # layout sidecar: manual overrides only
scripts/
  carve_pins.rs       # script node (Rust → WASM, the default)
  solve_field.py      # Python 3 script node (CPython subprocess)
.cicada-cache/        # opt-in project-local cache (default: user cache dir, doc 12)
wall.cic.lock         # local last-solve state (gitignored)
```

First line is a version pragma:

```
# cicada 1
```

## The worked example

```python
# cicada 1
# wall.cic — each binding is a node; this IS the canvas

amps = slider(value=12.0, min=0.0, max=30.0, step=0.5)
seeds = scatter(region=board, count=1500, seed=7)

field = solve_field(coil=coil, samples=samples, current=amps)
cells: parts = voronoi(seeds=seeds, boundary=board)

frusta = frustum(profile=each(cells), direction=each(field.dirs), height=each(heights))
labels = ids(cell=each(cells))
labeled = deboss(solid=each(frusta), text=each(labels))

cutters = pin_cutters(cell=each(cells), field=field)
carved = carve(solid=each(labeled), cutter=each(cutters))   # parts: Solid?

plates = pack(parts=carved, machines=machines)
dxf = export_dxf(outline=board_outline, path="out/board.dxf")
```

## Statement forms

The dialect allows exactly these statements — anything else (control
flow, imports, `def`, classes, bare calls) is a parse error with a
pointed message ("multi-statement logic goes in a script node; loops
are `each()`").

### 1 · Bindings — one node per statement

```python
name = node_fn(port=value, other=value2)
```

- **All arguments are keyword arguments.** Every port has a label in
  the UI, so positional args buy nothing; kwargs make every line
  self-documenting and diff-stable. Ports are marked **required or
  optional** in the node spec; required kwargs must appear, omitted
  optionals take defaults. Each kwarg is a field of the node's input
  struct (the node ABI, docs/08): required = a field without a default.
  The canvas may *display* ports in any user-chosen order — that is a
  sidecar UI mapping and never changes the text.
- **Single assignment**: a name binds once per file; rebinding is an
  error. Names are the node identity (canvas labels, sidecar keys,
  backward-picking reports).
- **No nested node calls.** `carve(solid=labeled, cutter=pin_cutters(…))`
  is an error: "name the intermediate." Every node is nameable,
  inspectable, and cacheable because every node has a binding.
- **Order-independent**: forward references are legal; the semantics
  are the DAG (cycles are errors). The writer appends new nodes after
  their last dependency so files read top-down.
- **Multi-output**: unpack in spec order
  (`points, tangents, t = divide_curve(curve=rail, count=40)`) or bind
  one name and select ports (`d.points`). Port selection (`field.dirs`)
  is a reference, not a node.
- **Disabled bindings**: prefixing a statement with `#off ` disables
  the node. To generic tools it is a comment; Cicada parses the
  binding, renders the node ghosted with its wiring intact, and skips
  it in solves. Downstream nodes go red with the precise reason
  ("fed by disabled node `frusta`") — never unknown-name. Re-enabling
  is usually free: content addressing means the prior results are
  still cached.

### 2 · Lifts — `each()` marks iteration on the argument

```python
moved = move(geometry=each(pts), motion=motion)        # map: motion closes over
labeled = deboss(solid=each(frusta), text=each(labels))  # zip: strict, same axis
deep = smooth(mesh=each(each(contours)))               # map ×2
```

`each(x)` is syntax, not a node — it belongs to the call's pairing
semantics (doc 09). Multiple `each` on one call is a strict zip; the
checker verifies lengths and axis names.

**Chips, defined** (the UI vocabulary used throughout these docs):

- A **chip** is a compact UI element rendered on a wire or port,
  smaller than a node box.
- A **lift chip** is the visual form of `each()`. Accepting it wraps
  the argument in the text; *no node is created*. It persists as the
  port's iteration badge; clicking it shows the pairing (depth, axis,
  counts).
- An **adapter chip** is a real, single-use adapter node (`as_closed`,
  `pad_last` with its `count`, `tessellate`, …) rendered compactly on the
  wire instead of as a box. Accepting it inserts a named binding in
  the text; it can be promoted to a full box, and it carries cost and
  status like any node.

The line between them: lift chips change the *pairing semantics* of an
existing call; adapter chips are *data transformations*, and data
transformations are nodes.

### 3 · Params — canvas widgets are constructor bindings

```python
amps = slider(value=12.0, min=0.0, max=30.0, step=0.5)   # Number Slider
show = toggle(value=True)                                 # Boolean Toggle
mode = choice(value="fast", options=["fast", "exact"])    # Value List (shipped 2026-08-24, catalog C2b)
count = 40                                                # bare literal = constant
```

Same catalog entries as docs/08 §1. **Dragging a slider rewrites the
one numeric literal in place** (shortest round-trip repr) — a slider
drag is a one-token diff.

**Scrub caching is a kwarg too** (v0.1 item 5, 2026-08-24; docs/12
§Speculative warming): `amps = slider(value=12.0, min=0.0, max=30.0,
step=0.5, scrub=True)` opts the slider into idle-time warming of its
step positions. The TEXT carries the opt-in, never the sidecar — it is
part of the model, diffable and reviewable like any kwarg — and the
canvas toggle (`set_scrub`) writes or removes that one kwarg. A slider
is eligible only while `min`, `max` and `step` are literals, `step > 0`
and the range has at most `SCRUB_MAX_POSITIONS` positions (32 at v0.1;
DECISIONS.md row 39 owns the number); a hand-written `scrub=True` on
any other slider is legal text that warms nothing (the canvas says why).

### 4 · Expressions — operator RHS is an Expression node

```python
h_total = base_h + cap_h * 0.5
z = x^2 + y^2          # ^ is power (math convention); ** also accepted
```

An operator-expression RHS is **one** Expression node; free variables
are its input ports in order of first appearance, the bound name is
the output. The expression language is language-neutral math syntax —
the same language as the canvas Expression node editor.

### 5 · Script nodes — sibling files, same namespace

User code beyond a formula lives in `scripts/` (doc 02 §4): **Rust by
default** — including AI-generated nodes — compiled to sandboxed WASM;
a **Python 3 script node** is available for ecosystem work. Script
files self-register by function name (`#[node]` in Rust,
`@cicada.node(title, description, effectful=False)` in Python, where the
string-annotated signature is the ports and the return annotation names
the outputs — `-> "T"`, `-> {"a": "T", …}`, or `-> None` for exporters;
docs/14 §script ABI); the dialect calls them like any stdlib node. Resolution
order: local bindings → project `scripts/` → stdlib; a collision is an
error demanding qualification (`std.move` / `scripts.carve_pins`). AI
prompt provenance and contract tests live in the script file itself as
structured header metadata — visible, diffable, traveling with the
code.

### 6 · Axis annotations

```python
cells: parts = voronoi(seeds=seeds, boundary=board)   # name the axis `parts`
frusta: parts[Solid] = frustum(…)                     # full form, for docs
```

The annotation names (or renames) the binding's element axis;
inference fills everything omitted; node specs may declare default
axis names (Voronoi's `cells`).

### 7 · Effectful nodes

Exporters bind like anything else but **never auto-run** (wall lesson
7): solving computes up to their inputs; executing the export is an
explicit action (button on the node / `cicada run wall.cic --node dxf`).
Run state lives in the lock file, never in the text. Relative paths in
their literals resolve against the pipeline's directory (both `cicada
run` and `cicada serve` work from it), never against the shell's cwd.

## Round-trip contract (canvas ⇄ text)

Every canvas gesture is specified as a text edit:

| Gesture | Text edit |
|---|---|
| Place node | Append `name = fn(…)` after last dependency (or EOF); auto-name `fn_1`/`fn_2`-style — a binding never takes the bare callable name, which would shadow it for later calls (§5 resolution order); renameable |
| Draw wire | Rewrite one kwarg in the target binding |
| Delete wire | Remove that kwarg (with one adjacent separator); a required port left unwired reds the node — the honest state of "this wire is gone" |
| Accept lift chip | Wrap that kwarg's value in `each(…)` |
| Accept adapter chip | Insert adapter binding (`outline_c = as_closed(curve=outline)`) + rewire |
| Drag slider | Rewrite one numeric literal |
| Type a literal into an unconnected port (the canvas / inspector chip, wave 4 B3) | Rewrite that kwarg's literal — or, when the call lacks the kwarg (a just-placed node's port: `place` writes `name = fn()`), insert `port=literal` at its spec-order position, as a wire is inserted (`construct_domain()` → `construct_domain(end=40.0)` → `construct_domain(start=0.0, end=40.0)`); one token either way, one op. A literal inside `each(…)` is rewritten INSIDE it — `start=each(1.0)` → `start=each(2.0)`, the lift stays (a wire, by contrast, replaces the whole value; the lift is the probe's to offer again) |
| Delete node | Delete its statement; downstream references become red unknown-name errors — **never cascade deletion** |
| Toggle disable | Prefix / unprefix the statement with `#off ` |
| Rename node | Rename binding + all references + sidecar key, atomically |
| Reorder ports / move / group / recolor / toggle preview / collapse or expand a slider (the `collapsed` override, wave 4 B4) | Sidecar only; text untouched |
| Scrub-cache a slider / stop (`set_scrub`, v0.1 item 5) | Insert `scrub=True` at its spec-order position (after `step`), or rewrite it; off REMOVES the kwarg with one adjacent separator — the default says the same thing and the file stays as it was written. One token either way, one op, undoable |
| The GH slider shortcut `1<20` / `0.0<0.5<1.0` in search-to-place (wave 4 B4) | Place node + its four literals in spec order, as ONE op: `slider_1 = slider(value=1.0, min=1.0, max=20.0, step=1.0)` |

Writer discipline:

- **Minimal edits**: the writer touches only the statements a gesture
  implies; it never reformats, reorders, or realigns existing lines.
- **Deterministic emission**: same graph → same text. Standard
  formatting only — single spaces, stable float repr, kwargs in spec
  order, newline at EOF. No alignment padding; `cicada fmt` normalizes
  spacing and (opt-in) topological order.
- **Comments survive**: a comment block attaches to the following
  binding and renders as its canvas note.

Node **identity for layout** is the binding name; **identity for
caching** is content hashes (code + inputs) — a rename moves the box
without invalidating a single cached result.

## The layout sidecar

Auto-layout is primary: deterministic, snappy (<10 ms for hundreds of
nodes), and good enough to trust. The sidecar stores **manual
overrides only** — a node that was never hand-moved has no entry, so
the file stays near-empty and near-auto by construction. Deleting an
override (or the file) snaps back to auto-layout; nothing but
aesthetics is ever at stake.

```json
{
  "version": 1,
  "overrides": {
    "carved": { "cell": [34, 7], "color": "#7a5c3f", "collapsed": true,
                "port_order": ["cutter", "solid"], "preview": false }
  },
  "groups": [
    { "title": "Carve stage", "members": ["cutters", "carved"],
      "collapsed": false }
  ],
  "views": { "bookmarks": [] }
}
```

- **Grid-native geometry**: the canvas has a unit grid, where one unit
  is the port-row pitch. Node sizes are integer units; movement snaps
  to the grid by default. No alignment or snapping tools needed —
  everything is always aligned. (Revisable if it chafes.)
- Coordinates (`cell`) are integer grid cells, not pixels.
- Port display order is a per-node override — a pure UI mapping over
  the kwargs, never a text change.
- Unknown keys are preserved (forward compatibility); reviewers and
  the differ ignore the file.

## Git integration, first-class

Because the writer emits one binding per line and edits minimally,
**line diffs are node diffs** — the canvas can render any git diff as
a graph overlay with no extra machinery. The app exposes this
directly:

- **Status strip**: current branch, dirty files, per-node change
  markers (added / modified / renamed since a chosen ref).
- **Visual graph diff**: pick a ref (working vs HEAD, branch, commit)
  → added nodes green, removed nodes ghosted red, param changes shown
  on the node as `12.0 → 14.5`, wire changes highlighted. It is a
  rendering of the text diff, so it can never disagree with `git
  diff`.
- **Commit from the app**: stages pipeline + scripts + sidecar
  together, prompts for a message.
- **Per-node history**: the binding line's log (`git log -L`
  underneath) plus its script file's history — "when did this stage
  last change, and what prompt produced it."
- Plain git underneath, no custom store: the repo stays fully usable
  from any shell or tool. Branching/merging stay in your shell for
  v0.1.

*Slice 1 (v0.1 item 2, server half shipped 2026-08-20 — doc 13 §HTTP
surface `/api/git/*`): the status strip's data (branch / detached /
unborn / upstream ahead-behind / `index.lock`), per-node markers for
**working tree vs HEAD only** — computed FROM `git diff -U0 HEAD --
<pipeline>` (hunks → binding lines → names: `added` / `modified` /
`removed` / `renamed` when a removed+added pair **within one hunk** has a
byte-identical right-hand side — the writer's `rename` gesture rewrites
one line, so a rename is always one hunk; a binding deleted here and an
unrelated one with the same literal added elsewhere are two hunks and
read as `removed` + `added`), so they are the text diff by construction —
commit from the app (scope = this pipeline's `.cic` + sidecar +
`scripts/*.py`, ignored files left out as git itself leaves them out,
message verbatim, never `add -A`), and revert-to-HEAD through the reload
barrier, under the session's write hold so an edit racing the revert
lands on the reverted text. Everything runs through the `git` binary with
`--no-optional-locks` (a status refresh never touches the project). A
merge / rebase / cherry-pick / revert the shell left unfinished is a
state (`operation`) and refuses commit and revert from the app — finish
it where you started it. Not yet: other refs, the visual graph-diff
overlay, per-node history — they follow once the markers have had weeks
of use.*

Determinism (doc 02) is what makes this worth surfacing: if a diff is
empty, the artifacts are identical; if artifacts differ, some diff
explains it.

## Reproducibility and the lock file

Content hashes + determinism guarantee: same code, same engine → same
bytes. "Same engine" is the part git alone can't pin — a later Cicada
or kernel version may legitimately produce different-but-valid output.
Resolution: **exported artifacts and manifests are stamped with engine,
stdlib, and kernel versions** (the wall workflow already treats
manifests as fabrication records). Reproducing a run = install the
stamped version, rerun, verify hashes. The lock file stays local and
gitignored — it records last-solve state, not truth.

## Errors (all red-before-solve)

- Rebinding a name; unknown name (with did-you-mean); cycles.
- Positional arguments ("ports are named; write `port=value`").
- Nested node calls ("name the intermediate").
- Control flow, imports, `def` ("multi-statement logic goes in a
  script node").
- Arity, type, axis-name, and zip-length mismatches (from the
  checker).
- Future-version pragma ("this file needs a newer Cicada").

A statement that fails to parse reds **its node** and anything
downstream; the rest of the file solves normally.

## Deferred, explicitly

- **Multi-file pipelines / `use`** and composite (subgraph) nodes —
  v0.2, together (the same namespace design problem).
- **Comment conventions promoting to canvas groups** — v1 groups live
  in the sidecar.
- **First-class node references** (higher-order nodes) — `each()` and
  the combinator nodes cover v1.

## Open questions (not yet locked)

- Expression node port order under edits: order of first appearance is
  stable, but adding a new leading variable reorders ports — accept,
  or pin order in an annotation?
- `choice()` param typing: string-valued enums v1 (shipped so in catalog
  C2b, 2026-08-24: `choice(value: Text, options: [Text]) → Text`); typed
  enum values (e.g., a `Plane` choice) when a real need appears.
