# AI integration: the editing loop

Doc 02 §6 states the model — the AI is a collaborator over the whole
program. This document specifies the mechanics: what an agent sees,
what it may do, and how the feedback loop stays fast and cheap. AI
features aren't the build focus yet, but the substrate is designed for
them now so nothing needs retrofitting.

## Three modes

1. **Compose** — "sweep these base curves along these lines, caps at
   50% scale" → the agent inserts stdlib bindings into the `.cic` file.
2. **Author** — a capability the stdlib lacks → the agent writes a
   script node (Rust→WASM by default) with a contract test, plus the
   binding that instantiates it.
3. **Automate** — the mode this doc exists for: direct edits to the
   pipeline text for otherwise-tedious changes ("insert a scale stage
   between every profile and its loft", "rename the `cells` axis to
   `regions` everywhere", "re-route all exports through the new
   packer").

All three produce the same artifact: a text diff. The canvas is
generated from text, so an AI edit appears on canvas exactly like a
human edit — one substrate, no separate "AI layer" to drift.

## Why the dialect is agent-friendly

The restrictions in doc 10 were chosen for humans and the canvas, but
they are precisely the properties that make agent editing cheap:

- **Order-independence kills the ordering problem.** The file is a DAG
  with forward references legal; insertion point is *semantically
  irrelevant*. Agent guidance is one line — "append new bindings at
  the end" — and `cicada fmt` / the writer beautify later. No tokens
  are ever spent reasoning about topological order.
- **Single assignment** — every name introduced exactly once;
  references are unambiguous; a diff's meaning is local.
- **Kwargs-only** — no positional-arity confusion; an unknown or
  missing kwarg error names the exact field.
- **No nested calls** — one line = one node = one diagnostic site.
- **No ambient state** — an edit's blast radius is exactly its
  downstream cone, which the engine can compute and report.
- **Tiny grammar** — the entire dialect spec fits in a few hundred
  tokens of system prompt; there is no language-lawyer tail to get
  confused by.

## The loop

```
propose diff
  → parse + typecheck          (milliseconds; no geometry solves)
  → JSON diagnostics with suggested fixes
  → iterate until green
→ present diff for review      (tier-dependent; see below)
→ apply → incremental solve of the dirty cone (everything else cached)
→ per-wire data summaries + contract results + red nodes
→ agent confirms intent or iterates
```

Two loops, sharply separated by cost:

- **The inner loop is the checker** — parse + shape/axis/type check in
  milliseconds with zero geometry computed. Diagnostics are structured
  (kind, span, expected/actual types, suggested fix: "wrap in
  `each()`", "insert `as_closed`", "name the intermediate"), the same
  machine-readable output rustc-style tooling taught everyone to
  consume. Nearly every mistake dies here, before anything solves.
- **The outer loop is the solve** — after apply, only the dirty cone
  recomputes (doc 02 scheduler). The agent gets back what the wire
  inspector shows humans: counts, bounds, samples per new wire, plus
  contract outcomes with counterexamples. "Did the filter actually
  drop ~half the parts?" is answered by data, not vibes.

## Read tools (grounding, not guessing)

Agents query the engine instead of inferring state from text. The
catalog and checker tools shipped in v0.1 as **`cicada mcp`** — a Model
Context Protocol server over stdio (the `rmcp` SDK; register it with
`.mcp.json.example` at the repo root, or point any MCP client at
`cicada mcp [--project <dir-or-pipeline>]`). It serves the SAME data as
`/api/catalog` (the server's renderer) and runs THE checker
(`cicada_server::compile::check_source`, the function `cicada run` and
the live session call) — never a second copy of either. Every tool's
description tells the model when to use it; every refusal is a
structured tool error (`{error, message, …}`) the model can read, never
an opaque protocol error; stdout carries nothing but JSON-RPC.

- `catalog_search {query, category?, limit?}` — ranked retrieval over
  the node catalog (docs/08): every query word scores against the
  dialect name, title, Grasshopper component name (`gh` — migrants
  search by the component they know), port names and description;
  returns name, title, gh, category and the one-line signature per hit
  — scoped retrieval instead of dumping the specs into context. An
  empty query lists the catalog; `category` scopes it.
- `node_doc {name}` — one node's full spec: the `/api/catalog` node
  object (title, description, category, tier, version, `pure`,
  `uses_tolerance`, `panics` = the red-when contract, `gh`, runnable
  `examples`, every input and output port with type / default / doc /
  dimension) plus `signature` and `effectful`. An unknown name returns
  the checker's own did-you-mean.
- `list_categories` — the ribbon tabs with node counts.
- `check {text | path}` — parse + typecheck in milliseconds, no
  geometry: `ok`, the doc-11 diagnostics (kind, node, span, message,
  expected/actual, fix with a machine-applicable `replacement` when the
  fix is a pure splice), and the binding names. `path` resolves against
  the `--project` directory; a file's own `scripts/` joins its catalog.

With `--project`, the project's Python script nodes join the catalog
(the server's discovery, re-run whenever `scripts/*.py` change on
disk). The server is read-only by construction — edits land through
the running app's atomic `POST /api/edit/apply_text` (doc 13).

Still to come, over the live session (they need a running solve, so
they belong to the app's server rather than a stdio process):

- `what_feeds(node)` / `who_consumes(node)` — the dependency cone.
- `wire_type(wire)` / `wire_summary(wire)` — checker type + cached
  data summary (counts, bounds, samples).
- `profile(node)` — compute + display cost from the profiler (powers
  "why is this slow").
- `diagnostics()` — current red nodes and errors of the open pipeline.

## Refactor primitives (mechanical ops as tools)

For bulk changes, hand-editing thirty lines burns tokens and risks
typos. The engine exposes the same AST-safe operations the canvas
uses, and agents invoke them by name:

- `rename(binding | axis | script)` — updates every reference and the
  sidecar key atomically.
- `insert_between(wire, call)` — splice a stage into a wire.
- `extract_script(bindings…)` — collapse a run of bindings into a
  generated script node, rewired.
- `delete_with_reroute(binding)` — remove a pass-through stage.

The landing pad for every agent edit is the engine's atomic `batch`
operation (doc 13, v0.1): whole new text + base hash + label + actor,
refused when stale or unparsable, otherwise one op / one delta — an
agent never produces a partial state. Raw text edits remain the
surface for creative changes; primitives are
guaranteed-correct for the tedious ones. ("Rename cells → regions
everywhere" is one primitive call, not an editing session.)

## Permission tiers

Every AI change lands as a git diff — the git-first-class UI (doc 10)
shows AI edits like any others, and per-node history records the
generating prompt. Review friction is tiered, user-configurable:

- **Additive, checker-green** (new bindings, new script nodes, new
  contracts, accepted lifts): may auto-apply; always visible in the
  diff panel. Default in v1: still shown as a proposed diff.
- **Guarded — always reviewed**: deletions, renames, script signature
  changes, rewires of existing stages, and **param value changes** —
  the human's tuned slider values are data, not style; an agent never
  silently retunes them.
- **Forbidden**: executing effectful nodes (exports run only by
  explicit human action — wall lesson 7 applies to agents doubly),
  touching cache/lock files, editing outside the project.

## Provenance

- Prompted edits record the prompt in the commit message.
- Authored script nodes carry the prompt in their file header (doc 10)
  with contracts colocated, so "same intent, but make the caps
  triangular" has history to refine against.

## Open questions (not yet locked)

- Whether auto-apply for green additive edits should be on by default
  once trust is established, or remain opt-in per project.
- Sub-agent parallelism (several stages authored concurrently) —
  single-writer semantics on the `.cic` file are trivially safe, but
  merge UX for parallel script authoring needs thought.
- Whether `extract_script` should also be offered in reverse
  (explode a script node into stdlib bindings when the AI recognizes
  the pattern).
