//! The `volume` node (v0.1 item 3 WP-C).

use cicada_core::geometry::Solid;
use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`volume`].
#[derive(Ports, Clone, Debug)]
pub struct VolumeIn {
    /// The solid to measure.
    pub solid: Solid,
}

/// Outputs of [`volume`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct VolumeOut {
    /// The enclosed volume, in document units cubed.
    pub volume: f64,
    /// The volume centroid.
    pub centroid: Point,
}

/// Volume — the enclosed volume and the volume centroid of a B-rep solid
/// (OCCT's adaptive integration over the faces — exact for planar and
/// quadric faces, to 1e-9 relative for the rest; the mesh tier has no
/// volume node yet — tessellate first).
///
/// # Panics
///
/// Panics when the kernel cannot integrate the solid.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// size, middle = volume(solid=block)
/// ```
#[node(category = "Surface & solid", tier = "1", version = 1, gh = "Volume")]
#[must_use]
pub fn volume(input: VolumeIn) -> VolumeOut {
    let props = red(cicada_geom::solid::volume(&input.solid));
    VolumeOut {
        volume: props.volume,
        centroid: props.centroid,
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{brep_box, close_rel, config, fixture, plane_at, with_kernel};

    #[test]
    fn volume_table_cases() {
        let Some(out) = with_kernel(|| {
            volume(VolumeIn {
                solid: brep_box([1.0, 2.0, 3.0], [2.0, 4.0, 6.0]),
            })
        }) else {
            return;
        };
        assert!(close_rel(out.volume, 48.0, 1e-12));
        assert!(tol::coincident(
            out.centroid,
            Point::new(2.0, 4.0, 6.0),
            1e-9
        ));
        // A sphere's volume and centre.
        let ball = fixture(cicada_geom::solid::sphere(
            &plane_at(0.0, 0.0, 5.0),
            2.0,
            config().tol(),
        ));
        let out = volume(VolumeIn { solid: ball });
        assert!(close_rel(out.volume, 4.0 / 3.0 * PI * 8.0, 1e-9));
        assert!(tol::coincident(
            out.centroid,
            Point::new(0.0, 0.0, 5.0),
            1e-9
        ));
        // A cone's centroid sits a quarter of the way up.
        let cone = fixture(cicada_geom::solid::cone(
            &plane_at(0.0, 0.0, 0.0),
            3.0,
            4.0,
            config().tol(),
        ));
        let out = volume(VolumeIn { solid: cone });
        assert!(close_rel(out.volume, PI * 9.0 * 4.0 / 3.0, 1e-9));
        assert!(tol::coincident(
            out.centroid,
            Point::new(0.0, 0.0, 1.0),
            1e-9
        ));
    }

    proptest::proptest! {
        // Boxes anywhere: volume = product of extents, centroid = centre.
        #[test]
        fn property_volume_of_boxes(
            ox in -50.0f64..50.0, oy in -50.0f64..50.0, oz in -50.0f64..50.0,
            sx in 0.1f64..20.0, sy in 0.1f64..20.0, sz in 0.1f64..20.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = volume(VolumeIn {
                    solid: brep_box([ox, oy, oz], [sx, sy, sz]),
                });
                proptest::prop_assert!(close_rel(out.volume, sx * sy * sz, 1e-9));
                let centre = Point::new(ox + sx / 2.0, oy + sy / 2.0, oz + sz / 2.0);
                proptest::prop_assert!(tol::coincident(out.centroid, centre, 1e-7 * (1.0 + ox.abs() + oy.abs() + oz.abs())));
            }
        }
    }

    #[test]
    fn volume_determinism_golden_hash() {
        // The volume of a 1 × 2 × 3 box is the Number 6 — exactly, by the
        // planar-face integration — and its centroid (0.5, 1, 1.5).
        let Some(out) = with_kernel(|| {
            volume(VolumeIn {
                solid: brep_box([0.0; 3], [1.0, 2.0, 3.0]),
            })
        }) else {
            return;
        };
        let number = HashedValue::new(ValueData::Number(out.volume)).unwrap();
        assert_eq!(
            number.hash().to_hex(),
            HashedValue::new(ValueData::Number(6.0))
                .unwrap()
                .hash()
                .to_hex(),
            "volume {} is not exactly 6",
            out.volume
        );
        let centroid = HashedValue::new(ValueData::Point(out.centroid)).unwrap();
        assert_eq!(
            centroid.hash().to_hex(),
            HashedValue::new(ValueData::Point(Point::new(0.5, 1.0, 1.5)))
                .unwrap()
                .hash()
                .to_hex(),
            "centroid {:?} is not exactly (0.5, 1, 1.5)",
            out.centroid
        );
    }
}
