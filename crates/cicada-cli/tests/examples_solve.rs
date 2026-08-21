//! Every `examples/**/*.cic` solves (docs/17 Follow-ups "CI solves every
//! example"; the rule is stated in `examples/README.md`): each example is
//! run headlessly with a fresh cache through the same compile → lower →
//! solve path as `cicada run` (no `--node`: every non-effectful leaf; the
//! exporters' inputs solve, the exporters never run), and every binding
//! must come out green — zero checker diagnostics, zero red, zero blocked.
//!
//! Why this lives in `cicada-cli`: like `node_examples.rs`, the runner
//! needs the server's compile/lower path plus the scheduler, and only this
//! crate may depend on the server (dependency law). Why in-process rather
//! than through the binary: the solve report names the failing BINDING and
//! its reason — a "red" line with the node's message, a "blocked … fed by
//! red …" line per dependant — instead of a stderr transcript to grep; the
//! functions are the ones `run.rs` calls, so the path is the same.
//!
//! Discovery is the point — a new example is picked up by its extension,
//! never by a list — so the test also pins its own discovery (the files
//! it must have found), and the runner's contract is pinned by the
//! broken-pipeline cases below (a red binding, a blocked one, a diagnostic
//! are each reported, never skipped).
//!
//! The wall is included. Measured cold in DEBUG on the 24-core dev machine
//! (2026-08-20): 6.9 s at the default thread count (cores − 2), 18 s at 4
//! threads, 34 s at 2 — under the minute the follow-up allowed. CI's
//! 4-vCPU runners land around a minute; if that ever dominates the suite,
//! exclude it here by an explicit list WITH the reason, and say so in
//! `examples/README.md`.
//!
//! Stricter than `cicada run` in exactly one way, on purpose: `run` gates
//! diagnostics to the target cone and prints the rest as warnings; an
//! example with a warning ANYWHERE is a wrong example (the app paints it).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cicada_core::config::ProjectConfig;
use cicada_sched::{
    CancelToken, DiskStore, MonotonicClock, NodeId, NodeOutcome, NoopObserver, Scheduler,
    SchedulerConfig,
};
use cicada_server::compile::{self, Loaded};
use cicada_server::lower::{Lowered, LoweredBinding, lower};
use cicada_server::scripts::ScriptCancel;

/// `examples/` at the workspace root (this crate is `crates/cicada-cli`).
fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .canonicalize()
        .expect("examples/ exists at the workspace root")
}

/// Every `*.cic` under `root`, recursively, in a stable (sorted) order.
fn discover_pipelines(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "cic") {
                out.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort();
    found
}

/// Solve one pipeline end to end the way `cicada run <file> --cache-dir
/// <fresh>` does; `Err` is the human-readable list of what went wrong,
/// every line naming a binding.
fn solve_pipeline(pipeline: &Path, cache_dir: &Path) -> Result<(), String> {
    let prepared = check_and_lower(pipeline)?;
    solve_prepared(&prepared, cache_dir)
}

/// A checked, lowered pipeline: the graph, the targets a headless run
/// solves, and the exporters it must leave alone.
struct Prepared {
    lowered: Lowered,
    target_ids: Vec<NodeId>,
    effectful: HashSet<String>,
}

/// Phase 1 of `cicada run`: parse + check (script discovery included),
/// zero diagnostics, resolve the targets, lower.
fn check_and_lower(pipeline: &Path) -> Result<Prepared, String> {
    let source = std::fs::read_to_string(pipeline).map_err(|e| format!("reading: {e}"))?;
    // Parse + check against stdlib + the `scripts/*.py` beside the file
    // (doc 10 §5) — Python discovery included, exactly as `run` does it.
    let Loaded {
        document,
        specs,
        scripts,
        resolution,
    } = compile::load(pipeline, &source, &ScriptCancel::new())
        .map_err(|e| format!("loading: {e}"))?;
    if !resolution.diagnostics.is_empty() {
        let rendered = resolution
            .diagnostics
            .iter()
            .map(|d| format!("  diagnostic: {d:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "the checker reports {} diagnostic(s):\n{rendered}",
            resolution.diagnostics.len()
        ));
    }

    // No `--node`: every non-effectful leaf; exporters are skipped and
    // their inputs solve (doc 10 §7: exporters never auto-run).
    let targets = compile::resolve_targets(&document, &specs, &[])
        .map_err(|e| format!("resolving targets: {e}"))?;
    let effectful = compile::effectful_bindings(&document, &specs);
    let config = ProjectConfig::default();
    let lowered = lower(
        &document,
        &resolution,
        &specs,
        &config,
        &targets.names,
        &scripts,
    )
    .map_err(|e| format!("lowering: {e}"))?;

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
            "nothing to solve — the leaves {:?} are all literals",
            targets.names
        ));
    }
    Ok(Prepared {
        lowered,
        target_ids,
        effectful,
    })
}

/// Phase 2: solve on a fresh store and judge the report — zero red, zero
/// blocked, every target computed, no exporter ran.
fn solve_prepared(prepared: &Prepared, cache_dir: &Path) -> Result<(), String> {
    let Prepared {
        lowered,
        target_ids,
        effectful,
    } = prepared;
    // A fresh store per pipeline: nothing can be a cache hit, so every
    // node really computes — the memo cannot hide a node that broke.
    let (store, _report) =
        DiskStore::open(cache_dir).map_err(|e| format!("opening the store: {e}"))?;
    let scheduler = Scheduler::new(
        Arc::new(store),
        Arc::new(MonotonicClock::new()),
        // `threads: 0` = cores − 2, the `cicada run` default.
        SchedulerConfig::default(),
    )
    .map_err(|e| format!("scheduler: {e}"))?;
    let report = scheduler
        .solve(
            &lowered.graph,
            target_ids,
            0,
            &CancelToken::new(),
            &NoopObserver,
        )
        .map_err(|e| format!("solving: {e}"))?;

    // Red and blocked bindings, in the words `cicada run` uses.
    let mut problems: Vec<String> = Vec::new();
    for failure in report.failures() {
        if failure.element_ids.is_empty() {
            problems.push(format!("  red `{}` — {}", failure.node, failure.message));
        } else {
            problems.push(format!(
                "  red `{}` — {} (elements {:?})",
                failure.node, failure.message, failure.element_ids
            ));
        }
    }
    for (index, outcome) in report.outcomes.iter().enumerate() {
        if let NodeOutcome::Blocked { upstream } = outcome {
            problems.push(format!(
                "  blocked `{}` — fed by red `{upstream}`, did not run",
                lowered.graph.node(NodeId(index)).name
            ));
        }
    }
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    // Every target computed (a fresh store cannot answer from the memo),
    // and no exporter ran.
    for id in target_ids {
        match report.outcome(*id) {
            NodeOutcome::Computed { .. } => {}
            other => {
                return Err(format!(
                    "  `{}` did not compute: {other:?}",
                    lowered.graph.node(*id).name
                ));
            }
        }
    }
    for name in effectful {
        if let Some(LoweredBinding::Node { node } | LoweredBinding::Port { node, .. }) =
            lowered.bindings.get(name)
            && !matches!(report.outcome(*node), NodeOutcome::Skipped)
        {
            return Err(format!(
                "  effectful `{name}` ran — exporters never auto-run"
            ));
        }
    }
    Ok(())
}

#[test]
fn every_example_solves() {
    let root = examples_dir();
    let pipelines = discover_pipelines(&root);
    let relative = |path: &Path| -> String {
        path.strip_prefix(&root)
            .expect("under examples/")
            .to_string_lossy()
            .replace('\\', "/")
    };
    let names: Vec<String> = pipelines.iter().map(|p| relative(p)).collect();

    // The discovery's own contract: a glob that found nothing (or missed
    // the wall's subdirectory) would pass vacuously.
    assert!(
        names.iter().any(|n| n == "02-solids.cic"),
        "discovery missed the top-level examples: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "wall/wall.cic"),
        "discovery missed the nested wall example: {names:?}"
    );
    assert!(
        names.len() >= 7,
        "only {} example(s) found — seven were committed when this test landed: {names:?}",
        names.len()
    );

    let caches = tempfile::tempdir().unwrap();
    let mut failures: Vec<String> = Vec::new();
    for (index, pipeline) in pipelines.iter().enumerate() {
        let cache_dir = caches.path().join(format!("cache_{index}"));
        if let Err(reason) = solve_pipeline(pipeline, &cache_dir) {
            failures.push(format!("examples/{}:\n{reason}", relative(pipeline)));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} example(s) do not solve (every example must — examples/README.md):\n{}",
        failures.len(),
        pipelines.len(),
        failures.join("\n")
    );
}

// The runner's own contract: a broken pipeline is REPORTED, naming the
// binding — never skipped, never folded into a pass.
#[test]
fn a_broken_pipeline_names_the_binding() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path
    };

    // A red binding, and the one it blocks downstream — both named, with
    // the red one's reason.
    let red = write(
        "red.cic",
        "# cicada 1\nxs = series(count=-1)\nys = reverse(list=xs)\n",
    );
    let err = solve_pipeline(&red, &dir.path().join("cache_red")).unwrap_err();
    assert!(err.contains("red `xs`"), "{err}");
    assert!(
        err.contains("count must be >= 0, got -1"),
        "the red line carries the node's reason: {err}"
    );
    assert!(
        err.contains("blocked `ys` — fed by red `xs`"),
        "the blocked binding is named with its cause: {err}"
    );

    // A checker diagnostic (unknown keyword) — refused before solving.
    let diag = write("diag.cic", "# cicada 1\nxs = series(cont=4)\n");
    let err = solve_pipeline(&diag, &dir.path().join("cache_diag")).unwrap_err();
    assert!(err.contains("diagnostic"), "{err}");

    // And a sound pipeline passes the same runner.
    let ok = write(
        "ok.cic",
        "# cicada 1\nxs = series(count=4)\nys = reverse(list=xs)\n",
    );
    solve_pipeline(&ok, &dir.path().join("cache_ok")).unwrap();
}
