//! The standalone carve benchmark (doc 15 stage 4 DoD): 1,500 labeled
//! frusta ∖ cutters, timed. The wall's carve stage at production scale,
//! isolated from the scheduler — this measures the KERNEL SEAM, the
//! stage-6 `cicada run corpus/wall.cic --node carved --time` measures the
//! whole system against the <10 s criterion.
//!
//! Run (release, or the numbers lie):
//!
//! ```text
//! cargo run --release -p cicada-geom --example carve_bench [parts]
//! ```
//!
//! Prints wall time, per-part mean, and throughput; exits nonzero if the
//! full-scale carve exceeds the 10 s budget (so the perf-check loop can
//! script it).

// A benchmark binary: a failed setup IS a loud abort (the lint ban on
// expect/unwrap guards library code paths, not measurement tools).
#![allow(clippy::expect_used)]

use std::time::Instant;

use cicada_core::geometry::Mesh;
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_geom::{boolean, meshbuild};
use rayon::prelude::*;

/// An irregular voronoi-cell-like frustum: pentagon base, top ring scaled
/// toward the centroid and lifted — the wall part shape, hand-built the
/// way the corpus builds them (analytic mesh construction, doc 15).
fn frustum(seed: u64) -> Mesh {
    let mix = |i: u64| {
        let mut z = seed
            .wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^= z >> 31;
        #[allow(clippy::cast_precision_loss)]
        let unit = (z >> 11) as f64 / 9_007_199_254_740_992.0;
        unit
    };
    let sides = 5;
    let base: Vec<(f64, f64)> = (0..sides)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let angle = std::f64::consts::TAU * (f64::from(i) + 0.35 * mix(u64::from(i)))
                / f64::from(sides);
            let radius = 14.0 + 6.0 * mix(u64::from(i) + 17);
            (radius * angle.cos(), radius * angle.sin())
        })
        .collect();
    let height = 9.0 + 3.0 * mix(97);
    let taper = 0.72;
    let (cx, cy) = base
        .iter()
        .fold((0.0, 0.0), |(sx, sy), &(x, y)| (sx + x, sy + y));
    #[allow(clippy::cast_precision_loss)]
    let (cx, cy) = (cx / f64::from(sides), cy / f64::from(sides));

    let mut positions = Vec::with_capacity(sides as usize * 2 * 3);
    for &(x, y) in &base {
        positions.extend_from_slice(&[x, y, 0.0]);
    }
    for &(x, y) in &base {
        positions.extend_from_slice(&[cx + (x - cx) * taper, cy + (y - cy) * taper, height]);
    }
    let mut indices: Vec<u32> = Vec::new();
    // Base fan (down) + top fan (up) + walls. Pentagon is convex enough
    // for fans by construction (radii stay positive).
    for i in 1..sides - 1 {
        indices.extend_from_slice(&[0, i + 1, i]);
        indices.extend_from_slice(&[sides, sides + i, sides + i + 1]);
    }
    for i in 0..sides {
        let j = (i + 1) % sides;
        indices.extend_from_slice(&[i, j, sides + j]);
        indices.extend_from_slice(&[i, sides + j, sides + i]);
    }
    let mesh = Mesh::new(positions, indices).expect("frustum builds");
    assert!(
        mesh.is_watertight(),
        "frustum is watertight by construction"
    );
    mesh
}

/// Label-stroke cutters: slender boxes debossed into the frustum top —
/// the wall's part labels (~10 strokes per part).
fn label_cutters(seed: u64) -> Vec<Mesh> {
    (0..10)
        .map(|stroke| {
            #[allow(clippy::cast_precision_loss)]
            let at = -6.0 + 1.3 * f64::from(stroke) + 0.1 * ((seed % 7) as f64);
            let vertical = stroke % 3 == 0;
            let (w, h) = if vertical { (0.4, 2.4) } else { (1.0, 0.4) };
            meshbuild::box_mesh(
                &Plane {
                    origin: Point::new(at, -1.0, 8.0),
                    x: Vector::new(1.0, 0.0, 0.0),
                    y: Vector::new(0.0, 1.0, 0.0),
                },
                Domain::new(0.0, w),
                Domain::new(0.0, h),
                Domain::new(0.0, 4.0), // cuts through the tapered top
                1e-6,
            )
            .expect("cutter builds")
        })
        .collect()
}

fn main() {
    let parts: u64 = std::env::args()
        .nth(1)
        .map_or(1500, |arg| arg.parse().expect("parts must be a number"));

    let build_started = Instant::now();
    let jobs: Vec<(Mesh, Vec<Mesh>)> = (0..parts)
        .into_par_iter()
        .map(|part| (frustum(part), label_cutters(part)))
        .collect();
    let build_wall = build_started.elapsed();

    let carve_started = Instant::now();
    let carved: Vec<Mesh> = jobs
        .par_iter()
        .map(|(part, cutters)| boolean::difference(part, cutters).expect("carve succeeds"))
        .collect();
    let carve_wall = carve_started.elapsed();

    let triangles: usize = carved.iter().map(Mesh::triangle_count).sum();
    let carve_secs = carve_wall.as_secs_f64();
    #[allow(clippy::cast_precision_loss)]
    let per_part_ms = carve_secs * 1000.0 / parts as f64;
    println!("carve_bench: {parts} labeled frusta (10 cutters each)");
    println!(
        "  build: {:.3} s   carve: {:.3} s   ({per_part_ms:.3} ms/part, {} result triangles)",
        build_wall.as_secs_f64(),
        carve_secs,
        triangles,
    );
    // Doc 15's stage-4 DoD: "lands in seconds". The 10 s line is the
    // stage-6 full-pipeline criterion; the bare seam must be well under.
    if parts >= 1500 && carve_secs > 10.0 {
        eprintln!("FAIL: carve exceeded the 10 s budget");
        std::process::exit(1);
    }
}
