//! Loading a pipeline into checkable, lowerable form — the semantics
//! `cicada run` established at stage 3 (target resolution, effectful-leaf
//! handling, the cone-scoped diagnostics gate), now owned by the server so
//! the headless run and the live session cannot drift apart. `run.rs` in
//! `cicada-cli` is a thin printer over these functions.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use cicada_core::spec::NodeSpec;
use cicada_lang::ast::Rhs;
use cicada_lang::diag::Diagnostic;
use cicada_lang::{Catalog, Document, Resolution, resolve};

use crate::lower::{reachable_bindings, split_diagnostics};
use crate::scripts::{ScriptCancel, ScriptNode, ScriptsError, discover_in};

/// A parsed + checked pipeline with its merged catalog: stdlib specs plus
/// the project's script nodes (doc 10 §5 resolution order — collisions
/// were refused at discovery).
pub struct Loaded {
    /// The parsed document.
    pub document: Document,
    /// Every spec the checker resolves against: stdlib first, then script
    /// nodes sorted by name.
    pub specs: Vec<&'static NodeSpec>,
    /// Discovered script nodes by dialect name.
    pub scripts: HashMap<String, ScriptNode>,
    /// The checker's output.
    pub resolution: Resolution,
}

/// The catalog specs for `pipeline`: stdlib + `scripts/*.py` next to it.
///
/// # Errors
///
/// [`ScriptsError`] from discovery.
pub fn catalog_specs(
    pipeline: &Path,
    cancel: &Arc<ScriptCancel>,
) -> Result<(Vec<&'static NodeSpec>, HashMap<String, ScriptNode>), ScriptsError> {
    catalog_specs_in(pipeline.parent().unwrap_or(Path::new(".")), cancel)
}

/// The catalog specs for a PROJECT directory: stdlib + `<dir>/scripts/*.py`
/// — exactly what `/api/catalog` serves for a pipeline in `project_dir`,
/// for callers with a project and no pipeline (`cicada mcp`).
///
/// # Errors
///
/// [`ScriptsError`] from discovery.
pub fn catalog_specs_in(
    project_dir: &Path,
    cancel: &Arc<ScriptCancel>,
) -> Result<(Vec<&'static NodeSpec>, HashMap<String, ScriptNode>), ScriptsError> {
    let stdlib = cicada_stdlib::registry();
    let scripts = discover_in(project_dir, stdlib, cancel)?;
    let mut specs: Vec<&'static NodeSpec> = stdlib.to_vec();
    let mut script_specs: Vec<&'static NodeSpec> = scripts.values().map(|node| node.spec).collect();
    script_specs.sort_by_key(|spec| spec.name);
    specs.extend(script_specs);
    Ok((specs, scripts))
}

/// Parse `source` (the text of `pipeline`), discover its scripts, and run
/// the checker.
///
/// # Errors
///
/// [`ScriptsError`] from discovery — parsing and checking never fail
/// (problems are diagnostics).
pub fn load(
    pipeline: &Path,
    source: &str,
    cancel: &Arc<ScriptCancel>,
) -> Result<Loaded, ScriptsError> {
    let (specs, scripts) = catalog_specs(pipeline, cancel)?;
    let (document, resolution) = check_source(source, &specs);
    Ok(Loaded {
        document,
        specs,
        scripts,
        resolution,
    })
}

/// Parse `source` and run the checker against an already-discovered
/// catalog — THE checker path (`load` is discovery plus this; the live
/// session's per-edit recheck and `cicada mcp`'s `check` tool call it too,
/// so there is exactly one checker). Never fails: problems are diagnostics.
#[must_use]
pub fn check_source(source: &str, specs: &[&'static NodeSpec]) -> (Document, Resolution) {
    let document = Document::parse(source);
    let resolution = resolve(&document, &Catalog::new(specs));
    (document, resolution)
}

impl Loaded {
    /// Re-parse and re-check a new source text against the SAME catalog
    /// (script set unchanged) — the live session's per-edit path.
    pub fn reload_text(&mut self, source: &str) {
        (self.document, self.resolution) = check_source(source, &self.specs);
    }

    /// Re-check the current document (after an in-place writer gesture).
    pub fn recheck(&mut self) {
        self.resolution = resolve(&self.document, &Catalog::new(&self.specs));
    }
}

/// The resolved target set of a headless run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Targets {
    /// Binding names to compute, in order.
    pub names: Vec<String>,
    /// Effectful leaves that were skipped (their inputs still solve; doc 10
    /// §7: exporters never auto-run) — printed as notes by the CLI.
    pub skipped_effectful: Vec<String>,
}

/// Why target resolution refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    /// A named target binds nothing.
    #[error("no binding named `{0}` in the pipeline")]
    Unknown(String),
    /// Nothing to compute (every leaf is effectful, or the file is empty).
    #[error(
        "the pipeline binds nothing to compute (effectful bindings run only via --node / explicit run)"
    )]
    Nothing,
}

/// The requested target names, or every leaf binding (nothing references
/// it) in definition order — MINUS effectful leaves (naming one explicitly
/// IS the explicit action).
///
/// # Errors
///
/// [`TargetError`].
pub fn resolve_targets(
    document: &Document,
    specs: &[&'static NodeSpec],
    requested: &[String],
) -> Result<Targets, TargetError> {
    if !requested.is_empty() {
        for name in requested {
            if document.find_binding(name).is_none() {
                return Err(TargetError::Unknown(name.clone()));
            }
        }
        return Ok(Targets {
            names: requested.to_vec(),
            skipped_effectful: Vec::new(),
        });
    }
    let effectful: HashSet<&str> = specs
        .iter()
        .filter(|spec| !spec.pure)
        .map(|spec| spec.name)
        .collect();
    let mut referenced: HashSet<String> = HashSet::new();
    for (_, statement, _) in document.statements() {
        for reference in statement.references() {
            referenced.insert(reference.name.clone());
        }
    }
    let mut leaves: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (_, statement, _) in document.statements() {
        let is_effectful = matches!(
            &statement.rhs,
            Rhs::Call(call) if effectful.contains(call.func.name.as_str())
        );
        for target in &statement.targets {
            if referenced.contains(&target.name) {
                continue;
            }
            if is_effectful {
                // Solve up to the exporter's INPUTS (doc 10 §7), just not
                // the export itself.
                skipped.push(target.name.clone());
                for reference in statement.references() {
                    if seen.insert(reference.name.clone()) {
                        leaves.push(reference.name.clone());
                    }
                }
            } else if seen.insert(target.name.clone()) {
                leaves.push(target.name.clone());
            }
        }
    }
    if leaves.is_empty() {
        return Err(TargetError::Nothing);
    }
    Ok(Targets {
        names: leaves,
        skipped_effectful: skipped,
    })
}

/// The cone-scoped diagnostics gate: diagnostics naming a binding in the
/// targets' cone (or naming no binding) BLOCK; the rest are warnings.
pub struct Gate<'d> {
    /// Diagnostics that refuse the run.
    pub blocking: Vec<&'d Diagnostic>,
    /// Diagnostics outside the requested cone (the run continues).
    pub outside: Vec<&'d Diagnostic>,
}

/// Split `diagnostics` against the cone reachable from `targets`.
#[must_use]
pub fn gate<'d>(
    document: &Document,
    diagnostics: &'d [Diagnostic],
    targets: &[String],
) -> Gate<'d> {
    let cone = reachable_bindings(document, targets);
    let (blocking, outside) = split_diagnostics(diagnostics, &cone);
    Gate { blocking, outside }
}

/// The names bound by effectful calls in `document` (doc 10 §7).
#[must_use]
pub fn effectful_bindings(document: &Document, specs: &[&'static NodeSpec]) -> HashSet<String> {
    let effectful: HashSet<&str> = specs
        .iter()
        .filter(|spec| !spec.pure)
        .map(|spec| spec.name)
        .collect();
    let mut names = HashSet::new();
    for (_, statement, _) in document.statements() {
        if matches!(&statement.rhs, Rhs::Call(call) if effectful.contains(call.func.name.as_str()))
        {
            for target in &statement.targets {
                names.insert(target.name.clone());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdlib() -> Vec<&'static NodeSpec> {
        cicada_stdlib::registry().to_vec()
    }

    #[test]
    fn default_targets_are_non_effectful_leaves_and_exporter_inputs() {
        let source = "# cicada 1\n\
                      a = 1.0\n\
                      b = add(a=a, b=2.0)\n\
                      c = 3.0\n\
                      m = box(x=d, y=d, z=d)\n\
                      d = construct_domain(start=0.0, end=1.0)\n\
                      dx = unit_x()\n\
                      ms = linear_array(geometry=m, direction=dx, count=1)\n\
                      dump = export_obj(meshes=ms, path=\"x.obj\")\n";
        let document = Document::parse(source);
        let targets = resolve_targets(&document, &stdlib(), &[]).unwrap();
        // b and c are leaves; m feeds ms feeds dump (effectful) — dump is
        // skipped and its INPUT ms becomes a target.
        assert_eq!(targets.names, vec!["b", "c", "ms"]);
        assert_eq!(targets.skipped_effectful, vec!["dump"]);
    }

    #[test]
    fn explicit_target_must_exist() {
        let document = Document::parse("# cicada 1\na = 1.0\n");
        assert_eq!(
            resolve_targets(&document, &stdlib(), &["zzz".to_owned()]).unwrap_err(),
            TargetError::Unknown("zzz".to_owned())
        );
        assert_eq!(
            resolve_targets(&Document::parse("# cicada 1\n"), &stdlib(), &[]).unwrap_err(),
            TargetError::Nothing
        );
    }

    #[test]
    fn gate_scopes_to_the_cone() {
        let source = "# cicada 1\n\
                      good = add(a=1.0, b=2.0)\n\
                      bad = add(a=nope, b=2.0)\n";
        let document = Document::parse(source);
        let specs = stdlib();
        let resolution = resolve(&document, &Catalog::new(&specs));
        assert!(!resolution.diagnostics.is_empty());
        let scoped = gate(&document, &resolution.diagnostics, &["good".to_owned()]);
        assert!(
            scoped.blocking.is_empty(),
            "the bad binding is outside good's cone"
        );
        assert_eq!(scoped.outside.len(), resolution.diagnostics.len());
        let scoped = gate(&document, &resolution.diagnostics, &["bad".to_owned()]);
        assert!(!scoped.blocking.is_empty());
    }
}
