//! The graph view-model (docs/13 §State ownership): the canvas is a
//! generated view of the `.cic` text — every node here IS one statement,
//! every wire IS one kwarg reference. Built from the parsed document, the
//! checker's resolution (types, diagnostics), the lowering's exclusions
//! (red / blocked cones), and the layout sidecar. Serialized as JSON in
//! `snapshot` / `delta` messages; the client renders it and never invents
//! structure of its own.

use std::collections::HashMap;

use cicada_core::geometry::{GEOMETRY_KINDS, TRANSFORMABLE_KINDS, VAR_TRANSFORMABLE};
use cicada_core::spec::NodeSpec;
use cicada_lang::ast::{Expr, LineSpan, Lit, Rhs, Statement, ValueExpr};
use cicada_lang::check::{BindingType, Resolution, WireType};
use cicada_lang::diag::{Diagnostic, DiagnosticKind};
use cicada_lang::document::{Document, Line};
use serde::Serialize;

use crate::layout::{LayoutNode, NODE_WIDTH, auto_layout};
use crate::lower::{Exclusion, Lowered};
use crate::sidecar::Sidecar;

/// Stable per-session node identifiers: a binding name maps to one `u32`
/// for the life of the session (renames move it), so binary frames and
/// pick tables can name nodes compactly. Names, not refs, remain the
/// identity in text and sidecar (docs/10).
#[derive(Debug, Default, Clone)]
pub struct NodeRefs {
    next: u32,
    by_name: HashMap<String, u32>,
}

impl NodeRefs {
    /// The ref for `name`, assigned on first sight.
    pub fn get_or_assign(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        self.next += 1;
        self.by_name.insert(name.to_owned(), self.next);
        self.next
    }

    /// The ref for `name`, if assigned.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    /// Move a ref to a new name (rename).
    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(id) = self.by_name.remove(old) {
            self.by_name.insert(new.to_owned(), id);
        }
    }

    /// The name owning `id`, if any (reverse lookup for picks).
    #[must_use]
    pub fn name_of(&self, id: u32) -> Option<&str> {
        self.by_name
            .iter()
            .find(|&(_, &candidate)| candidate == id)
            .map(|(name, _)| name.as_str())
    }
}

/// What kind of statement a node renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// `name = fn(...)`.
    Call,
    /// `name = 12.0` — a constant param.
    Literal,
    /// `name = a + b` — one Expression node.
    Expression,
    /// A statement that failed to parse (reds its node).
    Broken,
    /// A `#off`-disabled statement (ghost): when its body parses the node
    /// keeps its ports, literals and wires — only `kind` says it is off.
    Disabled,
}

/// One end of a wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct WireEnd {
    /// Binding name.
    pub node: String,
    /// Port name.
    pub port: String,
}

/// One input port as the canvas shows it.
#[derive(Debug, Clone, Serialize)]
pub struct InputView {
    /// Port name (the kwarg).
    pub name: String,
    /// Declared type notation (`[Point]`, `Curve?`, `T`).
    #[serde(rename = "type")]
    pub ty: String,
    /// Base kind.
    pub base: String,
    /// List depth.
    pub depth: u8,
    /// Element optionality.
    pub optional: bool,
    /// Required (no default).
    pub required: bool,
    /// The catalog default literal, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// The catalog default parsed to JSON when it is a scalar literal of
    /// the port's kind (numbers, booleans, text) — what a typed-literal
    /// chip starts from when the text carries no kwarg (wave 4 B3, finding
    /// U9). Absent when the port has no default, and for a default that is
    /// not a scalar literal (`plane = xy_plane`) — see [`default_json`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    /// Port doc line.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// `length` / `angle` dimension tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<&'static str>,
    /// Wired from another binding's port.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wired: Option<WireEnd>,
    /// Inline literal (raw source text), when the kwarg is a literal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literal: Option<String>,
    /// The literal parsed to JSON when it is a scalar (numbers, booleans,
    /// text) — inline widgets edit these.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literal_value: Option<serde_json::Value>,
    /// `each()` depth on this kwarg (the persistent lift badge).
    pub lift: u8,
    /// This kwarg is not a port of the node (`UnknownKwarg`) — shown so the
    /// user sees what the text says.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unknown: bool,
    /// Byte span of the kwarg value within the line (for the text panel).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<[usize; 2]>,
}

/// One output port.
#[derive(Debug, Clone, Serialize)]
pub struct OutputView {
    /// Port name.
    pub name: String,
    /// Declared type notation.
    #[serde(rename = "type")]
    pub ty: String,
    /// The type the checker resolved on this wire (type variables bound,
    /// lifts applied), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    /// Base kind of the resolved (or declared) type.
    pub base: String,
    /// True when values of this kind draw in the viewport.
    pub displayable: bool,
}

/// The parameter widget a node exposes (docs/10 §3 — canvas widgets are
/// constructor bindings). `port` names the kwarg the widget edits
/// (`value` for `slider`/`toggle`); `None` edits a bare literal binding.
#[derive(Debug, Clone, Serialize)]
pub struct ParamView {
    /// `slider` / `toggle` / `number` / `integer` / `boolean` / `text`.
    pub kind: &'static str,
    /// The kwarg the widget edits, if a call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// Current value.
    pub value: serde_json::Value,
    /// Slider bounds/step (spec defaults when unwritten).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Slider max.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Slider step (0 = continuous).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
}

/// Why a node is out of the solve (mirrors the scheduler's status words).
#[derive(Debug, Clone, Serialize)]
pub struct ExcludedView {
    /// `red` or `blocked`.
    pub status: &'static str,
    /// The honest reason.
    pub reason: String,
}

/// One node of the canvas.
// A serialized record the client mirrors field for field: `effectful`,
// `preview`, `manual` and (B4) `collapsed` are four independent wire flags,
// not a state machine — folding them into enums would change the JSON the
// client reads for no gain in meaning.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize)]
pub struct NodeView {
    /// Stable per-session ref (frames name nodes by this).
    #[serde(rename = "ref")]
    pub node_ref: u32,
    /// Binding name (identity for text + sidecar).
    pub name: String,
    /// All bound names (multi-output unpack lists every target).
    pub targets: Vec<String>,
    /// 0-based line index in the file.
    pub line: usize,
    /// The raw statement text.
    pub text: String,
    /// Statement kind.
    pub kind: NodeKind,
    /// Called function (dialect name), for calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub func: Option<String>,
    /// Catalog title (or a fallback for expressions/literals).
    pub title: String,
    /// Catalog category (or `Params & input` for literals, `Maths & logic`
    /// for expressions).
    pub category: String,
    /// Catalog description.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The node's runtime contract ("Red when: …"), when the catalog has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panics: Option<String>,
    /// Inputs in display order (spec order; sidecar port order later).
    pub inputs: Vec<InputView>,
    /// Outputs.
    pub outputs: Vec<OutputView>,
    /// The parameter widget, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<ParamView>,
    /// The attached comment block (canvas note), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Diagnostics naming this node.
    pub diagnostics: Vec<Diagnostic>,
    /// Out of the solve graph, and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded: Option<ExcludedView>,
    /// Effectful (exporter): never auto-runs; explicit run only.
    pub effectful: bool,
    /// Preview on (draws in the viewport when displayable).
    pub preview: bool,
    /// Grid cell `[x, y]`.
    pub cell: [i64; 2],
    /// Size in grid units `[w, h]`.
    pub size: [u32; 2],
    /// The cell is a manual sidecar override.
    pub manual: bool,
    /// Drawn collapsed — one grid unit tall, name · track · value on one
    /// row (wave 4 B4, a slider's GH-like compact form): the sidecar's
    /// `collapsed` override, honoured only while the node CAN collapse
    /// ([`collapse_refusal`] is `None`); `size` already says `[w, 1]` then.
    /// Omitted when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

/// The slider's ports the collapsed row has no handle for — its one row is
/// name · track · value · output, so a wire into any of these would have
/// nowhere to land (the track IS `value`). Spec order: the refusal names
/// them in this order, and the client mirror (`collapse.ts`) lists the
/// same four.
pub const COLLAPSED_ROW_PORTS: [&str; 4] = ["value", "min", "max", "step"];

/// Why `name` cannot be drawn collapsed (wave 4 B4): `None` when it can.
/// THE rule, read off the DOCUMENT — the binding's call, live or `#off` —
/// never off the view: inside a `batch` the view lags the document (it is
/// rebuilt at the commit), so a bound wired, unwired or a slider placed by
/// an earlier op of the same batch must be seen by the `set_collapsed`
/// that follows (the first cut read the view and let
/// `batch[connect n.out → size.max, set_collapsed size]` land the flag
/// silently — review finding, 2026-08-24). [`build`] decides the drawn
/// `collapsed` with it and the session's `set_collapsed` refuses with its
/// text, so the two cannot drift. Only a slider collapses (its compact row
/// is name · track · value · output), and only while every port of
/// [`COLLAPSED_ROW_PORTS`] is a literal: a kwarg whose unlifted value is a
/// reference is wired, and the collapsed row has no port for it. A name
/// the document does not bind as a parsed statement is not a slider.
#[must_use]
pub fn collapse_refusal(document: &Document, name: &str) -> Option<String> {
    let call = document
        .statements_including_disabled()
        .find(|(_, statement, _, _)| statement.targets.iter().any(|t| t.name == name))
        .and_then(|(_, statement, _, _)| match &statement.rhs {
            Rhs::Call(call) if call.func.name == "slider" => Some(call),
            _ => None,
        });
    let Some(call) = call else {
        return Some(format!(
            "`{name}` is not a slider — only sliders collapse (one row: name, track, value)"
        ));
    };
    let wired: Vec<&str> = COLLAPSED_ROW_PORTS
        .into_iter()
        .filter(|port| {
            call.kwargs.iter().any(|kwarg| {
                kwarg.name.name == *port && matches!(kwarg.value.unlifted(), ValueExpr::Ref(_))
            })
        })
        .collect();
    if wired.is_empty() {
        return None;
    }
    let verb = if wired.len() == 1 { "is" } else { "are" };
    Some(format!(
        "`{name}`: {} {verb} wired — a slider collapses only while value, min, max and step \
         are literals (the collapsed row has no port for a wire)",
        wired.join(" and "),
    ))
}

/// One wire.
#[derive(Debug, Clone, Serialize)]
pub struct WireView {
    /// `from.port→to.port` — stable within a document state.
    pub id: String,
    /// Source.
    pub from: WireEnd,
    /// Target.
    pub to: WireEnd,
    /// `each()` depth on the target kwarg.
    pub lift: u8,
    /// The value type carried (source's resolved type), when known.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    /// Depth of the carried type (styling).
    pub depth: u8,
    /// Red: a diagnostic sits on this kwarg.
    pub red: bool,
    /// The diagnostic message, when red.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The whole canvas.
#[derive(Debug, Clone, Serialize)]
pub struct GraphView {
    /// Nodes in file order.
    pub nodes: Vec<NodeView>,
    /// Wires.
    pub wires: Vec<WireView>,
    /// Every diagnostic (file-level ones included), sorted by position.
    pub diagnostics: Vec<Diagnostic>,
    /// The dialect version pragma, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dialect: Option<u32>,
}

impl GraphView {
    /// The node named `name`.
    #[must_use]
    pub fn node(&self, name: &str) -> Option<&NodeView> {
        self.nodes.iter().find(|node| node.name == name)
    }
}

/// Is a base kind drawable in the viewport?
#[must_use]
pub fn displayable_kind(base: &str) -> bool {
    GEOMETRY_KINDS.contains(&base) || base == "Geometry"
}

/// Build the view-model. `refs` assigns node refs on first sight.
#[must_use]
#[allow(clippy::too_many_lines)] // lines → nodes, diagnostics, layout: one linear pass
pub fn build(
    document: &Document,
    resolution: &Resolution,
    specs: &[&'static NodeSpec],
    lowered: &Lowered,
    sidecar: &Sidecar,
    refs: &mut NodeRefs,
) -> GraphView {
    let spec_by_name: HashMap<&str, &'static NodeSpec> =
        specs.iter().map(|spec| (spec.name, *spec)).collect();
    let mut nodes: Vec<NodeView> = Vec::new();
    let mut wires: Vec<WireView> = Vec::new();
    let mut pending_comment: Vec<String> = Vec::new();

    for (index, line) in document.lines().iter().enumerate() {
        match line {
            Line::Pragma { .. } => {}
            Line::Blank { .. } => pending_comment.clear(),
            Line::Comment { raw } => {
                pending_comment.push(raw.trim_start_matches('#').trim().to_owned());
            }
            Line::Statement { statement, raw } => {
                let comment = take_comment(&mut pending_comment);
                let mut node = statement_node(
                    index,
                    statement,
                    raw,
                    resolution,
                    &spec_by_name,
                    lowered,
                    &mut wires,
                );
                node.comment = comment;
                nodes.push(node);
            }
            Line::Disabled {
                raw,
                name,
                statement: Some(statement),
            } => {
                // A ghost WITH its ports and wiring (DECISIONS.md node-disable
                // row): the same node the statement would be, flagged off.
                // `lowered.excluded` carries the "disabled (`#off`)" reason.
                let comment = take_comment(&mut pending_comment);
                let mut node = statement_node(
                    index,
                    statement,
                    raw,
                    resolution,
                    &spec_by_name,
                    lowered,
                    &mut wires,
                );
                debug_assert_eq!(name.as_deref(), Some(node.name.as_str()));
                node.kind = NodeKind::Disabled;
                node.comment = comment;
                nodes.push(node);
            }
            Line::Disabled {
                raw,
                name,
                statement: None,
            } => {
                // The body behind the prefix does not parse: a port-less
                // ghost that shows its text (enabling it surfaces the error).
                let comment = take_comment(&mut pending_comment);
                let name = name
                    .clone()
                    .unwrap_or_else(|| format!("line_{}", index + 1));
                nodes.push(NodeView {
                    node_ref: 0,
                    targets: vec![name.clone()],
                    name,
                    line: index,
                    text: raw.clone(),
                    kind: NodeKind::Disabled,
                    func: None,
                    title: "disabled".to_owned(),
                    category: String::new(),
                    description: String::new(),
                    panics: None,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                    param: None,
                    comment,
                    diagnostics: Vec::new(),
                    excluded: Some(ExcludedView {
                        status: Exclusion::Disabled.status(),
                        reason: Exclusion::Disabled.reason(),
                    }),
                    effectful: false,
                    preview: false,
                    cell: [0, 0],
                    size: [NODE_WIDTH, 2],
                    manual: false,
                    collapsed: false,
                });
            }
            Line::Broken { raw, node, .. } => {
                let comment = take_comment(&mut pending_comment);
                let name = node
                    .clone()
                    .unwrap_or_else(|| format!("line_{}", index + 1));
                nodes.push(NodeView {
                    node_ref: 0,
                    targets: vec![name.clone()],
                    name,
                    line: index,
                    text: raw.clone(),
                    kind: NodeKind::Broken,
                    func: None,
                    title: "does not parse".to_owned(),
                    category: String::new(),
                    description: String::new(),
                    panics: None,
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                    param: None,
                    comment,
                    diagnostics: Vec::new(),
                    excluded: Some(ExcludedView {
                        status: "red",
                        reason: "the statement does not parse".to_owned(),
                    }),
                    effectful: false,
                    preview: false,
                    cell: [0, 0],
                    size: [NODE_WIDTH, 2],
                    manual: false,
                    collapsed: false,
                });
            }
        }
    }

    // Diagnostics: attach by node name, and by line for nameless ones.
    for node in &mut nodes {
        node.diagnostics = resolution
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .node
                    .as_ref()
                    .is_some_and(|named| node.targets.iter().any(|target| target == named))
                    || (diagnostic.node.is_none() && diagnostic.span.line == node.line + 1)
            })
            .cloned()
            .collect();
        node.node_ref = refs.get_or_assign(&node.name);
        // Collapsed (wave 4 B4): the sidecar's flag, honoured while the
        // node can collapse — the document-level rule the session's
        // `set_collapsed` refuses with, so a slider whose bound got wired
        // by a text edit is drawn expanded (the wire must reach a port)
        // until the wire goes; the flag stays in the sidecar. Decided
        // BEFORE the layout, which packs by size: a collapsed node is one
        // unit tall.
        node.collapsed = sidecar
            .overrides
            .get(&node.name)
            .and_then(|entry| entry.collapsed)
            .unwrap_or(false)
            && collapse_refusal(document, &node.name).is_none();
        if node.collapsed {
            node.size[1] = 1;
        }
    }

    // Layout: sidecar overrides + auto cells for the rest.
    let layout_nodes: Vec<LayoutNode<'_>> = nodes
        .iter()
        .map(|node| LayoutNode {
            name: &node.name,
            deps: node
                .inputs
                .iter()
                .filter_map(|input| input.wired.as_ref().map(|end| end.node.as_str()))
                .collect(),
            size: node.size,
            manual: sidecar
                .overrides
                .get(&node.name)
                .and_then(|entry| entry.cell),
        })
        .collect();
    let cells = auto_layout(&layout_nodes);
    for node in &mut nodes {
        if let Some(cell) = cells.get(&node.name) {
            node.cell = *cell;
        }
        let entry = sidecar.overrides.get(&node.name);
        node.manual = entry.is_some_and(|entry| entry.cell.is_some());
        let displayable = node.outputs.iter().any(|output| output.displayable);
        node.preview = entry.and_then(|entry| entry.preview).unwrap_or(displayable) && displayable;
    }

    GraphView {
        nodes,
        wires,
        diagnostics: resolution.diagnostics.clone(),
        dialect: document.version(),
    }
}

fn take_comment(pending: &mut Vec<String>) -> Option<String> {
    if pending.is_empty() {
        None
    } else {
        Some(std::mem::take(pending).join("\n"))
    }
}

/// The type a binding carries (resolved), rendered — `None` when the
/// checker poisoned it or it is a whole multi-output node.
fn binding_type(resolution: &Resolution, name: &str) -> Option<WireType> {
    match resolution.bindings.get(name) {
        Some(BindingType::Value { ty, .. }) => Some(ty.clone()),
        _ => None,
    }
}

/// The type of `binding.port` on a multi-output node binding — as the
/// checker resolved it for THAT call (type variables substituted, lift
/// applied: `cull.kept` reads `[Point]`, not `[E]`).
fn node_port_type(resolution: &Resolution, binding: &str, port: &str) -> Option<WireType> {
    let Some(BindingType::Node { outputs, .. }) = resolution.bindings.get(binding) else {
        return None;
    };
    outputs.get(port).cloned()
}

/// Does `diagnostic` sit on line `line` (0-based) overlapping `span`?
fn hits(diagnostic: &Diagnostic, line: usize, span: LineSpan) -> bool {
    diagnostic.span.line == line + 1
        && diagnostic.span.col_start < span.end
        && diagnostic.span.col_end > span.start
}

fn literal_json(lit: &Lit) -> Option<serde_json::Value> {
    match lit {
        Lit::Number { value, integer } => {
            if *integer && value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
                #[allow(clippy::cast_possible_truncation)]
                Some(serde_json::Value::from(*value as i64))
            } else {
                serde_json::Number::from_f64(*value).map(serde_json::Value::Number)
            }
        }
        Lit::Text(text) => Some(serde_json::Value::String(text.clone())),
        Lit::Boolean(flag) => Some(serde_json::Value::Bool(*flag)),
        Lit::List(_) => None,
    }
}

/// The catalog default of a scalar port as JSON (wave 4 B3: the value a
/// typed-literal chip starts from when the text carries no kwarg). The
/// `#[node]` macro renders a literal default as Rust spells it — numbers
/// and quoted text as the dialect does, booleans as `true` / `false` — so
/// the text is read as a dialect literal, the Rust booleans accepted for a
/// `Boolean` port. Anything else is `None`: a `default_doc` rendering
/// (`xy_plane`, `origin`), a list port, a value of another kind than the
/// port's — the chip then shows the rendering and starts empty.
fn default_json(base: &str, list_depth: u8, text: &str) -> Option<serde_json::Value> {
    if list_depth != 0 {
        return None;
    }
    let value = match (base, text) {
        ("Boolean", "true" | "True") => serde_json::Value::Bool(true),
        ("Boolean", "false" | "False") => serde_json::Value::Bool(false),
        _ => {
            let trial = Document::parse(&format!("# cicada 1\n_default = f(x={text})\n"));
            let (_, statement, _) = trial.statements().next()?;
            let Rhs::Call(call) = &statement.rhs else {
                return None;
            };
            let ValueExpr::Literal(literal) = &call.kwargs.first()?.value else {
                return None;
            };
            literal_json(&literal.lit)?
        }
    };
    let fits = match base {
        "Integer" => value.is_i64(),
        "Number" => value.is_number(),
        "Boolean" => value.is_boolean(),
        "Text" => value.is_string(),
        _ => false,
    };
    fits.then_some(value)
}

fn number_kwarg(call: &cicada_lang::ast::Call, name: &str) -> Option<f64> {
    call.kwargs
        .iter()
        .find(|k| k.name.name == name)
        .and_then(|k| match &k.value {
            ValueExpr::Literal(lit) => match lit.lit {
                Lit::Number { value, .. } => Some(value),
                _ => None,
            },
            _ => None,
        })
}

#[allow(clippy::too_many_lines)] // one statement → one node, all cases in one place
fn statement_node(
    line: usize,
    statement: &Statement,
    raw: &str,
    resolution: &Resolution,
    spec_by_name: &HashMap<&str, &'static NodeSpec>,
    lowered: &Lowered,
    wire_sink: &mut Vec<WireView>,
) -> NodeView {
    let name = statement.name().to_owned();
    let targets: Vec<String> = statement.targets.iter().map(|t| t.name.clone()).collect();
    let excluded = lowered.excluded.get(&name).map(|exclusion| ExcludedView {
        status: exclusion.status(),
        reason: exclusion.reason(),
    });
    let mut node = NodeView {
        node_ref: 0,
        name: name.clone(),
        targets,
        line,
        text: raw.to_owned(),
        kind: NodeKind::Call,
        func: None,
        title: String::new(),
        category: String::new(),
        description: String::new(),
        panics: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        param: None,
        comment: None,
        diagnostics: Vec::new(),
        excluded,
        effectful: false,
        preview: false,
        cell: [0, 0],
        size: [NODE_WIDTH, 2],
        manual: false,
        collapsed: false,
    };
    // A wire from a kwarg / free-var reference; red when a diagnostic
    // sits on it.
    let mut wire_for = |target_port: &str, value: &ValueExpr, lift: u8| -> Option<WireEnd> {
        let ValueExpr::Ref(port_ref) = value.unlifted() else {
            return None;
        };
        let (source_ty, source_port) = match &port_ref.port {
            Some(port) => (
                node_port_type(resolution, &port_ref.binding.name, &port.name),
                port.name.clone(),
            ),
            None => (
                binding_type(resolution, &port_ref.binding.name),
                "out".to_owned(),
            ),
        };
        let span = value.span();
        let red = resolution
            .diagnostics
            .iter()
            .find(|diagnostic| hits(diagnostic, line, span));
        let from = WireEnd {
            node: port_ref.binding.name.clone(),
            port: source_port,
        };
        wire_sink.push(WireView {
            id: format!("{}.{}->{}.{}", from.node, from.port, name, target_port),
            from: from.clone(),
            to: WireEnd {
                node: name.clone(),
                port: target_port.to_owned(),
            },
            lift,
            depth: source_ty.as_ref().map_or(0, |ty| ty.depth),
            ty: source_ty.map(|ty| ty.render()),
            red: red.is_some(),
            reason: red.map(|diagnostic| diagnostic.message.clone()),
        });
        Some(from)
    };

    match &statement.rhs {
        Rhs::Literal(lit) => {
            node.kind = NodeKind::Literal;
            "Constant".clone_into(&mut node.title);
            "Params & input".clone_into(&mut node.category);
            let ty = binding_type(resolution, &name);
            let base = ty
                .as_ref()
                .map_or_else(|| "?".to_owned(), |t| t.base.clone());
            node.outputs.push(OutputView {
                name: "out".to_owned(),
                ty: ty.as_ref().map_or_else(|| "?".to_owned(), WireType::render),
                resolved: ty.as_ref().map(WireType::render),
                displayable: displayable_kind(&base),
                base,
            });
            let kind = match &lit.lit {
                Lit::Number { integer: true, .. } => "integer",
                Lit::Number { .. } => "number",
                Lit::Boolean(_) => "boolean",
                Lit::Text(_) => "text",
                Lit::List(_) => "list",
            };
            node.param = literal_json(&lit.lit).map(|value| ParamView {
                kind,
                port: None,
                value,
                min: None,
                max: None,
                step: None,
            });
        }
        Rhs::Expression(expr) => {
            node.kind = NodeKind::Expression;
            "Expression".clone_into(&mut node.title);
            "Maths & logic".clone_into(&mut node.category);
            node.description = expr_text(expr, raw);
            for ident in expr.free_vars() {
                let source_ty = binding_type(resolution, &ident.name);
                let value = ValueExpr::Ref(cicada_lang::ast::PortRef {
                    binding: ident.clone(),
                    port: None,
                    span: ident.span,
                });
                let wired = wire_for(&ident.name, &value, 0);
                node.inputs.push(InputView {
                    name: ident.name.clone(),
                    ty: source_ty
                        .as_ref()
                        .map_or_else(|| "Number".to_owned(), WireType::render),
                    base: source_ty
                        .as_ref()
                        .map_or_else(|| "Number".to_owned(), |t| t.base.clone()),
                    depth: source_ty.as_ref().map_or(0, |t| t.depth),
                    optional: false,
                    required: true,
                    default: None,
                    default_value: None,
                    doc: String::new(),
                    dimension: None,
                    wired,
                    literal: None,
                    literal_value: None,
                    lift: 0,
                    unknown: false,
                    span: Some([ident.span.start, ident.span.end]),
                });
            }
            let ty = binding_type(resolution, &name);
            let base = ty
                .as_ref()
                .map_or_else(|| "Number".to_owned(), |t| t.base.clone());
            node.outputs.push(OutputView {
                name: "out".to_owned(),
                ty: ty
                    .as_ref()
                    .map_or_else(|| "Number".to_owned(), WireType::render),
                resolved: ty.as_ref().map(WireType::render),
                displayable: displayable_kind(&base),
                base,
            });
        }
        Rhs::Call(call) => {
            node.func = Some(call.func.name.clone());
            let spec = spec_by_name.get(call.func.name.as_str()).copied();
            if let Some(spec) = spec {
                {
                    spec.title.clone_into(&mut node.title);
                    spec.category.clone_into(&mut node.category);
                    spec.description.clone_into(&mut node.description);
                    node.panics = spec.panics.map(str::to_owned);
                    node.effectful = !spec.pure;
                    // The call's bound type variable (T → the wired kind),
                    // for output display.
                    let mut bound_var: Option<String> = None;
                    for port in spec.inputs {
                        let kwarg = call.kwargs.iter().find(|k| k.name.name == port.name);
                        let (wired, literal, literal_value, lift, span) = match kwarg {
                            None => (None, None, None, 0, None),
                            Some(kwarg) => {
                                let lift = kwarg.value.each_depth();
                                let span = kwarg.value.span();
                                let unlifted = kwarg.value.unlifted();
                                let wired = wire_for(port.name, &kwarg.value, lift);
                                if port.ty.base == VAR_TRANSFORMABLE
                                    && let Some(end) = &wired
                                {
                                    let source_ty = if end.port == "out" {
                                        binding_type(resolution, &end.node)
                                    } else {
                                        node_port_type(resolution, &end.node, &end.port)
                                    };
                                    if let Some(source_ty) = source_ty
                                        && TRANSFORMABLE_KINDS.contains(&source_ty.base.as_str())
                                    {
                                        bound_var = Some(source_ty.base);
                                    }
                                }
                                let (literal, literal_value) = match unlifted {
                                    ValueExpr::Literal(literal) => (
                                        Some(literal.span.slice(raw).to_owned()),
                                        literal_json(&literal.lit),
                                    ),
                                    _ => (None, None),
                                };
                                (
                                    wired,
                                    literal,
                                    literal_value,
                                    lift,
                                    Some([span.start, span.end]),
                                )
                            }
                        };
                        node.inputs.push(InputView {
                            name: port.name.to_owned(),
                            ty: port.ty.render(),
                            base: port.ty.base.to_owned(),
                            depth: port.ty.list_depth,
                            optional: port.ty.optional,
                            required: port.default.is_none(),
                            default: port.default.map(str::to_owned),
                            default_value: port.default.and_then(|text| {
                                default_json(port.ty.base, port.ty.list_depth, text)
                            }),
                            doc: port.doc.to_owned(),
                            dimension: port.dimension.map(|d| match d {
                                cicada_core::spec::Dimension::Length => "length",
                                cicada_core::spec::Dimension::Angle => "angle",
                            }),
                            wired,
                            literal,
                            literal_value,
                            lift,
                            unknown: false,
                            span,
                        });
                    }
                    // Kwargs the spec does not know: shown, flagged.
                    for kwarg in &call.kwargs {
                        if spec.inputs.iter().any(|port| port.name == kwarg.name.name) {
                            continue;
                        }
                        let lift = kwarg.value.each_depth();
                        let wired = wire_for(&kwarg.name.name, &kwarg.value, lift);
                        let span = kwarg.value.span();
                        node.inputs.push(InputView {
                            name: kwarg.name.name.clone(),
                            ty: "?".to_owned(),
                            base: "?".to_owned(),
                            depth: 0,
                            optional: false,
                            required: false,
                            default: None,
                            default_value: None,
                            doc: String::new(),
                            dimension: None,
                            wired,
                            literal: match kwarg.value.unlifted() {
                                ValueExpr::Literal(literal) => {
                                    Some(literal.span.slice(raw).to_owned())
                                }
                                _ => None,
                            },
                            literal_value: None,
                            lift,
                            unknown: true,
                            span: Some([span.start, span.end]),
                        });
                    }
                    let lift = call
                        .kwargs
                        .iter()
                        .map(|k| k.value.each_depth())
                        .max()
                        .unwrap_or(0);
                    for (index, output) in spec.outputs.iter().enumerate() {
                        let resolved = if statement.targets.len() > 1 {
                            statement
                                .targets
                                .get(index)
                                .and_then(|target| binding_type(resolution, &target.name))
                        } else if spec.outputs.len() == 1 {
                            binding_type(resolution, &name)
                        } else {
                            node_port_type(resolution, &name, output.name)
                        };
                        let mut declared = WireType::from_port(&output.ty);
                        if let Some(bound) = &bound_var
                            && declared.base == VAR_TRANSFORMABLE
                        {
                            declared.base.clone_from(bound);
                        }
                        declared.depth = declared.depth.saturating_add(lift);
                        let base = resolved
                            .as_ref()
                            .map_or_else(|| declared.base.clone(), |t| t.base.clone());
                        node.outputs.push(OutputView {
                            name: output.name.to_owned(),
                            ty: output.ty.render(),
                            resolved: Some(
                                resolved
                                    .as_ref()
                                    .map_or_else(|| declared.render(), WireType::render),
                            ),
                            displayable: displayable_kind(&base) || base == VAR_TRANSFORMABLE,
                            base,
                        });
                    }
                    node.param = match spec.name {
                        "slider" => number_kwarg(call, "value").map(|value| ParamView {
                            kind: "slider",
                            port: Some("value".to_owned()),
                            value: serde_json::json!(value),
                            min: Some(number_kwarg(call, "min").unwrap_or(0.0)),
                            max: Some(number_kwarg(call, "max").unwrap_or(10.0)),
                            step: Some(number_kwarg(call, "step").unwrap_or(0.0)),
                        }),
                        "toggle" => call
                            .kwargs
                            .iter()
                            .find(|k| k.name.name == "value")
                            .and_then(|k| match &k.value {
                                ValueExpr::Literal(literal) => match literal.lit {
                                    Lit::Boolean(flag) => Some(flag),
                                    _ => None,
                                },
                                _ => None,
                            })
                            .map(|flag| ParamView {
                                kind: "toggle",
                                port: Some("value".to_owned()),
                                value: serde_json::Value::Bool(flag),
                                min: None,
                                max: None,
                                step: None,
                            }),
                        _ => None,
                    };
                }
            } else {
                {
                    node.title = format!("unknown node `{}`", call.func.name);
                    node.category = String::new();
                    for kwarg in &call.kwargs {
                        let lift = kwarg.value.each_depth();
                        let wired = wire_for(&kwarg.name.name, &kwarg.value, lift);
                        let span = kwarg.value.span();
                        node.inputs.push(InputView {
                            name: kwarg.name.name.clone(),
                            ty: "?".to_owned(),
                            base: "?".to_owned(),
                            depth: 0,
                            optional: false,
                            required: false,
                            default: None,
                            default_value: None,
                            doc: String::new(),
                            dimension: None,
                            wired,
                            literal: match kwarg.value.unlifted() {
                                ValueExpr::Literal(literal) => {
                                    Some(literal.span.slice(raw).to_owned())
                                }
                                _ => None,
                            },
                            literal_value: None,
                            lift,
                            unknown: true,
                            span: Some([span.start, span.end]),
                        });
                    }
                    node.outputs.push(OutputView {
                        name: "out".to_owned(),
                        ty: "?".to_owned(),
                        resolved: None,
                        base: "?".to_owned(),
                        displayable: false,
                    });
                }
            }
        }
    }

    // Size in grid units: header row + port rows (+ a widget row).
    let rows = node.inputs.len().max(node.outputs.len()).max(1);
    let widget = usize::from(node.param.is_some());
    let longest_in = node.inputs.iter().map(|i| i.name.len()).max().unwrap_or(0);
    let longest_out = node.outputs.iter().map(|o| o.name.len()).max().unwrap_or(0);
    let label_units = (longest_in + longest_out + 4).div_ceil(3);
    let title_units = (node.name.len() + node.func.as_ref().map_or(0, |f| f.len() + 3)).div_ceil(3);
    #[allow(clippy::cast_possible_truncation)]
    let width = (label_units.max(title_units) as u32).clamp(NODE_WIDTH, 24);
    #[allow(clippy::cast_possible_truncation)]
    let height = (1 + rows + widget) as u32;
    node.size = [width, height];
    node
}

fn expr_text(expr: &Expr, raw: &str) -> String {
    let span = match expr {
        Expr::Number { span, .. } | Expr::Neg { span, .. } | Expr::Binary { span, .. } => *span,
        Expr::Var(ident) => ident.span,
    };
    span.slice(raw).to_owned()
}

/// A diagnostic-kind name for the wire-probe verdicts.
#[must_use]
pub fn kind_name(kind: DiagnosticKind) -> &'static str {
    match kind {
        DiagnosticKind::ParseError => "parse_error",
        DiagnosticKind::MissingPragma => "missing_pragma",
        DiagnosticKind::FutureVersion => "future_version",
        DiagnosticKind::Rebinding => "rebinding",
        DiagnosticKind::UnknownName => "unknown_name",
        DiagnosticKind::UnknownNode => "unknown_node",
        DiagnosticKind::PositionalArgument => "positional_argument",
        DiagnosticKind::NestedCall => "nested_call",
        DiagnosticKind::MissingKwarg => "missing_kwarg",
        DiagnosticKind::UnknownKwarg => "unknown_kwarg",
        DiagnosticKind::TypeMismatch => "type_mismatch",
        DiagnosticKind::NeedsLift => "needs_lift",
        DiagnosticKind::NeedsAdapter => "needs_adapter",
        DiagnosticKind::EachOnScalar => "each_on_scalar",
        DiagnosticKind::ZipLengthMismatch => "zip_length_mismatch",
        DiagnosticKind::UnpackArity => "unpack_arity",
        DiagnosticKind::UnknownPort => "unknown_port",
        DiagnosticKind::Cycle => "cycle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile;
    use crate::lower::lower_partial;
    use crate::scripts::ScriptCancel;
    use cicada_core::config::ProjectConfig;
    use std::path::Path;

    fn view(source: &str) -> GraphView {
        let loaded = compile::load(Path::new("v.cic"), source, &ScriptCancel::new()).unwrap();
        let lowered = lower_partial(
            &loaded.document,
            &loaded.resolution,
            &loaded.specs,
            &ProjectConfig::default(),
            &loaded.scripts,
        )
        .unwrap();
        build(
            &loaded.document,
            &loaded.resolution,
            &loaded.specs,
            &lowered,
            &Sidecar::default(),
            &mut NodeRefs::default(),
        )
    }

    // v0.1 item 3 WP-B: a `Solid` output previews (the kind is in core's
    // GEOMETRY_KINDS, which is the one list this predicate reads), its
    // list and optional forms included; the mesh tier's solid stays
    // displayable on its own terms; scalars do not draw.
    #[test]
    fn a_solid_is_a_displayable_kind() {
        assert!(displayable_kind("Solid"));
        assert!(displayable_kind("Watertight<Mesh>"));
        assert!(displayable_kind("Geometry"));
        assert!(!displayable_kind("Number"));
        assert!(!displayable_kind("Xform"));
    }

    #[test]
    fn nodes_wires_params_and_previews() {
        let g = view(
            "# cicada 1\n\
             # A note for size.\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n\
             n = 40\n\
             twice = n * 2\n",
        );
        assert_eq!(g.nodes.len(), 5);
        let size = g.node("size").unwrap();
        assert_eq!(size.kind, NodeKind::Call);
        assert_eq!(size.comment.as_deref(), Some("A note for size."));
        let param = size.param.as_ref().unwrap();
        assert_eq!(
            (param.kind, param.min, param.max),
            ("slider", Some(0.5), Some(5.0))
        );
        assert!(!size.preview, "a Number is not displayable");
        let block = g.node("block").unwrap();
        assert!(block.preview, "geometry previews by default");
        assert_eq!(
            block.outputs[0].resolved.as_deref(),
            Some("Solid"),
            "the B-rep box (WP-C): a Solid output"
        );
        assert!(
            block
                .inputs
                .iter()
                .filter(|input| input.name != "plane")
                .all(|input| input.wired.is_some())
        );
        assert!(
            block
                .inputs
                .iter()
                .any(|input| input.name == "plane" && input.wired.is_none())
        );
        let n = g.node("n").unwrap();
        assert_eq!(n.param.as_ref().unwrap().kind, "integer");
        let twice = g.node("twice").unwrap();
        assert_eq!(twice.kind, NodeKind::Expression);
        assert_eq!(twice.inputs[0].name, "n");
        assert_eq!(twice.outputs[0].resolved.as_deref(), Some("Integer"));
        let wire = g.wires.iter().find(|w| w.to.node == "span").unwrap();
        assert_eq!(
            wire.from,
            WireEnd {
                node: "size".into(),
                port: "out".into()
            }
        );
        assert_eq!(wire.ty.as_deref(), Some("Number"));
        assert!(!wire.red);
        // Layout: size and n are sources (column 0), stacked; block is deepest.
        assert_eq!(size.cell[0], 0);
        assert!(block.cell[0] > g.node("span").unwrap().cell[0]);
        assert!(g.nodes.iter().all(|node| node.node_ref > 0));
    }

    // Wave 4 B3 (finding U9): an unconnected literal-typed port's chip
    // starts from the catalog default when the text carries no kwarg, so
    // the view-model parses the default it already renders — in the
    // port's kind, the macro's Rust booleans included — and says nothing
    // for a default that is no scalar literal.
    #[test]
    fn catalog_defaults_ride_the_inputs_as_values() {
        let g = view(
            "# cicada 1\n\
             t = text_outlines(text=\"hi\", size=2.0)\n\
             l = loft(profiles=t)\n\
             c = cycle()\n\
             d = construct_domain()\n",
        );
        let input = |node: &str, port: &str| -> InputView {
            g.node(node)
                .unwrap()
                .inputs
                .iter()
                .find(|i| i.name == port)
                .cloned()
                .unwrap()
        };
        let font = input("t", "font");
        assert_eq!(font.default.as_deref(), Some("\"DejaVu Sans Bold\""));
        assert_eq!(
            font.default_value,
            Some(serde_json::json!("DejaVu Sans Bold"))
        );
        assert_eq!(
            input("t", "segments").default_value,
            Some(serde_json::json!(8))
        );
        assert_eq!(
            input("t", "line_gap").default_value,
            Some(serde_json::json!(1.35))
        );
        let plane = input("t", "plane");
        assert_eq!(plane.default.as_deref(), Some("xy_plane"));
        assert_eq!(
            plane.default_value, None,
            "a default_doc rendering is no value"
        );
        let ruled = input("l", "ruled");
        assert_eq!(
            ruled.default.as_deref(),
            Some("true"),
            "the macro's Rust spelling"
        );
        assert_eq!(ruled.default_value, Some(serde_json::Value::Bool(true)));
        assert_eq!(
            input("c", "period").default_value,
            Some(serde_json::json!(4.0))
        );
        let end = input("d", "end");
        assert!(end.required && end.default.is_none() && end.default_value.is_none());
        assert!(
            end.literal.is_none() && end.wired.is_none(),
            "placed bare: nothing yet"
        );
    }

    #[test]
    fn default_json_reads_the_catalog_spelling_in_the_ports_kind() {
        use serde_json::json;
        assert_eq!(default_json("Boolean", 0, "true"), Some(json!(true)));
        assert_eq!(default_json("Boolean", 0, "False"), Some(json!(false)));
        assert_eq!(default_json("Integer", 0, "8"), Some(json!(8)));
        assert_eq!(default_json("Integer", 0, "-3"), Some(json!(-3)));
        assert_eq!(default_json("Number", 0, "4.0"), Some(json!(4.0)));
        assert_eq!(
            default_json("Number", 0, "1"),
            Some(json!(1)),
            "an integer spelling fits a Number"
        );
        assert_eq!(default_json("Text", 0, "\"a b\""), Some(json!("a b")));
        assert_eq!(
            default_json("Text", 0, "\"q\\\"uote\""),
            Some(json!("q\"uote"))
        );
        // Not a scalar literal of the kind: the chip shows the rendering, starts empty.
        assert_eq!(default_json("Plane", 0, "xy_plane"), None);
        assert_eq!(default_json("Integer", 0, "2.5"), None);
        assert_eq!(
            default_json("Text", 0, "hi"),
            None,
            "an unquoted name is a reference"
        );
        assert_eq!(default_json("Boolean", 0, "1"), None);
        assert_eq!(
            default_json("Number", 1, "1.0"),
            None,
            "a list port has no chip"
        );
        assert_eq!(default_json("Number", 0, "[1.0]"), None);
    }

    #[test]
    fn red_wires_exclusions_and_broken_lines() {
        let g = view(
            "# cicada 1\n\
             pts = construct_point(x=1.0)\n\
             seg = line(a=pts, b=\"nope\")\n\
             dir = unit_x()\n\
             later = move(geometry=seg, motion=dir)\n\
             ??? = 1\n\
             lonely = add(a=ghost, b=1.0)\n",
        );
        let bad = g
            .wires
            .iter()
            .find(|w| w.to.node == "seg" && w.to.port == "a")
            .unwrap();
        assert!(!bad.red, "Point into a Point port is fine");
        let seg = g.node("seg").unwrap();
        assert!(!seg.diagnostics.is_empty(), "\"nope\" into a Point port");
        assert_eq!(seg.excluded.as_ref().unwrap().status, "red");
        let later = g.node("later").unwrap();
        assert_eq!(later.excluded.as_ref().unwrap().status, "blocked");
        assert!(later.excluded.as_ref().unwrap().reason.contains("seg"));
        let broken = g.nodes.iter().find(|n| n.kind == NodeKind::Broken).unwrap();
        assert!(
            !broken.diagnostics.is_empty(),
            "the parse diagnostic attaches by line"
        );
        let lonely = g.node("lonely").unwrap();
        let ghost_wire = g.wires.iter().find(|w| w.to.node == "lonely").unwrap();
        assert!(ghost_wire.red, "unknown-name reference is a red wire");
        assert_eq!(lonely.excluded.as_ref().unwrap().status, "red");
    }

    #[test]
    fn multi_output_and_lift_badges() {
        let g = view(
            "# cicada 1\n\
             c = circle(radius=2.0)\n\
             d = divide_curve(curve=c, count=8)\n\
             up = unit_z()\n\
             moved = move(geometry=each(d.points), motion=up)\n",
        );
        let d = g.node("d").unwrap();
        assert_eq!(d.outputs.len(), 3);
        assert_eq!(d.outputs[0].resolved.as_deref(), Some("[Point]"));
        let moved = g.node("moved").unwrap();
        let geometry = moved.inputs.iter().find(|i| i.name == "geometry").unwrap();
        assert_eq!(geometry.lift, 1);
        assert_eq!(
            geometry.wired,
            Some(WireEnd {
                node: "d".into(),
                port: "points".into()
            })
        );
        assert_eq!(moved.outputs[0].resolved.as_deref(), Some("[Point]"));
        assert!(moved.preview);
        let wire = g.wires.iter().find(|w| w.to.node == "moved").unwrap();
        assert_eq!((wire.lift, wire.depth), (1, 1));
    }

    // A generic output port of a multi-output node renders the kind its
    // variable bound to in THAT call (the checker's resolution), while the
    // declared notation stays `[E]`; the wire it feeds carries the same.
    #[test]
    fn generic_output_ports_render_resolved_kinds() {
        let g = view(
            "# cicada 1\n\
             c = circle(radius=2.0)\n\
             d = divide_curve(curve=c, count=4)\n\
             culled = cull(list=d.points, pattern=[True, False, True, False])\n\
             first = item(list=culled.kept, index=0)\n",
        );
        let culled = g.node("culled").unwrap();
        let kept = culled.outputs.iter().find(|o| o.name == "kept").unwrap();
        assert_eq!(kept.ty, "[E]", "declared notation stays generic");
        assert_eq!(kept.resolved.as_deref(), Some("[Point]"));
        assert_eq!(kept.base, "Point");
        assert!(kept.displayable, "a resolved Point list previews");
        let map = culled.outputs.iter().find(|o| o.name == "map").unwrap();
        assert_eq!(map.resolved.as_deref(), Some("IndexMap"));
        let wire = g.wires.iter().find(|w| w.to.node == "first").unwrap();
        assert_eq!(wire.ty.as_deref(), Some("[Point]"));
        assert!(!wire.red);
        let first = g.node("first").unwrap();
        assert_eq!(first.outputs[0].resolved.as_deref(), Some("Point"));
    }

    fn view_with_sidecar(source: &str, sidecar: &Sidecar) -> GraphView {
        let loaded = compile::load(Path::new("v.cic"), source, &ScriptCancel::new()).unwrap();
        let lowered = lower_partial(
            &loaded.document,
            &loaded.resolution,
            &loaded.specs,
            &ProjectConfig::default(),
            &loaded.scripts,
        )
        .unwrap();
        build(
            &loaded.document,
            &loaded.resolution,
            &loaded.specs,
            &lowered,
            sidecar,
            &mut NodeRefs::default(),
        )
    }

    // Wave 4 B4: the sidecar's `collapsed` reaches the view — one unit
    // tall — for a slider whose bounds are literals; a slider with a wired
    // port of the collapsed row (value, min, max or step — a reference
    // under a lift is wired too), and any other node, are drawn expanded
    // whatever the sidecar says, and `collapse_refusal` — read off the
    // DOCUMENT, the one rule the session's `set_collapsed` refuses with —
    // names every wired port in spec order. A `#off` slider collapses like
    // any other (docs/16).
    #[test]
    fn collapsed_reaches_the_view_only_where_a_slider_can_collapse() {
        let source = "# cicada 1\n\
             n = 3.0\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             bound = slider(value=1.0, min=0.0, max=size)\n\
             both = slider(value=4.0, min=n, max=5.0, step=n)\n\
             driven = slider(value=n, min=0.0, max=10.0)\n\
             lifted = slider(value=1.0, min=0.0, max=each(n))\n\
             #off ghost = slider(value=1.0, min=0.0, max=2.0)\n\
             span = construct_domain(start=0.0, end=size)\n";
        let document = Document::parse(source);
        let plain = view_with_sidecar(source, &Sidecar::default());
        let size = plain.node("size").unwrap();
        assert!(!size.collapsed);
        assert_eq!(size.size[1], 6, "header + four port rows + the widget row");
        assert_eq!(
            collapse_refusal(&document, "size"),
            None,
            "a slider with literal bounds"
        );
        assert_eq!(
            collapse_refusal(&document, "ghost"),
            None,
            "a #off slider is a slider: it collapses like any other"
        );
        for (name, needle) in [
            ("bound", "`bound`: max is wired"),
            ("both", "`both`: min and step are wired"),
            ("driven", "`driven`: value is wired"),
            ("lifted", "`lifted`: max is wired"),
        ] {
            let reason = collapse_refusal(&document, name).unwrap();
            assert!(
                reason.contains(needle) && reason.contains("value, min, max and step are literals"),
                "{name}: {reason}"
            );
            assert!(
                plain
                    .node(name)
                    .unwrap()
                    .inputs
                    .iter()
                    .any(|input| input.wired.is_some()),
                "{name}: the view agrees a port is wired"
            );
        }
        for name in ["span", "n", "nope"] {
            let reason = collapse_refusal(&document, name).unwrap();
            assert!(
                reason.contains(&format!("`{name}` is not a slider")),
                "{name}: {reason}"
            );
        }
        let json = serde_json::to_value(size).unwrap();
        assert!(json.get("collapsed").is_none(), "false is omitted: {json}");

        let mut sidecar = Sidecar::default();
        for name in ["size", "bound", "both", "driven", "lifted", "ghost", "span"] {
            sidecar.set_collapsed(name, Some(true));
        }
        let g = view_with_sidecar(source, &sidecar);
        let size = g.node("size").unwrap();
        assert!(size.collapsed);
        assert_eq!(
            size.size,
            [plain.node("size").unwrap().size[0], 1],
            "one grid unit tall, same width"
        );
        assert_eq!(serde_json::to_value(size).unwrap()["collapsed"], true);
        let ghost = g.node("ghost").unwrap();
        assert!(
            matches!(ghost.kind, NodeKind::Disabled) && ghost.collapsed && ghost.size[1] == 1,
            "a disabled slider collapses: {ghost:?}"
        );
        for name in ["bound", "both", "driven", "lifted", "span"] {
            let node = g.node(name).unwrap();
            assert!(
                !node.collapsed,
                "{name}: drawn expanded whatever the sidecar says"
            );
            assert_eq!(
                node.size[1],
                plain.node(name).unwrap().size[1],
                "{name}: full height"
            );
        }
    }
}
