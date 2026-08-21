//! `cicada run` (stage 3, doc 15): the headless end-to-end surface —
//! parse → check → lower → solve → report. The agent-facing verification
//! loop starts here (doc 14: headless-first).
//!
//! Diagnostics gate **scoped to the target cone** (docs/12: red cones are
//! excluded, everything else proceeds): a problem in the cone refuses the
//! run with doc-11 JSON on stderr; problems elsewhere are printed as
//! warnings and the run continues. The semantics live in
//! `cicada_server::compile` (shared with the live session since stage 5);
//! this file prints.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use cicada_core::config::ProjectConfig;
use cicada_core::value::{HashedValue, ValueData};
use cicada_sched::{
    CancelToken, DiskStore, MonotonicClock, NodeId, NodeOutcome, NoopObserver, Scheduler,
    SchedulerConfig, SolveReport, project_cache_dir,
};
use cicada_server::compile::{self, Loaded};
use cicada_server::lower::{Lowered, LoweredBinding, lower};
use cicada_server::scripts::ScriptCancel;

/// Arguments of `cicada run`.
pub struct RunArgs {
    /// The `.cic` file.
    pub pipeline: PathBuf,
    /// Bindings to compute; empty = every leaf (bindings nothing
    /// references).
    pub nodes: Vec<String>,
    /// Print per-node compute times and the solve summary.
    pub time: bool,
    /// Print stable `binding<TAB>port<TAB>hash` lines (scriptable).
    pub hashes: bool,
    /// Store override (tests, CI, `.cicada-cache/` opt-in). Default: the
    /// per-project user cache directory — never the project folder.
    pub cache_dir: Option<PathBuf>,
    /// Worker threads; 0 = cores − 2.
    pub threads: usize,
}

/// Run a pipeline headlessly.
///
/// # Errors
///
/// Anything loud: unreadable file, diagnostics in the target cone, store
/// failures, red nodes.
pub fn run(args: &RunArgs) -> anyhow::Result<()> {
    // Absolute paths first, then work FROM the pipeline's directory:
    // relative paths inside the pipeline (exporter `path=` literals)
    // resolve against it — the same rule as `cicada serve` — not against
    // the shell's working directory (stage-5 review: exports were landing
    // wherever the process was launched).
    let pipeline = fs::canonicalize(&args.pipeline)
        .with_context(|| format!("resolving {}", args.pipeline.display()))?;
    let cache_dir = args
        .cache_dir
        .as_ref()
        .map(|dir| std::path::absolute(dir).with_context(|| format!("resolving {}", dir.display())))
        .transpose()?;
    if let Some(dir) = pipeline.parent() {
        std::env::set_current_dir(dir).with_context(|| format!("entering {}", dir.display()))?;
    }
    let args = &RunArgs {
        pipeline,
        nodes: args.nodes.clone(),
        time: args.time,
        hashes: args.hashes,
        cache_dir,
        threads: args.threads,
    };
    let source = fs::read_to_string(&args.pipeline)
        .with_context(|| format!("reading {}", args.pipeline.display()))?;
    // Project script nodes (doc 10 §5): scripts/*.py next to the pipeline
    // join the catalog; no scripts, no Python requirement. The headless
    // run never cancels, so the cancel bridge's switch stays unkilled.
    let Loaded {
        document,
        specs,
        scripts,
        resolution,
    } = compile::load(&args.pipeline, &source, &ScriptCancel::new())?;

    let targets = compile::resolve_targets(&document, &specs, &args.nodes)?;
    for name in &targets.skipped_effectful {
        eprintln!(
            "note: `{name}` is effectful — skipped (its inputs still solve); run it \
             explicitly with `--node {name}`"
        );
    }
    let targets = targets.names;
    gate_diagnostics(&document, &resolution.diagnostics, &targets)?;

    let config = ProjectConfig::default();
    let lowered = lower(&document, &resolution, &specs, &config, &targets, &scripts)?;

    let store_dir = match &args.cache_dir {
        Some(dir) => dir.clone(),
        None => project_cache_dir(&args.pipeline)?,
    };
    let (store, open_report) = DiskStore::open(&store_dir)?;
    match open_report.recovery {
        None => {}
        Some(cicada_sched::store::LogRecovery::TornTail) => eprintln!(
            "note: memo log ended in a torn record (crash mid-write?); it was \
             truncated there — completed work before it was kept"
        ),
        Some(cicada_sched::store::LogRecovery::CorruptRecord {
            offset,
            bytes_dropped,
        }) => eprintln!(
            "note: memo log had an undecodable record at byte {offset}; {bytes_dropped} \
             bytes of later cached work were dropped and will recompute"
        ),
    }
    match open_report.pack_recovery {
        None => {}
        Some(cicada_sched::store::LogRecovery::TornTail) => eprintln!(
            "note: value pack ended in a torn frame (crash mid-write?); it was truncated \
             there — the values before it were kept, the torn one recomputes"
        ),
        Some(cicada_sched::store::LogRecovery::CorruptRecord {
            offset,
            bytes_dropped,
        }) => eprintln!(
            "note: value pack had an unframeable record at byte {offset}; {bytes_dropped} \
             bytes of later cached values were dropped and will recompute"
        ),
    }
    let scheduler = Scheduler::new(
        Arc::new(store),
        Arc::new(MonotonicClock::new()),
        SchedulerConfig {
            threads: args.threads,
            ..SchedulerConfig::default()
        },
    )?;

    let target_ids = target_node_ids(&lowered, &targets);
    let started = std::time::Instant::now();
    let report = scheduler.solve(
        &lowered.graph,
        &target_ids,
        0,
        &CancelToken::new(),
        &NoopObserver,
    )?;
    let wall = started.elapsed();

    report_failures(&lowered, &report)?;
    print_outputs(&scheduler, &lowered, &targets, &report, args)?;
    if args.time {
        print_times(&lowered, &report, wall);
    }
    Ok(())
}

/// Refuse when any diagnostic touches the target cone (or names no node);
/// print the rest as warnings.
fn gate_diagnostics(
    document: &cicada_lang::Document,
    diagnostics: &[cicada_lang::diag::Diagnostic],
    targets: &[String],
) -> anyhow::Result<()> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let compile::Gate { blocking, outside } = compile::gate(document, diagnostics, targets);
    if !outside.is_empty() {
        eprintln!(
            "warning: {} diagnostic(s) outside the requested cone (run continues):",
            outside.len()
        );
        eprintln!("{}", serde_json::to_string_pretty(&outside)?);
    }
    if !blocking.is_empty() {
        eprintln!("{}", serde_json::to_string_pretty(&blocking)?);
        bail!(
            "{} diagnostic(s) in the target cone — fix them first (JSON above)",
            blocking.len()
        );
    }
    Ok(())
}

/// The `NodeId`s a solve must pull for the requested bindings (a literal
/// binding needs no node at all).
fn target_node_ids(lowered: &Lowered, targets: &[String]) -> Vec<NodeId> {
    let mut ids: Vec<NodeId> = targets
        .iter()
        .filter_map(|name| match lowered.bindings.get(name) {
            Some(LoweredBinding::Port { node, .. } | LoweredBinding::Node { node }) => Some(*node),
            Some(LoweredBinding::Value(_)) | None => None,
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Red nodes end the run loudly, element IDs included (docs/12) — and the
/// blast radius is spelled out: every blocked downstream node gets its own
/// "fed by" line (doc 10's honest-reasons discipline; probe friction).
fn report_failures(lowered: &Lowered, report: &SolveReport) -> anyhow::Result<()> {
    let failures = report.failures();
    if failures.is_empty() {
        return Ok(());
    }
    for failure in &failures {
        if failure.element_ids.is_empty() {
            eprintln!("red: `{}` — {}", failure.node, failure.message);
        } else {
            eprintln!(
                "red: `{}` — {} (elements {:?})",
                failure.node, failure.message, failure.element_ids
            );
        }
    }
    let mut blocked = 0_usize;
    for (index, outcome) in report.outcomes.iter().enumerate() {
        if let NodeOutcome::Blocked { upstream } = outcome {
            blocked += 1;
            eprintln!(
                "blocked: `{}` — fed by red `{upstream}`, did not run",
                lowered.graph.node(NodeId(index)).name
            );
        }
    }
    if blocked > 0 {
        bail!(
            "{} node(s) red, {blocked} blocked downstream — nothing exported",
            failures.len()
        );
    }
    bail!("{} node(s) red — nothing exported", failures.len());
}

fn print_outputs(
    scheduler: &Scheduler,
    lowered: &Lowered,
    targets: &[String],
    report: &SolveReport,
    args: &RunArgs,
) -> anyhow::Result<()> {
    for name in targets {
        match lowered.bindings.get(name) {
            None => bail!("`{name}` vanished between checking and lowering — bug"),
            // A literal never passes through the store (constant params
            // are free, no node, no blob) — print the value IN HAND;
            // loading its hash would misreport "not in the store"
            // (regression: adversarial review, stage 3).
            Some(LoweredBinding::Value(value)) => {
                if args.hashes {
                    println!("{name}\tout\t{}", value.hash());
                } else {
                    println!("{name}.out = {}", render_value(value));
                }
            }
            Some(LoweredBinding::Port { node, output }) => {
                let Some(hashes) = report.outcome(*node).output_hashes() else {
                    bail!(
                        "`{name}` has no output — outcome {:?}",
                        report.outcome(*node)
                    );
                };
                let port = &lowered.output_names[node.0][*output];
                print_one(scheduler, name, port, &hashes[*output], args)?;
            }
            Some(LoweredBinding::Node { node }) => {
                let Some(hashes) = report.outcome(*node).output_hashes() else {
                    bail!(
                        "`{name}` has no outputs — outcome {:?}",
                        report.outcome(*node)
                    );
                };
                for (index, hash) in hashes.iter().enumerate() {
                    let port = &lowered.output_names[node.0][index];
                    print_one(scheduler, name, port, hash, args)?;
                }
            }
        }
    }
    Ok(())
}

fn print_one(
    scheduler: &Scheduler,
    name: &str,
    port: &str,
    hash: &cicada_core::hash::ValueHash,
    args: &RunArgs,
) -> anyhow::Result<()> {
    if args.hashes {
        println!("{name}\t{port}\t{hash}");
        return Ok(());
    }
    let value = scheduler.store().load_value(hash)?;
    println!("{name}.{port} = {}", render_value(&value));
    Ok(())
}

/// Compact human rendering — the inspector's job arrives at stage 5; this
/// is a terminal summary.
fn render_value(value: &HashedValue) -> String {
    match value.data() {
        ValueData::Number(x) => format!("{x}"),
        ValueData::Integer(i) => format!("{i}"),
        ValueData::Boolean(b) => format!("{b}"),
        ValueData::Text(s) => format!("{s:?}"),
        ValueData::Color(c) => format!("Color({}, {}, {}, {})", c.r, c.g, c.b, c.a),
        ValueData::Domain(d) => format!("{}..{}", d.start, d.end),
        ValueData::IndexMap(m) => format!("IndexMap(×{})", m.0.len()),
        ValueData::Point(p) => format!("({}, {}, {})", p.0.x, p.0.y, p.0.z),
        ValueData::Vector(v) => format!("({}, {}, {})", v.0.x, v.0.y, v.0.z),
        ValueData::Plane(_) => "Plane".to_owned(),
        ValueData::Xform(_) => "Xform".to_owned(),
        ValueData::Curve(curve) => {
            use cicada_core::geometry::Curve;
            match curve {
                Curve::Line(line) => format!(
                    "Line(({}, {}, {}) → ({}, {}, {}))",
                    line.a.0.x, line.a.0.y, line.a.0.z, line.b.0.x, line.b.0.y, line.b.0.z
                ),
                Curve::Polyline(p) => format!(
                    "Polyline(×{}{})",
                    p.vertices.len(),
                    if p.closed { ", closed" } else { "" }
                ),
                Curve::Circle(c) => format!(
                    "Circle(center ({}, {}, {}), r {})",
                    c.plane.origin.0.x, c.plane.origin.0.y, c.plane.origin.0.z, c.radius
                ),
                Curve::Rectangle(r) => format!(
                    "Rectangle({}..{} × {}..{})",
                    r.x.start, r.x.end, r.y.start, r.y.end
                ),
            }
        }
        ValueData::Mesh(mesh) => format!(
            "Mesh({} vertices, {} triangles)",
            mesh.vertex_count(),
            mesh.triangle_count()
        ),
        ValueData::Solid(solid) => format!("Solid({} bytes)", solid.bytes().len()),
        ValueData::List(list) => {
            let shown: Vec<String> = list
                .slots
                .iter()
                .take(6)
                .map(|slot| {
                    slot.as_ref()
                        .map_or_else(|| "∅".to_owned(), |element| render_value(element))
                })
                .collect();
            let ellipsis = if list.slots.len() > 6 { ", …" } else { "" };
            let axis = list
                .axis
                .as_ref()
                .map_or_else(String::new, |axis| format!("{axis}: "));
            format!(
                "[{axis}{}{ellipsis}] ×{}",
                shown.join(", "),
                list.slots.len()
            )
        }
        ValueData::Nothing => "Nothing".to_owned(),
    }
}

fn print_times(lowered: &Lowered, report: &SolveReport, wall: std::time::Duration) {
    let mut computed = 0_usize;
    let mut hits = 0_usize;
    for (index, outcome) in report.outcomes.iter().enumerate() {
        match outcome {
            NodeOutcome::Computed {
                elements, nanos, ..
            } => {
                computed += 1;
                println!(
                    "time: {} — {:.3} ms ({} element{})",
                    lowered.graph.node(NodeId(index)).name,
                    to_ms(*nanos),
                    elements,
                    if *elements == 1 { "" } else { "s" },
                );
            }
            NodeOutcome::CacheHit { .. } => hits += 1,
            _ => {}
        }
    }
    println!(
        "time: total {:.3} ms wall — {computed} computed, {hits} from cache",
        to_ms(u64::try_from(wall.as_nanos()).unwrap_or(u64::MAX)),
    );
}

#[allow(clippy::cast_precision_loss)] // display only
fn to_ms(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000.0
}

/// Resolve the store dir the way `run` does — exposed for the `cache`
/// subcommand later; keeps the never-inside-the-project rule in one place.
#[must_use]
pub fn default_store_dir(pipeline: &Path) -> Option<PathBuf> {
    project_cache_dir(pipeline).ok()
}
