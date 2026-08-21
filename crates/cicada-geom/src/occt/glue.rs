//! The cxx bridge to [`glue.hxx`](../occt/glue.hxx): cicada-geom's own
//! kernel glue for the node set (v0.1 item 3 WP-C), compiled by this
//! crate's `build.rs` against the prebuilt OCCT that `DEP_OCCT_ROOT` names
//! — the same prefix the fork's `opencascade-sys` links. The fork carries
//! the first glue (box, prism, cut, canonical bytes, tessellate); this
//! bridge carries the rest, in namespace `cicada_geom` so its symbols can
//! never collide with the fork's.
//!
//! Every function is declared `Result`: the header defines the
//! `rust::behavior::trycatch` hook that turns OCCT's `Standard_Failure`,
//! `std::exception` and anything else into `Err(cxx::Exception)` — the
//! exception policy of [`super`]. `TopoDS_Shape` is the fork's type, shared
//! across the two bridges, so handles flow between the fork's glue and this
//! one without conversion.
//!
//! The bridge is the ONLY unsafe code in this crate outside the fork: cxx's
//! `unsafe extern "C++"` block vouches that every declaration matches the
//! C++ signature in `glue.hxx` (the generated shims are checked against the
//! header at C++ compile time, so a drift is a build error, not UB).

// SAFETY: the `unsafe extern "C++"` block below asserts that the Rust
// declarations match the C++ definitions in glue.hxx; cxx compiles the
// generated shims against that header, so any mismatch fails the build.
// Ownership: `UniquePtr<TopoDS_Shape>` is a C++ `std::unique_ptr` freed by
// the C++ side; slices and `Vec`s are borrowed for the call's duration only;
// no function stores a pointer past its return.
#![allow(unsafe_code)]
// `section` takes eight arguments: the C++ signature is the contract, and a
// parameter struct across the bridge would buy a shared type for one call.
#![allow(clippy::too_many_arguments)]

pub use ffi::*;

#[cxx::bridge(namespace = "cicada_geom")]
mod ffi {
    unsafe extern "C++" {
        include!("cicada-geom/src/occt/glue.hxx");

        /// The fork's `TopoDS_Shape` (global namespace), shared between
        /// the two bridges.
        #[namespace = ""]
        type TopoDS_Shape = opencascade_sys::topo_ds::TopoDS_Shape;

        /// `BRepPrimAPI_MakeBox` in a frame (`frame` = origin, unit x,
        /// unit z — nine doubles) with positive extents along its axes.
        fn make_box(frame: &[f64], dx: f64, dy: f64, dz: f64) -> Result<UniquePtr<TopoDS_Shape>>;

        /// `BRepPrimAPI_MakeSphere` centred at the frame's origin.
        fn make_sphere(frame: &[f64], radius: f64) -> Result<UniquePtr<TopoDS_Shape>>;

        /// `BRepPrimAPI_MakeCylinder` standing on the frame's xy plane
        /// along its z.
        fn make_cylinder(
            frame: &[f64],
            radius: f64,
            height: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;

        /// `BRepPrimAPI_MakeCone` from `radius1` at the base to `radius2`
        /// at `height` along the frame's z.
        fn make_cone(
            frame: &[f64],
            radius1: f64,
            radius2: f64,
            height: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;

        /// A polyline wire over flat xyz triples (closing segment when
        /// `closed`).
        fn make_polyline_wire(xyz: &[f64], closed: bool) -> Result<UniquePtr<TopoDS_Shape>>;

        /// A full-circle wire in the frame's xy plane.
        fn make_circle_wire(frame: &[f64], radius: f64) -> Result<UniquePtr<TopoDS_Shape>>;

        /// An empty `TopoDS_Compound`.
        fn make_compound() -> Result<UniquePtr<TopoDS_Shape>>;

        /// Append `shape` to a compound made by [`make_compound`].
        fn compound_add(compound: Pin<&mut TopoDS_Shape>, shape: &TopoDS_Shape) -> Result<()>;

        /// A closed planar wire extruded along `(dx, dy, dz)` into a solid.
        fn prism(
            profile: &TopoDS_Shape,
            dx: f64,
            dy: f64,
            dz: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;

        /// `BRepOffsetAPI_ThruSections` over the wires in `sections` (a
        /// compound, in order), optionally converging to `apex` (empty or
        /// one xyz), as a solid.
        fn thru_sections(
            sections: &TopoDS_Shape,
            ruled: bool,
            apex: &[f64],
        ) -> Result<UniquePtr<TopoDS_Shape>>;

        /// A closed planar wire revolved about `axis` (point, direction —
        /// six doubles) by `angle` radians.
        fn revolve(
            profile: &TopoDS_Shape,
            axis: &[f64],
            angle: f64,
        ) -> Result<UniquePtr<TopoDS_Shape>>;

        /// A closed wire swept along a spine wire into a solid.
        fn sweep(spine: &TopoDS_Shape, profile: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;

        /// N-ary fuse of the children of `arguments` with the children of
        /// `tools`, unified.
        fn fuse(arguments: &TopoDS_Shape, tools: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;

        /// `shape` minus every child of `tools`, unified.
        fn cut(shape: &TopoDS_Shape, tools: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;

        /// The common volume of two shapes, unified.
        fn common(a: &TopoDS_Shape, b: &TopoDS_Shape) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Volume and centroid into `out` = `[volume, cx, cy, cz]`.
        fn volume_properties(shape: &TopoDS_Shape, eps: f64, out: &mut Vec<f64>) -> Result<()>;

        /// Tight world-aligned bounds into `out` = `[min xyz, max xyz]`.
        fn bounds(shape: &TopoDS_Shape, out: &mut Vec<f64>) -> Result<()>;

        /// The shape under a similarity given as 12 row-major 3×4
        /// coefficients, geometry copied and rewritten.
        fn transform(shape: &TopoDS_Shape, m: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;

        /// `BRepCheck_Analyzer::IsValid` over the whole shape.
        fn is_valid(shape: &TopoDS_Shape) -> Result<bool>;

        /// The `index`-th solid sub-shape (0-based).
        fn nth_solid(shape: &TopoDS_Shape, index: i32) -> Result<UniquePtr<TopoDS_Shape>>;

        /// Every distinct non-degenerate edge as curve records (see the
        /// header: `kinds` 0/1/2, `counts`, `data`); returns the face count.
        fn edges(
            shape: &TopoDS_Shape,
            linear: f64,
            angular: f64,
            kinds: &mut Vec<i32>,
            counts: &mut Vec<u32>,
            data: &mut Vec<f64>,
        ) -> Result<i32>;

        /// Every distinct vertex as xyz triples.
        fn vertices(shape: &TopoDS_Shape, out: &mut Vec<f64>) -> Result<()>;

        /// The planar section (`plane` = origin, normal — six doubles) as
        /// curve records, one per connected wire.
        fn section(
            shape: &TopoDS_Shape,
            plane: &[f64],
            tolerance: f64,
            linear: f64,
            angular: f64,
            kinds: &mut Vec<i32>,
            counts: &mut Vec<u32>,
            data: &mut Vec<f64>,
        ) -> Result<()>;

        /// Lower OCCT's default printers to failures only (STEP lock held).
        fn quiet_messenger() -> Result<()>;

        /// Write the children of `shapes` to a STEP AP214 file with a fixed
        /// header (STEP lock held).
        fn step_write(
            shapes: &TopoDS_Shape,
            path: &str,
            millimeters: f64,
            name: &str,
            timestamp: &str,
        ) -> Result<()>;

        /// Read a STEP file into one shape, scaled to the document unit
        /// (STEP lock held).
        fn step_read(path: &str, millimeters: f64) -> Result<UniquePtr<TopoDS_Shape>>;
    }
}
