//! The `mesh_plane` node — the mesh tier's flat grid (docs/08 §8).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Mesh;
use cicada_core::scalar::Domain;
use cicada_core::spatial::Plane;
use cicada_geom::frame::orthonormal;
use cicada_geom::tol;
use cicada_macros::{Ports, node};

use crate::{checked_floor, checked_size, red};

/// Inputs for [`mesh_plane`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct MeshPlaneIn {
    /// The grid's frame: the mesh lies in it, facing along its normal.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Extent along the frame's x axis.
    pub x: Domain,
    /// Extent along the frame's y axis.
    pub y: Domain,
    /// Cells along the frame's x axis (each cell is two triangles).
    #[port(default = 1)]
    pub x_count: i64,
    /// Cells along the frame's y axis.
    #[port(default = 1)]
    pub y_count: i64,
}

/// Mesh Plane — a flat rectangular grid mesh in a plane's frame.
///
/// `x_count × y_count` cells of two triangles each over the `x × y`
/// rectangle, `(x_count + 1) × (y_count + 1)` vertices — the mesh tier's
/// ground plane, field sampling grid and relief base. Triangles wind
/// counter-clockwise seen from the frame's +z, so the mesh faces along the
/// normal; it is open (a sheet, not a `Watertight<Mesh>`). Decreasing
/// domains are normalized. Vertices run x fastest, then y.
///
/// # Returns
///
/// The grid mesh: `2 × x_count × y_count` triangles facing the frame's +z.
///
/// # Panics
///
/// Panics when an extent is empty at tolerance, the plane is degenerate, a
/// count is below 1, or the vertex count `(x_count + 1) × (y_count + 1)` is
/// above the shared ceilings (2^22 slots, or 1 GiB of mesh).
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=-5.0, end=5.0)
/// ground = mesh_plane(x=span, y=span, x_count=10, y_count=10)
/// ```
#[node(
    category = "Mesh & field",
    tier = "1",
    version = 1,
    gh = "Mesh Plane",
    uses_tolerance
)]
#[must_use]
pub fn mesh_plane(config: &ProjectConfig, input: MeshPlaneIn) -> Mesh {
    // The floors per count, then the ceiling on the vertices the grid
    // emits (a product), before any buffer is sized.
    let cells_x = checked_floor("mesh_plane", "x_count", input.x_count, 1);
    let cells_y = checked_floor("mesh_plane", "y_count", input.y_count, 1);
    let vertices = checked_size(
        "mesh_plane",
        &format!(
            "vertices at {}={}, {}={} ((x + 1) × (y + 1))",
            "x_count", input.x_count, "y_count", input.y_count
        ),
        (cells_x + 1) * (cells_y + 1),
        // A position (three f64) and two triangles per cell, about two per
        // vertex (three u32 each).
        3 * 8 + 2 * 3 * 4,
    );
    let frame = red(orthonormal(&input.plane, config.tol()));
    let span = |domain: &Domain, name: &'static str| -> (f64, f64) {
        assert!(
            !tol::close(domain.start, domain.end, config.tol()),
            "mesh_plane: {name} = {}..{} is empty at tolerance — the grid would have no width",
            domain.start,
            domain.end
        );
        (domain.start.min(domain.end), domain.start.max(domain.end))
    };
    let (x0, x1) = span(&input.x, "x");
    let (y0, y1) = span(&input.y, "y");
    #[allow(clippy::cast_possible_truncation)] // checked_size bounded the product to 2^22
    let (cells_x, cells_y) = (cells_x as usize, cells_y as usize);
    #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
    let (nx, ny) = (cells_x as f64, cells_y as f64);
    let mut positions = Vec::with_capacity(vertices * 3);
    for j in 0..=cells_y {
        #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
        let v = y0 + (y1 - y0) * (j as f64 / ny);
        for i in 0..=cells_x {
            #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
            let u = x0 + (x1 - x0) * (i as f64 / nx);
            let p = frame.point_at(u, v);
            positions.extend_from_slice(&[p.0.x, p.0.y, p.0.z]);
        }
    }
    let columns = cells_x + 1;
    let mut indices = Vec::with_capacity(cells_x * cells_y * 6);
    for j in 0..cells_y {
        for i in 0..cells_x {
            let index = |i: usize, j: usize| -> u32 {
                u32::try_from(j * columns + i)
                    .unwrap_or_else(|_| unreachable!("vertex count is bounded by 2^22"))
            };
            let (a, b, c, d) = (
                index(i, j),
                index(i + 1, j),
                index(i + 1, j + 1),
                index(i, j + 1),
            );
            // Counter-clockwise seen from +z: the normal points along the
            // frame's z.
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    red(Mesh::new(positions, indices).map_err(cicada_geom::GeomError::from))
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::{Point, Vector};
    use cicada_core::value::{HashedValue, ValueData};
    use glam::DVec3;

    use super::*;
    use crate::solids::support::config;

    fn grid(x: Domain, y: Domain, x_count: i64, y_count: i64) -> Mesh {
        mesh_plane(
            &config(),
            MeshPlaneIn {
                plane: Plane::world_xy(),
                x,
                y,
                x_count,
                y_count,
            },
        )
    }

    /// The mesh's total area and the mean of its triangle normals' signs
    /// along `up` (1 when every triangle faces `up`).
    fn area_and_facing(mesh: &Mesh, up: DVec3) -> (f64, f64) {
        let positions = mesh.positions();
        let at = |index: u32| {
            let i = usize::try_from(index).unwrap() * 3;
            DVec3::new(positions[i], positions[i + 1], positions[i + 2])
        };
        let (mut area, mut facing, mut triangles) = (0.0, 0.0, 0.0);
        for &[i, j, k] in mesh.indices().as_chunks::<3>().0 {
            let (a, b, c) = (at(i), at(j), at(k));
            let n = (b - a).cross(c - a);
            area += n.length() / 2.0;
            facing += n.dot(up).signum();
            triangles += 1.0;
        }
        (area, facing / triangles)
    }

    #[test]
    fn mesh_plane_table_cases() {
        // One cell: two triangles, four vertices at the corners, facing +z.
        let one = grid(Domain::new(0.0, 2.0), Domain::new(0.0, 1.0), 1, 1);
        assert_eq!(one.vertex_count(), 4);
        assert_eq!(one.triangle_count(), 2);
        let (area, facing) = area_and_facing(&one, DVec3::Z);
        assert!((area - 2.0).abs() < 1e-12);
        assert!((facing - 1.0).abs() < 1e-12);
        // A 3 × 2 grid over a decreasing x domain (normalized): 12 vertices
        // x fastest, the corners where the domains say.
        let grid = grid(Domain::new(3.0, 0.0), Domain::new(-1.0, 1.0), 3, 2);
        assert_eq!(grid.vertex_count(), 12);
        assert_eq!(grid.triangle_count(), 12);
        let p = grid.positions();
        let vertex = |i: usize| Point::new(p[3 * i], p[3 * i + 1], p[3 * i + 2]);
        assert!(tol::coincident(
            vertex(0),
            Point::new(0.0, -1.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            vertex(1),
            Point::new(1.0, -1.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            vertex(11),
            Point::new(3.0, 1.0, 0.0),
            1e-12
        ));
        let (area, facing) = area_and_facing(&grid, DVec3::Z);
        assert!((area - 6.0).abs() < 1e-12);
        assert!((facing - 1.0).abs() < 1e-12);
        // In a turned frame the sheet faces the frame's normal (world x
        // here: x along world y, y along world z).
        let turned = mesh_plane(
            &config(),
            MeshPlaneIn {
                plane: Plane {
                    origin: Point::new(5.0, 0.0, 0.0),
                    x: Vector::new(0.0, 1.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 1.0),
                x_count: 2,
                y_count: 2,
            },
        );
        let (area, facing) = area_and_facing(&turned, DVec3::X);
        assert!((area - 1.0).abs() < 1e-12);
        assert!((facing - 1.0).abs() < 1e-12);
        assert!(
            turned
                .positions()
                .as_chunks::<3>()
                .0
                .iter()
                .all(|p| (p[0] - 5.0).abs() < 1e-12)
        );
    }

    #[test]
    #[should_panic(expected = "mesh_plane: y = 2..2 is empty at tolerance")]
    fn mesh_plane_empty_extent_is_red() {
        let _ = grid(Domain::new(0.0, 1.0), Domain::new(2.0, 2.0), 1, 1);
    }

    #[test]
    #[should_panic(expected = "mesh_plane: x_count must be >= 1, got 0")]
    fn mesh_plane_zero_count_is_red() {
        let _ = grid(Domain::new(0.0, 1.0), Domain::new(0.0, 1.0), 0, 1);
    }

    // The absurd count: 10^11 × 2 vertices is a buffer no machine holds —
    // with the guard after the loops this test binary would abort on
    // allocation failure, so passing proves the refusal precedes it.
    #[test]
    #[should_panic(
        expected = "mesh_plane: vertices at x_count=100000000000, y_count=1 ((x + 1) × (y + 1)) would be 200000000002 — above the 4194304 (2^22) slot ceiling"
    )]
    fn mesh_plane_absurd_count_is_refused_not_allocated() {
        let _ = grid(
            Domain::new(0.0, 1.0),
            Domain::new(0.0, 1.0),
            100_000_000_000,
            1,
        );
    }

    proptest::proptest! {
        // Any grid: the vertex and triangle counts follow the cells, the
        // area is the rectangle's, and every triangle faces +z.
        #[test]
        fn property_mesh_plane_counts_area_and_facing(
            x0 in -50.0f64..50.0, dx in 0.01f64..40.0,
            y0 in -50.0f64..50.0, dy in 0.01f64..40.0,
            x_count in 1i64..12, y_count in 1i64..12,
        ) {
            let mesh = grid(Domain::new(x0, x0 + dx), Domain::new(y0, y0 + dy), x_count, y_count);
            let (cx, cy) = (usize::try_from(x_count).unwrap(), usize::try_from(y_count).unwrap());
            proptest::prop_assert_eq!(mesh.vertex_count(), (cx + 1) * (cy + 1));
            proptest::prop_assert_eq!(mesh.triangle_count(), 2 * cx * cy);
            let (area, facing) = area_and_facing(&mesh, DVec3::Z);
            proptest::prop_assert!((area - dx * dy).abs() <= 1e-9 * (dx * dy).max(1.0));
            proptest::prop_assert!((facing - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn mesh_plane_determinism_golden_hash() {
        // Dyadic spans over power-of-two cell counts: every vertex is exact
        // arithmetic. Blessed via run-once (2026-08-24).
        let mesh = grid(Domain::new(0.0, 2.0), Domain::new(-1.0, 1.0), 2, 4);
        let sealed = HashedValue::new(ValueData::Mesh(mesh)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "43f0f29ce4ef6fa551a969d1009b1ef97b1c9dfb5cc30799d84685a34c780816"
        );
    }
}
