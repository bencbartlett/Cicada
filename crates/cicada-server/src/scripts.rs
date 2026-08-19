//! Project script-node discovery (doc 10 §5): every `scripts/*.py` next
//! to the pipeline file self-registers its `@cicada.node` functions; the
//! dialect calls them like stdlib nodes. Lives in the server's hydration
//! path (moved from `cicada-cli` at stage 5; `cicada run` still drives it).
//!
//! Cache keys: script nodes carry the SOURCE hash as `body_hash`
//! (docs/12: script `node_version` = hash of the source file) — editing
//! the `.py` recomputes exactly its cone; renaming the binding never
//! does.
//!
//! Cancellation (stage 5's job, docs/12 §Cancellation): every script node
//! runs against a [`ScriptCancel`] bridge — the solve loop installs one
//! [`KillSwitch`] per generation and kills it when the generation's
//! `CancelToken` is cancelled, so Esc kills the worker mid-call and the
//! node lands `cancelled`, never "still running in the background".

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cicada_core::hash::ValueHash;
use cicada_core::spec::{NodeSpec, PortSpec, PortType, Tier};
use cicada_core::value::{HashedValue, ValueData};
use cicada_sched::{NodeError, NodeFn};
use cicada_script::{KillSwitch, PortDesc, ScriptError, WorkerPool};

/// One discovered script node: the leaked spec plus its cache-key
/// material and run function.
pub struct ScriptNode {
    /// The spec (leaked — bounded by the script count of one session).
    pub spec: &'static NodeSpec,
    /// blake3 of the source file — the `NodeDecl.body_hash` slot.
    pub body_hash: ValueHash,
    /// The run function (calls the worker pool).
    pub run: NodeFn,
}

/// Why discovery refused — all loud, all typed (doc 14: thiserror in
/// libraries; `cicada run` wraps them in anyhow at the edge).
#[derive(Debug, thiserror::Error)]
pub enum ScriptsError {
    /// Reading `scripts/` or a file in it failed.
    #[error("reading {path}: {source}")]
    Io {
        /// The path.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// The Python host could not start (no interpreter, worker failure).
    #[error("starting the Python script host: {0}")]
    Host(#[source] ScriptError),
    /// `describe` failed for one file (import error, bad decorator).
    #[error("describing {path}: {source}")]
    Describe {
        /// The script file.
        path: PathBuf,
        /// The host's error (traceback tail included).
        #[source]
        source: ScriptError,
    },
    /// A script node's name collides with a stdlib node.
    #[error(
        "script node `{name}` ({path}) collides with a stdlib node — rename the \
         function (namespacing/qualification arrives with v0.1, doc 10 §5)"
    )]
    StdlibCollision {
        /// The node.
        name: String,
        /// The file.
        path: PathBuf,
    },
    /// The same node is defined twice in `scripts/`.
    #[error(
        "script node `{name}` is defined twice in scripts/ — collisions are errors (doc 10 §5)"
    )]
    Duplicate {
        /// The node.
        name: String,
    },
    /// A port annotation names a type that cannot cross the boundary.
    #[error("script node `{node}` ({path}), port `{port}`: {reason}")]
    BadPortType {
        /// The node.
        node: String,
        /// The file.
        path: PathBuf,
        /// The port (`return` for the output annotation).
        port: String,
        /// Why.
        reason: String,
    },
}

/// The wire kinds that cross the Python boundary (stage-4 subset —
/// mirrors `cicada_script::value`).
const MARSHALLABLE: &[&str] = &[
    "Number", "Integer", "Boolean", "Text", "Point", "Vector", "Domain",
];

/// The `CancelToken` → [`KillSwitch`] bridge: one switch per solve
/// generation, shared by every script node run function of a session.
/// [`Self::begin`] installs a fresh switch for a new generation;
/// [`Self::kill`] kills the current one (every in-flight script call of
/// that generation has its worker killed). Headless `cicada run` uses a
/// bridge it never kills — the switch simply stays unkilled.
#[derive(Debug, Default)]
pub struct ScriptCancel {
    current: Mutex<KillSwitch>,
}

impl ScriptCancel {
    /// A bridge with an unkilled switch installed.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Install a fresh switch for a new generation and return it.
    pub fn begin(&self) -> KillSwitch {
        let fresh = KillSwitch::new();
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = fresh.clone();
        fresh
    }

    /// Kill the current generation's switch.
    pub fn kill(&self) {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .kill();
    }

    fn switch(&self) -> KillSwitch {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Discover and describe every script node next to `pipeline`
/// (`<dir>/scripts/*.py`, sorted). No scripts → empty map and NO Python
/// requirement; the pool spawns only when scripts exist. Run functions
/// take their kill switch from `cancel` at call time.
///
/// # Errors
///
/// [`ScriptsError`]: unreadable files, Python/describe failures (with
/// tracebacks), bad type annotations, or a name collision.
pub fn discover(
    pipeline: &Path,
    stdlib: &[&'static NodeSpec],
    cancel: &Arc<ScriptCancel>,
) -> Result<HashMap<String, ScriptNode>, ScriptsError> {
    let dir = pipeline.parent().unwrap_or(Path::new(".")).join("scripts");
    if !dir.is_dir() {
        return Ok(HashMap::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|source| ScriptsError::Io {
            path: dir.clone(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "py"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(HashMap::new());
    }

    let pool = Arc::new(WorkerPool::new().map_err(ScriptsError::Host)?);
    let stdlib_names: std::collections::HashSet<&str> =
        stdlib.iter().map(|spec| spec.name).collect();
    let mut nodes = HashMap::new();
    for file in files {
        // Source is read ONCE and travels with every request from here on
        // (describe, invoke, and the body hash all see the SAME bytes —
        // no time-of-check/time-of-use window against concurrent edits).
        let source: Arc<str> = std::fs::read_to_string(&file)
            .map_err(|source| ScriptsError::Io {
                path: file.clone(),
                source,
            })?
            .into();
        let described = pool
            .describe(&file, &source)
            .map_err(|source| ScriptsError::Describe {
                path: file.clone(),
                source,
            })?;
        // docs/12: script node_version = source + toolchain version.
        let body_hash = cicada_script::source_hash(source.as_bytes(), &described.python_version);
        for desc in described.nodes {
            if stdlib_names.contains(desc.name.as_str()) {
                return Err(ScriptsError::StdlibCollision {
                    name: desc.name,
                    path: file,
                });
            }
            if nodes.contains_key(&desc.name) {
                return Err(ScriptsError::Duplicate { name: desc.name });
            }
            let node = build_node(
                &desc,
                &file,
                Arc::clone(&source),
                body_hash,
                Arc::clone(&pool),
                Arc::clone(cancel),
            )?;
            nodes.insert(desc.name.clone(), node);
        }
    }
    Ok(nodes)
}

/// `"[Point]"` / `"Number?"` → a [`PortType`], marshallable bases only.
fn parse_notation(text: &str) -> Result<PortType, String> {
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
        return Err(format!(
            "type `{text}` cannot cross the Python boundary — stage-4 script ports \
             take {}, with [list] nesting and ? optionality",
            MARSHALLABLE.join(", ")
        ));
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
    cancel: Arc<ScriptCancel>,
) -> Result<ScriptNode, ScriptsError> {
    let leak = |text: &str| -> &'static str { Box::leak(text.to_owned().into_boxed_str()) };
    let bad_port = |port: &str, reason: String| ScriptsError::BadPortType {
        node: desc.name.clone(),
        path: file.to_owned(),
        port: port.to_owned(),
        reason,
    };
    let inputs: Vec<PortSpec> = desc
        .inputs
        .iter()
        .map(|port: &PortDesc| {
            Ok(PortSpec {
                name: leak(&port.name),
                ty: parse_notation(&port.ty).map_err(|reason| bad_port(&port.name, reason))?,
                default: port
                    .default
                    .as_ref()
                    .map(|value| leak(&render_default(value))),
                doc: "",
                dimension: None,
            })
        })
        .collect::<Result<_, ScriptsError>>()?;
    let outputs = vec![PortSpec {
        name: "out",
        ty: parse_notation(&desc.output).map_err(|reason| bad_port("return", reason))?,
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
    // type reds HERE, at the boundary, not three nodes downstream. The
    // kill switch is the CURRENT generation's, read at call time.
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
            .invoke(&file, &source, &fn_name, &inputs, &cancel.switch())
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

    #[test]
    fn cancel_bridge_kills_only_the_current_generation() {
        let bridge = ScriptCancel::new();
        let first = bridge.begin();
        assert!(!first.is_killed());
        bridge.kill();
        assert!(first.is_killed(), "the installed switch is the one killed");
        let second = bridge.begin();
        assert!(!second.is_killed(), "a new generation starts unkilled");
        assert!(!bridge.switch().is_killed());
        bridge.kill();
        assert!(second.is_killed());
        assert!(
            first.is_killed(),
            "old switches stay killed — never resurrected"
        );
    }
}
