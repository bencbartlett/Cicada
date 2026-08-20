//! Mesh serialization for the debug exporter: Wavefront OBJ, the
//! lowest-common-denominator every mesh viewer opens. Pure mesh → text;
//! file writing (the effectful part) lives in the export node.

use cicada_core::geometry::Mesh;
use std::fmt::Write as _;

/// Render meshes as one OBJ document, one `o`bject per mesh, 1-indexed
/// faces with per-object vertex offsets. Deterministic: `{}` formatting of
/// f64 is the shortest round-trip representation, stable across platforms.
#[must_use]
pub fn to_obj(meshes: &[(String, &Mesh)]) -> String {
    let mut out = String::from("# Cicada debug mesh export (Wavefront OBJ)\n");
    let mut offset = 1_usize; // OBJ indices are 1-based, global
    for (name, mesh) in meshes {
        // Sanitize: OBJ object names are whitespace-delimited.
        let clean: String = name
            .chars()
            .map(|c| if c.is_whitespace() { '_' } else { c })
            .collect();
        let _ = writeln!(out, "o {clean}");
        let (vertices, _) = mesh.positions().as_chunks::<3>();
        for [x, y, z] in vertices {
            let _ = writeln!(out, "v {x} {y} {z}");
        }
        let (triangles, _) = mesh.indices().as_chunks::<3>();
        for &[a, b, c] in triangles {
            let _ = writeln!(
                out,
                "f {} {} {}",
                offset + a as usize,
                offset + b as usize,
                offset + c as usize
            );
        }
        offset += mesh.vertex_count();
    }
    out
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
    fn obj_shape_and_offsets() {
        let a = triangle();
        let b = triangle();
        let obj = to_obj(&[("first mesh".to_owned(), &a), ("second".to_owned(), &b)]);
        assert!(obj.contains("o first_mesh\n"), "whitespace sanitized");
        assert!(obj.contains("f 1 2 3\n"));
        assert!(
            obj.contains("f 4 5 6\n"),
            "second object offsets past the first"
        );
        assert_eq!(obj.matches("\nv ").count(), 6);
    }

    #[test]
    fn obj_is_deterministic() {
        let mesh = triangle();
        let a = to_obj(&[("m".to_owned(), &mesh)]);
        let b = to_obj(&[("m".to_owned(), &mesh)]);
        assert_eq!(a, b);
    }
}
