//! The `bounding_box` node (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Curve, GeometryValue, Rectangle, Solid};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point};
use cicada_geom::frame::{Frame, orthonormal};
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`bounding_box`].
#[derive(Ports, Clone, Debug)]
pub struct BoundingBoxIn {
    /// The geometry to bound — points, curves, meshes, solids; one box
    /// around all of them (lift with `each()` for one box per item).
    pub geometry: Vec<GeometryValue>,
    /// The frame the box is aligned to.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
}

/// Bounding Box — the smallest box aligned to a plane's frame that holds
/// all the geometry: exact on points, polylines, rectangles, circles and
/// meshes; a solid's bounds come from the kernel (its faces, not a
/// tessellation). The box is a B-rep `Solid`, so the geometry must have
/// extent along all three frame axes.
///
/// # Returns
///
/// The bounding box as a solid in the plane's frame.
///
/// # Panics
///
/// Panics when the list is empty, the plane is degenerate, the geometry is
/// flat along some frame axis (a box without volume cannot be a solid), or
/// the kernel refuses a solid.
///
/// # Examples
///
/// ```cic
/// ball = sphere(radius=1.5)
/// balls = duplicate(item=ball, count=1)
/// hull = bounding_box(geometry=balls)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Bounding Box",
    uses_tolerance
)]
#[must_use]
pub fn bounding_box(config: &ProjectConfig, input: BoundingBoxIn) -> Solid {
    assert!(
        !input.geometry.is_empty(),
        "bounding_box: no geometry to bound"
    );
    let tol = config.tol();
    let frame = red(orthonormal(&input.plane, tol));
    let world = red(orthonormal(&Plane::world_xy(), tol));
    let is_world = frame == world;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut take = |p: Point| {
        for (axis, value) in [p.0.x, p.0.y, p.0.z].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    };
    let local = |p: Point| Point(frame.coordinates(p));
    for item in &input.geometry {
        match item {
            GeometryValue::Point(p) => take(local(*p)),
            GeometryValue::Curve(curve) => curve_extremes(curve, &frame, tol, &mut take),
            GeometryValue::Mesh(mesh) => {
                for &[x, y, z] in mesh.positions().as_chunks::<3>().0 {
                    take(local(Point::new(x, y, z)));
                }
            }
            GeometryValue::Solid(solid) => {
                // The kernel measures world-aligned bounds: move the solid
                // into the frame's coordinates first (a rigid motion, exact
                // on its analytic faces) unless the frame IS the world.
                let (lo, hi) = if is_world {
                    red(cicada_geom::solid::bounds(solid))
                } else {
                    let into_frame = Similarity::plane_remap(&frame, &world);
                    let local = red(cicada_geom::solid::transform(solid, &into_frame));
                    red(cicada_geom::solid::bounds(&local))
                };
                take(lo);
                take(hi);
            }
        }
    }
    red(cicada_geom::solid::box_in_plane(
        &input.plane,
        Domain::new(min[0], max[0]),
        Domain::new(min[1], max[1]),
        Domain::new(min[2], max[2]),
        tol,
    ))
}

/// Feed a curve's extreme points (in frame coordinates) to `take`: the
/// vertices of a line / polyline / rectangle, and for a circle its exact
/// per-axis extent — `r · √(1 − (u·n)²)` along each frame axis `u`.
fn curve_extremes(curve: &Curve, frame: &Frame, tol: f64, take: &mut impl FnMut(Point)) {
    let local = |p: Point| Point(frame.coordinates(p));
    match curve {
        Curve::Line(line) => {
            take(local(line.a));
            take(local(line.b));
        }
        Curve::Polyline(polyline) => {
            for &v in &polyline.vertices {
                take(local(v));
            }
        }
        Curve::Rectangle(Rectangle { plane, x, y }) => {
            let rect = red(orthonormal(plane, tol));
            for (u, v) in [
                (x.start, y.start),
                (x.end, y.start),
                (x.end, y.end),
                (x.start, y.end),
            ] {
                take(local(rect.point_at(u, v)));
            }
        }
        Curve::Circle(circle) => {
            let own = red(orthonormal(&circle.plane, tol));
            let centre = frame.coordinates(circle.plane.origin);
            let half = |cosine: f64| circle.radius * (1.0 - cosine * cosine).max(0.0).sqrt();
            let (hx, hy, hz) = (
                half(frame.x.dot(own.z)),
                half(frame.y.dot(own.z)),
                half(frame.z.dot(own.z)),
            );
            take(Point::new(centre.x - hx, centre.y - hy, centre.z - hz));
            take(Point::new(centre.x + hx, centre.y + hy, centre.z + hz));
        }
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Circle;
    use cicada_core::spatial::Vector;
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{
        bounds_of, brep_box, close_rel, config, plane_at, platform_golden, solid_hash, volume_of,
        with_kernel,
    };

    #[test]
    fn bounding_box_table_cases() {
        // Points and a mesh-free mixed list: the hull of everything.
        let Some(hull) = with_kernel(|| {
            bounding_box(
                &config(),
                BoundingBoxIn {
                    geometry: vec![
                        GeometryValue::Point(Point::new(-1.0, 0.0, 0.0)),
                        GeometryValue::Point(Point::new(2.0, 3.0, 4.0)),
                        GeometryValue::Curve(Curve::Circle(Circle {
                            plane: plane_at(0.0, 0.0, 5.0),
                            radius: 1.0,
                        })),
                    ],
                    plane: Plane::world_xy(),
                },
            )
        }) else {
            return;
        };
        let (min, max) = bounds_of(&hull);
        assert!(
            tol::coincident(min, Point::new(-1.0, -1.0, 0.0), 1e-9),
            "{min:?}"
        );
        assert!(
            tol::coincident(max, Point::new(2.0, 3.0, 5.0), 1e-9),
            "{max:?}"
        );
        // A sphere's box is its diameter cube (kernel bounds, no
        // tessellation slack).
        let ball =
            cicada_geom::solid::sphere(&plane_at(1.0, 1.0, 1.0), 2.0, config().tol()).unwrap();
        let cube = bounding_box(
            &config(),
            BoundingBoxIn {
                geometry: vec![GeometryValue::Solid(ball.clone())],
                plane: Plane::world_xy(),
            },
        );
        assert!(close_rel(volume_of(&cube), 64.0, 1e-6));
        // In a frame with permuted axes the box follows the frame: a
        // 1 × 2 × 3 world box seen in a frame whose x is world y and y is
        // world z has frame extents 2 × 3 × 1 — and the same volume.
        let turned = Plane {
            origin: Point::origin(),
            x: Vector::new(0.0, 1.0, 0.0),
            y: Vector::new(0.0, 0.0, 1.0),
        };
        let boxed = bounding_box(
            &config(),
            BoundingBoxIn {
                geometry: vec![GeometryValue::Solid(brep_box([0.0; 3], [1.0, 2.0, 3.0]))],
                plane: turned,
            },
        );
        assert!(close_rel(volume_of(&boxed), 6.0, 1e-9));
        let (min, max) = bounds_of(&boxed);
        assert!(tol::coincident(min, Point::origin(), 1e-9), "{min:?}");
        assert!(
            tol::coincident(max, Point::new(1.0, 2.0, 3.0), 1e-9),
            "{max:?}"
        );
    }

    #[test]
    #[should_panic(expected = "no geometry to bound")]
    fn bounding_box_of_nothing_is_red() {
        let _ = bounding_box(
            &config(),
            BoundingBoxIn {
                geometry: vec![],
                plane: Plane::world_xy(),
            },
        );
    }

    #[test]
    #[should_panic(expected = "box extent must be above tolerance")]
    fn bounding_box_of_flat_geometry_is_red() {
        let _ = bounding_box(
            &config(),
            BoundingBoxIn {
                geometry: vec![
                    GeometryValue::Point(Point::origin()),
                    GeometryValue::Point(Point::new(1.0, 1.0, 0.0)),
                ],
                plane: Plane::world_xy(),
            },
        );
    }

    proptest::proptest! {
        // Two points anywhere (apart along every axis): the box spans
        // exactly their extents.
        #[test]
        fn property_bounding_box_of_two_points(
            ax in -50.0f64..50.0, ay in -50.0f64..50.0, az in -50.0f64..50.0,
            dx in 0.1f64..20.0, dy in 0.1f64..20.0, dz in 0.1f64..20.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = bounding_box(
                    &config(),
                    BoundingBoxIn {
                        geometry: vec![
                            GeometryValue::Point(Point::new(ax, ay, az)),
                            GeometryValue::Point(Point::new(ax + dx, ay + dy, az + dz)),
                        ],
                        plane: Plane::world_xy(),
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), dx * dy * dz, 1e-9));
            }
        }
    }

    #[test]
    fn bounding_box_determinism_golden_hash() {
        // The box around two exact points in the world frame is the
        // 1 × 2 × 3 box at the origin — the same bytes `box` makes.
        let Some(hull) = with_kernel(|| {
            bounding_box(
                &config(),
                BoundingBoxIn {
                    geometry: vec![
                        GeometryValue::Point(Point::origin()),
                        GeometryValue::Point(Point::new(1.0, 2.0, 3.0)),
                    ],
                    plane: Plane::world_xy(),
                },
            )
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&hull),
            platform_golden("2cd192d819ac8e052a47658c65e323883485a996c32a35bb8c69bf1f3e0bffce")
        );
    }
}
