//! Geometry value types (stage 4): analytic curves, `SoA` meshes, the
//! B-rep [`Solid`] (v0.1 item 3), and the two spike refinement wrappers.
//!
//! Geometry VALUE types live in core — `ValueData` is core's, and the
//! dependency law says geom depends on core, never the reverse. What lives
//! here is pure data plus the structural predicates the marshalling layer
//! needs (`is_closed`, `is_watertight`). Constructive and tolerance-aware
//! operations (tessellation, triangulation, kernel seams, transforms) live
//! in `cicada-geom`.
//!
//! Representation follows the ledger (DECISIONS.md rows 41–42): curves stay
//! analytic — tessellation is a derived op, never the stored form; meshes
//! are flat structure-of-arrays `f64` position buffers plus u32 triangle
//! indices in `Arc`s, hashed at construction like every value; a solid IS
//! its kernel's canonical bytes (row 42, revised 2026-08-20) — core never
//! links the kernel, so the value holds the bytes and nothing else.

use std::fmt;
use std::sync::Arc;

use crate::scalar::Domain;
use crate::spatial::{Plane, Point};

/// A straight segment between two points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    /// Start point.
    pub a: Point,
    /// End point.
    pub b: Point,
}

/// A vertex chain. `closed` marks the implicit final edge back to
/// `vertices[0]`; the closing vertex is NOT duplicated in `vertices`.
#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    /// The vertices, in order.
    pub vertices: Vec<Point>,
    /// Whether an implicit edge joins the last vertex back to the first.
    pub closed: bool,
}

/// An analytic circle: `plane.origin` is the center, the radius sweeps the
/// plane's x/y axes. Constructors in `cicada-geom` normalize the plane;
/// the value stores what it is given (like [`Plane`] itself).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle {
    /// The circle's frame; origin = center.
    pub plane: Plane,
    /// The radius.
    pub radius: f64,
}

/// An analytic axis-aligned rectangle in a plane: corners span `x` × `y`
/// in plane coordinates. Always closed (docs/08).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rectangle {
    /// The rectangle's frame.
    pub plane: Plane,
    /// Extent along the plane's x axis, in plane coordinates.
    pub x: Domain,
    /// Extent along the plane's y axis, in plane coordinates.
    pub y: Domain,
}

/// The curve sum type (docs/08 §Core value model, spike subset): analytic
/// variants only — Arc/Ellipse/Nurbs/Compound arrive with v0.1.
#[derive(Debug, Clone, PartialEq)]
pub enum Curve {
    /// A straight segment.
    Line(Line),
    /// A vertex chain.
    Polyline(Polyline),
    /// An analytic circle.
    Circle(Circle),
    /// An analytic rectangle.
    Rectangle(Rectangle),
}

impl Curve {
    /// Structural closedness — the `Closed<Curve>` refinement predicate.
    /// Exact (flag/variant based), no tolerance: closing an almost-closed
    /// polyline within tolerance is `as_closed`'s job (cicada-geom).
    #[must_use]
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Line(_) => false,
            Self::Polyline(p) => p.closed,
            Self::Circle(_) | Self::Rectangle(_) => true,
        }
    }

    /// Variant name for diagnostics (`Line`, `Polyline`, …).
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Line(_) => "Line",
            Self::Polyline(_) => "Polyline",
            Self::Circle(_) => "Circle",
            Self::Rectangle(_) => "Rectangle",
        }
    }
}

/// Why a mesh refused construction. All structural — NaN refusal happens at
/// value construction like every float, and watertightness is a refinement
/// (`Watertight<Mesh>`), not a construction invariant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MeshError {
    /// `positions.len()` is not a multiple of 3.
    #[error("mesh positions length {len} is not a multiple of 3 (x,y,z per vertex)")]
    RaggedPositions {
        /// The offending length.
        len: usize,
    },
    /// `indices.len()` is not a multiple of 3.
    #[error("mesh indices length {len} is not a multiple of 3 (triangles only)")]
    RaggedIndices {
        /// The offending length.
        len: usize,
    },
    /// An index points past the last vertex.
    #[error("mesh index {index} out of range (vertex count {vertices})")]
    IndexOutOfRange {
        /// The offending index value.
        index: u32,
        /// The vertex count.
        vertices: usize,
    },
    /// A triangle repeats a vertex — degenerate by construction, always a
    /// producer bug; refused loudly rather than poisoning downstream
    /// booleans.
    #[error("mesh triangle {triangle} repeats a vertex (indices {a}, {b}, {c})")]
    DegenerateTriangle {
        /// Triangle ordinal.
        triangle: usize,
        /// First index.
        a: u32,
        /// Second index.
        b: u32,
        /// Third index.
        c: u32,
    },
}

/// A triangle mesh: flat `SoA` buffers (DECISIONS.md row 41). `positions` is
/// `[x0, y0, z0, x1, …]`; `indices` is `[a0, b0, c0, a1, …]`, three per
/// triangle, counter-clockwise = outward when the mesh is oriented.
///
/// Buffers are `Arc`s: cloning a mesh (marshalling, interning, transforms
/// of *other* values) never copies vertex data.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    positions: Arc<[f64]>,
    indices: Arc<[u32]>,
}

impl Mesh {
    /// Validate and construct. The structural invariants (shape, index
    /// range, no degenerate triangles) hold for every `Mesh` in the system;
    /// NaN refusal happens at value construction.
    ///
    /// # Errors
    ///
    /// [`MeshError`] on ragged buffers, out-of-range indices, or a
    /// triangle repeating a vertex.
    pub fn new(positions: Vec<f64>, indices: Vec<u32>) -> Result<Self, MeshError> {
        if !positions.len().is_multiple_of(3) {
            return Err(MeshError::RaggedPositions {
                len: positions.len(),
            });
        }
        if !indices.len().is_multiple_of(3) {
            return Err(MeshError::RaggedIndices { len: indices.len() });
        }
        let vertices = positions.len() / 3;
        // Ragged input was refused above, so the remainder is empty.
        let (triangles, _) = indices.as_chunks::<3>();
        for (triangle, &[a, b, c]) in triangles.iter().enumerate() {
            for &index in &[a, b, c] {
                if index as usize >= vertices {
                    return Err(MeshError::IndexOutOfRange { index, vertices });
                }
            }
            if a == b || b == c || a == c {
                return Err(MeshError::DegenerateTriangle { triangle, a, b, c });
            }
        }
        Ok(Self {
            positions: positions.into(),
            indices: indices.into(),
        })
    }

    /// The position buffer, `[x0, y0, z0, x1, …]`.
    #[must_use]
    pub fn positions(&self) -> &[f64] {
        &self.positions
    }

    /// The index buffer, three per triangle.
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Vertex count.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Triangle count.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Structural watertightness — the `Watertight<Mesh>` refinement
    /// predicate: every directed edge appears exactly once and its reverse
    /// exists (closed, 2-manifold, consistently oriented). Combinatorial —
    /// no tolerance involved, so it lives in core; this is the ONE
    /// system-wide watertight definition (`as_watertight`, marshalling,
    /// and the Manifold seam's precondition all use it). The empty mesh is
    /// watertight (it is the empty solid — booleans produce it).
    #[must_use]
    pub fn is_watertight(&self) -> bool {
        let mut directed: std::collections::HashMap<(u32, u32), u32> =
            std::collections::HashMap::with_capacity(self.indices.len());
        // A constructed mesh has a whole number of triangles; the remainder is empty.
        let (triangles, _) = self.indices.as_chunks::<3>();
        for &[a, b, c] in triangles {
            for (from, to) in [(a, b), (b, c), (c, a)] {
                *directed.entry((from, to)).or_insert(0) += 1;
            }
        }
        directed
            .iter()
            .all(|(&(from, to), &count)| count == 1 && directed.get(&(to, from)) == Some(&1))
    }

    /// Rebuild with a canonicalized position buffer — the `-0.0` → `0.0`
    /// rewrite path of value canonicalization (crate-internal; rare).
    #[must_use]
    pub(crate) fn with_positions_canonicalized(&self, positions: Vec<f64>) -> Self {
        debug_assert_eq!(positions.len(), self.positions.len());
        Self {
            positions: positions.into(),
            indices: Arc::clone(&self.indices),
        }
    }
}

/// The first bytes of every canonical solid serialization: OCCT's
/// `BinTools` header at the PINNED format version 4 (DECISIONS.md row 42,
/// revised 2026-08-20; `cicada_geom::occt::CANONICAL_FORMAT_VERSION` is the
/// same pin on the kernel side and a test holds the two together). Core
/// checks this prefix so garbage never becomes a hashed value; it cannot
/// check more without the kernel, and does not try.
pub const SOLID_CANONICAL_HEADER: &[u8] = b"\nOpen CASCADE Topology V4";

/// Why bytes were refused as a solid's canonical form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SolidError {
    /// The bytes do not start with [`SOLID_CANONICAL_HEADER`]: not a
    /// `BinTools` V4 stream, so not a canonical serialization.
    #[error(
        "{len} bytes are not a canonical solid serialization (BinTools V4): the stream \
         starts with {head:?}"
    )]
    NotCanonical {
        /// The byte count offered.
        len: usize,
        /// The first bytes, lossily decoded, for the message.
        head: String,
    },
}

/// A B-rep solid (DECISIONS.md row 42): the value IS the kernel's canonical
/// serialization — OCCT `BinTools` at a pinned format version, without
/// triangulation or normals, history flags normalized — and nothing else.
/// Core never depends on the kernel; `cicada-geom`'s `occt` seam produces
/// these bytes and rebuilds a kernel handle from them when an operation
/// needs one. The content hash is blake3 over the bytes like every other
/// value, so interning, instancing, early cutoff and the disk store work
/// unchanged; `Watertight<Mesh>` stays the mesh-tier solid and
/// `tessellate: Solid → Watertight<Mesh>` is the explicit, costed bridge
/// (docs/03 §The seam).
///
/// The bytes are an `Arc`: cloning a solid (marshalling, interning, list
/// slots) never copies the serialization.
#[derive(Clone, PartialEq, Eq)]
pub struct Solid {
    bytes: Arc<[u8]>,
}

impl Solid {
    /// Wrap canonical bytes, refusing anything that does not carry the
    /// `BinTools` V4 header. The kernel seam is the only producer of
    /// canonical bytes; this door exists for it and for the store's
    /// reload path.
    ///
    /// # Errors
    ///
    /// [`SolidError::NotCanonical`] when the bytes lack the header.
    pub fn from_canonical_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<Self, SolidError> {
        let bytes: Arc<[u8]> = bytes.into();
        if !bytes.starts_with(SOLID_CANONICAL_HEADER) {
            let shown = &bytes[..bytes.len().min(24)];
            return Err(SolidError::NotCanonical {
                len: bytes.len(),
                head: String::from_utf8_lossy(shown).into_owned(),
            });
        }
        Ok(Self { bytes })
    }

    /// The canonical bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The canonical bytes, shared (the seam hands them to the kernel's
    /// reader; the store writes them).
    #[must_use]
    pub fn shared_bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }
}

impl fmt::Debug for Solid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Solid")
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// A curve statically known to be closed (DECISIONS.md row 22): entered via
/// the checked `as_closed` conversion (or constructors closed by nature —
/// circles, rectangles). The wrapper is port-type-level; on the wire the
/// value is the plain curve, and marshalling re-verifies the predicate on
/// the way in (loud beats fast; the check is O(1)).
#[derive(Debug, Clone, PartialEq)]
pub struct Closed<C>(pub C);

/// A mesh statically known to be watertight (docs/08: the mesh-tier solid):
/// entered via the checked `as_watertight` conversion or watertight-by-
/// construction nodes (box, sphere, extrude, booleans). Same wire semantics
/// as [`Closed`]: plain mesh on the wire, predicate re-verified at marshal
/// (O(edges) — revisit only on profiling evidence).
#[derive(Debug, Clone, PartialEq)]
pub struct Watertight<M>(pub M);

/// The runtime value of a kind-preserving transform port (`T` in the
/// catalog): any transformable kind, dispatched at runtime. The checker
/// guarantees the STATIC kind is preserved end to end
/// ([`VAR_TRANSFORMABLE`]); this enum is the erased runtime carrier.
#[derive(Debug, Clone, PartialEq)]
pub enum Transformable {
    /// A position.
    Point(Point),
    /// A displacement (transforms by the linear part only).
    Vector(crate::spatial::Vector),
    /// An oriented frame.
    Plane(Plane),
    /// Any curve variant.
    Curve(Curve),
    /// A mesh.
    Mesh(Mesh),
    /// A B-rep solid. In the type lattice from v0.1 item 3 WP-B (a `T` port
    /// accepts it); the kernel-backed move/rotate/scale arrive with WP-C —
    /// until then a transform of a solid is a loud red, never a silent
    /// pass-through (`cicada_geom::transform::Similarity::apply`).
    Solid(Solid),
}

/// The runtime value of a display-sink geometry port (`Geometry` in the
/// catalog): anything drawable. Wider than [`Transformable`] conceptually,
/// identical for the spike.
#[derive(Debug, Clone, PartialEq)]
pub enum GeometryValue {
    /// A position.
    Point(Point),
    /// Any curve variant.
    Curve(Curve),
    /// A mesh.
    Mesh(Mesh),
    /// A B-rep solid (drawn through the display tessellation cache).
    Solid(Solid),
}

/// The catalog base name of the kind-preserving transform type variable.
/// A port typed `T` accepts any kind in [`TRANSFORMABLE_KINDS`]; the
/// checker unifies every `T` occurrence in one call and substitutes the
/// bound kind into `T`-typed outputs (kind-preserving generics,
/// DECISIONS.md row 22).
pub const VAR_TRANSFORMABLE: &str = "T";

/// The catalog base name of the unconstrained element type variable used
/// by list nodes (`item(list: [E]) → E`): unifies with ANY kind, and binds
/// so outputs preserve the element kind.
pub const VAR_ELEMENT: &str = "E";

/// The wire kinds a `T` port accepts. Refined names are listed explicitly:
/// every spike transform is a similarity, which preserves closedness and
/// watertightness, so the refinement rides through the type variable.
/// `Solid` is a transformable kind (B-rep transforms are kind-preserving);
/// its kernel-backed transforms land with v0.1 item 3 WP-C.
pub const TRANSFORMABLE_KINDS: &[&str] = &[
    "Point",
    "Vector",
    "Plane",
    "Curve",
    "Closed<Curve>",
    "Mesh",
    "Watertight<Mesh>",
    "Solid",
];

/// The wire kinds a `Geometry` port accepts (display sinks). The checker
/// widens each of these into `Geometry`; there is no narrowing back.
pub const GEOMETRY_KINDS: &[&str] = &[
    "Point",
    "Curve",
    "Closed<Curve>",
    "Mesh",
    "Watertight<Mesh>",
    "Solid",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::Vector;

    fn tetrahedron() -> Mesh {
        // A tetrahedron: the smallest watertight mesh.
        Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
        )
        .expect("tetrahedron is valid")
    }

    #[test]
    fn curve_closedness_by_variant() {
        let line = Curve::Line(Line {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 0.0, 0.0),
        });
        assert!(!line.is_closed());
        let open = Curve::Polyline(Polyline {
            vertices: vec![Point::new(0.0, 0.0, 0.0), Point::new(1.0, 0.0, 0.0)],
            closed: false,
        });
        assert!(!open.is_closed());
        let closed = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        });
        assert!(closed.is_closed());
        let circle = Curve::Circle(Circle {
            plane: Plane {
                origin: Point::new(0.0, 0.0, 0.0),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
            radius: 2.0,
        });
        assert!(circle.is_closed());
    }

    #[test]
    fn mesh_construction_validates_structure() {
        assert_eq!(
            Mesh::new(vec![0.0, 0.0], vec![]),
            Err(MeshError::RaggedPositions { len: 2 })
        );
        assert_eq!(
            Mesh::new(vec![0.0, 0.0, 0.0], vec![0, 0]),
            Err(MeshError::RaggedIndices { len: 2 })
        );
        assert_eq!(
            Mesh::new(vec![0.0, 0.0, 0.0], vec![0, 1, 2]),
            Err(MeshError::IndexOutOfRange {
                index: 1,
                vertices: 1
            })
        );
        assert_eq!(
            Mesh::new(
                vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                vec![0, 1, 1]
            ),
            Err(MeshError::DegenerateTriangle {
                triangle: 0,
                a: 0,
                b: 1,
                c: 1
            })
        );
    }

    #[test]
    fn tetrahedron_is_watertight() {
        assert!(tetrahedron().is_watertight());
    }

    #[test]
    fn open_mesh_is_not_watertight() {
        // Drop one face of the tetrahedron.
        let open = Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3],
        )
        .expect("valid open mesh");
        assert!(!open.is_watertight());
    }

    #[test]
    fn inconsistent_orientation_is_not_watertight() {
        // Flip one face: the mesh is closed but not consistently oriented.
        let flipped = Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 2, 3],
        )
        .expect("valid mesh");
        assert!(!flipped.is_watertight());
    }

    #[test]
    fn empty_mesh_is_watertight() {
        // The empty solid — booleans produce it; it must stay legal.
        let empty = Mesh::new(vec![], vec![]).expect("empty mesh is valid");
        assert!(empty.is_watertight());
    }

    #[test]
    fn mesh_clone_shares_buffers() {
        let mesh = tetrahedron();
        let clone = mesh.clone();
        assert!(Arc::ptr_eq(&mesh.positions, &clone.positions));
        assert!(Arc::ptr_eq(&mesh.indices, &clone.indices));
    }

    /// Header plus an arbitrary tail: what core can verify of canonical
    /// bytes without the kernel.
    fn pseudo_canonical(tail: &[u8]) -> Vec<u8> {
        let mut bytes = SOLID_CANONICAL_HEADER.to_vec();
        bytes.extend_from_slice(tail);
        bytes
    }

    #[test]
    fn solid_wraps_canonical_bytes_and_refuses_the_rest() {
        let bytes = pseudo_canonical(b", (c) Open Cascade\nLocations 0\n");
        let solid = Solid::from_canonical_bytes(bytes.clone()).expect("header present");
        assert_eq!(solid.bytes(), &bytes[..]);
        // Exactly the header is canonical as far as core can tell.
        assert!(Solid::from_canonical_bytes(SOLID_CANONICAL_HEADER.to_vec()).is_ok());
        for garbage in [
            &b""[..],
            b"not a brep",
            b"\nOpen CASCADE Topology V3, (c) Open Cascade",
        ] {
            let error = Solid::from_canonical_bytes(garbage.to_vec())
                .expect_err("bytes without the V4 header are refused");
            assert!(
                matches!(&error, SolidError::NotCanonical { len, .. } if *len == garbage.len()),
                "{error}"
            );
        }
    }

    #[test]
    fn solid_clone_shares_bytes_and_debug_hides_them() {
        let solid = Solid::from_canonical_bytes(pseudo_canonical(&[0u8; 4000])).expect("solid");
        let clone = solid.clone();
        assert!(Arc::ptr_eq(solid.shared_bytes(), clone.shared_bytes()));
        assert_eq!(solid, clone, "equality is by bytes");
        let shown = format!("{solid:?}");
        assert_eq!(shown, "Solid { bytes: 4025 }", "{shown}");
    }

    #[test]
    fn solid_is_a_transformable_and_a_geometry_kind() {
        // The checker's lattice reads these lists (cicada-lang) and so does
        // the view-model's displayable predicate (cicada-server): one
        // entry each admits a `Solid` into `T` ports and display sinks.
        assert!(TRANSFORMABLE_KINDS.contains(&"Solid"));
        assert!(GEOMETRY_KINDS.contains(&"Solid"));
    }
}
