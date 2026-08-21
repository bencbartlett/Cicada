//! Compiles cicada-geom's own OCCT glue (`src/occt/glue.rs` +
//! `src/occt/glue.hxx`) when the `occt` feature is on — the node-set half
//! of the seam (docs/03 §The OCCT seam as built; v0.1 item 3 WP-C). It
//! links the same prebuilt `OpenCASCADE` the fork's `opencascade-sys` does,
//! found through `DEP_OCCT_ROOT` (`tools/fetch_occt.py --print-env`), so
//! the two glues share one kernel, one `TopoDS_Shape`, one loader path.
//!
//! Without the feature this script does nothing: a default build that
//! turns `occt` off compiles no C++ and links no OCCT.

use std::env;
use std::path::{Path, PathBuf};

/// The OCCT toolkits the glue's translation unit needs. The fork links a
/// superset; naming ours keeps the link honest if that list ever shrinks.
const OCCT_LIBS: &[&str] = &[
    "TKernel",
    "TKMath",
    "TKG2d",
    "TKG3d",
    "TKGeomBase",
    "TKGeomAlgo",
    "TKBRep",
    "TKTopAlgo",
    "TKPrim",
    "TKBO",
    "TKBool",
    "TKOffset",
    "TKShHealing",
    "TKMesh",
    "TKXSBase",
    "TKDE",
    "TKDESTEP",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_OCCT").is_none() {
        return;
    }
    println!("cargo:rerun-if-env-changed=DEP_OCCT_ROOT");
    println!("cargo:rerun-if-changed=src/occt/glue.rs");
    println!("cargo:rerun-if-changed=src/occt/glue.hxx");

    let Some(root) = env::var_os("DEP_OCCT_ROOT") else {
        panic!(
            "cicada-geom feature `occt`: DEP_OCCT_ROOT is not set. Run \
             `python tools/fetch_occt.py --print-env bash` (or `powershell`) and export what it \
             prints before building (AGENTS.md §Command palette)."
        );
    };
    let root = PathBuf::from(root);
    let include = root.join("include").join("opencascade");
    assert!(
        include.join("TopoDS_Shape.hxx").is_file(),
        "cicada-geom feature `occt`: DEP_OCCT_ROOT={} holds no OCCT headers at {} — not the \
         prefix tools/fetch_occt.py installs",
        root.display(),
        include.display()
    );
    let lib_dir = root.join("lib");
    assert!(
        lib_dir.is_dir(),
        "cicada-geom feature `occt`: {} has no lib/ directory",
        root.display()
    );

    let manifest_dir = match env::var_os("CARGO_MANIFEST_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => panic!("cargo sets CARGO_MANIFEST_DIR for build scripts"),
    };
    // `include!("cicada-geom/src/occt/glue.hxx")` resolves through the
    // crates/ directory (the manifest dir's parent) on every OS — cxx-build's
    // default mapping relies on a symlink Windows does not always permit.
    let crates_dir: &Path = match manifest_dir.parent() {
        Some(parent) => parent,
        None => panic!("the crate lives inside a workspace directory"),
    };

    let mut build = cxx_build::bridge("src/occt/glue.rs");
    build
        .cpp(true)
        .std("c++17")
        .define("_USE_MATH_DEFINES", "TRUE")
        .include(&include)
        .include(crates_dir);
    build.compile("cicada-occt-glue");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    for lib in OCCT_LIBS {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}
