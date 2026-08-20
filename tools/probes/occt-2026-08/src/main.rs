//! OCCT probe — the executable half of `docs/probes/occt-2026-08.md`.
//!
//! Throwaway code: `unwrap`/`expect` are fine here (this is not library
//! code), but every failure is loud — a probe that silently skips a step
//! would produce a false PASS.
//!
//! Subcommands:
//!
//! - `smoke` — build the four probe shapes once, tessellate them, print
//!   counts. Proves compile + link + run.
//! - `dump <out_dir>` — write every probe shape with `BinTools::Write` and
//!   `BRepTools::Write` (both WITHOUT triangulation — the binding passes
//!   only the path, so OCCT's defaults apply), tessellate, and print the
//!   sha256 of each file and of each triangle buffer. Run it twice in two
//!   processes and compare the lines (question 2).
//! - `bench <parts>` — time box / extrude / difference / tessellate over
//!   `<parts>` independent parts, report wall-clock per op and per part
//!   (question 3).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::{DVec3, dvec3};
use opencascade::mesh::Mesh;
use opencascade::primitives::{Face, Shape, Solid, Wire};
use sha2::{Digest, Sha256};

/// The binding's `Shape::mesh()` default; recorded in the memo as "the
/// default deflection" we timed with.
const LINEAR_DEFLECTION: f64 = 0.01;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("smoke") => smoke(),
        Some("dump") => {
            let out = args.get(2).expect("usage: occt-probe dump <out_dir>");
            dump(Path::new(out));
        }
        Some("bench") => {
            let parts: usize = args
                .get(2)
                .expect("usage: occt-probe bench <parts>")
                .parse()
                .expect("<parts> must be an integer");
            bench(parts);
        }
        _ => {
            eprintln!("usage: occt-probe smoke | dump <out_dir> | bench <parts>");
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------
// The probe shapes. All inputs are transcendental-free: axis-aligned boxes,
// rectangles built from explicit points (no rotations, no transforms),
// extrusions along Z, a loft between two rectangles.
// ---------------------------------------------------------------------------

/// Box 10×20×30 with its min corner at `origin`.
fn probe_box(origin: DVec3) -> Shape {
    Shape::box_from_corners(origin, origin + dvec3(10.0, 20.0, 30.0))
}

/// Axis-aligned rectangle in the plane z = `origin.z`, corners at
/// `origin` and `origin + (w, h, 0)`, as a closed wire of four segments.
fn rect_wire(origin: DVec3, w: f64, h: f64) -> Wire {
    Wire::from_ordered_points([
        origin,
        origin + dvec3(w, 0.0, 0.0),
        origin + dvec3(w, h, 0.0),
        origin + dvec3(0.0, h, 0.0),
    ])
    .expect("four points make a wire")
}

/// Rectangle 4×6 extruded 40 along Z, positioned to pierce `probe_box`
/// from below to above (z from -5 to 35 when `origin` is the box origin).
fn probe_extrude(origin: DVec3) -> Solid {
    let wire = rect_wire(origin + dvec3(3.0, 7.0, -5.0), 4.0, 6.0);
    let face = Face::from_wire(&wire);
    face.extrude(dvec3(0.0, 0.0, 40.0))
}

/// `probe_box` minus `probe_extrude`: a box with a rectangular through-hole.
fn probe_difference(origin: DVec3) -> Shape {
    let b = probe_box(origin);
    let cutter: Shape = probe_extrude(origin).into();
    b.subtract(&cutter).into()
}

/// Loft between the 10×20 rectangle at z = 0 and a 6×12 rectangle at
/// z = 30 (centered on the first), as a solid.
fn probe_loft(origin: DVec3) -> Solid {
    let bottom = rect_wire(origin, 10.0, 20.0);
    let top = rect_wire(origin + dvec3(2.0, 4.0, 30.0), 6.0, 12.0);
    Solid::loft([&bottom, &top])
}

struct Probe {
    name: &'static str,
    shape: Shape,
}

fn probe_shapes(origin: DVec3) -> Vec<Probe> {
    vec![
        Probe { name: "box", shape: probe_box(origin) },
        Probe { name: "extrude", shape: probe_extrude(origin).into() },
        Probe { name: "difference", shape: probe_difference(origin) },
        Probe { name: "loft", shape: probe_loft(origin).into() },
    ]
}

// ---------------------------------------------------------------------------
// smoke
// ---------------------------------------------------------------------------

fn smoke() {
    for probe in probe_shapes(DVec3::ZERO) {
        let mesh = probe.shape.mesh_with_tolerance(LINEAR_DEFLECTION).expect("tessellation");
        let faces = probe.shape.faces().count();
        let edges = probe.shape.edges().count();
        println!(
            "{:<11} faces={:<3} edges={:<3} vertices={:<6} triangles={:<6} shape_type={:?}",
            probe.name,
            faces,
            edges,
            mesh.vertices.len(),
            mesh.indices.len() / 3,
            probe.shape.shape_type()
        );
    }
    println!("smoke OK");
}

// ---------------------------------------------------------------------------
// dump (determinism)
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        write!(s, "{b:02x}").expect("write to String");
    }
    s
}

/// Hash of the triangle buffer: vertex positions as f64 LE then indices as
/// u32 LE. Normals hashed separately (they come from a second OCCT pass).
fn mesh_hashes(mesh: &Mesh) -> (String, String) {
    let mut geom = Vec::with_capacity(mesh.vertices.len() * 24 + mesh.indices.len() * 4);
    for v in &mesh.vertices {
        geom.extend_from_slice(&v.x.to_le_bytes());
        geom.extend_from_slice(&v.y.to_le_bytes());
        geom.extend_from_slice(&v.z.to_le_bytes());
    }
    for i in &mesh.indices {
        let i = u32::try_from(*i).expect("index fits u32");
        geom.extend_from_slice(&i.to_le_bytes());
    }
    let mut normals = Vec::with_capacity(mesh.normals.len() * 24);
    for n in &mesh.normals {
        normals.extend_from_slice(&n.x.to_le_bytes());
        normals.extend_from_slice(&n.y.to_le_bytes());
        normals.extend_from_slice(&n.z.to_le_bytes());
    }
    (sha256_hex(&geom), sha256_hex(&normals))
}

fn dump(out_dir: &Path) {
    std::fs::create_dir_all(out_dir).expect("create out dir");
    for probe in probe_shapes(DVec3::ZERO) {
        let bin_path: PathBuf = out_dir.join(format!("{}.bin", probe.name));
        let txt_path: PathBuf = out_dir.join(format!("{}.brep", probe.name));
        probe.shape.write_brep_bin(&bin_path).expect("BinTools::Write");
        probe.shape.write_brep_text(&txt_path).expect("BRepTools::Write");
        let bin = std::fs::read(&bin_path).expect("read .bin");
        let txt = std::fs::read(&txt_path).expect("read .brep");
        println!("{:<11} bintools  bytes={:<8} sha256={}", probe.name, bin.len(), sha256_hex(&bin));
        println!("{:<11} breptools bytes={:<8} sha256={}", probe.name, txt.len(), sha256_hex(&txt));

        let mesh = probe.shape.mesh_with_tolerance(LINEAR_DEFLECTION).expect("tessellation");
        let (geom, normals) = mesh_hashes(&mesh);
        println!(
            "{:<11} mesh      verts={:<6} tris={:<6} sha256={} normals={}",
            probe.name,
            mesh.vertices.len(),
            mesh.indices.len() / 3,
            geom,
            normals
        );

        // Round-trip: does the serialized form re-read to the same bytes?
        let reread = Shape::read_brep_bin(&bin_path).expect("BinTools::Read");
        let rt_path = out_dir.join(format!("{}.roundtrip.bin", probe.name));
        reread.write_brep_bin(&rt_path).expect("BinTools::Write (round trip)");
        let rt = std::fs::read(&rt_path).expect("read round trip");
        println!(
            "{:<11} roundtrip bytes={:<8} sha256={} identical={}",
            probe.name,
            rt.len(),
            sha256_hex(&rt),
            rt == bin
        );
    }
}

// ---------------------------------------------------------------------------
// bench (timings)
// ---------------------------------------------------------------------------

fn report(op: &str, total: Duration, parts: usize, extra: &str) {
    let per_part = total.as_secs_f64() * 1e3 / parts as f64;
    println!(
        "{:<12} parts={:<5} total={:>9.3} ms  per_part={:>9.4} ms  {}",
        op,
        parts,
        total.as_secs_f64() * 1e3,
        per_part,
        extra
    );
}

fn bench(parts: usize) {
    // Each part sits at its own offset so nothing is shared or cached
    // between parts inside OCCT.
    let origins: Vec<DVec3> = (0..parts).map(|i| dvec3(i as f64 * 15.0, 0.0, 0.0)).collect();

    let t = Instant::now();
    let boxes: Vec<Shape> = origins.iter().map(|o| probe_box(*o)).collect();
    report("box", t.elapsed(), parts, "");

    let t = Instant::now();
    let cutters: Vec<Shape> = origins.iter().map(|o| probe_extrude(*o).into()).collect();
    report("extrude", t.elapsed(), parts, "");

    let t = Instant::now();
    let diffs: Vec<Shape> =
        boxes.iter().zip(&cutters).map(|(b, c)| b.subtract(c).into()).collect();
    report("difference", t.elapsed(), parts, "");

    let t = Instant::now();
    let mut tris = 0usize;
    let mut verts = 0usize;
    for d in &diffs {
        let mesh = d.mesh_with_tolerance(LINEAR_DEFLECTION).expect("tessellation");
        tris += mesh.indices.len() / 3;
        verts += mesh.vertices.len();
    }
    report("tessellate", t.elapsed(), parts, &format!("(deflection={LINEAR_DEFLECTION}, tris={tris}, verts={verts})"));

    // The loft is not part of the per-part pipeline above but is a probe
    // shape; time it too so the memo has a number.
    let t = Instant::now();
    let lofts: Vec<Shape> = origins.iter().map(|o| probe_loft(*o).into()).collect();
    report("loft", t.elapsed(), parts, "");

    // One union chain of all the parts would be the wall-style workload;
    // keep the boxes alive until here so drop cost is not inside the timers.
    drop(lofts);
    drop(diffs);
    drop(cutters);
    drop(boxes);
}
