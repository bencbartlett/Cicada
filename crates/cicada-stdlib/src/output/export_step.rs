//! The `export_step` node (v0.1 item 3 WP-C): the B-rep interchange
//! exporter (docs/04, docs/08 §11).

use std::fs;
use std::path::Path;

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`export_step`].
#[derive(Ports, Clone, Debug)]
pub struct ExportStepIn {
    /// The solids to write, one STEP product each, in order.
    pub solids: Vec<Solid>,
    /// Output path (`.step` / `.stp`); relative paths resolve against the
    /// pipeline's directory. The file stem names the STEP header and its
    /// products.
    pub path: String,
}

/// Export STEP — write B-rep solids to a STEP AP214 file (the CAD
/// interchange format — OCCT's own translator) in the document's unit,
/// byte-deterministic: the header's name, timestamp, author and
/// organisation are fixed and the products are numbered in file order, so
/// the same solids always give the same bytes and a re-export diffs clean.
///
/// # Panics
///
/// Panics when the list is empty (a STEP file without products is not an
/// export), when a solid cannot be translated, or when the file cannot be
/// written (missing directory, permissions) — an export that silently wrote
/// nothing is the worst outcome (wall lesson 7).
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=20.0)
/// block = box(x=span, y=span, z=span)
/// parts = duplicate(item=block, count=1)
/// step = export_step(solids=parts, path="block.step")
/// ```
#[node(
    category = "Output, display & export",
    tier = "1",
    version = 1,
    gh = none,
    effectful,
    uses_tolerance
)]
pub fn export_step(config: &ProjectConfig, input: ExportStepIn) {
    let stem = Path::new(&input.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("cicada");
    red(cicada_geom::solid::write_step(
        &input.solids,
        &input.path,
        config.unit().millimeters(),
        stem,
    ));
    // Say WHERE it landed, absolutely (as export_obj does): relative paths
    // resolve against the pipeline's directory, which `cicada run` and
    // `cicada serve` enter before solving.
    let resolved = fs::canonicalize(&input.path)
        .map_or_else(|_| input.path.clone(), |p| p.display().to_string());
    eprintln!(
        "export_step: wrote {} solid(s) to {resolved}",
        input.solids.len()
    );
}

#[cfg(test)]
mod tests {
    use cicada_core::config::Unit;

    use super::*;
    use crate::solids::support::{brep_box, config, with_kernel};

    fn path_in(dir: &tempfile::TempDir, name: &str) -> String {
        dir.path().join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn export_step_writes_a_step_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir, "parts.step");
        let Some(()) = with_kernel(|| {
            export_step(
                &config(),
                ExportStepIn {
                    solids: vec![
                        brep_box([0.0; 3], [1.0, 2.0, 3.0]),
                        brep_box([5.0; 3], [1.0; 3]),
                    ],
                    path: path.clone(),
                },
            );
        }) else {
            return;
        };
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("ISO-10303-21;"));
        assert!(
            text.contains("FILE_NAME('parts'"),
            "the stem names the file"
        );
        assert!(text.contains("'parts 1'") && text.contains("'parts 2'"));
        assert!(text.contains("MILLI"), "the document unit is declared");
        // The file round-trips through the kernel with the volumes intact.
        let back = cicada_geom::solid::read_step(&path, 1.0).unwrap();
        assert_eq!(back.len(), 2);
        let v: Vec<f64> = back
            .iter()
            .map(|s| cicada_geom::solid::volume(s).unwrap().volume)
            .collect();
        assert!(
            (v[0] - 6.0).abs() < 1e-9 && (v[1] - 1.0).abs() < 1e-9,
            "{v:?}"
        );
        // An inch document declares inches.
        let inch = path_in(&dir, "inch.stp");
        export_step(
            &ProjectConfig::new(Unit::Inch, 1e-6, 1e-9).unwrap(),
            ExportStepIn {
                solids: vec![brep_box([0.0; 3], [1.0; 3])],
                path: inch.clone(),
            },
        );
        assert!(std::fs::read_to_string(&inch).unwrap().contains("INCH"));
    }

    #[test]
    #[should_panic(expected = "at least one solid")]
    fn export_step_of_nothing_is_red() {
        let dir = tempfile::tempdir().unwrap();
        export_step(
            &config(),
            ExportStepIn {
                solids: vec![],
                path: path_in(&dir, "empty.step"),
            },
        );
    }

    #[test]
    fn export_step_unwritable_path_is_red() {
        let Some(()) = with_kernel(|| {
            let outcome = std::panic::catch_unwind(|| {
                export_step(
                    &config(),
                    ExportStepIn {
                        solids: vec![brep_box([0.0; 3], [1.0; 3])],
                        path: "no/such/dir/anywhere/out.step".to_owned(),
                    },
                );
            });
            let payload = outcome.expect_err("an unwritable path must be red");
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            assert!(message.contains("failed"), "{message}");
        }) else {
            return;
        };
    }

    #[test]
    fn spec_is_marked_effectful() {
        let spec = crate::registry()
            .iter()
            .find(|s| s.name == "export_step")
            .copied()
            .expect("registered");
        assert!(!spec.pure, "exporters are effectful — never auto-run");
    }

    proptest::proptest! {
        // Any box exports and reads back with its volume, in any document
        // unit — the unit declaration and the coordinates agree.
        #[test]
        fn property_export_step_round_trips_boxes(
            sx in 0.1f64..50.0, sy in 0.1f64..50.0, sz in 0.1f64..50.0,
            unit_index in 0usize..5,
        ) {
            if cicada_geom::solid::kernel_available() {
                let unit = [Unit::Millimeter, Unit::Centimeter, Unit::Meter, Unit::Inch, Unit::Foot][unit_index];
                let project = ProjectConfig::new(unit, 1e-6, 1e-9).unwrap();
                let dir = tempfile::tempdir().unwrap();
                let path = path_in(&dir, "prop.step");
                export_step(
                    &project,
                    ExportStepIn {
                        solids: vec![brep_box([0.0; 3], [sx, sy, sz])],
                        path: path.clone(),
                    },
                );
                let back = cicada_geom::solid::read_step(&path, unit.millimeters()).unwrap();
                proptest::prop_assert_eq!(back.len(), 1);
                let volume = cicada_geom::solid::volume(&back[0]).unwrap().volume;
                let want = sx * sy * sz;
                proptest::prop_assert!((volume - want).abs() <= 1e-7 * want.max(1.0), "{} vs {}", volume, want);
            }
        }
    }

    // Determinism for an effectful sink is the WRITTEN BYTES: two exports
    // of the same solids — in the same process, where OCCT's own product
    // counter would otherwise have moved — are identical.
    #[test]
    fn export_step_determinism_golden_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let Some(first) = with_kernel(|| {
            let path = path_in(&dir, "a.step");
            export_step(
                &config(),
                ExportStepIn {
                    solids: vec![brep_box([0.0; 3], [1.0, 2.0, 3.0])],
                    path: path.clone(),
                },
            );
            std::fs::read(&path).unwrap()
        }) else {
            return;
        };
        // A different file name between the two writes would change the
        // header's name: write the second under the same stem in a second
        // directory.
        let other = tempfile::tempdir().unwrap();
        let path = path_in(&other, "a.step");
        export_step(
            &config(),
            ExportStepIn {
                solids: vec![brep_box([0.0; 3], [1.0, 2.0, 3.0])],
                path: path.clone(),
            },
        );
        let second = std::fs::read(&path).unwrap();
        assert_eq!(first, second, "same solids, same bytes");
        let text = String::from_utf8(first).unwrap();
        assert!(text.contains(cicada_geom::solid::STEP_TIMESTAMP));
        assert!(!text.contains("2026-"), "no wall-clock anywhere: {text}");
    }
}
