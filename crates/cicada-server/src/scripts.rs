//! Project script-node discovery (doc 10 §5): every `scripts/*.py` next
//! to the pipeline file self-registers its `@cicada.node` functions; the
//! dialect calls them like stdlib nodes. Lives in the server's hydration
//! path (moved from `cicada-cli` at stage 5; `cicada run` still drives it).
//!
//! The script ABI (stage 6; `cicada_script::WORKER_SOURCE`'s header is the
//! Python-side mirror): `@cicada.node(title, description, effectful=False)`
//! — `effectful=True` makes the spec impure (never memoized, never
//! auto-run; `cicada run --node <name>` / `POST /api/run/{node}` run it,
//! exactly like `export_obj`). The return annotation declares the output
//! ports: `-> "T"` is one port `out`; `-> {"a": "T", ...}` is multi-output
//! in dict order (the function returns a dict with exactly those keys —
//! the worker refuses missing/extra keys with counts); `-> None` declares
//! no outputs (the function returns None). Port kinds that cross the
//! boundary are [`MARSHALLABLE`] with `[..]` nesting and `?` optionality;
//! declared refinements (`Watertight<Mesh>`, `Closed<Curve>`) are
//! RE-CHECKED on values coming back from Python (red with counts), and
//! dropped to the base kind on the way out (the wire carries plain
//! meshes/curves, like every boundary).
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

use cicada_core::geometry::{Curve, Mesh};
use cicada_core::hash::ValueHash;
use cicada_core::spec::{NodeSpec, PortSpec, PortType, Tier};
use cicada_core::value::{HashedValue, ValueData};
use cicada_sched::{NodeError, NodeFn};
use cicada_script::{KillSwitch, OutputDesc, PortDesc, ScriptError, WorkerPool};

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
        /// The port (`return/<name>` for an output annotation).
        port: String,
        /// Why.
        reason: String,
    },
}

/// The port base kinds that cross the Python boundary (mirrors
/// `cicada_script::value`; the refinements ride as their base kind and
/// are re-checked by [`conforms`] on the way back in).
pub const MARSHALLABLE: &[&str] = &[
    "Number",
    "Integer",
    "Boolean",
    "Text",
    "Point",
    "Vector",
    "Domain",
    "Plane",
    "Mesh",
    "Watertight<Mesh>",
    "Curve",
    "Closed<Curve>",
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
            "type `{text}` cannot cross the Python boundary — script ports take {}, \
             with [list] nesting and ? optionality",
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
    // One output port per declared output (`out` for a string return
    // annotation, the dict keys for multi-output, none for `-> None`).
    let outputs: Vec<PortSpec> = desc
        .outputs
        .iter()
        .map(|port: &OutputDesc| {
            Ok(PortSpec {
                name: leak(&port.name),
                ty: parse_notation(&port.ty)
                    .map_err(|reason| bad_port(&format!("return/{}", port.name), reason))?,
                default: None,
                doc: "",
                dimension: None,
            })
        })
        .collect::<Result<_, ScriptsError>>()?;
    let spec: &'static NodeSpec = Box::leak(Box::new(NodeSpec {
        name: leak(&desc.name),
        title: leak(&desc.title),
        description: leak(&desc.description),
        category: "Script",
        tier: Tier::S,
        version: 1, // source changes ride body_hash, not the version
        // Scripts are pure BY CONTRACT (docs/08 rule 1; stated in the
        // worker's decorator too) unless the decorator says
        // `effectful=True`: the engine memoizes pure results, so a
        // side-effectful script that forgot the flag would skip its
        // effect on warm runs. Effectful nodes are never memoized and
        // never auto-run (doc 10 §7) — exactly export_obj's contract.
        pure: !desc.effectful,
        uses_tolerance: false,
        panics: None,
        inputs: Box::leak(inputs.into_boxed_slice()),
        outputs: Box::leak(outputs.into_boxed_slice()),
        module: "scripts",
        line: 0,
    }));

    // The run function: port slots (spec order) → named inputs. An absent
    // optional slot is OMITTED so the Python default applies — one source
    // of default truth. Every returned value is VALIDATED against its
    // declared output annotation (kind, depth, optionality, AND the
    // refinement predicate for Watertight<Mesh>/Closed<Curve>) — a script
    // whose return lies about its type reds HERE, at the boundary, not
    // three nodes downstream. The kill switch is the CURRENT generation's,
    // read at call time.
    let port_names: Vec<String> = desc.inputs.iter().map(|p| p.name.clone()).collect();
    let (file, fn_name) = (file.to_owned(), desc.name.clone());
    let out_ports: Vec<(&'static str, PortType)> = spec
        .outputs
        .iter()
        .map(|port| (port.name, port.ty))
        .collect();
    let run: NodeFn = Arc::new(move |values| {
        let mut inputs = BTreeMap::new();
        for (name, slot) in port_names.iter().zip(values) {
            if let Some(value) = slot {
                inputs.insert(name.clone(), Arc::clone(value));
            }
        }
        let outs = pool
            .invoke(&file, &source, &fn_name, &inputs, &cancel.switch())
            .map_err(|error| NodeError::new(error.to_string()))?;
        if outs.len() != out_ports.len() {
            return Err(NodeError::new(format!(
                "script `{fn_name}` returned {} output value(s) for {} declared output \
                 port(s) — the worker and the host disagree on the signature",
                outs.len(),
                out_ports.len()
            )));
        }
        for (out, (port, ty)) in outs.iter().zip(&out_ports) {
            if let Err(reason) = conforms(out, ty.base, ty.list_depth, ty.optional) {
                return Err(NodeError::new(format!(
                    "script `{fn_name}` output `{port}` declared `{}` but returned {reason}",
                    ty.render()
                )));
            }
        }
        Ok(outs)
    });
    Ok(ScriptNode {
        spec,
        body_hash,
        run,
    })
}

/// Does a returned value conform to the declared output notation? Integer
/// where Number is declared is legal (the system-wide exact widening);
/// a declared refinement (`Watertight<Mesh>`, `Closed<Curve>`) requires
/// the base kind AND the predicate — the wire takes no one's word for a
/// refinement (core `marshal` does the same for Rust nodes), and the
/// refusal carries counts; everything else must match the base kind,
/// depth, and optionality.
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
    match (base, value.data()) {
        ("Watertight<Mesh>", ValueData::Mesh(mesh)) => {
            if mesh.is_watertight() {
                Ok(())
            } else {
                Err(format!(
                    "a Mesh that is not watertight ({} triangles, {} open or inconsistently \
                     oriented edges)",
                    mesh.triangle_count(),
                    leaky_edge_count(mesh)
                ))
            }
        }
        ("Closed<Curve>", ValueData::Curve(curve)) => {
            if curve.is_closed() {
                Ok(())
            } else {
                let detail = match curve {
                    Curve::Polyline(polyline) => {
                        format!(" ({} vertices)", polyline.vertices.len())
                    }
                    Curve::Line(_) | Curve::Circle(_) | Curve::Rectangle(_) => String::new(),
                };
                Err(format!("an open {}{detail}", curve.variant_name()))
            }
        }
        ("Watertight<Mesh>" | "Closed<Curve>", _) => Err(format!("a {kind}")),
        _ if kind == base || (base == "Number" && kind == "Integer") => Ok(()),
        _ => Err(format!("a {kind}")),
    }
}

/// How many directed edges break watertightness: duplicated (an edge with
/// three faces, or two faces oriented the same way) or unpaired (a
/// boundary edge). Diagnostic currency for the refinement refusal; the
/// predicate itself is core's `Mesh::is_watertight`.
fn leaky_edge_count(mesh: &Mesh) -> usize {
    let mut directed: HashMap<(u32, u32), u32> = HashMap::with_capacity(mesh.indices().len());
    for tri in mesh.indices().chunks_exact(3) {
        for (from, to) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            *directed.entry((from, to)).or_insert(0) += 1;
        }
    }
    directed
        .iter()
        .filter(|&(&(from, to), &count)| count != 1 || directed.get(&(to, from)) != Some(&1))
        .count()
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Polyline;
    use cicada_core::spatial::Point;

    use super::*;

    #[test]
    fn notation_parses_depth_and_optionality() {
        let t = parse_notation("[Point]").unwrap();
        assert_eq!((t.base, t.list_depth, t.optional), ("Point", 1, false));
        let t = parse_notation("Number?").unwrap();
        assert_eq!((t.base, t.list_depth, t.optional), ("Number", 0, true));
        let t = parse_notation("[[Number]]").unwrap();
        assert_eq!(t.list_depth, 2);
        let t = parse_notation("[[Watertight<Mesh>]]").unwrap();
        assert_eq!(
            (t.base, t.list_depth, t.optional),
            ("Watertight<Mesh>", 2, false)
        );
        let t = parse_notation("Closed<Curve>?").unwrap();
        assert_eq!(
            (t.base, t.list_depth, t.optional),
            ("Closed<Curve>", 0, true)
        );
        for base in ["Mesh", "Plane", "Curve"] {
            assert_eq!(parse_notation(base).unwrap().base, base);
        }
        let error = parse_notation("Frobnicator").unwrap_err();
        assert!(
            error.contains("Watertight<Mesh>"),
            "lists the kinds: {error}"
        );
        assert!(parse_notation("Xform").is_err(), "outside the set");
    }

    // ---- the script ABI through discover() + the run functions ----
    // (Python 3 on PATH is a dev/CI requirement, as for the pool tests.)

    const FIXTURE: &str = r#"
import cicada

@cicada.node(title="Triple", description="x times three.")
def triple(x: "Number") -> "Number":
    return x * 3.0

@cicada.node(title="Split", description="two outputs.")
def split(x: "Number") -> {"twice": "Number", "labels": "[Text]"}:
    return {"twice": x * 2.0, "labels": ["a", "b"]}

@cicada.node(title="Forgetful", description="drops an output key.")
def forgetful(x: "Number") -> {"twice": "Number", "thrice": "Number"}:
    return {"twice": x * 2.0}

@cicada.node(title="Inventive", description="adds an output key.")
def inventive(x: "Number") -> {"twice": "Number"}:
    return {"twice": x * 2.0, "surprise": 1.0}

@cicada.node(title="Bare", description="returns a bare value for a multi-output node.")
def bare(x: "Number") -> {"twice": "Number", "thrice": "Number"}:
    return x

@cicada.node(title="Write Note", description="writes text to a file.", effectful=True)
def write_note(path: "Text", text: "Text") -> None:
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)

@cicada.node(title="Chatty", description="declared -> None but returns.", effectful=True)
def chatty(x: "Number") -> None:
    return x

@cicada.node(title="Shift Mesh", description="mesh moved along x.")
def shift_mesh(mesh: "Mesh", dx: "Number" = 1.0) -> "Mesh":
    moved = [(x + dx, y, z) for (x, y, z) in mesh.vertices]
    return cicada.Mesh.from_triangles(moved, mesh.triangles)

@cicada.node(title="Tetra", description="a watertight tetrahedron, or an open one.")
def tetra(open_face: "Boolean" = False) -> "Watertight<Mesh>":
    tris = [(0, 2, 1), (0, 1, 3), (1, 2, 3), (0, 3, 2)]
    if open_face:
        tris = tris[:3]
    return cicada.Mesh.from_triangles([(0, 0, 0), (1, 0, 0), (0, 1, 0), (0, 0, 1)], tris)

@cicada.node(title="Ring", description="a closed or open triangle polyline.")
def ring(closed: "Boolean" = True) -> "[Closed<Curve>]":
    return [cicada.Polyline([(0, 0, 0), (1, 0, 0), (0, 1, 0)], closed=closed)]

@cicada.node(title="Liar", description="declares Number, returns Text.")
def liar(x: "Number") -> "Number":
    return "nope"
"#;

    fn discover_fixture() -> (tempfile::TempDir, HashMap<String, ScriptNode>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("scripts")).unwrap();
        std::fs::write(dir.path().join("scripts").join("fixture.py"), FIXTURE).unwrap();
        let nodes = discover(
            &dir.path().join("pipeline.cic"),
            cicada_stdlib::registry(),
            &ScriptCancel::new(),
        )
        .expect("fixture discovers");
        (dir, nodes)
    }

    // Input SLOTS for the run functions (`Option` = the absent-optional
    // slot type of `NodeFn`, not an unnecessary wrap).
    #[allow(clippy::unnecessary_wraps)]
    fn number(x: f64) -> Option<Arc<HashedValue>> {
        Some(HashedValue::new(ValueData::Number(x)).unwrap())
    }

    #[allow(clippy::unnecessary_wraps)]
    fn boolean(b: bool) -> Option<Arc<HashedValue>> {
        Some(HashedValue::new(ValueData::Boolean(b)).unwrap())
    }

    #[allow(clippy::unnecessary_wraps)]
    fn text(s: &str) -> Option<Arc<HashedValue>> {
        Some(HashedValue::new(ValueData::Text(Arc::from(s))).unwrap())
    }

    fn tetrahedron() -> Mesh {
        Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
        )
        .unwrap()
    }

    fn port_names(spec: &NodeSpec) -> Vec<&'static str> {
        spec.outputs.iter().map(|port| port.name).collect()
    }

    #[test]
    fn effectful_flag_makes_the_spec_impure_and_none_declares_no_outputs() {
        let (_dir, nodes) = discover_fixture();
        let triple = &nodes["triple"];
        assert!(triple.spec.pure, "pure by default");
        assert_eq!(port_names(triple.spec), ["out"]);

        let note = &nodes["write_note"];
        assert!(!note.spec.pure, "effectful=True → pure: false");
        assert!(note.spec.outputs.is_empty(), "-> None → no output ports");
        assert_eq!(note.spec.inputs.len(), 2);
        assert_eq!(note.spec.category, "Script");
    }

    #[test]
    fn none_return_runs_the_effect_and_refuses_a_value() {
        let (dir, nodes) = discover_fixture();
        let path = dir.path().join("note.txt");
        let outs =
            (nodes["write_note"].run)(&[text(&path.to_string_lossy()), text("hi")]).expect("runs");
        assert!(outs.is_empty(), "no output values for -> None");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi");

        let error = (nodes["chatty"].run)(&[number(1.0)]).expect_err("a value for -> None");
        assert!(
            error.message.contains("declared `-> None`") && error.message.contains("float"),
            "{}",
            error.message
        );
    }

    #[test]
    fn multi_output_dict_return_maps_to_ports_in_declared_order() {
        let (_dir, nodes) = discover_fixture();
        let split = &nodes["split"];
        assert_eq!(port_names(split.spec), ["twice", "labels"]);
        assert_eq!(split.spec.outputs[1].ty.render(), "[Text]");
        let outs = (split.run)(&[number(2.0)]).expect("runs");
        assert_eq!(outs.len(), 2);
        assert_eq!(*outs[0].data(), ValueData::Number(4.0));
        let ValueData::List(labels) = outs[1].data() else {
            panic!("list")
        };
        assert_eq!(labels.slots.len(), 2);
    }

    #[test]
    fn multi_output_key_mismatches_are_loud_with_counts() {
        let (_dir, nodes) = discover_fixture();
        let error = (nodes["forgetful"].run)(&[number(1.0)]).expect_err("missing key");
        assert!(
            error.message.contains("1 missing [thrice]") && error.message.contains("0 extra []"),
            "{}",
            error.message
        );
        let error = (nodes["inventive"].run)(&[number(1.0)]).expect_err("extra key");
        assert!(
            error.message.contains("0 missing []") && error.message.contains("1 extra [surprise]"),
            "{}",
            error.message
        );
        let error = (nodes["bare"].run)(&[number(1.0)]).expect_err("non-dict");
        assert!(
            error
                .message
                .contains("must return a dict with exactly those keys"),
            "{}",
            error.message
        );
    }

    #[test]
    fn mesh_round_trips_through_a_script_node() {
        let (_dir, nodes) = discover_fixture();
        let shift = &nodes["shift_mesh"];
        assert_eq!(shift.spec.inputs[0].ty.render(), "Mesh");
        assert_eq!(shift.spec.outputs[0].ty.render(), "Mesh");
        let input = HashedValue::new(ValueData::Mesh(tetrahedron())).unwrap();
        let outs = (shift.run)(&[Some(input), number(0.5)]).expect("runs");
        let ValueData::Mesh(out) = outs[0].data() else {
            panic!("mesh")
        };
        assert_eq!(out.indices(), tetrahedron().indices());
        assert_eq!(&out.positions()[0..6], &[0.5, 0.0, 0.0, 1.5, 0.0, 0.0]);
        assert!(out.is_watertight());
    }

    #[test]
    fn declared_refinements_are_checked_on_the_way_in_with_counts() {
        let (_dir, nodes) = discover_fixture();
        let tetra = &nodes["tetra"];
        assert_eq!(tetra.spec.outputs[0].ty.render(), "Watertight<Mesh>");
        let outs = (tetra.run)(&[boolean(false)]).expect("a watertight mesh passes");
        assert!(matches!(outs[0].data(), ValueData::Mesh(m) if m.is_watertight()));
        let error = (tetra.run)(&[boolean(true)]).expect_err("an open mesh is refused");
        assert!(
            error
                .message
                .contains("output `out` declared `Watertight<Mesh>`")
                && error
                    .message
                    .contains("not watertight (3 triangles, 3 open"),
            "{}",
            error.message
        );

        let ring = &nodes["ring"];
        assert_eq!(ring.spec.outputs[0].ty.render(), "[Closed<Curve>]");
        let outs = (ring.run)(&[boolean(true)]).expect("a closed polyline passes");
        assert!(matches!(outs[0].data(), ValueData::List(_)));
        let error = (ring.run)(&[boolean(false)]).expect_err("an open polyline is refused");
        assert!(
            error.message.contains("declared `[Closed<Curve>]`")
                && error
                    .message
                    .contains("an open Polyline (3 vertices) at slot 0"),
            "{}",
            error.message
        );

        // The boundary lie from stage 4 still reds at the boundary.
        let error = (nodes["liar"].run)(&[number(1.0)]).expect_err("type lie");
        assert!(
            error.message.contains("declared `Number`") && error.message.contains("a Text"),
            "{}",
            error.message
        );
    }

    #[test]
    fn conforms_checks_refinement_predicates_not_just_kinds() {
        let open = HashedValue::new(ValueData::Curve(Curve::Polyline(Polyline {
            vertices: vec![Point::new(0.0, 0.0, 0.0), Point::new(1.0, 0.0, 0.0)],
            closed: false,
        })))
        .unwrap();
        assert!(conforms(&open, "Curve", 0, false).is_ok());
        assert_eq!(
            conforms(&open, "Closed<Curve>", 0, false).unwrap_err(),
            "an open Polyline (2 vertices)"
        );
        let mesh = HashedValue::new(ValueData::Mesh(tetrahedron())).unwrap();
        assert!(conforms(&mesh, "Watertight<Mesh>", 0, false).is_ok());
        assert_eq!(
            conforms(&mesh, "Closed<Curve>", 0, false).unwrap_err(),
            "a Mesh",
            "wrong kind for a refinement names the kind"
        );
        let leaky = Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3],
        )
        .unwrap();
        assert_eq!(
            leaky_edge_count(&leaky),
            3,
            "the missing face leaves 3 unpaired edges"
        );
        let leaky = HashedValue::new(ValueData::Mesh(leaky)).unwrap();
        assert!(conforms(&leaky, "Mesh", 0, false).is_ok());
        assert!(
            conforms(&leaky, "Watertight<Mesh>", 0, false)
                .unwrap_err()
                .contains("3 triangles, 3 open")
        );
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
