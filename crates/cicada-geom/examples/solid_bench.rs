//! The solid seam's cost table (v0.1 item 3 WP-B): what the sharing model's
//! "every operation re-reads its inputs from bytes" rule costs next to the
//! kernel work it feeds, at 1 / 100 / 1,000 parts — the numbers docs/03
//! §The OCCT seam as built quotes.
//!
//! Run (release, with the kernel, or the numbers lie):
//!
//! ```text
//! eval "$(python tools/fetch_occt.py --print-env bash --quiet)"
//! cargo run --release -p cicada-geom --features occt --example solid_bench [parts...]
//! ```
//!
//! Per part it times, single-threaded: the constructors (`box`, `extrude`)
//! including serialization; the `BinTools` read that turns a value back
//! into a kernel handle (the "deep copy" and the would-be cache hit);
//! `difference` and `tessellate` both FROM VALUES (the model as shipped:
//! read, operate, serialize) and FROM HANDLES (the operation alone — the
//! most a handle cache could ever save); and the whole chain
//! box → extrude → difference → tessellate both ways. Then the same chain
//! across the rayon pool, to show the seam scales without a kernel lock.

// A benchmark binary: a failed setup IS a loud abort (the lint ban on
// expect/unwrap guards library code paths, not measurement tools).
#![allow(clippy::expect_used, clippy::cast_precision_loss)]

use std::time::{Duration, Instant};

use cicada_core::geometry::Solid;
use cicada_core::spatial::{Point, Vector};
use cicada_geom::occt::Handle;
use cicada_geom::solid::{self, Deflection};
use rayon::prelude::*;

const TOL: f64 = 1e-6;

fn deflection() -> Deflection {
    Deflection::new(0.02, 0.1).expect("valid")
}

/// Part `i`'s block: 10 × 20 × 30 at x = 40·i (distinct bytes per part).
fn block(i: usize) -> Point {
    Point::new(40.0 * i as f64, 0.0, 0.0)
}

/// Part `i`'s cutter: a 4 × 6 rectangle through the block along Z.
fn cutter_profile(i: usize) -> [Point; 4] {
    let x = 40.0 * i as f64;
    [
        Point::new(x + 3.0, 7.0, -5.0),
        Point::new(x + 7.0, 7.0, -5.0),
        Point::new(x + 7.0, 13.0, -5.0),
        Point::new(x + 3.0, 13.0, -5.0),
    ]
}

fn per_part(total: Duration, parts: usize) -> String {
    let micros = total.as_secs_f64() * 1e6 / parts as f64;
    if micros >= 1000.0 {
        format!("{:8.2} ms", micros / 1000.0)
    } else {
        format!("{micros:8.1} µs")
    }
}

struct Row {
    label: &'static str,
    total: Duration,
}

// One table, one function: splitting the steps apart would hide the order
// they are measured in (each row feeds the next).
#[allow(clippy::too_many_lines)]
fn run(parts: usize) {
    let mut rows: Vec<Row> = Vec::new();
    let mut time = |label: &'static str, f: &mut dyn FnMut()| {
        let start = Instant::now();
        f();
        rows.push(Row {
            label,
            total: start.elapsed(),
        });
    };

    // Constructors, value level (construct + serialize).
    let mut blocks: Vec<Solid> = Vec::with_capacity(parts);
    time("box (construct + serialize)", &mut || {
        blocks = (0..parts)
            .map(|i| solid::box_at(block(i), Vector::new(10.0, 20.0, 30.0)).expect("box"))
            .collect();
    });
    let mut cutters: Vec<Solid> = Vec::with_capacity(parts);
    time("extrude (construct + serialize)", &mut || {
        cutters = (0..parts)
            .map(|i| {
                solid::extrude_polygon(&cutter_profile(i), Vector::new(0.0, 0.0, 40.0), TOL)
                    .expect("prism")
            })
            .collect();
    });

    // The read: bytes → handle (the deep copy; a cache's would-be hit).
    let mut handles: Vec<Handle> = Vec::with_capacity(parts);
    time("read block (bytes → handle)", &mut || {
        handles = blocks
            .iter()
            .map(|b| Handle::from_value(b).expect("read"))
            .collect();
    });
    let mut cutter_handles: Vec<Handle> = Vec::with_capacity(parts);
    time("read cutter (bytes → handle)", &mut || {
        cutter_handles = cutters
            .iter()
            .map(|c| Handle::from_value(c).expect("read"))
            .collect();
    });
    // Serialize alone (the other half of a value round trip).
    time("serialize block (handle → bytes)", &mut || {
        for h in &handles {
            let _ = h.canonical_bytes().expect("bytes");
        }
    });

    // Difference from values (read ×2 + cut + serialize) vs from handles.
    let mut holes: Vec<Solid> = Vec::with_capacity(parts);
    time("difference FROM VALUES", &mut || {
        holes = blocks
            .iter()
            .zip(&cutters)
            .map(|(b, c)| solid::difference(b, c).expect("cut"))
            .collect();
    });
    let mut hole_handles: Vec<Handle> = Vec::with_capacity(parts);
    time("difference FROM HANDLES (+ serialize)", &mut || {
        hole_handles = handles
            .drain(..)
            .zip(cutter_handles.drain(..))
            .map(|(b, c)| {
                let hole = b.difference(c).expect("cut");
                let _ = hole.canonical_bytes().expect("bytes");
                hole
            })
            .collect();
    });

    // Tessellate from values (read + mesh + weld) vs from handles.
    time("tessellate hole FROM VALUE", &mut || {
        for hole in &holes {
            let _ = solid::tessellate(hole, deflection()).expect("mesh");
        }
    });
    time("tessellate hole FROM HANDLE", &mut || {
        for hole in hole_handles.drain(..) {
            let _ = hole.tessellate(deflection()).expect("mesh");
        }
    });

    // The chain, both ways.
    time("CHAIN values: box→extrude→diff→tess", &mut || {
        for i in 0..parts {
            let b = solid::box_at(block(i), Vector::new(10.0, 20.0, 30.0)).expect("box");
            let c = solid::extrude_polygon(&cutter_profile(i), Vector::new(0.0, 0.0, 40.0), TOL)
                .expect("prism");
            let hole = solid::difference(&b, &c).expect("cut");
            let _ = solid::tessellate(&hole, deflection()).expect("mesh");
        }
    });
    time("CHAIN handles (no re-reads)", &mut || {
        for i in 0..parts {
            let b = Handle::box_at(block(i), Vector::new(10.0, 20.0, 30.0)).expect("box");
            let c = Handle::extrude_polygon(&cutter_profile(i), Vector::new(0.0, 0.0, 40.0), TOL)
                .expect("prism");
            let hole = b.difference(c).expect("cut");
            let _ = hole.canonical_bytes().expect("bytes");
            let _ = hole.tessellate(deflection()).expect("mesh");
        }
    });
    time("CHAIN values on the rayon pool", &mut || {
        (0..parts).into_par_iter().for_each(|i| {
            let b = solid::box_at(block(i), Vector::new(10.0, 20.0, 30.0)).expect("box");
            let c = solid::extrude_polygon(&cutter_profile(i), Vector::new(0.0, 0.0, 40.0), TOL)
                .expect("prism");
            let hole = solid::difference(&b, &c).expect("cut");
            let _ = solid::tessellate(&hole, deflection()).expect("mesh");
        });
    });

    println!("parts = {parts}");
    println!("  {:<40} {:>12} {:>12}", "step", "per part", "total");
    for row in &rows {
        println!(
            "  {:<40} {} {:>9.1} ms",
            row.label,
            per_part(row.total, parts),
            row.total.as_secs_f64() * 1e3
        );
    }
    println!();
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().expect("part counts"))
        .collect();
    let sizes = if sizes.is_empty() {
        vec![1, 100, 1000]
    } else {
        sizes
    };
    println!(
        "solid seam cost table — release, {} rayon threads, deflection {:?}",
        rayon::current_num_threads(),
        deflection()
    );
    // Warm the kernel once (first-call static initialization) so the
    // 1-part row measures the operation, not OCCT's start-up.
    let _ = solid::box_at(Point::origin(), Vector::new(1.0, 1.0, 1.0)).expect("warm");
    println!();
    for parts in sizes {
        run(parts);
    }
}
