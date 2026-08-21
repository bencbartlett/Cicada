# The standard library

The v1 stdlib clones the most-used slice of Grasshopper's component set —
the nodes a GH migrant reaches for daily — implemented as typed Rust
functions registered into the engine. GH names are kept wherever they
exist (Move, not Translate); one concept, one name. Every node must be
justified by the wall corpus or by common GH usage; the 500-component
long tail stays a non-goal.

**Tiers** used in the catalog below:

- **S** — the vertical-slice spike set (~20 nodes).
- **1** — v0.1 (the full mesh-tier catalog).
- **2** — v0.2 (B-rep tier, implicits, and stragglers).

## Design rules

1. **Pure typed functions.** No ambient state (no document styles, units,
   or globals — wall lesson 10); a node's output is a function of its
   inputs, period. Effectful nodes (exporters) are explicitly marked and
   run only on explicit action, never on open (wall lesson 7).
2. **Deterministic.** Stable sorts, explicit tie-breaks; every random
   node takes an explicit `seed` input. Identical inputs → byte-identical
   outputs, so caching and diffs stay meaningful.
3. **Element-wise definitions, runtime lifting.** Nodes are written over
   single elements wherever sensible; the scheduler lifts them over axes
   (`map`) with rayon parallelism and cancellation checks between
   elements. Hot homogeneous paths (point sets, transforms) drop to
   struct-of-arrays fast paths.
4. **Kind-preserving generics.** `Move(g: T, motion: Vector) → T` with
   `T: Transformable` — moving a `[Circle]` yields `[Circle]`.
5. **Refinements for data-dependent properties.** `Closed<Curve>`,
   `Planar<Curve>`, and Solid's watertight guarantee are checked wrapper
   types entered via explicit conversion nodes that go red with offending
   element IDs on failure — never silent runtime surprises.
6. **Slot-preserving nulls.** `T?` (Optional) elements survive every
   combinator; removal only via explicit `compact`/`Cull`-family nodes
   that also return an `IndexMap` (wall lesson 2).
7. **Loud refusal.** A node missing required inputs or shown invalid data
   is red with a reason and IDs; it never guesses (wall lesson 13). Every
   count that sizes an allocation shares one **two-part ceiling**, checked
   before the allocation: 2^22 = 4,194,304 slots
   (`cicada_stdlib::MAX_SLOTS`) and 1 GiB of what the count makes the node
   allocate (`MAX_BYTES`, at `bytes_per_slot` = the element the buffer
   holds PLUS whatever the node builds per slot) — whichever bites first
   is red with the count, the bytes and the ceiling in the message, never
   an allocation attempt (an unbounded `count` once aborted the whole
   engine on allocation failure, which is not a panic and so could not go
   red — C1 review; one fat slot type later the slot ceiling alone was an
   allocation the allocator may refuse, hence the byte half — v0.1
   follow-up 2; DECISIONS.md row of 2026-08-21 is the binding record).
   The two halves bound different things. The SLOT half
   bounds what a slot costs beyond the node's buffer — the value model
   hashes every slot, the memo log serialises it, zstd compresses it —
   and that cost is measured, not assumed: `series` at 2^24 slots peaked
   at 9.76 GB of working set and wrote 1.4 GB to the cache (~580 bytes a
   slot end to end, for a 128 MiB `Vec<f64>`), which is why the ceiling
   is 2^22 and not the 15112fb 2^24: at 2^22 the process peaks at
   2,478 MiB, what an 8 GB machine survives with room for the rest of
   the pipeline (the measurements live on the constant's doc comment;
   they are headless `cicada run` numbers — what `cicada serve` adds by
   encoding display frames is the frame follow-up's to measure). Because
   the cost is per slot the value model hashes, the slot half is charged
   on what the node EMITS, all outputs together: a node with several
   list outputs charges every one (`divide_curve` emits points AND
   tangents AND parameters — charged per port it admitted 3 × 2^22
   slots and measured 5,332 MiB at `count = 2^22`, 2.15× the figure the
   ceiling is justified by; charged on the total, `count = 1398100` is
   the last allowed on an open curve), and a fence-post node charges the
   length it emits (`range` emits `steps + 1`, so `steps = 4194303` is
   the last allowed). The BYTE half bounds the node's own allocation,
   per slot at what a slot really costs: `linear_array` charges each
   copy its `Transformable` AND the mesh or polyline it transforms
   (every copy is a distinct geometry; a million-vertex mesh is refused
   at 30 copies), while `duplicate`'s `Arc`-shared slots cost the slot
   alone. A count port that is exactly the emitted length of one list
   goes through `checked_count` (`series`, `random`, `repeat`,
   `duplicate`, `pad_last`, `linear_array`; and `segments` where it is
   the vertex count an allocation takes: `extrude`'s circle profile,
   `loft`'s analytic sections, `voronoi`'s circle boundary — a chain
   profile never tessellates, so its unused port is not policed); every
   other emitted total goes through `checked_size` on the derived count
   after the node's own floor (`checked_floor`): `range`'s `steps + 1`
   values, `divide_curve`'s `count + 1` (open) or `count` (closed)
   samples × 3 outputs, `sphere`'s `segments × rings` vertices (2,898
   segments is the first refused), `text_outlines` / `text_solids`: the
   text's span bound from the font's outline spans without flattening —
   a contour start or a line span is one vertex at any density, a bézier
   span `segments` — so a line-only glyph is never refused for its
   density and the bound holds by construction of the flattener. The
   floors (`count < 0`, `segments < 3`, `segments < 1`) stay where they
   were — the node's or the kernel's own message. Chunkers (`chunk`,
   `partition`, `truncate`, `split_list`) allocate no more than their
   input and need no ceiling. **Versions bump with the ceilings** when
   the newly refused band previously produced output (docs/12: any
   behaviour change; a memo hit must not serve what a cold solve refuses)
   — the fourteen nodes above went to version 2 with the 2^22 ceiling
   and the payload charge, and `range` / `divide_curve` to version 3
   when the review moved their charge to the emitted total (their
   version-2 band had been admitted by the branch's engines); 15112fb's
   2^24 landed without a bump only because its refused band had never
   produced anything (it aborted). **Every guarded node's file proves
   the order** — that the refusal precedes the allocation — with a test
   named `…refused_not_allocated` whose count no guard-after survives
   (≥ 10^10 slots: an 80 GB buffer, so a guard moved after the allocation
   aborts the test binary instead of passing); cap + 1 cases pin the
   boundary and the message and are named `…one_past_the_ceiling…` /
   `…is_red` (`tests/conformance.rs` holds the rule — the review's
   mutation found nine files whose "not allocated" tests passed with the
   guard after the allocation). The ceilings bound memory, not time: a prism or loft
   profile of a few hundred thousand `segments` passes them and is an
   O(n²) ear clip that Esc cannot interrupt today (measured: 50k
   segments 2.0 s, 100k 8.7 s, 200k 37 s) — that is the cost model's
   and the cancellable kernel worker's business (docs/12), named in
   docs/17 as a follow-up.
8. **Conversions are explicit, costed nodes** (`Tessellate`, `As Closed`,
   `As Solid`) — never silent coercions. Only total, lossless upcasts
   (Circle → Curve) are implicit.
9. **Display is an edge.** Preview/material/tag nodes feed the viewer;
   the profiler shows their cost next to compute cost.
10. **One source of truth per node.** Title, ports, docs, palette entry,
    and the AI-facing catalog all derive from the signature + attribute —
    nothing is hand-maintained twice.

## Core value model

Value kinds (each liftable into the axis layer of doc 02 — `[T]`, `T?`,
named axes like `parts: Solid`):

- **Scalars**: `Number`, `Integer`, `Boolean`, `Text`, `Color`,
  `Domain` (interval), `IndexMap` (from reordering/culling ops),
  `Table` (for the CSV surface).
- **Spatial**: `Point`, `Vector` (distinct from Point — kills a real bug
  class), `Plane`, `Xform`.
- **Geometry**: `Curve` (Line | Arc | Circle | Ellipse | Polyline |
  Nurbs | Compound), `Surface`, `Mesh`, `Solid`, `Field` (implicit/SDF,
  v0.2), `Brep` (v0.2, behind the OCCT seam).
- **Display/output**: `Material`, plus fabrication types that arrive with
  the ported exporters (`Plate`, machine profiles).

**Representations: analytic + B-rep first; meshing is
post-processing.** `Curve` and `Surface` are analytic; **`Solid` is
B-rep-backed (OCCT) from v0.1** — primitives, extrude, loft, revolve,
sweep, booleans, STEP. The mesh tier stays first-class for
mesh-destined work: `Watertight<Mesh>` is a refinement whose
constructor *is* the Manifold watertightness check, and
`Tessellate: Solid → Watertight<Mesh>` is the explicit bridge. The
typical pipeline is analytic/B-rep until the end, then tessellate once
— the GH workflow, typed. The wall corpus runs the mesh tier (FDM-bound
parts; the spike builds frusta as analytic mesh constructions and
carves with Manifold — wall lesson 6 still holds for mesh-destined
geometry).

The storage model in Rust (checker lattice is separate — see doc 02):

```rust
#[derive(Clone, Hash)]
enum Geometry { Point(Point3), Curve(Curve), Surface(Surface),
                Mesh(Mesh), Solid(Solid) /* B-rep; Field: v0.2 */ }

enum Curve { Line(Line), Arc(Arc), Circle(Circle), Ellipse(Ellipse),
             Polyline(Polyline), Nurbs(NurbsCurve), Compound(Vec<Curve>) }

trait Transformable { fn transform(&self, x: &Xform) -> Self; }
trait CurveOps { fn length(&self) -> f64; fn point_at(&self, t: f64) -> Point3;
                 fn frame_at(&self, t: f64) -> Plane; fn is_closed(&self) -> bool; }

struct Closed<C>(C);   // refinement: only constructible via checked conversion
struct Planar<C>(C);
```

Values are immutable and content-hashable (the scheduler contract
requires it); exhaustive `match` forces every stdlib node to handle or
explicitly reject every kind.

## The node registry

The node ABI is **struct-in / struct-out**: every node function takes
one input struct with named fields and returns an output struct with
named fields — this is how Rust gets named arguments, and it makes the
field names the single nomenclature used by Rust call sites, the JSON
catalog, the canvas port labels, and dialect kwargs (doc 10).

```rust
/// Move — translate geometry along a vector.
#[node(category = "Transform", tier = "S")]
fn move_<T: Transformable>(input: MoveIn<T>) -> T {
    input.geometry.transform(&Xform::translation(input.motion))
}

#[derive(Ports)]
struct MoveIn<T> {
    geometry: T,
    motion: Vector,
}

/// Divide Curve — points, tangents, and parameters along a curve.
#[node(category = "Curve", tier = "S")]
fn divide_curve(input: DivideIn) -> DivideOut { /* … */ }

#[derive(Ports)]
struct DivideIn {
    curve: Curve,
    #[port(default = 10)]
    count: Integer,
}

#[derive(Ports)]
struct DivideOut {
    points: Vec<Point>,
    tangents: Vec<Vector>,
    parameters: Vec<Number>,
}
```

`#[derive(Ports)]` reflects fields into typed ports — a field with a
default is an optional port; `#[node]` assembles the `NodeSpec`: name,
title (doc comment first line), category, ports, generic bounds,
purity, tier, and the runtime contract (the rustdoc `# Panics` section,
rendered in the catalog as "Red when: …") — registered at compile time.
Single-output nodes may return a bare value (port named `out`). The
registry serializes to a **JSON catalog** consumed by three clients: the
palette (search-to-place), the checker (wire compatibility), and the AI
(the machine-readable surface it composes nodes from).

**Kind-preserving generics, as implemented (stage 4)**: generic ports use
per-call type VARIABLES at the spec level — `T` (any transformable kind:
Point, Vector, Plane, Curve, `Closed<Curve>`, Mesh, `Watertight<Mesh>`)
and `E` (any kind; list nodes) — which the checker binds once per call
and substitutes into variable-typed outputs, so `move` of a
`Closed<Curve>` is statically a `Closed<Curve>`. At runtime one concrete
function dispatches over an erased enum; no Rust generics in node fns.
`Any` is the display-sink catch-all (absorbs any wire at any depth,
binds nothing). `Geometry` is a widening target (display sinks); nothing
narrows back out of it.

Wire-time behavior, from the same specs (doc 02): implicit widening
upcasts; offered one-click **map-lifts** (recorded in the text);
offered **checked refinements** (inserts the validating conversion
node); everything else red with a reason.

## Catalog

Signature notation: `name(port: Type, …) → (port: Type, …)`; `[T]` list
along an axis; `T?` optional; defaults shown as `= x`. Rows marked with
variants ("/") compress sibling nodes.

### 1 · Params & input

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Number Slider | `slider(value: Number, min = 0, max = 10, step = 0) → Number` | S | the GH workhorse; out-of-range value = red, never a silent clamp; scrub caching (doc 12) is a per-slider opt-in offered only when the step-quantized range has a bounded position count (2026-08-19) |
| Boolean Toggle | `toggle(value: Boolean) → Boolean` | S | |
| Literals: Number, Integer, Boolean, Text, Color, Point, Vector, Plane | `() → T` | S | bare literal bindings in the dialect ARE the constant params (doc 10 §3); no zero-input literal nodes exist |
| Value List | `() → T` | 1 | dropdown enum param |
| Cycle | `(period: Number = 4, frames: Integer = 120) → Number` | 1 | looping time 0→1, transport-driven (play/pause/speed); frame-quantized so one full loop warms the cache — subsequent loops are pure playback (docs 12, 13) |
| Clock | `(speed: Number = 1) → Number` | 1 | unbounded time 0→∞, transport-driven (play/pause/reset); deterministic per value, uncached by design |
| Panel | `(data: Any) → ()` | S | display sink; `Any` absorbs any wire at any depth (scalar or list), so one panel shows anything |

### 2 · Sequences & random

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Series | `(start: Number = 0, step: Number = 1, count: Integer) → [Number]` | S | |
| Range | `(domain: Domain, steps: Integer) → [Number]` | 1 | shipped (C1): `steps + 1` values, both ends exactly the domain's; red when `steps < 1` |
| Random | `(domain: Domain, count: Integer, seed: Integer) → [Number]` | S | explicit seed, always |
| Jitter | `(list: [E], strength: Number = 1, seed: Integer) → (list: [E], map: IndexMap)` | 1 | shipped (C1): shuffle with provenance — seeded key per slot (`splitmix64`, the same generator as Random), stable sort; strength 0 = identity |
| Repeat | `(pattern: [E], count: Integer) → [E]` | 1 | shipped (C1): GH Repeat Data — the node form of the cyclic zip policy (docs/09 calls it `cycle`; that name is the §1 time param's) |

### 3 · Maths & logic

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Add / Subtract / Multiply / Divide / Modulo / Power | `(a: Number, b: Number) → Number` | S | overloads for Vector where sensible (Vector+Vector, Vector×Number) |
| Negative / Absolute / Round / Floor / Ceiling / Min / Max | `(x: Number) → Number` / `(a, b: Number) → Number` | 1 | shipped (C1): `negative`, `absolute`, `round` (ties away from zero — stated, GH rounds to even), `floor`, `ceiling`, `min`, `max`. GH tags: `floor`/`ceiling` carry `Round` — Grasshopper has no Floor/Ceiling components, they are the Round component's F/C outputs, and the tag is what a migrant types |
| Sin / Cos / Tan / Asin / Acos / Atan / Atan2 / Radians / Degrees | `(x: Number) → Number` / `atan2(y, x)` / `radians(degrees)` / `degrees(radians)` | 1 | shipped (C1): radians throughout (angle-dimension ports); `asin`/`acos` red outside [−1, 1] |
| Square Root / Natural Logarithm / Logarithm / Exponential | `sqrt(x)` / `ln(x)` / `log(x, base = 10)` / `exp(x)` | 1 | shipped (C1) — not in the original list; Grasshopper's Maths tab staples (Square Root, Natural logarithm, Logarithm, Power of E = `exp`), which a migrant reaches for daily; red for a negative `x`, a base of 1 or below 0; `log` uses the dedicated base-10/base-2 routines (`log(1000)` is exactly `3` — a tested contract) |
| Expression | `(free variables…) → Number` | S | math syntax: write `z = x^2 + y^2` (`^` = power); `x`, `y` auto-become input ports, `z` names the output; the checker infers Integer for `+ − ×` over all-Integer inputs (so computed counts feed Integer ports), `/` and `^` always Number |
| Remap | `(value: Number, source: Domain, target: Domain) → Number` | S | the wall used this constantly |
| Construct / Deconstruct Domain | `(start, end) ↔ Domain` | S | |
| Smaller / Larger / Equals | `(a: Number, b: Number) → Boolean` / `equals(a, b, tolerance = 0)` | 1 | shipped (C1): strict `<` / `>`; `equals` is exact by default, `abs(a − b) <= tolerance` otherwise |
| And / Or / Not / Xor | `(a: Boolean, b: Boolean) → Boolean` / `not(x)` | 1 | shipped (C1) |
| Pick | `(pattern: Boolean, true: E, false: E) → E` | 1 | shipped (C1): per-element if over any kind (both branches bind one `E`; both are solved — selection is data, not control flow) |
| Mass Addition / Average / Bounds | `mass_addition(list) → (result: Number, partial: [Number])` / `average(list) → Number` / `bounds(list) → Domain` | 1 | shipped (C1): left-to-right sums (the order is the contract); `average`/`bounds` red on an empty list |

### 4 · List & axis

| Node | Signature | Tier | Notes |
|---|---|---|---|
| List Item | `(list: [E], index: Integer, wrap: Boolean = false) → E` | S | `E` binds per call: item of a `[Point]` is a Point; hole-aware since stage 6 — an absent slot selects as an absent element (`E` carries `?`) |
| List Length | `(list: [E]) → Integer` | S | counts slots, absent included (slot-preserving; `E` carries `?`, so `[T?]` is an ordinary `[E]`) |
| Reverse | `(list: [E]) → [E]` | 1 | shipped (C1) |
| Shift List | `(list: [E], offset: Integer, wrap: Boolean = true) → [E]` | 1 | shipped (C1): wrapping rotates; `wrap=false` drops what falls off (no map — a dropped end shifts no surviving index), and an `|offset|` at or past the slot count empties the list, as GH does — stated in the node's `# Returns`, not a hidden clamp |
| Sort | `(keys: [Number], values: [E]) → (keys: [Number], sorted: [E], map: IndexMap)` | 1 | shipped (C1): stable; map preserves identity; also returns the sorted keys (GH Sort List's K) |
| Cull | `(list: [E], pattern: [Boolean]) → (kept: [E], map: IndexMap)` | S | the only way elements leave a list; strict zip (counts in the error), no pattern repetition; `map` = kept index → source index |
| Dispatch | `(list: [E], pattern: [Boolean]) → (a: [E], b: [E], map_a: IndexMap, map_b: IndexMap)` | 1 | shipped (C1): strict zip; one map per half (a single map for two lists was ambiguous) |
| Weave / Merge / Insert Items / Split List | `weave(pattern: [Integer], a: [E], b: [E]) → [E]` / `insert_items(list: [E], items: [E], indices: [Integer]) → [E]` / `split_list(list: [E], index: Integer) → (a: [E], b: [E])` | 1 | slot-preserving throughout; all shipped (C1); Merge = `concat`. `weave` is the two-stream form (`0`/`1` turns; the output is the repeated pattern realized on the streams, cut where both are used up — the two lengths must be the turn counts of some prefix of the repeated pattern, a length-independent rule; a pair that does not fit is red at the first turn for an exhausted stream while the other has slots — GH pads nulls there). `insert_items` applies its insertions in order (each index addresses the list as the previous insertions left it, as GH does); no `wrap` — an index past the current length is red |
| Duplicate | `(item: E, count: Integer) → [E]` | 1 | shipped (C1): GH Duplicate Data; `count=1` is the idiomatic singleton list (geometry lists come from nodes) |
| flatten / partition / chunk / concat | `(list: [[E]]) → [E]` / `(list: [E], sizes: [Integer]) → [[E]]` / `(list: [E], size: Integer) → [[E]]` / `(a: [E], b: [E]) → [E]` | S | shipped (stage 6, the wall's per-part cutter groups): `flatten` drops one level (absent outer slots refuse, inner holes survive); `partition` sizes must cover the list exactly (counts in the error); `chunk`'s last group may be short (GH Partition List); `concat` is slot-preserving |
| map / zip / cross / nest / squeeze / transpose / group_by / compact | list & nesting combinators | S | wire-level lifts with node forms; strict `zip` (mismatch = error; the opt-in adapters are the nodes `pad_last(list, count)` / `repeat(pattern, count)` / `truncate(list, count)` — shipped C1; GH tags `Longest List` / `Repeat Data` / `Shortest List`, the components whose matching they make visible); shipped C1 as nodes: `nest` (Graft), `transpose` (Flip Matrix; rectangular only, ragged is red), `group_by(keys: [Number], values: [E]) → (groups: [[E]], keys: [Number])` (first-occurrence order), `compact(list: [E?]) → (values: [E], map: IndexMap)` (an `E?` port keeps the wired `?` on the port, so `values` types present — the checker's `compact` advice is satisfiable; C1 review fix); `cross` needs a second type variable and `squeeze`'s depth is data-dependent — both pending; doc 09 |

Path Mapper does not exist; standard combinators and named axes replace
it (doc 09).

### 5 · Point · Vector · Plane

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Construct / Deconstruct Point | `(x, y, z: Number) ↔ Point` | S | |
| Distance | `(a: Point, b: Point) → Number` | 1 | |
| Closest Point | `(point: Point, cloud: [Point]) → (closest: Point, index: Integer, distance: Number)` | 1 | kd-tree backed |
| Cull Duplicates | `(points: [Point], tolerance: Number) → (points: [Point], map: IndexMap)` | 1 | |
| Unit X / Y / Z | `(factor: Number = 1) → Vector` | S | |
| Vector 2Pt | `(a: Point, b: Point, unitize: Boolean = false) → Vector` | S | |
| Vector XYZ / Deconstruct Vector | `(x, y, z) ↔ Vector` | 1 | |
| Amplitude / Vector Length | `(v: Vector, length: Number) → Vector` / `(v) → Number` | 1 | |
| Cross / Dot / Angle | `(a: Vector, b: Vector) → …` | 1 | |
| Rotate Vector | `(vector: Vector, angle: Number, axis: Vector) → Vector` | 1 | |
| XY / XZ / YZ Plane | `(origin: Point = origin) → Plane` | S | |
| Construct Plane | `(origin: Point = origin, x: Vector = unit_x, y: Vector = unit_y) → Plane` | S | `y` is orthonormalized against `x`; red when `x` is zero-length or `y` is parallel to `x` at tolerance (stage 6: the wall's part/plate frames) |
| Plane Normal | `(origin: Point, z: Vector) → Plane` | 1 | |

### 6 · Curve

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Line | `(a: Point, b: Point) → Line` | S | |
| Line SDL | `(start: Point, direction: Vector, length: Number) → Line` | 1 | |
| Polyline | `(vertices: [Point], closed: Boolean = false) → Polyline` | S | |
| Circle | `(plane: Plane = xy_plane, radius: Number) → Closed<Curve>` | S | variants: CNR, 3Pt (tier 1); the stored frame orthonormalizes at construction |
| Arc | `(plane: Plane, radius: Number, angle: Domain) → Arc` | 1 | variants: 3Pt, SED |
| Ellipse | `(plane: Plane, r1: Number, r2: Number) → Ellipse` | 1 | |
| Rectangle | `(plane: Plane = xy_plane, x: Domain, y: Domain) → Closed<Curve>` | S | always closed; the rounded-`corner` param arrives with compound curves (v0.1); spike curve outputs type as `Curve`/`Closed<Curve>` (per-variant wire kinds like `Circle <: Curve` arrive with the full lattice) |
| Polygon | `(plane: Plane, radius: Number, sides: Integer, corner: Number = 0) → Polygon` | 1 | |
| Interpolate | `(points: [Point], degree: Integer = 3, periodic: Boolean = false) → Nurbs` | 1 | curvo-backed |
| Nurbs Curve | `(points: [Point], degree: Integer, periodic: Boolean) → Nurbs` | 1 | control points |
| Join | `(curves: [Curve], tolerance: Number) → (joined: [Curve], map: IndexMap)` | 1 | |
| Explode | `(curve: Curve) → (segments: [Curve], vertices: [Point])` | 1 | |
| End Points | `(curve: Curve) → (start: Point, end: Point)` | 1 | |
| Divide Curve | `(curve: Curve, count: Integer) → (points: [Point], tangents: [Vector], parameters: [Number])` | S | |
| Divide by Length / Distance | `(curve: Curve, length: Number) → …` | 1 | |
| Evaluate Curve | `(curve: Curve, t: Number) → (point: Point, tangent: Vector)` | 1 | |
| Curve Closest Point | `(point: Point, curve: Curve) → (point: Point, t: Number, distance: Number)` | 1 | |
| Length | `(curve: Curve) → Number` | 1 | |
| Perp Frames | `(curve: Curve, count: Integer) → [Plane]` | 1 | Horizontal Frames: tier 2 |
| Offset Curve | `(curve: Planar<Curve>, distance: Number, corners: Corner = sharp) → Curve` | 1 | cavalier_contours |
| Fillet Corners | `(curve: Polyline, radius: Number) → Curve` | 1 | 2D corner fillets |
| Shatter | `(curve: Curve, parameters: [Number]) → [Curve]` | 1 | |
| As Closed | `(curve: Curve) → Closed<Curve>` | S | checked refinement; red with the failing distance; closes an open polyline whose endpoints coincide within tolerance |
| As Planar | `(curve: Curve) → Planar<Curve>` | 1 | the `Planar` refinement is v0.1 (doc 15 scopes the spike to As Closed/As Watertight); spike Extrude checks planarity at runtime instead |

### 7 · Surface & solid

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Extrude | `(profile: Closed<Curve>, direction: Vector, segments: Integer = 64) → Watertight<Mesh>` | S | spike form: mesh-backed (doc 15's honest shim), planarity checked at runtime within tolerance (the `Planar` refinement is v0.1); `segments` tessellates curved profiles; v0.1 restores `→ Solid` |
| Extrude Open | `(curve: Curve, direction: Vector) → Surface` | 1 | |
| Extrude to Point | `(profile: Closed<Planar<Curve>>, apex: Point) → Solid` | 1 | the wall's frusta |
| Loft | `(start: Closed<Curve>, end: Closed<Curve>, segments: Integer = 64) → Watertight<Mesh>` | S | spike form (stage 6): a ruled solid between two sections, capped — the wall's frusta (Voronoi cell → triangular tip cap) and its cone/chamfer cutters; polylines pair vertex i ↔ i (seam = vertex 0), analytic sections tessellate to `segments`; red when counts differ, a section is degenerate, the sections coincide, or wind oppositely; v0.1 restores `(profiles: [Closed<Curve>]) → Solid` + `Loft Open` |
| Sweep1 | `(rail: Curve, profile: Closed<Planar<Curve>>) → Solid` | 1 | Sweep2: tier 2 |
| Revolve | `(profile: Curve, axis: Line, angle: Domain = full) → Solid` | 1 | |
| Pipe | `(rail: Curve, radius: Number) → Solid` | 1 | |
| Boundary Surface | `(boundary: Closed<Planar<Curve>>, holes: [Closed<Planar<Curve>>]?) → Surface` | 1 | |
| Box / Center Box | `(plane: Plane = xy_plane, x, y, z: Domain) → Watertight<Mesh>` | S | spike form: mesh-backed under the v0.1 name (doc 15's honest shim); decreasing domains normalize; v0.1 restores `→ Solid` |
| Sphere | `(plane: Plane = xy_plane, radius: Number, segments: Integer = 32) → Watertight<Mesh>` | S | spike form: mesh-backed UV sphere (`segments` = longitudes); v0.1 restores `→ Solid` |
| Cylinder / Cone | `(plane: Plane, radius: Number, height: Number) → Solid` | 1 | |
| Solid Union / Difference / Intersection | `(a: [Solid], b: [Solid]) → [Solid]` | 1 | OCCT B-rep booleans; known ceilings + Rhino.Compute rescue (doc 03) |
| Offset Solid | `(solid: Solid, distance: Number) → Solid` | 1 | shell/inflate, Manifold |
| Bounding Box | `(geometry: [Geometry], plane: Plane = world) → Solid` | 1 | union or per-item |
| Area | `(g: Surface \| Closed<Planar<Curve>>) → (area: Number, centroid: Point)` | 1 | |
| Volume | `(solid: Solid) → (volume: Number, centroid: Point)` | 1 | |
| Deconstruct Solid | `(solid: Solid) → (faces: [Surface], edges: [Curve], vertices: [Point])` | 1 | |
| Evaluate / Divide Surface | `(surface: Surface, uv…) → …` | 1 | Isotrim: tier 2 |
| Fillet Edge / Chamfer Edge | `(brep: Brep, edges…, radius) → Brep` | 2 | B-rep tier, OCCT |

### 8 · Mesh & field

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Mesh Box / Sphere / Plane | `(…) → Mesh` | 1 | parameterized density |
| Mesh Box / Mesh Sphere / Mesh Extrude / Mesh Loft | `mesh_box`, `mesh_sphere`, `mesh_extrude`, `mesh_loft` — the §7 S-tier signatures `→ Watertight<Mesh>` | S | the spike's mesh-backed implementations of Box/Sphere/Extrude/Loft, renamed in the SAME commit that lands the OCCT-backed `→ Solid` nodes under the bare names (DECISIONS.md 2026-08-19: B-rep is the default working mode; the wall example stays on these) |
| Construct Mesh | `(vertices: [Point], faces: [Face]) → Mesh` | 1 | faces = index triples/quads |
| Deconstruct Mesh | `(mesh: Mesh) → (vertices: [Point], faces: [Face], normals: [Vector])` | 1 | |
| Tessellate | `(solid: Solid, density…) → Watertight<Mesh>` | 1 | the explicit analytic/B-rep → mesh bridge |
| As Watertight | `(mesh: Mesh) → Watertight<Mesh>` | S | Manifold check; the mesh-tier solid |
| Mesh Union | `(meshes: [Watertight<Mesh>]) → Watertight<Mesh>` | S | Manifold; n-ary — empty list = the empty solid |
| Mesh Difference | `(mesh: Watertight<Mesh>, cutters: [Watertight<Mesh>]) → Watertight<Mesh>` | S | Manifold; the wall's carve is `mesh_difference(mesh=each(frusta), cutters=each(cutter_groups))` — explicit `each()` zip replaced the original list×list signature (typed combinators over implicit matching, doc 09) |
| Mesh Intersection | `(a: Watertight<Mesh>, b: Watertight<Mesh>) → Watertight<Mesh>` | S | Manifold; may be the empty solid |
| Weld / Smooth / Reduce | `(mesh: Mesh, …) → Mesh` | 1 | |
| Field primitives | `(…) → Field` | 2 | sphere/box/gyroid/from-mesh SDF; fidget |
| Field ops | `(a: Field, b: Field, k: Number) → Field` | 2 | union/smooth-union/offset/shell |
| Isosurface | `(field: Field, bounds: Solid, resolution: Number) → Mesh` | 2 | marching cubes; CPU first, GPU optimization later |

### 9 · Intersect & regions

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Voronoi | `(seeds: [Point], boundary: Closed<Curve>, segments: Integer = 64) → cells: [Closed<Curve>]` | S | spade; the wall's partition; cells index-aligned with seeds; spike scope: convex planar boundary (concave arrives with `i_overlay`, v0.1); `segments` tessellates curved boundaries |
| Delaunay | `(points: [Point]) → Mesh` | 1 | |
| Curve \| Curve | `(a: Curve, b: Curve) → (points: [Point], tA: [Number], tB: [Number])` | 1 | |
| Curve \| Plane | `(curve: Curve, plane: Plane) → (points: [Point], t: [Number])` | 1 | |
| Section | `(solid: Solid, plane: Plane) → [Closed<Curve>]` | 1 | |
| Contours | `(solid: Solid, plane: Plane, step: Number) → levels: [[Closed<Curve>]]` | 1 | named-axis output |
| Project to Plane | `(g: Curve \| [Point], plane: Plane) → same` | 1 | kind-preserving |
| Region Union / Difference / Intersection | `(a: [Closed<Planar<Curve>>], b: […]) → […]` | 1 | i_overlay |
| Point in Curve / in Solid | `(point: Point, region…) → Boolean` | 1 | |

### 10 · Transform

All kind-preserving over `T: Transformable`.

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Move | `(geometry: T, motion: Vector) → T` | S | |
| Rotate | `(geometry: T, angle: Number, plane: Plane) → T` | S | Rotate Axis: tier 1 |
| Scale | `(geometry: T, center: Point, factor: Number) → T` | S | Scale NU: tier 1 |
| Mirror | `(geometry: T, plane: Plane) → T` | 1 | |
| Orient | `(geometry: T, source: Plane, target: Plane) → T` | S | the wall's part-to-plate workhorse |
| Linear Array | `(geometry: T, direction: Vector, count: Integer) → [T]` | S | |
| Polar / Rectangular Array | `(geometry: T, …) → [T]` | 1 | |
| Compose Xform / Transform | `([Xform]) → Xform` / `(g: T, x: Xform) → T` | 1 | |

### 11 · Output, display & export

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Custom Preview | `(geometry: Geometry, color: Color = white) → ()` | S | display edge; cost visible in profiler; scalar port — lift with `each()` per element (per-element display cost falls out); the `Material` port returns with the real display pipeline (v0.1, with the Material node) |
| Material | `(color: Color, roughness: Number = 0.5, metallic: Number = 0) → Material` | 1 | names map to Blender shaders (doc 04) |
| Gradient | `(t: Number, stops: [(Number, Color)]) → Color` | 1 | |
| Text Tag | `(location: Plane, text: Text, size: Number) → ()` | S | display-only |
| Text Outlines | `(text: Text, size: Number, plane: Plane = xy_plane, font: Text = "DejaVu Sans Bold", segments: Integer = 8, line_gap: Number = 1.35) → [Closed<Curve>]` | S | real geometry (stage 6): glyph contours as closed polylines, baseline on the plane's x axis, `size` = CAP HEIGHT (how fabrication text is specified), `\n` stacks lines by `line_gap × size`; fonts are BUNDLED in the stdlib (reproducibility — the spike bundles DejaVu Sans Bold; system fonts never) |
| Text Solids | `(text: Text, size: Number, depth: Number, plane: Plane = xy_plane, font: Text = "DejaVu Sans Bold", segments: Integer = 8, line_gap: Number = 1.35) → [Watertight<Mesh>]` | S | one watertight solid per glyph, counters (holes) handled, extruded `depth` along the plane normal — the wall's deboss cutters; GH needed TextEntity → curves → Boundary Surfaces → Extrude + tree gymnastics for this |
| Export OBJ (debug) | `(meshes: [Mesh], path: Text) → ()` | S | the stage-4 window into headless geometry (any viewer opens it); effectful — explicit-run only, never memoized; takes ANY meshes (a debug viewer must not demand watertightness) |
| Export STL / 3MF (plain) | `(solids: [Solid], path: Text) → ()` | 1 | explicit-run |
| Export 3MF (Bambu project) | `(plates: [Plate], path: Text) → ()` | 1 | ported wall writer; brings `Plate` types |
| Export DXF | `(layers…, path: Text) → ()` | 1 | ported wall writer; datum discipline |
| Table / Export CSV | `(ids: [Text], columns…) → Table` / `(table: Table, path: Text) → ()` | 1 | the manifest surface (doc 04) |

## Deliberately absent

- **Path Mapper, flatten/graft/simplify port toggles, implicit tree
  matching** — standard combinators and named axes replace the entire
  mechanism (docs 02 & 09).
- **Data Dam, triggers, and GH's Timer** (the ambient re-solve hack) —
  scheduling is the runtime's job and nothing fires on an ambient
  clock. `Cycle`/`Clock` transport-driven params are the sanctioned
  replacement: pure values fed by an explicit player, so determinism
  and caching survive animation.
- **Latching state (GH-style gates that persist ON)** — runs are explicit
  actions (wall lesson 7).
- **Kangaroo-tier physics, Galapagos-tier solvers** — ecosystem
  non-goals for v1; script nodes are the pressure valve.
- **Clusters** — subgraph/composite nodes are a v0.2 design question
  (they interact with the dialect and the differ); script nodes cover
  the encapsulation need until then.

## Open questions (not yet locked)

- **Units**: v0.1 numbers are unitless with a document-level unit tag
  consumed by exporters; typed units (mm vs in) in the checker are a
  candidate for v0.2.
- **Subgraph nodes** (user-defined composites) and how they serialize in
  the dialect.
- **Expression node scope**: how large the mini-language gets before it
  should just be a script node.
- **Fabrication types** (`Plate`, machine profiles): namespaced with the
  exporters or promoted to core kinds.
- ~~**Font resolution determinism** for Text Outlines (bundle fonts vs
  system lookup — reproducibility says bundle).~~ Resolved stage 6:
  fonts are bundled in the stdlib binary and named by the `font` port;
  a name that is not bundled is red with the bundled list. Which fonts
  ship beyond DejaVu Sans Bold is a v0.1 catalog question, not a design one.
