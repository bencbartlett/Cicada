//! The `import_step` node (v0.1 item 3 WP-C): the B-rep interchange
//! importer (docs/04, docs/08 §11).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`import_step`].
#[derive(Ports, Clone, Debug)]
pub struct ImportStepIn {
    /// The STEP file to read (`.step` / `.stp`); relative paths resolve
    /// against the pipeline's directory.
    pub path: String,
}

/// Import STEP — read every solid of a STEP file (OCCT's translator),
/// scaled into the document's unit, in the file's order. A file on disk is
/// external state, so this node is `volatile`: it is read again in every
/// solve, never served from the memo — a memo keyed by the path alone would
/// hand back stale geometry after the file changed. Everything downstream
/// keys on the solids' value hashes as usual, so an unchanged file
/// recomputes nothing below it.
///
/// # Returns
///
/// The file's solids.
///
/// # Panics
///
/// Panics when the file cannot be read or translated, or holds no solid
/// (shells and faces alone are not solids).
///
/// # Examples
///
/// The path is a fixture committed with the stdlib (a 1 × 2 × 3 block
/// written by `export_step`), relative to where the example runner starts;
/// point `path` at your own file.
///
/// ```cic
/// parts = import_step(path="../cicada-stdlib/fixtures/block.step")
/// count = length(list=parts)
/// ```
#[node(
    category = "Output, display & export",
    tier = "1",
    version = 1,
    gh = none,
    volatile,
    uses_tolerance
)]
#[must_use]
pub fn import_step(config: &ProjectConfig, input: ImportStepIn) -> Vec<Solid> {
    red(cicada_geom::solid::read_step(
        &input.path,
        config.unit().millimeters(),
    ))
}

#[cfg(test)]
mod tests {
    use cicada_core::config::Unit;
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{bounds_of, brep_box, config, volume_of, with_kernel};

    /// The file under test, written by the kernel; without the kernel the
    /// path is returned unwritten, so the node under test — not this
    /// fixture — is what refuses.
    fn write(dir: &tempfile::TempDir, name: &str, solids: &[Solid], mm: f64) -> String {
        let path = dir.path().join(name).to_string_lossy().into_owned();
        match cicada_geom::solid::write_step(solids, &path, mm, "t") {
            Ok(()) | Err(cicada_geom::GeomError::KernelUnavailable { .. }) => path,
            Err(error) => panic!("fixture: {error}"),
        }
    }

    #[test]
    fn import_step_reads_every_solid_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let Some(parts) = with_kernel(|| {
            let path = write(
                &dir,
                "two.step",
                &[
                    brep_box([0.0; 3], [1.0, 2.0, 3.0]),
                    brep_box([10.0; 3], [1.0; 3]),
                ],
                1.0,
            );
            import_step(&config(), ImportStepIn { path })
        }) else {
            return;
        };
        assert_eq!(parts.len(), 2);
        assert!((volume_of(&parts[0]) - 6.0).abs() < 1e-9);
        assert!((volume_of(&parts[1]) - 1.0).abs() < 1e-9);
        let (min, _) = bounds_of(&parts[1]);
        assert!(tol::coincident(
            min,
            cicada_core::spatial::Point::new(10.0, 10.0, 10.0),
            1e-6
        ));
        // Units: a millimetre file read into an inch document is scaled.
        let path = write(
            &dir,
            "mm.step",
            &[brep_box([0.0; 3], [25.4, 25.4, 25.4])],
            1.0,
        );
        let in_inches = import_step(
            &ProjectConfig::new(Unit::Inch, 1e-6, 1e-9).unwrap(),
            ImportStepIn { path },
        );
        assert!(
            (volume_of(&in_inches[0]) - 1.0).abs() < 1e-6,
            "{}",
            volume_of(&in_inches[0])
        );
    }

    #[test]
    fn import_step_reads_the_committed_fixture() {
        // The file the `# Examples` snippet reads (written by `export_step`
        // from a 1 × 2 × 3 block): it must stay a readable STEP solid.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/block.step").to_owned();
        let Some(parts) = with_kernel(|| import_step(&config(), ImportStepIn { path })) else {
            return;
        };
        assert_eq!(parts.len(), 1);
        assert!((volume_of(&parts[0]) - 6.0).abs() < 1e-9);
        let (min, max) = bounds_of(&parts[0]);
        assert!(tol::coincident(
            min,
            cicada_core::spatial::Point::origin(),
            1e-6
        ));
        assert!(tol::coincident(
            max,
            cicada_core::spatial::Point::new(1.0, 2.0, 3.0),
            1e-6
        ));
    }

    #[test]
    fn import_step_of_a_missing_file_is_red() {
        let Some(()) = with_kernel(|| {
            let outcome = std::panic::catch_unwind(|| {
                import_step(
                    &config(),
                    ImportStepIn {
                        path: "no/such/dir/anywhere/missing.step".to_owned(),
                    },
                )
            });
            let payload = outcome.expect_err("a missing file must be red");
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            assert!(message.contains("missing.step"), "{message}");
        }) else {
            return;
        };
    }

    #[test]
    fn spec_is_marked_volatile() {
        let spec = crate::registry()
            .iter()
            .find(|s| s.name == "import_step")
            .copied()
            .expect("registered");
        assert!(spec.volatile, "a file read is never memoized");
        assert!(spec.pure, "volatile, not effectful: it runs every solve");
    }

    proptest::proptest! {
        // Any box survives the round trip with its volume and its place.
        #[test]
        fn property_import_step_round_trips_boxes(
            ox in -20.0f64..20.0, sx in 0.1f64..30.0, sy in 0.1f64..30.0, sz in 0.1f64..30.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let dir = tempfile::tempdir().unwrap();
                let path = write(&dir, "p.step", &[brep_box([ox, 0.0, 0.0], [sx, sy, sz])], 1.0);
                let back = import_step(&config(), ImportStepIn { path });
                proptest::prop_assert_eq!(back.len(), 1);
                let want = sx * sy * sz;
                proptest::prop_assert!((volume_of(&back[0]) - want).abs() <= 1e-7 * want.max(1.0));
                let (min, _) = bounds_of(&back[0]);
                proptest::prop_assert!(tol::close(min.0.x, ox, 1e-6));
            }
        }
    }

    // A pass-through of external bytes: determinism is the identity of two
    // reads of the same file — the same canonical bytes, hence the same
    // value hash — and, for a file written by `export_step` from a solid,
    // the volume of that solid.
    #[test]
    fn import_step_determinism_two_reads_agree() {
        let dir = tempfile::tempdir().unwrap();
        let Some((first, second)) = with_kernel(|| {
            let path = write(&dir, "d.step", &[brep_box([0.0; 3], [1.0, 2.0, 3.0])], 1.0);
            (
                import_step(&config(), ImportStepIn { path: path.clone() }),
                import_step(&config(), ImportStepIn { path }),
            )
        }) else {
            return;
        };
        assert_eq!(first, second);
    }
}
