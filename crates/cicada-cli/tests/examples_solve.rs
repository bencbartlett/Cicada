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
//! it must have found), and the runner's contract is pinned both ways by
//! the cases below: a red binding, a blocked one, a diagnostic are each
//! reported, never skipped; a computed target, a memo hit within the
//! solve, an exporter left alone (its inputs solved, its file unwritten)
//! each pass.
//!
//! The wall is included. Measured cold in DEBUG on the 24-core dev machine
//! (2026-08-20): 6.9 s at the default thread count (cores − 2), 18 s at 4
//! threads, 34 s at 2 — under the minute the follow-up allowed. CI's
//! 4-vCPU runners land around a minute; if that ever dominates the suite,
//! exclude it here by an explicit list WITH the reason, and say so in
//! `examples/README.md`.
//!
//! Same functions as `cicada run`, with exactly two differences, both
//! deliberate and both stated in `examples/README.md`:
//!
//! 1. Diagnostics ANYWHERE refuse the example. `run` gates them to the
//!    target cone and prints the rest as warnings; an example with a
//!    warning outside the cone is still a wrong example (the app paints
//!    it red).
//! 2. The working directory is NOT the pipeline's directory. `run` enters
//!    it (`set_current_dir`, so exporter `path=` literals resolve against
//!    the pipeline, the `serve` rule) — a process-global switch this test
//!    cannot make: its two tests run concurrently in one process. Nothing
//!    in `examples/` depends on the cwd today (the only path-taking
//!    stdlib node is the effectful `export_obj`, which never runs here,
//!    and the wall's scripts resolve their `inputs/` against their own
//!    `__file__`), so a relative path in a NON-effectful node — a future
//!    reader node, a script using the cwd — is the one thing that could
//!    pass `run` and fail here, or the reverse. The rule for examples
//!    follows: relative paths in non-effectful nodes must not rely on
//!    the cwd.
//!
//! Not a difference, though the first version of this test made it one:
//! a target answered by the memo WITHIN the solve is as green as a
//! computed one. The store is fresh, so nothing comes from an earlier
//! run — but two nodes of one pipeline with the same content-addressed
//! key (the same function over the same values, e.g. `reverse` applied
//! twice more to its own result) share one memo entry, and the later one
//! is a `CacheHit`. `run` prints it as "from cache" and is happy; so is
//! this test (`a_sound_pipeline_passes` pins the `reverse³` shape, whose
//! hit is deterministic at any thread count: the first `reverse` sits two
//! waves upstream of its twin).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cicada_core::config::ProjectConfig;
use cicada_lang::{Document, writer};
use cicada_sched::{
    CancelToken, DiskStore, MonotonicClock, NodeId, NodeOutcome, NoopObserver, Scheduler,
    SchedulerConfig, SolveReport,
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
/// every line naming a binding. `Ok` hands the solve back so a test can
/// pin HOW a binding passed (computed, or a memo hit within the solve).
fn solve_pipeline(pipeline: &Path, cache_dir: &Path) -> Result<Solved, String> {
    let prepared = check_and_lower(pipeline)?;
    let report = solve_prepared(&prepared, cache_dir)?;
    Ok(Solved { prepared, report })
}

/// A checked, lowered pipeline: the graph, the targets a headless run
/// solves, and the exporters it must leave alone.
struct Prepared {
    lowered: Lowered,
    target_ids: Vec<NodeId>,
    effectful: HashSet<String>,
}

/// A pipeline that passed the runner, with the evidence.
struct Solved {
    prepared: Prepared,
    report: SolveReport,
}

impl std::fmt::Debug for Solved {
    /// What a test that expected a refusal sees instead: every node's
    /// outcome by name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for (index, outcome) in self.report.outcomes.iter().enumerate() {
            map.entry(
                &self.prepared.lowered.graph.node(NodeId(index)).name,
                outcome,
            );
        }
        map.finish()
    }
}

impl Solved {
    /// The outcome of a lowered binding's node.
    fn outcome(&self, name: &str) -> &NodeOutcome {
        match self.prepared.lowered.bindings.get(name) {
            Some(LoweredBinding::Port { node, .. } | LoweredBinding::Node { node }) => {
                self.report.outcome(*node)
            }
            other => panic!("`{name}` is not a lowered node: {other:?}"),
        }
    }
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
/// blocked, every target answered (computed, or a memo hit within this
/// very solve), no exporter lowered.
fn solve_prepared(prepared: &Prepared, cache_dir: &Path) -> Result<SolveReport, String> {
    let Prepared {
        lowered,
        target_ids,
        effectful,
    } = prepared;
    // A fresh store per pipeline: nothing comes from an EARLIER run, so a
    // node that broke cannot hide behind yesterday's memo entry. Hits
    // within the solve itself (two nodes with one key) are real work done
    // once, and accepted below.
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
    // Every target answered: computed this solve, or — two nodes of this
    // pipeline sharing one content-addressed key — served from the entry
    // its twin wrote moments ago (`run` says "from cache" and accepts it
    // too). Anything else (`Skipped`, `Cancelled`) is a runner bug, not
    // an example's: red and blocked were reported above.
    for id in target_ids {
        match report.outcome(*id) {
            NodeOutcome::Computed { .. } | NodeOutcome::CacheHit { .. } => {}
            other => {
                return Err(format!(
                    "  `{}` was neither computed nor answered: {other:?}",
                    lowered.graph.node(*id).name
                ));
            }
        }
    }
    // Exporters never run — by construction, not by a status: `lower`
    // takes the UPSTREAM closure of the leaves, and an exporter is a leaf
    // nothing references (its inputs are the targets, not it), so it is
    // not lowered at all and has no outcome to inspect. Pin exactly that;
    // the observable half (no file written) is pinned by the test below.
    for name in effectful {
        if lowered.bindings.contains_key(name) {
            return Err(format!(
                "  effectful `{name}` was lowered into the solve graph — exporters never \
                 auto-run (only their inputs are targets)"
            ));
        }
    }
    Ok(report)
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
}

// The runner's other contract: what `cicada run` accepts, this accepts —
// including the two shapes the first version of the test got wrong or
// never reached (the 2026-08-21 review).
#[test]
fn a_sound_pipeline_passes() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path
    };

    // The plain case: every target computed.
    let ok = write(
        "ok.cic",
        "# cicada 1\nxs = series(count=4)\nys = reverse(list=xs)\n",
    );
    let solved = solve_pipeline(&ok, &dir.path().join("cache_ok")).unwrap();
    assert!(
        matches!(solved.outcome("ys"), NodeOutcome::Computed { .. }),
        "{:?}",
        solved.outcome("ys")
    );

    // A memo hit WITHIN the solve is green. `c` reverses `b` back into
    // `a`'s value, so `d = reverse(c)` has `b`'s content-addressed key and
    // is answered from the entry `b` wrote two waves earlier — at any
    // thread count (the first version demanded `Computed` of every target
    // and refused this valid pipeline, which `run` prints as "3 computed,
    // 1 from cache"). The assertion on `d`'s outcome is deliberate: if the
    // scheduler ever stopped deduplicating keys within a solve, this case
    // would no longer exercise what it claims to, and must say so.
    let twins = write(
        "twins.cic",
        "# cicada 1\na = series(count=3)\nb = reverse(list=a)\nc = reverse(list=b)\nd = reverse(list=c)\n",
    );
    let solved = solve_pipeline(&twins, &dir.path().join("cache_twins")).unwrap();
    assert!(
        matches!(solved.outcome("b"), NodeOutcome::Computed { .. }),
        "{:?}",
        solved.outcome("b")
    );
    assert!(
        matches!(solved.outcome("d"), NodeOutcome::CacheHit { .. }),
        "`d` shares `b`'s key and must be the intra-solve memo hit this case pins: {:?}",
        solved.outcome("d")
    );

    // An exporter in the pipeline: its inputs solve, it is never lowered,
    // and — the half a user can see — its file is never written. The path
    // is absolute so the assertion does not depend on the cwd (see the
    // header: this test does not enter the pipeline's directory).
    let never = dir.path().join("never.obj");
    let never_literal = never.to_string_lossy().replace('\\', "/");
    let export = write(
        "export.cic",
        &format!(
            "# cicada 1\nspan = construct_domain(start=0.0, end=1.0)\nblock = mesh_box(x=span, y=span, z=span)\n\
             meshes = duplicate(item=block, count=1)\ndump = export_obj(meshes=meshes, path=\"{never_literal}\")\n"
        ),
    );
    let solved = solve_pipeline(&export, &dir.path().join("cache_export")).unwrap();
    assert!(
        solved.prepared.effectful.contains("dump"),
        "the runner saw the exporter: {:?}",
        solved.prepared.effectful
    );
    assert!(
        !solved.prepared.lowered.bindings.contains_key("dump"),
        "an exporter is never lowered (upstream closure of the leaves)"
    );
    assert!(
        matches!(solved.outcome("meshes"), NodeOutcome::Computed { .. }),
        "the exporter's input is a target and solves: {:?}",
        solved.outcome("meshes")
    );
    assert!(
        !never.exists(),
        "the exporter must not have run: {} exists",
        never.display()
    );
}

// An example's sliders are a contract too: "three sliders to drag in the
// app" (examples/README.md) promises that NO position of them paints a
// node red — `every_example_solves` sees only the committed defaults. The
// first version of 09-vectors kept the probe in the ring's plane, so the
// probe ON the centre — the x/y sliders' midpoint, a one-keystroke literal
// in the inspector — made `to_probe` the zero vector and `angle` /
// `amplitude` red (the C2a review, 2026-08-24); the probe now floats above
// the plane, and this test holds the example total at the positions that
// can degenerate: the centre at the radius's default and bounds, the four
// corners, and straight above a post. Each position is set the way a drag
// sets it — the writer's `set_param`, the gesture the canvas commits — and
// solved like any example.
#[test]
fn the_vectors_example_is_total_over_its_sliders() {
    let source = std::fs::read_to_string(examples_dir().join("09-vectors.cic")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    // (radius, probe_x, probe_y).
    let positions: [(f64, f64, f64); 8] = [
        (6.0, 0.0, 0.0),
        (2.0, 0.0, 0.0),
        (10.0, 0.0, 0.0),
        (6.0, 6.0, 0.0),
        (6.0, -10.0, -10.0),
        (6.0, 10.0, -10.0),
        (6.0, -10.0, 10.0),
        (6.0, 10.0, 10.0),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (index, &(radius, probe_x, probe_y)) in positions.iter().enumerate() {
        let mut document = Document::parse(&source);
        for (binding, value) in [
            ("radius", radius),
            ("probe_x", probe_x),
            ("probe_y", probe_y),
        ] {
            writer::set_param(&mut document, binding, "value", &format!("{value:?}"), None)
                .unwrap_or_else(|e| panic!("setting `{binding}` to {value}: {e}"));
        }
        let path = dir.path().join(format!("vectors_{index}.cic"));
        std::fs::write(&path, document.emit()).unwrap();
        if let Err(reason) = solve_pipeline(&path, &dir.path().join(format!("cache_{index}"))) {
            failures.push(format!(
                "radius {radius}, probe ({probe_x}, {probe_y}):\n{reason}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "09-vectors goes red at {} of {} slider position(s) — an example's sliders must keep it green:\n{}",
        failures.len(),
        positions.len(),
        failures.join("\n")
    );
}
