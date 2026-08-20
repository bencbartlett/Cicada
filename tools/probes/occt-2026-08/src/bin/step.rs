//! `occt-probe-step` — the STEP-linking twin of `occt-probe`.
//!
//! Exists for one measurement (memo §Q1, "how many DLLs would a shipped
//! cicada.exe need"): the main probe references no STEP symbols, so the
//! linker drops `TKDESTEP` and its closure, and the 15-DLL answer it gives
//! is the MODELING answer only. This binary calls `Shape::write_step` /
//! `read_step`, which pulls `TKDESTEP -> TKXCAF -> TKV3d -> TKService ->
//! freetype.dll + FreeImage.dll (+ FreeImage's codec DLLs)` into the load
//! set. Run `dll_closure.py` on both executables and launch this one with
//! and without the extra DLL dirs on PATH to reproduce the numbers.
//!
//! Usage: `occt-probe-step <out_dir>` — writes `box.step`, reads it back,
//! prints the `FILE_NAME` header line (the run-dependent timestamp the
//! memo's Q2 mentions) and the round-tripped face count. Exit 0 = the
//! process loaded and STEP worked end to end.

use std::path::Path;

use glam::dvec3;
use opencascade::primitives::Shape;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).expect("usage: occt-probe-step <out_dir>");
    let out = Path::new(out);
    std::fs::create_dir_all(out).expect("create out dir");

    let shape = Shape::box_from_corners(dvec3(0.0, 0.0, 0.0), dvec3(10.0, 20.0, 30.0));
    let path = out.join("box.step");
    shape.write_step(&path).expect("STEPControl_Writer");
    let text = std::fs::read_to_string(&path).expect("read box.step");
    let file_name_line = text
        .lines()
        .find(|l| l.starts_with("FILE_NAME"))
        .expect("STEP header has a FILE_NAME entity");
    println!("step bytes={} {}", text.len(), file_name_line);

    let back = Shape::read_step(&path).expect("STEPControl_Reader");
    let faces = back.faces().count();
    assert_eq!(faces, 6, "round-tripped box must have 6 faces, got {faces}");
    println!(
        "step round-trip OK: faces={faces} shape_type={:?}",
        back.shape_type()
    );
}
