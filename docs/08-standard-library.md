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
   count port (`series`, `random`, `range`, `repeat`, `duplicate`,
   `pad_last`, `linear_array`, `divide_curve`) shares one **slot ceiling**,
   2^24 = 16,777,216 slots (`cicada_stdlib::MAX_SLOTS`): a larger count is
   red with the count in the message, never an allocation attempt — an
   unbounded `count` once aborted the whole engine on allocation failure,
   which is not a panic and so could not go red (C1 review). It is a slot
   ceiling, not a memory bound; geometry `segments` ports are mesh
   resolution and keep their own contracts.
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
| Extrude | `(profile: Closed<Curve>, direction: Vector) → Solid` | S | **shipped 2026-08-20 (WP-C), OCCT-backed**: exact edges for every curve kind — a polyline/rectangle is a prism of planar faces, a circle an exact cylinder (no `segments`: the mesh tier's `mesh_extrude` tessellates); planarity, simplicity and the direction-leaves-the-plane check are the mesh tier's, word for word, before the kernel sees anything; oblique prisms are legal |
| Extrude Open | `(curve: Curve, direction: Vector) → Surface` | 1 | |
| Extrude to Point | `extrude_to_point(profile: Closed<Curve>, apex: Point) → Solid` | 1 | **shipped 2026-08-20 (WP-C)**: `ThruSections` from the profile's wire to the apex vertex — a pyramid over a polyline, a cone over a circle; red when the apex lies in the profile plane |
| Loft | `(profiles: [Closed<Curve>], ruled: Boolean = true) → Solid` | S | **shipped 2026-08-20 (WP-C), OCCT-backed** `BRepOffsetAPI_ThruSections` as a solid through two or more sections: `ruled = true` (the default — GH Loft's "Straight", the wall's frusta and the mesh tier's behaviour) makes straight surfaces between consecutive sections, `false` one smooth B-spline through all of them (GH's "Normal"); sections are made compatible first (edge counts matched, orientations and seams aligned), so polylines of different vertex counts loft; red below two profiles, for a degenerate/non-planar/self-intersecting profile (named by index), for coincident sections. `Loft Open` (a surface) waits for the `Surface` kind |
| Sweep1 | `sweep(rail: Curve, profile: Closed<Curve>) → Solid` | 1 | **shipped 2026-08-20 (WP-C)**: `BRepOffsetAPI_MakePipeShell`, corrected-Frenet trihedron, mitred (`RightCorner`) transitions at a polyline rail's corners, closed into a solid; the profile sits where the user put it (no contact/correction) — place it at the rail's start, normal to it, for the classic sweep. Sweep2: tier 2 |
| Revolve | `(profile: Closed<Curve>, axis: Curve, angle: Domain = full turn) → Solid` | 1 | **shipped 2026-08-20 (WP-C)**: `axis` is a `Line` curve (the `line` node — there is no separate `Line` kind; anything else is red) lying in the profile's plane; the profile may touch the axis (a disc) but never cross it; `angle` is radians — `start` rotates the result into place and a negative sweep turns the other way (one rigid kernel transform), at most a full turn; closed profiles only (`Revolve` of an open curve is a surface: waits for `Surface`); GH: Revolution |
| Pipe | `(rail: Curve, radius: Number) → Solid` | 1 | **shipped 2026-08-20 (WP-C)**: a circle of `radius` normal to the rail at its start, swept as `sweep` does — a straight rail is a cylinder, a circle rail a torus |
| Boundary Surface | `(boundary: Closed<Planar<Curve>>, holes: [Closed<Planar<Curve>>]?) → Surface` | 1 | |
| Box / Center Box | `box(plane: Plane = xy_plane, x, y, z: Domain) → Solid` | S | **shipped 2026-08-20 (WP-C), OCCT-backed** (`BRepPrimAPI_MakeBox` in the plane's frame; decreasing domains normalize; the world-frame box is byte-identical to the seam's `box_at`). The spike's mesh-backed box continues as `mesh_box` (§8). Center Box: tier 1, not yet |
| Sphere | `(plane: Plane = xy_plane, radius: Number) → Solid` | S | **shipped 2026-08-20 (WP-C), OCCT-backed**: one exact spherical face, the plane's z the polar axis; the mesh-backed UV sphere continues as `mesh_sphere` (§8) |
| Cylinder / Cone | `cylinder(plane: Plane = xy_plane, radius: Number, height: Number) → Solid` / `cone(plane, radius, height) → Solid` | 1 | **shipped 2026-08-20 (WP-C)**: standing on the plane, centred at its origin, rising along its normal; the cone's apex is `height` up (a frustum is `loft` of two circles) |
| Solid Union / Difference / Intersection | `solid_union(solids: [Solid]) → Solid` / `solid_difference(solid: Solid, cutters: [Solid]) → Solid` / `solid_intersection(a: Solid, b: Solid) → Solid` | 1 | **shipped 2026-08-20 (WP-C)**: the `mesh_*` booleans' shapes (n-ary union in ONE general-fuse pass; one solid minus a cutter list in one cut; two-operand common), every result followed by `ShapeUpgrade_UnifySameDomain` (coplanar faces and collinear edges merged — two fused blocks are one six-face box); a `Solid` is always ONE body, so disjoint unions, cuts that split or empty the solid and empty intersections are red — never compounds, never an empty solid (the mesh tier has the empty solid; the B-rep tier does not). OCCT's known ceilings + the Rhino.Compute rescue hatch (doc 03) |
| Offset Solid | `(solid: Solid, distance: Number) → Solid` | 1 | shell/inflate, Manifold |
| Bounding Box | `(geometry: [Geometry], plane: Plane = xy_plane) → Solid` | 1 | **shipped 2026-08-20 (WP-C)**: one box around everything in the list (per item: lift with `each()` over singleton lists); exact on points, polylines, rectangles, circles and meshes, a solid's bounds from the kernel's faces (`BRepBndLib::AddOptimal`, moved into the frame first when the plane is not the world); the box is a `Solid`, so flat geometry (no extent along a frame axis) is red |
| Area | `(g: Surface \| Closed<Planar<Curve>>) → (area: Number, centroid: Point)` | 1 | |
| Volume | `(solid: Solid) → (volume: Number, centroid: Point)` | 1 | **shipped 2026-08-20 (WP-C)**: `BRepGProp::VolumeProperties`, adaptive (exact on planar/quadric faces, 1e-9 relative elsewhere) |
| Deconstruct Solid | `(solid: Solid) → (faces: [Surface], edges: [Curve], vertices: [Point])` | 1 | |
| Evaluate / Divide Surface | `(surface: Surface, uv…) → …` | 1 | Isotrim: tier 2 |
| Fillet Edge / Chamfer Edge | `(brep: Brep, edges…, radius) → Brep` | 2 | B-rep tier, OCCT |

### 8 · Mesh & field

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Mesh Box / Sphere / Plane | `(…) → Mesh` | 1 | parameterized density |
| Mesh Box / Mesh Sphere / Mesh Extrude / Mesh Loft | `mesh_box(plane = xy_plane, x, y, z: Domain) → Watertight<Mesh>` · `mesh_sphere(plane = xy_plane, radius, segments: Integer = 32) → Watertight<Mesh>` · `mesh_extrude(profile: Closed<Curve>, direction: Vector, segments: Integer = 64) → Watertight<Mesh>` · `mesh_loft(start: Closed<Curve>, end: Closed<Curve>, segments: Integer = 64) → Watertight<Mesh>` | S | **renamed 2026-08-20 (WP-C)** — the spike's mesh-backed implementations of Box/Sphere/Extrude/Loft, moved here in the SAME commit that landed the OCCT-backed `→ Solid` nodes under the bare names (DECISIONS.md row 42: B-rep is the default working mode); outputs byte-identical to before (the wall's carve hash unchanged); GH names: Mesh Box, Mesh Sphere, and — honestly — Extrude and Loft for the two GH reaches only via Extrude/Loft + Mesh Brep; the wall example (`mesh_loft`), 03-voronoi and 04-field (`mesh_extrude`, `mesh_sphere`) stay on these |
| Construct Mesh | `(vertices: [Point], faces: [Face]) → Mesh` | 1 | faces = index triples/quads |
| Deconstruct Mesh | `(mesh: Mesh) → (vertices: [Point], faces: [Face], normals: [Vector])` | 1 | |
| Tessellate | `(solid: Solid, deflection: Number = 0.01, angle: Number = 0.1) → Watertight<Mesh>` | 1 | **shipped 2026-08-20 (WP-C)**: the explicit B-rep → mesh bridge — OCCT's mesher at an absolute chord deviation (`deflection`, document units) and angular deviation (`angle`, radians), per-face vertices welded, checked watertight AND accepted by Manifold, so the result feeds `mesh_union`/`mesh_difference`/the exporters directly; red below the kernel's floors (1e-7 / 1e-12 rad — the mesher refuses anything finer), when the mesher fails or leaves a boundary, when Manifold refuses. GH: Mesh Brep. The display path tessellates on its own (docs/03 §Display tessellation) — this node is for geometry that continues on the mesh tier |
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
