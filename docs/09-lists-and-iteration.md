# Lists and iteration: replacing the tree

Grasshopper's "data tree" is nested lists wearing positional path
addresses (`{0;1;2}`) with implicit per-component matching. Cicada keeps
nested data and discards the vocabulary, the implicit matching, and the
hidden state. A list is a list (`[T]`); a "tree" is just nesting
(`[[T]]`, `[[[T]]]`), optionally with named levels. Standard programming
terms transfer; path voodoo doesn't.

## Terminology mapping

| Grasshopper | Cicada | Notes |
|---|---|---|
| Data tree | Nested list (`[[T]]`, …) | just data, no special machinery |
| Branch | Sublist | |
| Path `{0;1}` | Plain indices — or a named level (`parts`) | no positional address language |
| Item / List / Tree access | Port types `T` / `[T]` / `[[T]]` | declared, checked, visible |
| Graft | `nest` | each element becomes a singleton sublist |
| Flatten (all levels, always) | `flatten` (one level) / `flatten_all` | says what it does |
| Simplify (hidden toggle) | `squeeze` (explicit node/chip) | drops singleton levels, visibly |
| Flip Matrix | `transpose` | |
| Path Mapper | typed combinators (`transpose`, `chunk`, `group_by`, `nest`, `flatten`) | total, documented operations |
| Longest-list matching | gone — strict `zip` + opt-in policies | |
| Cross reference | `cross` | explicit Cartesian product |
| Principal parameter | gone | pairing is never inferred from a blessed input |

## The documented pain (what we're fixing)

Researched from the GH community plus the wall project's production
failures:

1. **Cargo-cult structure surgery.** Users graft/flatten/simplify
   "chaotically … to the point of accidentally achieving the desired
   result" — that's from a *tutorial site teaching the tool*
   ([bimcorner](https://bimcorner.com/6-rules-how-to-work-with-grasshopper-data-tree/)).
   The mental model is so weak that trial-and-error is the documented
   working method.
2. **Path Mapper is write-only.** "Perhaps the least intuitive to use
   and can cause a loss of data"
   ([Rhino developer docs](https://developer.rhino3d.com/guides/grasshopper/gh-algorithms-and-data-structures/advanced-data-structures/));
   entire forum threads exist to decode it
   ([Dear Path Mapper](https://discourse.mcneel.com/t/dear-path-mapper/99186),
   [Understanding path mapper](https://discourse.mcneel.com/t/understanding-path-mapper/93660)).
   Worse, its mappings are *positionally coupled*: if upstream structure
   changes, the mapping silently stops meaning what it meant.
3. **Longest-list matching.** The shorter list's last item is silently
   repeated — plausible-but-wrong pairings that surface as wrong
   geometry, not errors.
4. **Hidden per-port state.** Flatten/graft/simplify toggles live in
   context menus, invisible on the canvas; two identical-looking
   definitions behave differently.
5. **Per-component tree conventions.** Some components graft their
   outputs, some don't; structure surprises ripple downstream and get
   "fixed" by more surgery (pain 1).
6. **Empty/missing branches break pairing** — branch-order alignment
   silently shifts (wall lesson 3).
7. **Item-access port fed a list re-runs the whole component per item**
   — the wall's 66× export (wall lesson 1).
8. **Null-dropping flatteners shift every later index** — the wall's
   nastiest bug class (wall lesson 2).

## The Cicada model

- **Types carry depth.** `T`, `[T]`, `[[T]]` are distinct wire types.
  Wire styling keeps GH's familiar convention — single line for a
  scalar, double for a list, hatched (with a depth number) for nested —
  but backed by the checker instead of vibes.
- **Levels may be named** (axes, doc 02): `parts: Solid` means "one
  Solid per part." Naming is optional for quick work and encouraged for
  long pipelines; cross-axis mistakes become type errors.
- **Iteration is a lift, and it is visible.** Drop a `Move` (expects
  `T: Transformable`) onto a `[Point]` output: the connection completes
  only through an offered `map` chip, the input port wears a persistent
  **`map` badge** (`×2` when mapping two levels deep), and the lift is
  recorded in the dialect text. Nothing iterates silently — GH's
  invisible implicit loop is the single largest source of
  wrong-but-plausible output.
- **Pairing rules, in order:**
  - **Scalar closes over a map.** One `motion: Vector` against 1,500
    geometries is the common case and needs no list semantics at all.
  - **Two lists pair by strict `zip`.** Length mismatch is a red wire
    with the counts in the error ("1,499 vs 1,500"), not a guess.
  - **GH-style behaviors exist only as opt-in, visible adapters** — the
    nodes `pad_last(list, count)` (the longest-list emulation),
    `repeat(pattern, count)` (the cyclic policy; named `cycle` until it
    shipped on 2026-08-20 — the §1 time param owns that name),
    `truncate(list, count)` (the shortest-list emulation).
  - **All-pairs is `cross`**, never inferred.
- **Empties and nulls are values.** `[]` is data, not a pruned branch;
  `T?` slots are preserved by every combinator; slots leave a list only
  through `cull` (by pattern) and `compact` (the absent ones), each
  returning an `IndexMap` so identity survives (docs/08 §4).
- **Structure changes fail at the wire.** If an upstream edit changes
  nesting depth, downstream lifts become type errors immediately —
  not runtime weirdness three components later, and never a silently
  re-aimed Path Mapper.

## Combinator inventory

| Combinator | Type | Replaces (GH) | Notes |
|---|---|---|---|
| `map` | `(T → U) over [T] → [U]` | implicit iteration | offered as chip; port badge |
| `zip` | pairing semantics: multiple `each()` on one call (doc 10) | longest-list matching | strict lengths; mismatch = error with counts |
| `pad_last` / `repeat` / `truncate` | length-policy adapters: `(list, count) → [E]` / `(pattern, count) → [E]` | Longest List / Repeat Data / Shortest List | explicit, visible, recorded; shipped C1 (the cyclic one was `cycle` until 2026-08-20) |
| `cross` | `(a: [A], b: [B]) → (a: [[A]], b: [[B]])` | Cross Reference | two aligned outputs — no tuple wires |
| `flatten` | `[[T]] → [T]` | Flatten | one level; `flatten_all` for all, and says so |
| `nest` | `[T] → [[T]]` | Graft | each element its own singleton; shipped C1 |
| `squeeze` | drop singleton levels | Simplify | explicit node, not hidden toggle |
| `transpose` | `[[T]] → [[T]]` | Flip Matrix | rectangular only, ragged is red; shipped C1 |
| `chunk` / `partition` | `[T] → [[T]]` | Partition List | by size / by sizes |
| `group_by` | `(keys: [Number], values: [T]) → (groups: [[T]], keys: [Number])` | Path Mapper folklore | the honest version of most path recipes; shipped C1 with numeric keys (first-occurrence order, exact compare) — text keys wait for a second type variable, like `cross` |
| `compact` | `[T?] → ([T], IndexMap)` | Clean Tree | how absent slots disappear (`cull` is the other exit, by pattern); shipped C1 — an `E?` port keeps the wired `?` on the port, so `values` types present |
| `concat` / `wrap` / `unzip` | list assembly/disassembly | Merge / Entwine / Explode Tree | `concat` shipped (stage 6) |

Shipped in the spike (stage 6, for the wall's per-part cutter groups):
`flatten` (one level; absent outer slots refuse, inner holes survive),
`partition(list, sizes)` (sizes must cover the list exactly — counts in
the error), `chunk(list, size)` (last group may be short), `concat`,
and `cull(list, pattern)` (strict zip, no pattern repetition; returns
`kept` + the `IndexMap`). The element variable `E` carries optionality:
a `[T?]` flows through every slot-preserving list node as `[T?]`, and
`item` of an absent slot is an absent element — never green at check
time and red at run time.

## Path Mapper recipes, typed

| Classic recipe | Cicada |
|---|---|
| `{A;B} → {B;A}` | `transpose` |
| `{A;B} → {A}` | `flatten` (one level) |
| `{A} → {A;i}` | `nest` |
| Trim/simplify paths | `squeeze` |
| Renumber/regroup gymnastics | `chunk` / `group_by` |

Every recipe becomes a named, total, documented operation — and when
upstream depth changes, these fail loudly at the wire instead of
silently remapping.

## UI contract

- **Blocked wires**: a type-incompatible connection cannot be completed;
  during drag, incompatible ports are dimmed with a reason on hover.
- **Adapter chips**: liftable mismatches (`[Curve]` → `Curve`-port,
  `Curve` → `Closed<Curve>`-port) connect only through the offered chip;
  the adapter is recorded in text and visible on the wire forever.
- **Iteration badges**: every lifted port shows `map` (with `×depth`),
  zipped groups are bracketed visually; nothing iterates without a
  badge.
- **Hover the wire, see the pairing**: counts on both sides, which
  policy is active, and the first few paired samples — the Param
  Viewer's job, in place, from cache.

Chip vocabulary, precisely: a **chip** is a compact UI element on a
wire or port, smaller than a node box. A **lift chip** is the visual
form of `each()` — pure syntax, no node created; it persists as the
port's iteration badge. An **adapter chip** is a real, single-use
adapter node (`as_closed`, `pad_last` with its `count`, `tessellate`, …)
rendered compactly on the wire; it has a binding in the text, can be promoted to
a full box, and carries cost and status like any node. The line between
them: lift chips change the *pairing semantics* of an existing call;
adapter chips are *data transformations*, and data transformations are
nodes (doc 10).

## Open questions (not yet locked)

- How deep the auto-offered lift goes (`map ×2` seems like the sane
  ceiling; deeper asks for an explicit combinator).
- Ragged nesting (`[[T]]` with uneven sublists) is legal data; named
  axes probably only attach to rectangular levels — confirm when the
  axis syntax lands in the dialect spec.
- Whether same-named axes auto-align in `zip` (xarray-style, by name
  rather than position) — attractive, deferred until axes are in use.
