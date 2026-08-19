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
    /// A `#off`-disabled statement (ghost).
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
            Line::Disabled { raw, name } => {
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
                        status: "red",
                        reason: "disabled (`#off`)".to_owned(),
                    }),
                    effectful: false,
                    preview: false,
                    cell: [0, 0],
                    size: [NODE_WIDTH, 2],
                    manual: false,
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

/// The type of `binding.port` on a multi-output node binding.
fn node_port_type(
    resolution: &Resolution,
    spec_by_name: &HashMap<&str, &'static NodeSpec>,
    binding: &str,
    port: &str,
) -> Option<WireType> {
    let Some(BindingType::Node { node, lift }) = resolution.bindings.get(binding) else {
        return None;
    };
    let spec = spec_by_name.get(node.as_str())?;
    let output = spec.outputs.iter().find(|output| output.name == port)?;
    let mut ty = WireType::from_port(&output.ty);
    ty.depth = ty.depth.saturating_add(*lift);
    Some(ty)
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
    let excluded = lowered.excluded.get(&name).map(|reason| ExcludedView {
        status: reason.status(),
        reason: match reason {
            Exclusion::Diagnostics => "has diagnostics".to_owned(),
            Exclusion::Disabled => "disabled (`#off`)".to_owned(),
            Exclusion::Lowering(message) => message.clone(),
            Exclusion::FedBy(upstream) => format!("fed by red `{upstream}`"),
        },
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
    };
    // A wire from a kwarg / free-var reference; red when a diagnostic
    // sits on it.
    let mut wire_for = |target_port: &str, value: &ValueExpr, lift: u8| -> Option<WireEnd> {
        let ValueExpr::Ref(port_ref) = value.unlifted() else {
            return None;
        };
        let (source_ty, source_port) = match &port_ref.port {
            Some(port) => (
                node_port_type(resolution, spec_by_name, &port_ref.binding.name, &port.name),
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
                                        node_port_type(
                                            resolution,
                                            spec_by_name,
                                            &end.node,
                                            &end.port,
                                        )
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
                            node_port_type(resolution, spec_by_name, &name, output.name)
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
            Some("Watertight<Mesh>")
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
}
