//! Every stdlib node's `# Examples` snippet solves (DECISIONS.md
//! documentation row, revised 2026-08-19: `# Examples` is a REQUIRED
//! node-doc section — runnable, CI-solved; doc 14 §Documentation pipeline:
//! "an example that stops solving fails the build").
//!
//! Why this lives in `cicada-cli`: the runner needs the registry, the
//! server's compile/lower path (`cicada run` is a printer over it), and the
//! scheduler. `cicada-stdlib` never depends on `cicada-sched` (dependency
//! law), and only this crate may depend on the server — so the crate that
//! owns `cicada run` owns the example runner. The static conformance checks
//! (an example exists, it calls the node) are `cicada-stdlib`'s own
//! integration test; this one proves the examples are TRUE.
//!
//! For each registered node and each of its examples: add the `# cicada 1`
//! header, parse + check against the stdlib catalog (ZERO diagnostics — an
//! example with a warning outside its cone is still a wrong example),
//! lower, solve headlessly with a fresh temp cache, and require every
//! target green. Effectful nodes (the exporters) are checked and their
//! inputs solved, but the exporter itself is never run — they never
//! auto-run (doc 10 §7), and a test that wrote files would prove nothing
//! about the example anyway. The test asserts the exporter WAS the skipped
//! leaf, so an exporter example that forgot to call the exporter cannot
//! pass. Display sinks (`panel`, `custom_preview`, `text_tag`) are pure and
//! solve like any node.
//!
//! One test, all nodes, all failures reported together: a tranche of new
//! nodes gets one list, not one failure per rerun.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use cicada_core::config::ProjectConfig;
use cicada_core::spec::NodeSpec;
use cicada_lang::ast::Rhs;
use cicada_sched::{
    CancelToken, DiskStore, MonotonicClock, NodeId, NodeOutcome, NoopObserver, Scheduler,
    SchedulerConfig,
};
use cicada_server::compile::{self, Loaded};
use cicada_server::lower::{LoweredBinding, lower};
use cicada_server::scripts::ScriptCancel;

const HEADER: &str = "# cicada 1\n";

/// Assemble one example into a pipeline text the way the docs describe it:
/// the snippet is header-less, the runner adds the header.
fn pipeline_text(example: &str) -> String {
    format!("{HEADER}{example}\n")
}

/// A checked example: the loaded pipeline plus the targets a headless run
/// would solve.
struct Checked {
    loaded: Loaded,
    targets: compile::Targets,
}

/// Run one example end to end; `Err` carries the reason.
fn solve_example(dir: &Path, spec: &NodeSpec, index: usize, example: &str) -> Result<(), String> {
    let checked = check_example(dir, spec, index, example)?;
    solve_checked(dir, spec, index, &checked)
}

/// Phase 1: parse + check (zero diagnostics), the node is called, and the
/// targets resolve the way `cicada run` resolves them.
fn check_example(
    dir: &Path,
    spec: &NodeSpec,
    index: usize,
    example: &str,
) -> Result<Checked, String> {
    let pipeline = dir.join(format!("{}_{index}.cic", spec.name));
    let source = pipeline_text(example);
    std::fs::write(&pipeline, &source).map_err(|e| format!("writing the pipeline: {e}"))?;

    // Parse + check. No script discovery happens (no scripts/ next to the
    // file), so the catalog is exactly the stdlib.
    let loaded = compile::load(&pipeline, &source, &ScriptCancel::new())
        .map_err(|e| format!("loading: {e}"))?;
    let (document, specs, resolution) = (&loaded.document, &loaded.specs, &loaded.resolution);
    if !resolution.diagnostics.is_empty() {
        let rendered = resolution
            .diagnostics
            .iter()
            .map(|d| format!("    {d:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "the checker reports {} diagnostic(s):\n{rendered}",
            resolution.diagnostics.len()
        ));
    }

    // The example must exercise the node it documents (a call, not just a
    // mention in a name).
    let calls_node = document.statements().any(|(_, statement, _)| {
        matches!(&statement.rhs, Rhs::Call(call) if call.func.name == spec.name)
    });
    if !calls_node {
        return Err(format!("no statement calls `{}`", spec.name));
    }

    // Targets as `cicada run` resolves them with no `--node`: every
    // non-effectful leaf, plus the INPUTS of effectful leaves, which are
    // skipped (exporters never auto-run).
    let targets = compile::resolve_targets(document, specs, &[])
        .map_err(|e| format!("resolving targets: {e}"))?;
    if spec.pure {
        if !targets.skipped_effectful.is_empty() {
            return Err(format!(
                "a pure node's example must not end in an exporter (skipped: {:?})",
                targets.skipped_effectful
            ));
        }
    } else {
        // The exporter must be the leaf that was skipped — otherwise the
        // example documents something else.
        let exporter_skipped = targets.skipped_effectful.iter().any(|name| {
            document.statements().any(|(_, statement, _)| {
                statement.targets.iter().any(|t| &t.name == name)
                    && matches!(&statement.rhs, Rhs::Call(call) if call.func.name == spec.name)
            })
        });
        if !exporter_skipped {
            return Err(format!(
                "the effectful call to `{}` must be a leaf the run skips; skipped: {:?}",
                spec.name, targets.skipped_effectful
            ));
        }
    }
    Ok(Checked { loaded, targets })
}

/// Phase 2: lower and solve with a fresh cache; every target computes,
/// nothing is red, and no effectful node ran.
fn solve_checked(
    dir: &Path,
    spec: &NodeSpec,
    index: usize,
    checked: &Checked,
) -> Result<(), String> {
    let Loaded {
        document,
        specs,
        scripts,
        resolution,
    } = &checked.loaded;
    let targets = &checked.targets;
    let effectful_bindings = compile::effectful_bindings(document, specs);

    let config = ProjectConfig::default();
    let lowered = lower(
        document,
        resolution,
        specs,
        &config,
        &targets.names,
        scripts,
    )
    .map_err(|e| format!("lowering: {e}"))?;

    // A fresh store per example: nothing is ever a cache hit here, so the
    // node really computes (the memo cannot hide a broken node).
    let cache_dir = dir.join(format!("cache_{}_{index}", spec.name));
    let (store, _report) =
        DiskStore::open(&cache_dir).map_err(|e| format!("opening the store: {e}"))?;
    let scheduler = Scheduler::new(
        Arc::new(store),
        Arc::new(MonotonicClock::new()),
        SchedulerConfig {
            threads: 2,
            ..SchedulerConfig::default()
        },
    )
    .map_err(|e| format!("scheduler: {e}"))?;

    let mut target_ids: Vec<NodeId> = targets
        .names
        .iter()
        .filter_map(|name| match lowered.bindings.get(name) {
            Some(LoweredBinding::Port { node, .. } | LoweredBinding::Node { node }) => Some(*node),
            Some(LoweredBinding::Value(_)) | None => None,
        })
        .collect();
    target_ids.sort_unstable();
    target_ids.dedup();
    if target_ids.is_empty() {
        return Err(format!(
            "nothing to solve — the targets {:?} are all literals",
            targets.names
        ));
    }

    let report = scheduler
        .solve(
            &lowered.graph,
            &target_ids,
            0,
            &CancelToken::new(),
            &NoopObserver,
        )
        .map_err(|e| format!("solving: {e}"))?;

    let failures = report.failures();
    if !failures.is_empty() {
        let rendered = failures
            .iter()
            .map(|f| format!("    red `{}` — {}", f.node, f.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("{} red node(s):\n{rendered}", failures.len()));
    }
    for id in &target_ids {
        match report.outcome(*id) {
            NodeOutcome::Computed { .. } => {}
            NodeOutcome::CacheHit { .. } => {
                return Err(format!(
                    "`{}` was a cache hit in a fresh store — the runner is not exercising it",
                    lowered.graph.node(*id).name
                ));
            }
            other => {
                return Err(format!(
                    "target `{}` did not compute: {other:?}",
                    lowered.graph.node(*id).name
                ));
            }
        }
    }
    // Exporters never ran: they are not in the target set, and the graph
    // only ever pulls ancestors of targets.
    for name in &effectful_bindings {
        if let Some(LoweredBinding::Node { node } | LoweredBinding::Port { node, .. }) =
            lowered.bindings.get(name)
            && !matches!(report.outcome(*node), NodeOutcome::Skipped)
        {
            return Err(format!("effectful `{name}` ran — exporters never auto-run"));
        }
    }
    Ok(())
}

#[test]
fn every_registered_node_example_solves() {
    let dir = tempfile::tempdir().unwrap();
    let mut failures: Vec<String> = Vec::new();
    let mut solved = 0_usize;
    let mut effectful_checked = 0_usize;
    for spec in cicada_stdlib::registry() {
        if spec.examples.is_empty() {
            // The conformance test in cicada-stdlib reports this with the
            // rest of the format; here it is a failure too, so this test
            // alone is a complete statement of "examples solve".
            failures.push(format!("`{}`: no example to run", spec.name));
            continue;
        }
        for (index, example) in spec.examples.iter().enumerate() {
            match solve_example(dir.path(), spec, index, example) {
                Ok(()) => {
                    solved += 1;
                    if !spec.pure {
                        effectful_checked += 1;
                    }
                }
                Err(reason) => failures.push(format!(
                    "`{}` example {index}:\n  {reason}\n  snippet:\n{}",
                    spec.name,
                    example
                        .lines()
                        .map(|l| format!("    | {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} example(s) do not solve:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // The test's own assumptions: it ran the whole catalog (57 nodes at the
    // spike's close, one example each) and exercised the exporter path.
    assert!(solved >= 57, "only {solved} examples ran");
    assert!(
        effectful_checked >= 1,
        "no effectful node was checked — the exporter path of this test is dead"
    );
}

// The runner's own contract, so a silent change in how examples are
// assembled cannot make the test above vacuous.
#[test]
fn a_broken_example_is_reported_not_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let series = cicada_stdlib::registry()
        .iter()
        .find(|s| s.name == "series")
        .copied()
        .expect("series registered");
    // A checker error (unknown kwarg) — refused before solving.
    let err = solve_example(dir.path(), series, 0, "xs = series(cont=4)").unwrap_err();
    assert!(err.contains("diagnostic"), "{err}");
    // A solve-time refusal (negative count goes red) — reported as red.
    let err = solve_example(dir.path(), series, 1, "xs = series(count=-1)").unwrap_err();
    assert!(err.contains("red `xs`"), "{err}");
    // An example that never calls the node it documents.
    let err = solve_example(dir.path(), series, 2, "x = add(a=1.0, b=2.0)").unwrap_err();
    assert!(err.contains("no statement calls `series`"), "{err}");
    // And the real one solves.
    solve_example(dir.path(), series, 3, series.examples[0]).unwrap();
}
