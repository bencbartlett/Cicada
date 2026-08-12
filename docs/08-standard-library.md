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
   is red with a reason and IDs; it never guesses (wall lesson 13).
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

**Solid in v0.1 is a watertight mesh** — a refinement of `Mesh` whose
constructor *is* the Manifold watertightness check. The `Brep` kind lands
in v0.2 with explicit `Tessellate: Brep → Mesh` conversion. This keeps
the mesh-native default of doc 03 honest in the type system.

The storage model in Rust (checker lattice is separate — see doc 02):

```rust
#[derive(Clone, Hash)]
enum Geometry { Point(Point3), Curve(Curve), Surface(Surface),
                Mesh(Mesh), Solid(Solid) /* Field, Brep: v0.2 */ }

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
purity, tier — registered at compile time. Single-output nodes may
return a bare value (port named `out`). The registry serializes to a
**JSON catalog** consumed by three clients: the palette
(search-to-place), the checker (wire compatibility), and the AI (the
machine-readable surface it composes nodes from).

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
| Number Slider | `() → Number` | S | min/max/step/precision on node; the GH workhorse |
| Literals: Number, Integer, Boolean, Text, Color, Point, Vector, Plane | `() → T` | S | typed constant params |
| Value List | `() → T` | 1 | dropdown enum param |
| Cycle | `(period: Number = 4, frames: Integer = 120) → Number` | 1 | looping time 0→1, transport-driven (play/pause/speed); frame-quantized so one full loop warms the cache — subsequent loops are pure playback (docs 12, 13) |
| Clock | `(speed: Number = 1) → Number` | 1 | unbounded time 0→∞, transport-driven (play/pause/reset); deterministic per value, uncached by design |
| Panel | `(data: [Any]) → ()` | S | display sink; shows counts + samples |

### 2 · Sequences & random

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Series | `(start: Number = 0, step: Number = 1, count: Integer) → [Number]` | S | |
| Range | `(domain: Domain, steps: Integer) → [Number]` | 1 | inclusive ends |
| Random | `(domain: Domain, count: Integer, seed: Integer) → [Number]` | S | explicit seed, always |
| Jitter | `(list: [T], strength: Number, seed: Integer) → (list: [T], map: IndexMap)` | 1 | shuffle with provenance |
| Repeat | `(pattern: [T], count: Integer) → [T]` | 1 | |

### 3 · Maths & logic

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Add / Subtract / Multiply / Divide / Modulo / Power | `(a: Number, b: Number) → Number` | S | overloads for Vector where sensible (Vector+Vector, Vector×Number) |
| Negative / Absolute / Round / Floor / Ceiling / Min / Max | `(x: Number…) → Number` | 1 | |
| Sin / Cos / Tan / Asin / Acos / Atan2 / Radians / Degrees | `(x: Number) → Number` | 1 | |
| Expression | `(free variables…) → Number` | S | math syntax: write `z = x^2 + y^2` (`^` = power); `x`, `y` auto-become input ports, `z` names the output |
| Remap | `(value: Number, source: Domain, target: Domain) → Number` | S | the wall used this constantly |
| Construct / Deconstruct Domain | `(start, end) ↔ Domain` | S | |
| Smaller / Larger / Equals | `(a: Number, b: Number, tolerance…) → Boolean` | 1 | |
| And / Or / Not / Xor | `(a: Boolean, b: Boolean) → Boolean` | 1 | |
| Pick | `(pattern: Boolean, true: T, false: T) → T` | 1 | per-element if |
| Mass Addition / Average / Bounds | `(list: [Number]) → Number / Domain` | 1 | reducers |

### 4 · List & axis

| Node | Signature | Tier | Notes |
|---|---|---|---|
| List Item | `(list: [T], index: Integer, wrap: Boolean = false) → T` | S | |
| List Length | `(list: [T]) → Integer` | S | |
| Reverse | `(list: [T]) → [T]` | 1 | |
| Shift List | `(list: [T], offset: Integer, wrap: Boolean = true) → [T]` | 1 | |
| Sort | `(keys: [Number], values: [T]) → (sorted: [T], map: IndexMap)` | 1 | stable; map preserves identity |
| Cull | `(list: [T], pattern: [Boolean]) → (kept: [T], map: IndexMap)` | 1 | the only way elements leave a list |
| Dispatch | `(list: [T], pattern: [Boolean]) → (a: [T], b: [T], map: IndexMap)` | 1 | |
| Weave / Merge / Insert Items / Split List | — | 1 | slot-preserving throughout |
| Duplicate | `(item: T, count: Integer) → [T]` | 1 | |
| map / zip / cross / flatten / nest / squeeze / transpose / chunk / group_by / compact | list & nesting combinators | S | wire-level lifts with node forms; strict `zip` (mismatch = error; `pad_last`/`cycle`/`truncate` opt-in); `compact` returns `IndexMap`; doc 09 |

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
| Construct Plane | `(origin: Point, x: Vector, y: Vector) → Plane` | 1 | |
| Plane Normal | `(origin: Point, z: Vector) → Plane` | 1 | |

### 6 · Curve

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Line | `(a: Point, b: Point) → Line` | S | |
| Line SDL | `(start: Point, direction: Vector, length: Number) → Line` | 1 | |
| Polyline | `(vertices: [Point], closed: Boolean = false) → Polyline` | S | |
| Circle | `(plane: Plane, radius: Number) → Circle` | S | variants: CNR, 3Pt (tier 1) |
| Arc | `(plane: Plane, radius: Number, angle: Domain) → Arc` | 1 | variants: 3Pt, SED |
| Ellipse | `(plane: Plane, r1: Number, r2: Number) → Ellipse` | 1 | |
| Rectangle | `(plane: Plane, x: Domain, y: Domain, corner: Number = 0) → Rectangle` | S | always closed |
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
| As Closed / As Planar | `(curve: Curve) → Closed<Curve> / Planar<Curve>` | S | checked refinements; red with IDs on failure |

### 7 · Surface & solid

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Extrude | `(profile: Closed<Planar<Curve>>, direction: Vector) → Solid` | S | |
| Extrude Open | `(curve: Curve, direction: Vector) → Surface` | 1 | |
| Extrude to Point | `(profile: Closed<Planar<Curve>>, apex: Point) → Solid` | 1 | the wall's frusta |
| Loft | `(profiles: [Closed<Curve>]) → Solid` | 1 | `Loft Open` for surfaces; explicit `zip over parts` idiom for base/cap pairing |
| Sweep1 | `(rail: Curve, profile: Closed<Planar<Curve>>) → Solid` | 1 | Sweep2: tier 2 |
| Revolve | `(profile: Curve, axis: Line, angle: Domain = full) → Solid` | 1 | |
| Pipe | `(rail: Curve, radius: Number) → Solid` | 1 | |
| Boundary Surface | `(boundary: Closed<Planar<Curve>>, holes: [Closed<Planar<Curve>>]?) → Surface` | 1 | |
| Box / Center Box | `(plane: Plane, x, y, z: Domain) → Solid` | S | |
| Sphere | `(plane: Plane, radius: Number) → Solid` | S | |
| Cylinder / Cone | `(plane: Plane, radius: Number, height: Number) → Solid` | 1 | |
| Solid Union / Difference / Intersection | `(a: [Solid], b: [Solid]) → [Solid]` | S | Manifold; the wall's carve in seconds |
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
| Construct Mesh | `(vertices: [Point], faces: [Face]) → Mesh` | 1 | faces = index triples/quads |
| Deconstruct Mesh | `(mesh: Mesh) → (vertices: [Point], faces: [Face], normals: [Vector])` | 1 | |
| Tessellate | `(solid: Solid \| Brep, density…) → Mesh` | 1 | the explicit conversion |
| As Solid | `(mesh: Mesh) → Solid` | 1 | watertight refinement check (Manifold) |
| Weld / Smooth / Reduce | `(mesh: Mesh, …) → Mesh` | 1 | |
| Field primitives | `(…) → Field` | 2 | sphere/box/gyroid/from-mesh SDF; fidget |
| Field ops | `(a: Field, b: Field, k: Number) → Field` | 2 | union/smooth-union/offset/shell |
| Isosurface | `(field: Field, bounds: Solid, resolution: Number) → Mesh` | 2 | marching cubes; CPU first, GPU optimization later |

### 9 · Intersect & regions

| Node | Signature | Tier | Notes |
|---|---|---|---|
| Voronoi | `(seeds: [Point], boundary: Closed<Planar<Curve>>) → cells: [Closed<Polyline>]` | S | spade; the wall's partition |
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
| Custom Preview | `(geometry: [Geometry], material: Material) → ()` | S | display edge; cost visible in profiler |
| Material | `(color: Color, roughness: Number = 0.5, metallic: Number = 0) → Material` | 1 | names map to Blender shaders (doc 04) |
| Gradient | `(t: Number, stops: [(Number, Color)]) → Color` | 1 | |
| Text Tag | `(location: Plane, text: Text, size: Number) → ()` | S | display-only |
| Text Outlines | `(text: Text, font: Text, size: Number, plane: Plane) → [Closed<Curve>]` | 1 | real geometry; ttf-parser; the wall's labels |
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
- **Font resolution determinism** for Text Outlines (bundle fonts vs
  system lookup — reproducibility says bundle).
