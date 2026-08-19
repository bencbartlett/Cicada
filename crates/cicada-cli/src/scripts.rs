//! Project script-node discovery (doc 10 §5): every `scripts/*.py` next
//! to the pipeline file self-registers its `@cicada.node` functions; the
//! dialect calls them like stdlib nodes. Lives in the CLI with the
//! lowering (both move to the server's hydration path at stage 5).
//!
//! Cache keys: script nodes carry the SOURCE hash as `body_hash`
//! (docs/12: script `node_version` = hash of the source file) — editing
//! the `.py` recomputes exactly its cone; renaming the binding never
//! does.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use cicada_core::hash::ValueHash;
use cicada_core::spec::{NodeSpec, PortSpec, PortType, Tier};
use cicada_core::value::{HashedValue, ValueData};
use cicada_sched::{NodeError, NodeFn};
use cicada_script::{KillSwitch, PortDesc, WorkerPool};

/// One discovered script node: the leaked spec plus its cache-key
/// material and run function.
pub struct ScriptNode {
    /// The spec (leaked — bounded by the script count of one run).
    pub spec: &'static NodeSpec,
    /// blake3 of the source file — the `NodeDecl.body_hash` slot.
    pub body_hash: ValueHash,
    /// The run function (calls the worker pool).
    pub run: NodeFn,
}

/// The wire kinds that cross the Python boundary (stage-4 subset —
/// mirrors `cicada_script::value`).
const MARSHALLABLE: &[&str] = &[
    "Number", "Integer", "Boolean", "Text", "Point", "Vector", "Domain",
];

/// Discover and describe every script node next to `pipeline`
/// (`<dir>/scripts/*.py`, sorted). No scripts → empty map and NO Python
/// requirement; the pool spawns only when scripts exist.
///
/// # Errors
///
/// Unreadable files, Python/describe failures (with tracebacks), bad
/// type annotations, or a name collision with the stdlib.
pub fn discover(
    pipeline: &Path,
    stdlib: &[&'static NodeSpec],
) -> anyhow::Result<HashMap<String, ScriptNode>> {
    let dir = pipeline.parent().unwrap_or(Path::new(".")).join("scripts");
    if !dir.is_dir() {
        return Ok(HashMap::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "py"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(HashMap::new());
    }

    let pool = Arc::new(WorkerPool::new().context("starting the Python script host")?);
    let stdlib_names: std::collections::HashSet<&str> =
        stdlib.iter().map(|spec| spec.name).collect();
    let mut nodes = HashMap::new();
    for file in files {
        // Source is read ONCE and travels with every request from here on
        // (describe, invoke, and the body hash all see the SAME bytes —
        // no time-of-check/time-of-use window against concurrent edits).
        let source: Arc<str> = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?
            .into();
        let described = pool
            .describe(&file, &source)
            .with_context(|| format!("describing {}", file.display()))?;
        // docs/12: script node_version = source + toolchain version.
        let body_hash = cicada_script::source_hash(source.as_bytes(), &described.python_version);
        for desc in described.nodes {
            if stdlib_names.contains(desc.name.as_str()) {
                bail!(
                    "script node `{}` ({}) collides with a stdlib node — rename the \
                     function (namespacing/qualification arrives with v0.1, doc 10 §5)",
                    desc.name,
                    file.display()
                );
            }
            if nodes.contains_key(&desc.name) {
                bail!(
                    "script node `{}` is defined twice in scripts/ — collisions are \
                     errors (doc 10 §5)",
                    desc.name
                );
            }
            let node = build_node(
                &desc,
                &file,
                Arc::clone(&source),
                body_hash,
                Arc::clone(&pool),
            )
            .with_context(|| format!("script node `{}` ({})", desc.name, file.display()))?;
            nodes.insert(desc.name.clone(), node);
        }
    }
    Ok(nodes)
}

/// `"[Point]"` / `"Number?"` → a [`PortType`], marshallable bases only.
fn parse_notation(text: &str) -> anyhow::Result<PortType> {
    let mut rest = text.trim();
    let mut depth: u8 = 0;
    while let Some(inner) = rest
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
    {
        depth += 1;
        rest = inner;
    }
    let (base, optional) = rest
        .strip_suffix('?')
        .map_or((rest, false), |base| (base, true));
    let Some(&known) = MARSHALLABLE.iter().find(|&&candidate| candidate == base) else {
        bail!(
            "type `{text}` cannot cross the Python boundary — stage-4 script ports \
             take {}, with [list] nesting and ? optionality",
            MARSHALLABLE.join(", ")
        );
    };
    Ok(PortType {
        base: known,
        list_depth: depth,
        optional,
    })
}

/// Terminal-style short rendering for a default's catalog literal.
fn render_default(value: &HashedValue) -> String {
    match value.data() {
        ValueData::Number(x) => format!("{x}"),
        ValueData::Integer(i) => format!("{i}"),
        ValueData::Boolean(b) => format!("{b}"),
        ValueData::Text(s) => format!("{s:?}"),
        other => format!("<{}>", other.kind_name()),
    }
}

fn build_node(
    desc: &cicada_script::ScriptNodeDesc,
    file: &Path,
    source: Arc<str>,
    body_hash: ValueHash,
    pool: Arc<WorkerPool>,
) -> anyhow::Result<ScriptNode> {
    let leak = |text: &str| -> &'static str { Box::leak(text.to_owned().into_boxed_str()) };
    let inputs: Vec<PortSpec> = desc
        .inputs
        .iter()
        .map(|port: &PortDesc| {
            Ok(PortSpec {
                name: leak(&port.name),
                ty: parse_notation(&port.ty).with_context(|| format!("port `{}`", port.name))?,
                default: port
                    .default
                    .as_ref()
                    .map(|value| leak(&render_default(value))),
                doc: "",
                dimension: None,
            })
        })
        .collect::<anyhow::Result<_>>()?;
    let outputs = vec![PortSpec {
        name: "out",
        ty: parse_notation(&desc.output).context("return annotation")?,
        default: None,
        doc: "",
        dimension: None,
    }];
    let spec: &'static NodeSpec = Box::leak(Box::new(NodeSpec {
        name: leak(&desc.name),
        title: leak(&desc.title),
        description: leak(&desc.description),
        category: "Script",
        tier: Tier::S,
        version: 1, // source changes ride body_hash, not the version
        // Scripts are pure BY CONTRACT (docs/08 rule 1; stated in the
        // worker's decorator too): the engine memoizes their results, so
        // a side-effectful script would skip its effect on warm runs.
        // Effectful script nodes (ported exporters) arrive with stage 6
        // and will carry a decorator flag.
        pure: true,
        uses_tolerance: false,
        panics: None,
        inputs: Box::leak(inputs.into_boxed_slice()),
        outputs: Box::leak(outputs.into_boxed_slice()),
        module: "scripts",
        line: 0,
    }));

    // The run function: port slots (spec order) → named inputs. An absent
    // optional slot is OMITTED so the Python default applies — one source
    // of default truth. The returned value is VALIDATED against the
    // declared output annotation — a script whose return lies about its
    // type reds HERE, at the boundary, not three nodes downstream.
    // Cancellation: the headless run wires no token yet (stage 5 connects
    // the scheduler's CancelToken to a KillSwitch).
    let port_names: Vec<String> = desc.inputs.iter().map(|p| p.name.clone()).collect();
    let (file, fn_name) = (file.to_owned(), desc.name.clone());
    let out_ty = spec.outputs[0].ty;
    let run: NodeFn = Arc::new(move |values| {
        let mut inputs = BTreeMap::new();
        for (name, slot) in port_names.iter().zip(values) {
            if let Some(value) = slot {
                inputs.insert(name.clone(), Arc::clone(value));
            }
        }
        let out = pool
            .invoke(&file, &source, &fn_name, &inputs, &KillSwitch::new())
            .map_err(|error| NodeError::new(error.to_string()))?;
        if let Err(reason) = conforms(&out, out_ty.base, out_ty.list_depth, out_ty.optional) {
            return Err(NodeError::new(format!(
                "script `{fn_name}` declared `{}` but returned {reason}",
                out_ty.render()
            )));
        }
        Ok(vec![out])
    });
    Ok(ScriptNode {
        spec,
        body_hash,
        run,
    })
}

/// Does a returned value conform to the declared output notation? Integer
/// where Number is declared is legal (the system-wide exact widening);
/// everything else must match the base kind, depth, and optionality.
fn conforms(value: &HashedValue, base: &str, depth: u8, optional: bool) -> Result<(), String> {
    use cicada_core::value::ValueData;
    if depth > 0 {
        let ValueData::List(list) = value.data() else {
            return Err(format!("a {}", value.data().kind_name()));
        };
        for (index, slot) in list.slots.iter().enumerate() {
            match slot {
                None if optional => {}
                None => return Err(format!("an absent element at slot {index}")),
                Some(element) => conforms(element, base, depth - 1, optional)
                    .map_err(|reason| format!("{reason} at slot {index}"))?,
            }
        }
        return Ok(());
    }
    let kind = value.data().kind_name();
    let fits = kind == base || (base == "Number" && kind == "Integer");
    if fits {
        Ok(())
    } else {
        Err(format!("a {kind}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notation_parses_depth_and_optionality() {
        let t = parse_notation("[Point]").unwrap();
        assert_eq!((t.base, t.list_depth, t.optional), ("Point", 1, false));
        let t = parse_notation("Number?").unwrap();
        assert_eq!((t.base, t.list_depth, t.optional), ("Number", 0, true));
        let t = parse_notation("[[Number]]").unwrap();
        assert_eq!(t.list_depth, 2);
        assert!(parse_notation("Mesh").is_err(), "outside the subset");
        assert!(parse_notation("Frobnicator").is_err());
    }
}
