//! The `export_obj` node.

use std::fs;

use cicada_core::geometry::Mesh;
use cicada_macros::{Ports, node};

/// Inputs for [`export_obj`].
#[derive(Ports, Clone, Debug)]
pub struct ExportObjIn {
    /// The meshes to write — ANY meshes (this is the debug viewer's door;
    /// watertightness is not required to look at geometry).
    pub meshes: Vec<Mesh>,
    /// Output path (`.obj`); relative paths resolve against the working
    /// directory of the `cicada run` invocation.
    pub path: String,
}

/// Export OBJ — write meshes to a Wavefront OBJ file for external
/// viewers (the stage-4 debug window into headless geometry; the real
/// exporters — 3MF, DXF — arrive with the wall corpus).
///
/// # Panics
///
/// Panics when the file cannot be written (missing directory,
/// permissions) — an export that silently wrote nothing is the worst
/// outcome (wall lesson 7).
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = mesh_box(x=span, y=span, z=span)
/// still = unit_x(factor=0.0)
/// blocks = linear_array(geometry=block, direction=still, count=1)
/// dump = export_obj(meshes=blocks, path="block.obj")
/// ```
#[node(
    category = "Output, display & export",
    tier = "S",
    version = 1, gh = none,
    effectful
)]
pub fn export_obj(input: ExportObjIn) {
    let named: Vec<(String, &Mesh)> = input
        .meshes
        .iter()
        .enumerate()
        .map(|(index, mesh)| (format!("mesh_{index}"), mesh))
        .collect();
    let obj = cicada_geom::export::to_obj(&named);
    if let Err(error) = fs::write(&input.path, obj) {
        panic!("export_obj: writing `{}` failed: {error}", input.path);
    }
    // Say WHERE it landed, absolutely — an export whose destination the
    // user has to hunt for is halfway to wall lesson 7. Relative paths
    // resolve against the pipeline's directory: `cicada run` and `cicada
    // serve` both enter it before solving (stage 5).
    let resolved = fs::canonicalize(&input.path)
        .map_or_else(|_| input.path.clone(), |p| p.display().to_string());
    eprintln!(
        "export_obj: wrote {} mesh(es) to {resolved}",
        input.meshes.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Mesh {
        Mesh::new(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
        )
        .expect("valid")
    }

    #[test]
    fn writes_an_obj_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.obj");
        export_obj(ExportObjIn {
            meshes: vec![triangle(), triangle()],
            path: path.to_string_lossy().into_owned(),
        });
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("o mesh_0"));
        assert!(written.contains("o mesh_1"));
        assert!(written.contains("f 4 5 6"));
    }

    #[test]
    #[should_panic(expected = "failed")]
    fn unwritable_path_is_red() {
        export_obj(ExportObjIn {
            meshes: vec![triangle()],
            path: "no/such/dir/anywhere/out.obj".to_owned(),
        });
    }

    #[test]
    fn spec_is_marked_effectful() {
        let spec = crate::registry()
            .iter()
            .find(|s| s.name == "export_obj")
            .copied()
            .expect("registered");
        assert!(!spec.pure, "exporters are effectful — never auto-run");
    }

    proptest::proptest! {
        // Any structurally valid mesh writes without panicking (fan
        // triangulations over arbitrary vertex clouds).
        #[test]
        fn property_export_accepts_any_valid_mesh(
            coords in proptest::collection::vec(-100.0..100.0_f64, 9..30),
            triangles in 1usize..8,
        ) {
            let vertex_count = coords.len() / 3;
            let mut positions = coords;
            positions.truncate(vertex_count * 3);
            let mut indices: Vec<u32> = Vec::new();
            for i in 0..triangles {
                // Fan around vertex 0; a and b are distinct and nonzero
                // because vertex_count >= 3.
                let a = 1 + (i % (vertex_count - 1));
                let b = 1 + ((i + 1) % (vertex_count - 1));
                indices.push(0);
                indices.push(u32::try_from(a).unwrap());
                indices.push(u32::try_from(b).unwrap());
            }
            let mesh = Mesh::new(positions, indices).expect("structurally valid");
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("prop.obj");
            export_obj(ExportObjIn {
                meshes: vec![mesh],
                path: path.to_string_lossy().into_owned(),
            });
            let written = std::fs::read_to_string(&path).unwrap();
            proptest::prop_assert!(written.starts_with("# Cicada debug mesh export"));
            proptest::prop_assert!(written.contains("o mesh_0"));
        }
    }

    // Determinism for an effectful sink is the WRITTEN BYTES: two writes of
    // the same input are identical, and match the golden text exactly (the
    // OBJ writer is pure string formatting — no libm, cross-platform
    // stable).
    #[test]
    fn export_determinism_golden_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str| {
            let path = dir.path().join(name);
            export_obj(ExportObjIn {
                meshes: vec![triangle()],
                path: path.to_string_lossy().into_owned(),
            });
            std::fs::read(&path).unwrap()
        };
        let first = write("a.obj");
        let second = write("b.obj");
        assert_eq!(first, second, "same input, same bytes");
        let expected = "# Cicada debug mesh export (Wavefront OBJ)\n\
                        o mesh_0\n\
                        v 0 0 0\n\
                        v 1 0 0\n\
                        v 0 1 0\n\
                        f 1 2 3\n";
        assert_eq!(String::from_utf8(first).unwrap(), expected);
    }
}
