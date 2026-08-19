//! Point · Vector · Plane nodes (docs/08 §Catalog 5).

use cicada_core::config::ProjectConfig;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`construct_point`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructPointIn {
    /// X coordinate.
    #[port(default = 0.0, dimension = length)]
    pub x: f64,
    /// Y coordinate.
    #[port(default = 0.0, dimension = length)]
    pub y: f64,
    /// Z coordinate.
    #[port(default = 0.0, dimension = length)]
    pub z: f64,
}

/// Construct Point — a point from x/y/z coordinates.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn construct_point(input: ConstructPointIn) -> Point {
    Point::new(input.x, input.y, input.z)
}

/// Inputs for [`deconstruct_point`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructPointIn {
    /// The point.
    pub point: Point,
}

/// Outputs of [`deconstruct_point`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructPointOut {
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
}

/// Deconstruct Point — the x/y/z coordinates of a point.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn deconstruct_point(input: DeconstructPointIn) -> DeconstructPointOut {
    DeconstructPointOut {
        x: input.point.0.x,
        y: input.point.0.y,
        z: input.point.0.z,
    }
}

/// Inputs for the unit-vector nodes.
#[derive(Ports, Clone, Copy, Debug)]
pub struct UnitIn {
    /// Length of the produced vector.
    #[port(default = 1.0)]
    pub factor: f64,
}

/// Unit X — the world x direction, scaled.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn unit_x(input: UnitIn) -> Vector {
    Vector::new(input.factor, 0.0, 0.0)
}

/// Unit Y — the world y direction, scaled.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn unit_y(input: UnitIn) -> Vector {
    Vector::new(0.0, input.factor, 0.0)
}

/// Unit Z — the world z direction, scaled.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn unit_z(input: UnitIn) -> Vector {
    Vector::new(0.0, 0.0, input.factor)
}

/// Inputs for [`vector_2pt`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct Vector2PtIn {
    /// Tail point.
    pub a: Point,
    /// Head point.
    pub b: Point,
    /// Normalize the result to unit length.
    #[port(default = false)]
    pub unitize: bool,
}

/// Vector 2Pt — the vector from `a` to `b`, optionally unitized.
///
/// # Panics
///
/// Panics when `unitize` is on and the points coincide within tolerance —
/// a zero vector has no direction.
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    uses_tolerance
)]
#[must_use]
pub fn vector_2pt(config: &ProjectConfig, input: Vector2PtIn) -> Vector {
    let v = input.b.0 - input.a.0;
    if !input.unitize {
        return Vector(v);
    }
    let len = v.length();
    assert!(
        len > config.tol(),
        "vector_2pt: points coincide within tolerance ({len} apart) — \
         a zero vector has no direction to unitize"
    );
    Vector(v / len)
}

/// Inputs for the world-plane constructors.
#[derive(Ports, Clone, Copy, Debug)]
pub struct WorldPlaneIn {
    /// Plane origin.
    #[port(default = Point::origin(), default_doc = "origin")]
    pub origin: Point,
}

/// XY Plane — the world XY frame at an origin.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn xy_plane(input: WorldPlaneIn) -> Plane {
    Plane {
        origin: input.origin,
        ..Plane::world_xy()
    }
}

/// XZ Plane — the world XZ frame at an origin.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn xz_plane(input: WorldPlaneIn) -> Plane {
    Plane {
        origin: input.origin,
        ..Plane::world_xz()
    }
}

/// YZ Plane — the world YZ frame at an origin.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn yz_plane(input: WorldPlaneIn) -> Plane {
    Plane {
        origin: input.origin,
        ..Plane::world_yz()
    }
}

/// Inputs for [`construct_plane`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructPlaneIn {
    /// Plane origin.
    #[port(default = Point::origin(), default_doc = "origin")]
    pub origin: Point,
    /// The x axis (any length; unitized).
    #[port(default = Vector::new(1.0, 0.0, 0.0), default_doc = "unit_x")]
    pub x: Vector,
    /// The y direction (any length; its component along x is removed, then
    /// unitized — Gram–Schmidt).
    #[port(default = Vector::new(0.0, 1.0, 0.0), default_doc = "unit_y")]
    pub y: Vector,
}

/// Construct Plane — a frame from an origin and two axes: x unitized, y
/// orthonormalized against x (Gram–Schmidt), so the stored plane is a
/// right-handed orthonormal frame with normal x × y.
///
/// # Panics
///
/// Panics when `x` has no length at tolerance, or `y` is parallel to `x`
/// at tolerance (its component off the x line has no length) — red with
/// the measured length, never a NaN frame.
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    uses_tolerance
)]
#[must_use]
pub fn construct_plane(config: &ProjectConfig, input: ConstructPlaneIn) -> Plane {
    let frame = red(cicada_geom::frame::orthonormal(
        &Plane {
            origin: input.origin,
            x: input.x,
            y: input.y,
        },
        config.tol(),
    ));
    Plane {
        origin: frame.origin,
        x: Vector(frame.x),
        y: Vector(frame.y),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn construct_plane_table() {
        let config = ProjectConfig::default();
        // Already orthonormal: passes through exactly (unit axes, exact
        // arithmetic).
        let plane = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::new(1.0, 2.0, 3.0),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
        );
        assert_eq!(plane.origin, Point::new(1.0, 2.0, 3.0));
        assert_eq!(plane.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(plane.y, Vector::new(0.0, 1.0, 0.0));
        // Scaled axes unitize exactly here: (3,4,0) → (0.6,0.8,0), and the
        // already-orthogonal (0,0,2) → (0,0,1).
        let skewed = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(3.0, 4.0, 0.0),
                y: Vector::new(0.0, 0.0, 2.0),
            },
        );
        assert_eq!(skewed.x, Vector::new(0.6, 0.8, 0.0));
        assert_eq!(skewed.y, Vector::new(0.0, 0.0, 1.0));
        let tilted = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(2.0, 0.0, 0.0),
                y: Vector::new(5.0, 0.0, -0.25),
            },
        );
        // Gram–Schmidt: y = (5,0,-0.25) loses its x component, leaving
        // (0,0,-0.25) → (0,0,-1) — exact arithmetic.
        assert_eq!(tilted.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(tilted.y, Vector::new(0.0, 0.0, -1.0));
        // The defaults are the world XY frame.
        let world = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
        );
        assert_eq!(world, Plane::world_xy());
    }

    #[test]
    #[should_panic(expected = "x axis has length 0")]
    fn construct_plane_zero_x_is_red() {
        let _ = construct_plane(
            &ProjectConfig::default(),
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(0.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
        );
    }

    #[test]
    #[should_panic(expected = "y axis is parallel to x")]
    fn construct_plane_parallel_y_is_red() {
        let _ = construct_plane(
            &ProjectConfig::default(),
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(1.0, 1.0, 0.0),
                y: Vector::new(-2.0, -2.0, 0.0),
            },
        );
    }

    proptest::proptest! {
        // Any non-degenerate axes give a right-handed orthonormal frame
        // whose x is the unitized input x and whose plane contains the
        // input y (z ⟂ y): |x| = |y| = 1, x·y = 0, (x×y)·y_in = 0.
        #[test]
        fn property_construct_plane_is_orthonormal(
            xx in -10.0..10.0_f64, xy in -10.0..10.0_f64, xz in -10.0..10.0_f64,
            yx in -10.0..10.0_f64, yy in -10.0..10.0_f64, yz in -10.0..10.0_f64,
        ) {
            let x_in = Vector::new(xx, xy, xz).0;
            let y_in = Vector::new(yx, yy, yz).0;
            // Stay clear of the refusal band (length / parallel within 1e-6).
            proptest::prop_assume!(x_in.length() > 1e-3);
            proptest::prop_assume!(x_in.cross(y_in).length() > 1e-3 * x_in.length());
            let plane = construct_plane(
                &ProjectConfig::default(),
                ConstructPlaneIn {
                    origin: Point::origin(),
                    x: Vector(x_in),
                    y: Vector(y_in),
                },
            );
            let (x, y) = (plane.x.0, plane.y.0);
            proptest::prop_assert!((x.length() - 1.0).abs() < 1e-9);
            proptest::prop_assert!((y.length() - 1.0).abs() < 1e-9);
            proptest::prop_assert!(x.dot(y).abs() < 1e-9);
            proptest::prop_assert!((x - x_in.normalize()).length() < 1e-9);
            proptest::prop_assert!(x.cross(y).dot(y_in).abs() < 1e-9 * y_in.length());
            // Right-handed: y has a positive component along the input y.
            proptest::prop_assert!(y.dot(y_in) > 0.0);
        }
    }

    #[test]
    fn construct_plane_determinism_golden_hash() {
        // Exact arithmetic inputs (3-4-5 triangles, no transcendental).
        let plane = construct_plane(
            &ProjectConfig::default(),
            ConstructPlaneIn {
                origin: Point::new(1.0, -2.0, 0.5),
                x: Vector::new(3.0, 4.0, 0.0),
                y: Vector::new(0.0, 0.0, -2.0),
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Plane(plane))
                .unwrap()
                .hash()
                .to_hex(),
            "e07712f0f01191672c675a04261f321a7ae29c7702b4d0b9909d8080739b8fbb"
        );
    }

    #[test]
    fn point_roundtrip_table() {
        let point = construct_point(ConstructPointIn {
            x: 1.5,
            y: -2.0,
            z: 0.25,
        });
        assert_eq!(point, Point::new(1.5, -2.0, 0.25));
        let out = deconstruct_point(DeconstructPointIn { point });
        assert_eq!((out.x, out.y, out.z), (1.5, -2.0, 0.25));
    }

    #[test]
    fn unit_vectors_table() {
        assert_eq!(unit_x(UnitIn { factor: 2.0 }), Vector::new(2.0, 0.0, 0.0));
        assert_eq!(unit_y(UnitIn { factor: -1.0 }), Vector::new(0.0, -1.0, 0.0));
        assert_eq!(unit_z(UnitIn { factor: 0.5 }), Vector::new(0.0, 0.0, 0.5));
    }

    #[test]
    fn vector_2pt_table() {
        let config = ProjectConfig::default();
        let v = vector_2pt(
            &config,
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(4.0, 5.0, 1.0),
                unitize: false,
            },
        );
        assert_eq!(v, Vector::new(3.0, 4.0, 0.0));
        let unit = vector_2pt(
            &config,
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(4.0, 5.0, 1.0),
                unitize: true,
            },
        );
        assert_eq!(unit, Vector::new(0.6, 0.8, 0.0));
    }

    #[test]
    #[should_panic(expected = "coincide within tolerance")]
    fn vector_2pt_zero_unitize_is_red() {
        let _ = vector_2pt(
            &ProjectConfig::default(),
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(1.0, 1.0, 1.0),
                unitize: true,
            },
        );
    }

    #[test]
    fn world_planes_table() {
        let at = Point::new(5.0, 6.0, 7.0);
        let xy = xy_plane(WorldPlaneIn { origin: at });
        assert_eq!(xy.origin, at);
        assert_eq!(xy.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(xy.y, Vector::new(0.0, 1.0, 0.0));
        let xz = xz_plane(WorldPlaneIn { origin: at });
        assert_eq!(xz.y, Vector::new(0.0, 0.0, 1.0));
        let yz = yz_plane(WorldPlaneIn { origin: at });
        assert_eq!(yz.x, Vector::new(0.0, 1.0, 0.0));
    }

    proptest::proptest! {
        // Construct/deconstruct is the exact identity.
        #[test]
        fn property_point_roundtrip(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let out = deconstruct_point(DeconstructPointIn {
                point: construct_point(ConstructPointIn { x, y, z }),
            });
            proptest::prop_assert_eq!((out.x, out.y, out.z), (x, y, z));
        }

        // Unitized vectors have length 1 whenever the points differ.
        #[test]
        fn property_vector_2pt_unit_length(
            bx in 0.001f64..1.0e3,
            by in -1.0e3..1.0e3_f64,
        ) {
            let v = vector_2pt(
                &ProjectConfig::default(),
                Vector2PtIn {
                    a: Point::origin(),
                    b: Point::new(bx, by, 0.0),
                    unitize: true,
                },
            );
            proptest::prop_assert!((v.0.length() - 1.0).abs() < 1e-12);
        }

        // Each unit node places the factor on its own axis, exactly.
        #[test]
        fn property_unit_vectors_place_factor(factor in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(unit_x(UnitIn { factor }), Vector::new(factor, 0.0, 0.0));
            proptest::prop_assert_eq!(unit_y(UnitIn { factor }), Vector::new(0.0, factor, 0.0));
            proptest::prop_assert_eq!(unit_z(UnitIn { factor }), Vector::new(0.0, 0.0, factor));
        }

        // Each world-plane node carries the origin through exactly and
        // keeps its fixed world axes.
        #[test]
        fn property_world_planes_carry_origin(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let origin = Point::new(x, y, z);
            let input = WorldPlaneIn { origin };
            proptest::prop_assert_eq!(
                xy_plane(input),
                Plane { origin, ..Plane::world_xy() }
            );
            proptest::prop_assert_eq!(
                xz_plane(input),
                Plane { origin, ..Plane::world_xz() }
            );
            proptest::prop_assert_eq!(
                yz_plane(input),
                Plane { origin, ..Plane::world_yz() }
            );
        }
    }

    #[test]
    fn determinism_golden_hash() {
        let point = construct_point(ConstructPointIn {
            x: 3.0,
            y: -4.0,
            z: 5.0,
        });
        assert_eq!(
            HashedValue::new(ValueData::Point(point))
                .unwrap()
                .hash()
                .to_hex(),
            "6c5c651282fb21573785b37b6586208778691bfc17435d1180c89f47749be416"
        );
    }

    // Golden hashes for the rest of the family — one representative output
    // per node, arithmetic-exact inputs only (blessed via run-once).
    #[test]
    fn family_determinism_golden_hashes() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();

        // deconstruct_point: each output through the value model.
        let out = deconstruct_point(DeconstructPointIn {
            point: Point::new(1.5, -2.0, 0.25),
        });
        assert_eq!(
            hash(ValueData::Number(out.x)),
            "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03"
        );
        assert_eq!(
            hash(ValueData::Number(out.y)),
            "cc547e4fc9487f8991958b5f3d38e5a199bba3cbbdfe302c611d7f6ba944ad12"
        );
        assert_eq!(
            hash(ValueData::Number(out.z)),
            "71b099e9be5351c658523316836088b7b65d8d393e485cc825e0ce991ef90f01"
        );
        assert_eq!(
            hash(ValueData::Vector(unit_x(UnitIn { factor: 2.0 }))),
            "1b6e3426dcd04d7a833c119bf56008d39f59fac41d63367746e88cae9da50cda"
        );
        assert_eq!(
            hash(ValueData::Vector(unit_y(UnitIn { factor: -1.0 }))),
            "9f5accc6c03d40db6656244c5438ff823aa1ee28d72372f03461dfe78995d775"
        );
        assert_eq!(
            hash(ValueData::Vector(unit_z(UnitIn { factor: 0.5 }))),
            "e28e1b86a745a362445331e3d3aded83354ef778308f5bab9e7ed43467b037f6"
        );
        // vector_2pt: exact subtraction, no unitize (sqrt-free).
        let v = vector_2pt(
            &ProjectConfig::default(),
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(4.0, 5.0, 1.0),
                unitize: false,
            },
        );
        assert_eq!(
            hash(ValueData::Vector(v)),
            "2361344b7d2889cf286ff64869f15fe205f4a324384183711c0afc0645a762ef"
        );
        let at = Point::new(5.0, 6.0, 7.0);
        assert_eq!(
            hash(ValueData::Plane(xy_plane(WorldPlaneIn { origin: at }))),
            "0f65ed040fc802a10e2801932f6d6860f516f2b97d87305adb5f33292ccebc44"
        );
        assert_eq!(
            hash(ValueData::Plane(xz_plane(WorldPlaneIn { origin: at }))),
            "e6ae5bb16a22a69b55b52449e36ad07afa2effbf4b4aae8c6ce1b9070e67635e"
        );
        assert_eq!(
            hash(ValueData::Plane(yz_plane(WorldPlaneIn { origin: at }))),
            "b2042f66a665a6876ebc489fd83419cf03e7811727f50242cee206e969a8b5d7"
        );
    }
}
