//! `cicada run` (stage 3, doc 15): the headless end-to-end surface —
//! parse → check → lower → solve → report. The agent-facing verification
//! loop starts here (doc 14: headless-first).
//!
//! Diagnostics gate **scoped to the target cone** (docs/12: red cones are
//! excluded, everything else proceeds): a problem in the cone refuses the
//! run with doc-11 JSON on stderr; problems elsewhere are printed as
//! warnings and the run continues.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use cicada_core::config::ProjectConfig;
use cicada_core::value::{HashedValue, ValueData};
use cicada_lang::{Catalog, Document, diag::Diagnostic, resolve};
use cicada_sched::{
    CancelToken, DiskStore, MonotonicClock, NodeId, NodeOutcome, NoopObserver, Scheduler,
    SchedulerConfig, SolveReport, project_cache_dir,
};

use crate::lower::{Lowered, LoweredBinding, lower, reachable_bindings};

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
    let source = fs::read_to_string(&args.pipeline)
        .with_context(|| format!("reading {}", args.pipeline.display()))?;
    let document = Document::parse(&source);
    let specs = cicada_stdlib::registry();
    let catalog = Catalog::new(specs);
    let resolution = resolve(&document, &catalog);

    let targets = resolve_targets(&document, &args.nodes)?;
    gate_diagnostics(&document, &resolution.diagnostics, &targets)?;

    let config = ProjectConfig::default();
    let lowered = lower(&document, &resolution, specs, &config, &targets)?;

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

    report_failures(&report)?;
    print_outputs(&scheduler, &lowered, &targets, &report, args)?;
    if args.time {
        print_times(&lowered, &report, wall);
    }
    Ok(())
}

/// The requested target names, or every leaf binding (nothing references
/// it) in definition order.
fn resolve_targets(document: &Document, nodes: &[String]) -> anyhow::Result<Vec<String>> {
    if !nodes.is_empty() {
        for name in nodes {
            if document.find_binding(name).is_none() {
                bail!("no binding named `{name}` in the pipeline");
            }
        }
        return Ok(nodes.to_vec());
    }
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, statement, _) in document.statements() {
        for reference in statement.references() {
            referenced.insert(reference.name.clone());
        }
    }
    let leaves: Vec<String> = document
        .statements()
        .flat_map(|(_, statement, _)| statement.targets.iter())
        .filter(|target| !referenced.contains(&target.name))
        .map(|target| target.name.clone())
        .collect();
    if leaves.is_empty() {
        bail!("the pipeline binds nothing to compute");
    }
    Ok(leaves)
}

/// Refuse when any diagnostic touches the target cone (or names no node);
/// print the rest as warnings.
fn gate_diagnostics(
    document: &Document,
    diagnostics: &[Diagnostic],
    targets: &[String],
) -> anyhow::Result<()> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let cone = reachable_bindings(document, targets);
    let (blocking, outside): (Vec<&Diagnostic>, Vec<&Diagnostic>) =
        diagnostics.iter().partition(|diagnostic| {
            diagnostic
                .node
                .as_ref()
                .is_none_or(|node| cone.contains(node))
        });
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

/// Red nodes end the run loudly, element IDs included (docs/12).
fn report_failures(report: &SolveReport) -> anyhow::Result<()> {
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
