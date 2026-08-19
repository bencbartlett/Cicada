//! The checker-lite (doc 15 stage 2): kind lattice + lists + `each()`
//! pairing + the two spike refinements, over a parsed document and a node
//! catalog. Milliseconds, no geometry — the AI's inner loop (docs/11).
//!
//! Every problem is a typed [`Diagnostic`] with expected/actual types and a
//! suggested fix where one exists ("wrap in `each()`", "insert
//! `as_closed`"). A red statement never cascades: downstream of an errored
//! binding stays quiet about that error, and a broken statement reds one
//! node plus honest "fed by" messages, never the file.
//!
//! Resolution is **iterative** (Kahn's algorithm over the binding
//! dependency graph): forward references of any length are legal (docs/10)
//! and must never overflow the stack.

use std::collections::{BTreeSet, HashMap};

use cicada_core::geometry::{GEOMETRY_KINDS, TRANSFORMABLE_KINDS, VAR_ELEMENT, VAR_TRANSFORMABLE};
use cicada_core::spec::{NodeSpec, PortSpec, PortType};

use crate::ast::{BinOp, Expr, Kwarg, Lit, LitWithSpan, PortRef, Rhs, Statement, ValueExpr};
use crate::diag::{Diagnostic, DiagnosticKind};
use crate::document::{Document, Line, line_span_to_file};

/// Implicit widening upcasts (doc 02): `(from, to)` pairs.
const WIDENINGS: &[(&str, &str)] = &[("Integer", "Number")];

/// A type variable's constraint (stage 4's kind-preserving generics,
/// DECISIONS.md row 22): every occurrence of a variable in one call binds
/// to ONE kind, and variable-typed outputs carry it.
enum VarConstraint {
    /// `T`: only the listed kinds may bind.
    Kinds(&'static [&'static str]),
    /// `E`: any kind binds.
    Unconstrained,
}

impl VarConstraint {
    fn allows(&self, base: &str) -> bool {
        match self {
            Self::Kinds(kinds) => kinds.contains(&base),
            Self::Unconstrained => true,
        }
    }
}

/// Is `base` a per-call type variable, and what may bind it?
///
/// Known limitation (accepted for the spike): variables bind BASE KINDS
/// only — `[[Number]]` into `item`'s `[E]` port cannot bind `E =
/// [Number]`, so it diagnoses as a lift offer instead; `each()` over the
/// outer axis is the sanctioned (and semantically sound) route. Depth-
/// carrying variables arrive if a v0.1 node needs them.
fn type_var(base: &str) -> Option<VarConstraint> {
    if base == VAR_TRANSFORMABLE {
        Some(VarConstraint::Kinds(TRANSFORMABLE_KINDS))
    } else if base == VAR_ELEMENT {
        Some(VarConstraint::Unconstrained)
    } else {
        None
    }
}

/// The two spike refinements (doc 15): `(from, refined, adapter node)`.
/// Wiring `from` into a `refined` port offers the validating adapter;
/// dropping a refinement (`refined` into a `from` port) is a free total
/// upcast (doc 02).
const REFINEMENTS: &[(&str, &str, &str)] = &[
    ("Curve", "Closed<Curve>", "as_closed"),
    ("Mesh", "Watertight<Mesh>", "as_watertight"),
];

/// The unknown/element-free kind: an empty list literal's element type,
/// and the element type of catch-all ports (`[Any]`, docs/08 Panel).
/// Unifies with everything.
const ANY: &str = "Any";

/// Base-kind compatibility: exact, an implicit widening upcast, dropping a
/// refinement wrapper (a `Closed<Curve>` IS a `Curve`), widening into the
/// display-sink `Geometry` kind, or `Any` on either side. Public: the
/// stage-5 canvas needs live wire compatibility.
#[must_use]
pub fn compatible(from: &str, to: &str) -> bool {
    from == to
        || from == ANY
        || to == ANY
        || WIDENINGS.iter().any(|(f, t)| *f == from && *t == to)
        || REFINEMENTS
            .iter()
            .any(|(base, refined, _)| *refined == from && *base == to)
        || (to == "Geometry" && GEOMETRY_KINDS.contains(&from))
}

/// The node catalog the checker resolves calls against. Built from
/// [`NodeSpec`]s — the registry in production, hand-built specs in tests.
pub struct Catalog<'a> {
    by_name: HashMap<&'a str, &'a NodeSpec>,
}

impl<'a> Catalog<'a> {
    /// Index the specs by dialect name.
    ///
    /// # Panics
    ///
    /// Panics on duplicate dialect names — the same loud refusal as the
    /// registry's `registered()`; a silent shadow would misresolve calls
    /// (docs/10 §5: collisions are errors).
    #[must_use]
    pub fn new(specs: &'a [&'a NodeSpec]) -> Self {
        let mut by_name = HashMap::new();
        for spec in specs {
            assert!(
                by_name.insert(spec.name, *spec).is_none(),
                "duplicate node name `{}` in catalog",
                spec.name
            );
        }
        Self { by_name }
    }

    fn get(&self, name: &str) -> Option<&'a NodeSpec> {
        self.by_name.get(name).copied()
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().copied()
    }
}

/// An inferred wire type: base kind + list depth + element optionality,
/// plus the top-level length when statically known (literal lists) — the
/// checker's zip counts come from these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireType {
    /// Base kind name (`Number`, `Curve`, …).
    pub base: String,
    /// List nesting depth.
    pub depth: u8,
    /// Element-level optionality.
    pub optional: bool,
    /// Statically known top-level element count.
    pub len: Option<usize>,
}

impl WireType {
    fn scalar(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            depth: 0,
            optional: false,
            len: None,
        }
    }

    /// The wire type a catalog port carries.
    #[must_use]
    pub fn from_port(port: &PortType) -> Self {
        Self {
            base: port.base.to_owned(),
            depth: port.list_depth,
            optional: port.optional,
            len: None,
        }
    }

    /// Catalog notation (`[Number]`, `Curve?`).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for _ in 0..self.depth {
            out.push('[');
        }
        out.push_str(&self.base);
        if self.optional {
            out.push('?');
        }
        for _ in 0..self.depth {
            out.push(']');
        }
        out
    }

    fn stripped(&self, levels: u8) -> Self {
        Self {
            base: self.base.clone(),
            depth: self.depth - levels,
            optional: self.optional,
            len: None,
        }
    }

    fn lifted(&self, levels: u8) -> Self {
        Self {
            base: self.base.clone(),
            depth: self.depth.saturating_add(levels),
            optional: self.optional,
            len: None,
        }
    }
}

/// What a name is bound to — the checker's resolved view of one binding
/// (the public shape; see [`Resolution`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingType {
    /// A single value.
    Value {
        /// The wire type.
        ty: WireType,
        /// True when this binds a single-output node call (so `.out`
        /// selection is legal per the ABI's port naming).
        from_single_out: bool,
    },
    /// A whole multi-output node — references select ports (`d.points`).
    Node {
        /// The node's dialect name.
        node: String,
        /// The call's lift depth.
        lift: u8,
    },
    /// Errored — downstream stays quiet about it.
    Poisoned,
}

/// The checker's resolved output: diagnostics plus each binding's type —
/// what the canvas (live wire compatibility), the AI read-tools
/// (`wire_type`, docs/11), and the scheduler's blocked-set (docs/12) build
/// on. Stage-2 shape: binding types only; edges and cones arrive with
/// their consumers.
#[derive(Debug, Clone)]
pub struct Resolution {
    /// Parse + semantic diagnostics, sorted by position.
    pub diagnostics: Vec<Diagnostic>,
    /// Each binding's resolved type (unpack targets included).
    pub bindings: HashMap<String, BindingType>,
}

/// Full resolution: diagnostics + binding types.
#[must_use]
pub fn resolve(document: &Document, catalog: &Catalog<'_>) -> Resolution {
    let (mut semantic, bindings) = Checker::new(document, catalog).run();
    let mut diagnostics = document.parse_diagnostics();
    diagnostics.append(&mut semantic);
    diagnostics.sort_by_key(|d| (d.span.line, d.span.col_start));
    Resolution {
        diagnostics,
        bindings,
    }
}

/// Parse + semantic diagnostics, sorted by position — the one call most
/// consumers want.
#[must_use]
pub fn diagnostics(document: &Document, catalog: &Catalog<'_>) -> Vec<Diagnostic> {
    resolve(document, catalog).diagnostics
}

/// Semantic diagnostics only (the checker proper).
#[must_use]
pub fn check(document: &Document, catalog: &Catalog<'_>) -> Vec<Diagnostic> {
    Checker::new(document, catalog).run().0
}

/// Internal binding state (borrows the spec).
#[derive(Debug, Clone)]
enum BindingKind<'a> {
    Value { ty: WireType, from_single_out: bool },
    Node { spec: &'a NodeSpec, lift: u8 },
    Poisoned,
}

struct Checker<'a> {
    document: &'a Document,
    catalog: &'a Catalog<'a>,
    /// name → defining line (first definition wins; rest are rebinding).
    definitions: HashMap<&'a str, usize>,
    /// Names of statements that failed to parse (for honest messages).
    broken: Vec<&'a str>,
    /// Names of `#off`-disabled bindings (for honest messages).
    disabled: Vec<&'a str>,
    /// line → resolved bindings of that statement's targets.
    resolved: HashMap<usize, HashMap<String, BindingKind<'a>>>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    fn new(document: &'a Document, catalog: &'a Catalog<'a>) -> Self {
        Self {
            document,
            catalog,
            definitions: HashMap::new(),
            broken: Vec::new(),
            disabled: Vec::new(),
            resolved: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn run(mut self) -> (Vec<Diagnostic>, HashMap<String, BindingType>) {
        self.collect_definitions();
        self.resolve_all();
        self.diagnostics
            .sort_by_key(|d| (d.span.line, d.span.col_start));

        let mut bindings = HashMap::new();
        for per_line in self.resolved.values() {
            for (name, kind) in per_line {
                bindings.insert(
                    name.clone(),
                    match kind {
                        BindingKind::Value {
                            ty,
                            from_single_out,
                        } => BindingType::Value {
                            ty: ty.clone(),
                            from_single_out: *from_single_out,
                        },
                        BindingKind::Node { spec, lift } => BindingType::Node {
                            node: spec.name.to_owned(),
                            lift: *lift,
                        },
                        BindingKind::Poisoned => BindingType::Poisoned,
                    },
                );
            }
        }
        (self.diagnostics, bindings)
    }

    /// Pass 1: definitions, rebinding, broken and disabled names.
    fn collect_definitions(&mut self) {
        for (line, statement, _) in self.document.statements() {
            for target in &statement.targets {
                if self.definitions.contains_key(target.name.as_str()) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            DiagnosticKind::Rebinding,
                            line_span_to_file(line, target.span),
                            format!(
                                "`{}` is already bound — names bind once per file",
                                target.name
                            ),
                        )
                        .with_node(target.name.clone()),
                    );
                } else {
                    self.definitions.insert(&target.name, line);
                }
            }
        }
        for line in self.document.lines() {
            match line {
                Line::Broken {
                    node: Some(node), ..
                } => self.broken.push(node),
                Line::Disabled {
                    name: Some(name), ..
                } => self.disabled.push(name),
                _ => {}
            }
        }
    }

    /// Pass 2: Kahn's algorithm over statement dependencies — iterative,
    /// deterministic (ready set drained in line order), cycles reported
    /// once each then poisoned so their dependents resolve quietly.
    fn resolve_all(&mut self) {
        let mut deps: HashMap<usize, BTreeSet<usize>> = HashMap::new();
        let mut dependents: HashMap<usize, Vec<usize>> = HashMap::new();
        for (line, statement, _) in self.document.statements() {
            let mut lines = BTreeSet::new();
            for reference in statement.references() {
                // Self-edges included: `x = x + 1` is a length-1 cycle and
                // must earn the SAME Cycle diagnostic as `a → b → a` — an
                // excluded self-edge made it resolve silently and surface
                // downstream as an internal lowering error (regression:
                // adversarial review, stage 3).
                if let Some(&definition) = self.definitions.get(reference.name.as_str()) {
                    lines.insert(definition);
                }
            }
            for &definition in &lines {
                dependents.entry(definition).or_default().push(line);
            }
            deps.insert(line, lines);
        }

        let mut pending: HashMap<usize, usize> = deps.iter().map(|(&l, d)| (l, d.len())).collect();
        let mut ready: BTreeSet<usize> = pending
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(&l, _)| l)
            .collect();

        loop {
            while let Some(&line) = ready.iter().next() {
                ready.remove(&line);
                if self.resolved.contains_key(&line) {
                    continue;
                }
                let Some(statement) = self.statement_at(line) else {
                    continue;
                };
                let bindings = self.check_statement(line, statement);
                self.resolved.insert(line, bindings);
                self.release_dependents(line, &dependents, &mut pending, &mut ready);
            }

            // Leftovers are cycle members (or blocked behind one). Report
            // ONE cycle, poison its members, keep draining — dependents
            // then resolve with quiet poisoned inputs.
            let Some(&start) = deps.keys().filter(|l| !self.resolved.contains_key(l)).min() else {
                break;
            };
            let members = self.trace_cycle(start, &deps);
            self.report_cycle(&members);
            for &line in &members {
                let targets = self
                    .statement_at(line)
                    .map(|s| s.targets.iter().map(|t| t.name.clone()).collect::<Vec<_>>())
                    .unwrap_or_default();
                self.resolved.insert(
                    line,
                    targets
                        .into_iter()
                        .map(|name| (name, BindingKind::Poisoned))
                        .collect(),
                );
                self.release_dependents(line, &dependents, &mut pending, &mut ready);
            }
        }
    }

    fn release_dependents(
        &self,
        line: usize,
        dependents: &HashMap<usize, Vec<usize>>,
        pending: &mut HashMap<usize, usize>,
        ready: &mut BTreeSet<usize>,
    ) {
        for &dependent in dependents.get(&line).into_iter().flatten() {
            if let Some(count) = pending.get_mut(&dependent) {
                *count = count.saturating_sub(1);
                if *count == 0 && !self.resolved.contains_key(&dependent) {
                    ready.insert(dependent);
                }
            }
        }
    }

    /// Walk unresolved dependency edges from `start` until a line repeats;
    /// the loop from its first occurrence is the cycle.
    fn trace_cycle(&self, start: usize, deps: &HashMap<usize, BTreeSet<usize>>) -> Vec<usize> {
        let mut path = vec![start];
        let mut current = start;
        loop {
            let next = deps
                .get(&current)
                .and_then(|d| d.iter().find(|l| !self.resolved.contains_key(l)))
                .copied();
            let Some(next) = next else {
                // A leftover always has an unresolved dep; never loop
                // forever on a logic slip.
                return path;
            };
            if let Some(position) = path.iter().position(|&l| l == next) {
                return path[position..].to_vec();
            }
            path.push(next);
            current = next;
        }
    }

    fn report_cycle(&mut self, members: &[usize]) {
        let names: Vec<String> = members
            .iter()
            .filter_map(|&l| self.statement_at(l).map(|s| s.name().to_owned()))
            .collect();
        let Some(&first) = members.first() else {
            return;
        };
        let Some(statement) = self.statement_at(first) else {
            return;
        };
        self.diagnostics.push(
            Diagnostic::new(
                DiagnosticKind::Cycle,
                line_span_to_file(first, statement.targets[0].span),
                format!(
                    "cycle — the graph must be a DAG: {} → {}",
                    names.join(" → "),
                    names.first().map_or("?", String::as_str),
                ),
            )
            .with_node(statement.name().to_owned()),
        );
    }

    fn statement_at(&self, line: usize) -> Option<&'a Statement> {
        match &self.document.lines()[line] {
            Line::Statement { statement, .. } => Some(statement),
            _ => None,
        }
    }

    /// Type and check one statement; returns its targets' bindings.
    fn check_statement(
        &mut self,
        line: usize,
        statement: &'a Statement,
    ) -> HashMap<String, BindingKind<'a>> {
        let node = statement.name().to_owned();
        match &statement.rhs {
            Rhs::Literal(lit) => {
                let kind =
                    self.literal_type(line, &node, lit)
                        .map_or(BindingKind::Poisoned, |ty| BindingKind::Value {
                            ty,
                            from_single_out: false,
                        });
                HashMap::from([(node, kind)])
            }
            Rhs::Expression(expr) => {
                let ty = self.check_expression(line, &node, expr);
                HashMap::from([(
                    node,
                    BindingKind::Value {
                        ty,
                        from_single_out: false,
                    },
                )])
            }
            Rhs::Call(call) => self.check_call(line, statement, call),
        }
    }

    // ------------------------------------------------------- literals --

    fn literal_type(&mut self, line: usize, node: &str, lit: &LitWithSpan) -> Option<WireType> {
        match &lit.lit {
            Lit::Number { integer, .. } => Some(WireType::scalar(if *integer {
                "Integer"
            } else {
                "Number"
            })),
            Lit::Text(_) => Some(WireType::scalar("Text")),
            Lit::Boolean(_) => Some(WireType::scalar("Boolean")),
            Lit::List(items) => {
                let mut base: Option<&str> = None;
                for item in items {
                    let item_base = match &item.lit {
                        Lit::Number { integer: true, .. } => "Integer",
                        Lit::Number { integer: false, .. } => "Number",
                        Lit::Text(_) => "Text",
                        Lit::Boolean(_) => "Boolean",
                        // Unreachable while the parser rejects nested
                        // lists (spike subset); kept for the day it stops.
                        Lit::List(_) => "List",
                    };
                    match base {
                        None => base = Some(item_base),
                        Some(seen)
                            if compatible(item_base, seen) || compatible(seen, item_base) =>
                        {
                            if compatible(seen, item_base) {
                                base = Some(item_base);
                            }
                        }
                        Some(seen) => {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    DiagnosticKind::TypeMismatch,
                                    line_span_to_file(line, item.span),
                                    "lists are homogeneous — every element has one type",
                                )
                                .with_types(seen, item_base)
                                .with_node(node.to_owned()),
                            );
                            return None;
                        }
                    }
                }
                Some(WireType {
                    base: base.unwrap_or(ANY).to_owned(),
                    depth: 1,
                    optional: false,
                    len: Some(items.len()),
                })
            }
        }
    }

    // ---------------------------------------------------- expressions --

    /// Check an expression's inputs and infer its output type: Integer
    /// when every leaf is Integer and every op preserves integrality
    /// (`+ - *` and unary minus; `/` and `^` produce Number), else Number.
    fn check_expression(&mut self, line: usize, node: &str, expr: &'a Expr) -> WireType {
        let mut all_integer = expr_preserves_integer(expr);
        for var in expr.free_vars() {
            let Some(kind) = self.resolve_name(line, node, &var.name, var.span) else {
                all_integer = false;
                continue;
            };
            match kind {
                BindingKind::Poisoned => {
                    all_integer = false;
                }
                BindingKind::Node { spec, .. } => {
                    all_integer = false;
                    self.diagnostics.push(
                        Diagnostic::new(
                            DiagnosticKind::UnknownPort,
                            line_span_to_file(line, var.span),
                            format!(
                                "`{}` has outputs ({}) — expressions need a single number; \
                                 unpack the outputs first (`a, b = …`)",
                                var.name,
                                port_names(spec.outputs)
                            ),
                        )
                        .with_node(node.to_owned()),
                    );
                }
                BindingKind::Value { ty, .. } => {
                    let is_number = ty.depth == 0 && (ty.base == "Number" || ty.base == "Integer");
                    if ty.base != "Integer" {
                        all_integer = false;
                    }
                    if !is_number {
                        self.diagnostics.push(
                            Diagnostic::new(
                                DiagnosticKind::TypeMismatch,
                                line_span_to_file(line, var.span),
                                format!("expressions work on numbers; `{}` is not one", var.name),
                            )
                            .with_types("Number", ty.render())
                            .with_node(node.to_owned()),
                        );
                    }
                }
            }
        }
        WireType::scalar(if all_integer { "Integer" } else { "Number" })
    }

    // ----------------------------------------------------------- calls --

    fn check_call(
        &mut self,
        line: usize,
        statement: &'a Statement,
        call: &'a crate::ast::Call,
    ) -> HashMap<String, BindingKind<'a>> {
        let node = statement.name().to_owned();
        let poisoned = |targets: &[crate::ast::Ident]| {
            targets
                .iter()
                .map(|t| (t.name.clone(), BindingKind::Poisoned))
                .collect::<HashMap<_, _>>()
        };

        let Some(spec) = self.catalog.get(&call.func.name) else {
            let mut diagnostic = Diagnostic::new(
                DiagnosticKind::UnknownNode,
                line_span_to_file(line, call.func.span),
                format!("no node named `{}` in the catalog", call.func.name),
            )
            .with_node(node);
            if let Some(suggestion) = did_you_mean(&call.func.name, self.catalog.names()) {
                diagnostic =
                    diagnostic.with_fix(format!("did you mean `{suggestion}`?"), Some(suggestion));
            }
            self.diagnostics.push(diagnostic);
            return poisoned(&statement.targets);
        };

        let mut vars: HashMap<String, String> = HashMap::new();
        let lift = self.check_call_kwargs(line, &node, call, spec, &mut vars);

        // A variable-typed output carries the kind its variable bound to
        // in THIS call (kind-preserving); unbound (arguments missing or
        // poisoned — already diagnosed) degrades quietly to Any.
        let substitute = |mut ty: WireType| {
            if type_var(&ty.base).is_some() {
                ty.base = vars
                    .get(&ty.base)
                    .cloned()
                    .unwrap_or_else(|| ANY.to_owned());
            }
            ty
        };

        // Outputs → target bindings.
        let mut bindings = HashMap::new();
        if statement.targets.len() > 1 {
            if statement.targets.len() != spec.outputs.len() {
                self.diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::UnpackArity,
                        line_span_to_file(line, statement.targets[0].span),
                        format!(
                            "`{}` has {} {} ({}); {} names given",
                            call.func.name,
                            spec.outputs.len(),
                            plural(spec.outputs.len(), "output", "outputs"),
                            port_names(spec.outputs),
                            statement.targets.len()
                        ),
                    )
                    .with_node(node),
                );
                return poisoned(&statement.targets);
            }
            // Duplicate targets already earned a rebinding diagnostic;
            // poison them so downstream stays quiet instead of silently
            // taking the last output's type.
            let mut seen = BTreeSet::new();
            for (target, port) in statement.targets.iter().zip(spec.outputs) {
                let kind = if seen.insert(target.name.as_str()) {
                    BindingKind::Value {
                        ty: substitute(WireType::from_port(&port.ty)).lifted(lift),
                        from_single_out: false,
                    }
                } else {
                    BindingKind::Poisoned
                };
                bindings.insert(target.name.clone(), kind);
            }
        } else if let [single] = spec.outputs {
            bindings.insert(
                node,
                BindingKind::Value {
                    ty: substitute(WireType::from_port(&single.ty)).lifted(lift),
                    from_single_out: true,
                },
            );
        } else {
            // Multi-output or sink (zero outputs) — both bind the node.
            // NOTE: a multi-output node with variable-typed outputs would
            // lose its per-call binding here (port selection happens in a
            // later statement, outside this call's scope); no such node
            // exists — keep it that way until BindingKind::Node carries
            // substitutions.
            bindings.insert(node, BindingKind::Node { spec, lift });
        }
        bindings
    }

    /// Kwarg checks for one call: unknown names, missing required ports,
    /// value-vs-port typing, and strict-zip pairing. Returns the call's
    /// lift depth (the `each()` map/zip level).
    fn check_call_kwargs(
        &mut self,
        line: usize,
        node: &str,
        call: &'a crate::ast::Call,
        spec: &'a NodeSpec,
        vars: &mut HashMap<String, String>,
    ) -> u8 {
        // Unknown kwargs — remembering did-you-mean targets so a typo'd
        // required port isn't ALSO reported missing (one root cause, one
        // diagnostic).
        let mut suggested: Vec<String> = Vec::new();
        for kwarg in &call.kwargs {
            if !spec.inputs.iter().any(|p| p.name == kwarg.name.name) {
                let mut diagnostic = Diagnostic::new(
                    DiagnosticKind::UnknownKwarg,
                    line_span_to_file(line, kwarg.name.span),
                    format!(
                        "`{}` has no port `{}` (ports: {})",
                        call.func.name,
                        kwarg.name.name,
                        port_names(spec.inputs)
                    ),
                )
                .with_node(node);
                if let Some(suggestion) =
                    did_you_mean(&kwarg.name.name, spec.inputs.iter().map(|p| p.name))
                {
                    suggested.push(suggestion.clone());
                    diagnostic = diagnostic
                        .with_fix(format!("did you mean `{suggestion}`?"), Some(suggestion));
                }
                self.diagnostics.push(diagnostic);
            }
        }
        for port in spec.inputs {
            if port.default.is_none()
                && !call.kwargs.iter().any(|k| k.name.name == port.name)
                && !suggested.iter().any(|s| s == port.name)
            {
                self.diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::MissingKwarg,
                        line_span_to_file(line, call.func.span),
                        format!("required port `{}` is not wired", port.name),
                    )
                    .with_types(port.ty.render(), "nothing")
                    .with_node(node)
                    .with_fix(format!("add `{}=…`", port.name), None),
                );
            }
        }

        // Values: type each kwarg against its port; collect the lift.
        let mut lift_depths: Vec<(u8, &Kwarg)> = Vec::new();
        let mut zip_lens: Vec<(usize, &Kwarg)> = Vec::new();
        for kwarg in &call.kwargs {
            let Some(port) = spec.inputs.iter().find(|p| p.name == kwarg.name.name) else {
                continue; // already diagnosed
            };
            let each_depth = kwarg.value.each_depth();
            let Some(value_type) = self.value_type(line, node, kwarg.value.unlifted()) else {
                continue; // unresolved or poisoned — already diagnosed
            };
            if each_depth > 0 {
                if value_type.depth < each_depth {
                    self.diagnostics.push(
                        Diagnostic::new(
                            DiagnosticKind::EachOnScalar,
                            line_span_to_file(line, kwarg.value.span()),
                            format!(
                                "each() needs a list at depth {each_depth}; `{}` is {}",
                                kwarg_value_name(kwarg),
                                value_type.render()
                            ),
                        )
                        .with_node(node),
                    );
                    continue;
                }
                lift_depths.push((each_depth, kwarg));
                if let Some(len) = value_type.len {
                    zip_lens.push((len, kwarg));
                }
                let stripped = value_type.stripped(each_depth);
                self.check_port_fit(line, node, kwarg, port, &stripped, true, vars);
            } else {
                self.check_port_fit(line, node, kwarg, port, &value_type, false, vars);
            }
        }

        self.check_strict_zip(line, node, &lift_depths, &zip_lens);
        lift_depths.iter().map(|(d, _)| *d).max().unwrap_or(0)
    }

    /// Strict zip (docs/09): multiple `each()` on one call pair the same
    /// depth, and statically known lengths must agree — counts in the error.
    fn check_strict_zip(
        &mut self,
        line: usize,
        node: &str,
        lift_depths: &[(u8, &Kwarg)],
        zip_lens: &[(usize, &Kwarg)],
    ) {
        if lift_depths.len() >= 2 {
            let first_depth = lift_depths[0].0;
            for (depth, kwarg) in &lift_depths[1..] {
                if *depth != first_depth {
                    self.diagnostics.push(
                        Diagnostic::new(
                            DiagnosticKind::TypeMismatch,
                            line_span_to_file(line, kwarg.value.span()),
                            format!(
                                "each() depths differ ({first_depth} vs {depth}) — \
                                 zip pairs elements at one depth"
                            ),
                        )
                        .with_node(node),
                    );
                }
            }
            if let Some((first_len, _)) = zip_lens.first() {
                for (len, kwarg) in &zip_lens[1..] {
                    if len != first_len {
                        self.diagnostics.push(
                            Diagnostic::new(
                                DiagnosticKind::ZipLengthMismatch,
                                line_span_to_file(line, kwarg.value.span()),
                                format!(
                                    "zip is strict: {first_len} vs {len} elements — \
                                     pad_last / cycle / truncate are the opt-in policies"
                                ),
                            )
                            .with_node(node),
                        );
                    }
                }
            }
        }
    }

    /// Does `value` fit `port`? Emits the docs/09 ladder of diagnostics:
    /// ok → lift offer → adapter offer → mismatch. Type-variable ports
    /// (`T`, `E`) bind `vars` on their first fitting value; later
    /// occurrences in the same call must agree.
    #[allow(clippy::too_many_arguments)] // one coherent fit check; splitting obscures it
    fn check_port_fit(
        &mut self,
        line: usize,
        node: &str,
        kwarg: &Kwarg,
        port: &PortSpec,
        value: &WireType,
        already_lifted: bool,
        vars: &mut HashMap<String, String>,
    ) {
        let span = line_span_to_file(line, kwarg.value.span());
        // `Any` ports are display-sink catch-alls: they absorb any wire at
        // any depth, as-is (docs/08 Panel).
        if port.ty.base == ANY {
            return;
        }
        let Some((fits_base, bindable)) = self.var_base_fit(line, node, kwarg, port, value, vars)
        else {
            return; // constraint violation — already diagnosed
        };
        if fits_base && value.depth == port.ty.list_depth {
            if bindable {
                vars.insert(port.ty.base.to_owned(), value.base.clone());
            }
            if value.optional && !port.ty.optional {
                self.diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::TypeMismatch,
                        span,
                        format!(
                            "`{}` carries optional elements; `{}` wants them present — \
                             `compact` removes the holes (and returns the IndexMap)",
                            kwarg_value_name(kwarg),
                            port.name
                        ),
                    )
                    .with_types(port.ty.render(), value.render())
                    .with_node(node.to_owned()),
                );
            }
            return;
        }
        if fits_base && value.depth > port.ty.list_depth {
            // A (deeper) lift fixes it — offered whether or not some
            // each() is already present (then it's "N more levels").
            let levels = value.depth - port.ty.list_depth;
            let label = match (already_lifted, levels) {
                (false, 1) => "wrap in each()".to_owned(),
                (false, n) => format!("wrap in each() ×{n}"),
                (true, n) => format!("add each() ×{n} more"),
            };
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::NeedsLift,
                    span,
                    format!(
                        "{} into {} port — map over it?",
                        value.render(),
                        indefinite(&port.ty.render())
                    ),
                )
                .with_types(port.ty.render(), value.render())
                .with_node(node.to_owned())
                .with_fix(label, None),
            );
            return;
        }
        if value.depth == port.ty.list_depth
            && let Some((_, _, adapter)) = REFINEMENTS
                .iter()
                .find(|(from, refined, _)| *from == value.base && *refined == port.ty.base)
        {
            let how = if port.ty.list_depth > 0 {
                format!("insert `{adapter}` under each()")
            } else {
                format!("insert `{adapter}`")
            };
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::NeedsAdapter,
                    span,
                    format!(
                        "`{}` wants {} — a checked conversion; {how}",
                        port.name,
                        port.ty.render(),
                    ),
                )
                .with_types(port.ty.render(), value.render())
                .with_node(node.to_owned())
                .with_fix(how.clone(), None),
            );
            return;
        }
        self.diagnostics.push(
            Diagnostic::new(
                DiagnosticKind::TypeMismatch,
                span,
                format!(
                    "{} into {} port",
                    value.render(),
                    indefinite(&port.ty.render())
                ),
            )
            .with_types(port.ty.render(), value.render())
            .with_node(node.to_owned()),
        );
    }

    /// Base-kind fit for a port, type variables included: `Some((fits,
    /// bindable))`, or `None` after diagnosing a constraint violation.
    /// `bindable` = a fresh variable may bind to this value's base once
    /// depth also matches.
    fn var_base_fit(
        &mut self,
        line: usize,
        node: &str,
        kwarg: &Kwarg,
        port: &PortSpec,
        value: &WireType,
        vars: &HashMap<String, String>,
    ) -> Option<(bool, bool)> {
        let Some(constraint) = type_var(port.ty.base) else {
            return Some((compatible(&value.base, port.ty.base), false));
        };
        if value.base == ANY {
            return Some((true, false)); // unknowable element type — nothing to bind
        }
        if let Some(bound) = vars.get(port.ty.base) {
            return Some((bound == &value.base, false));
        }
        if constraint.allows(&value.base) {
            return Some((true, true));
        }
        self.diagnostics.push(
            Diagnostic::new(
                DiagnosticKind::TypeMismatch,
                line_span_to_file(line, kwarg.value.span()),
                format!(
                    "`{}` is kind-preserving over transformable kinds ({}); `{}` is not one",
                    port.name,
                    TRANSFORMABLE_KINDS.join(", "),
                    kwarg_value_name(kwarg)
                ),
            )
            .with_types(port.ty.render(), value.render())
            .with_node(node.to_owned()),
        );
        None
    }

    /// The type a kwarg value (already stripped of `each`) carries.
    fn value_type(&mut self, line: usize, node: &str, value: &'a ValueExpr) -> Option<WireType> {
        match value {
            ValueExpr::Literal(lit) => self.literal_type(line, node, lit),
            ValueExpr::Each { .. } => unreachable!("caller unlifts"),
            ValueExpr::Ref(port_ref) => self.ref_type(line, node, port_ref),
        }
    }

    fn ref_type(&mut self, line: usize, node: &str, port_ref: &'a PortRef) -> Option<WireType> {
        let kind = self.resolve_name(line, node, &port_ref.binding.name, port_ref.binding.span)?;
        match (kind, &port_ref.port) {
            (BindingKind::Poisoned, _) => None,
            (BindingKind::Value { ty, .. }, None) => Some(ty),
            (
                BindingKind::Value {
                    ty,
                    from_single_out,
                },
                Some(port),
            ) => {
                // The ABI names a single output `out` (DECISIONS.md), so
                // `s.out` on a single-output call is the bare reference.
                if from_single_out && port.name == "out" {
                    return Some(ty);
                }
                let message = if from_single_out {
                    format!(
                        "`{}` binds a single-output node — its only port is `out`; \
                         reference `{0}` directly",
                        port_ref.binding.name
                    )
                } else {
                    format!(
                        "`{}` is a single value — it has no port `{}` to select",
                        port_ref.binding.name, port.name
                    )
                };
                self.diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::UnknownPort,
                        line_span_to_file(line, port_ref.span),
                        message,
                    )
                    .with_node(node.to_owned())
                    .with_fix(
                        format!("reference `{}` directly", port_ref.binding.name),
                        Some(port_ref.binding.name.clone()),
                    ),
                );
                None
            }
            (BindingKind::Node { spec, lift }, Some(_)) => {
                self.node_port_type(line, node, port_ref, spec, lift)
            }
            (BindingKind::Node { spec, .. }, None) => {
                let message = if spec.outputs.is_empty() {
                    format!(
                        "`{}` is a sink — it produces nothing to reference",
                        port_ref.binding.name
                    )
                } else {
                    format!(
                        "`{}` has outputs ({}) — select one, e.g. `{}.{}`",
                        port_ref.binding.name,
                        port_names(spec.outputs),
                        port_ref.binding.name,
                        spec.outputs.first().map_or("out", |p| p.name)
                    )
                };
                self.diagnostics.push(
                    Diagnostic::new(
                        DiagnosticKind::UnknownPort,
                        line_span_to_file(line, port_ref.span),
                        message,
                    )
                    .with_node(node.to_owned()),
                );
                None
            }
        }
    }

    /// Port selection on a multi-output (or sink) node binding.
    fn node_port_type(
        &mut self,
        line: usize,
        node: &str,
        port_ref: &'a PortRef,
        spec: &'a NodeSpec,
        lift: u8,
    ) -> Option<WireType> {
        let port = port_ref.port.as_ref()?;
        if let Some(out) = spec.outputs.iter().find(|p| p.name == port.name) {
            return Some(WireType::from_port(&out.ty).lifted(lift));
        }
        if spec.outputs.is_empty() {
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::UnknownPort,
                    line_span_to_file(line, port_ref.span),
                    format!(
                        "`{}` is a sink — it produces nothing to reference",
                        port_ref.binding.name
                    ),
                )
                .with_node(node.to_owned()),
            );
            return None;
        }
        let mut diagnostic = Diagnostic::new(
            DiagnosticKind::UnknownPort,
            line_span_to_file(line, port.span),
            format!(
                "`{}` has no output `{}` (outputs: {})",
                port_ref.binding.name,
                port.name,
                port_names(spec.outputs)
            ),
        )
        .with_node(node.to_owned());
        if let Some(suggestion) = did_you_mean(&port.name, spec.outputs.iter().map(|p| p.name)) {
            diagnostic =
                diagnostic.with_fix(format!("did you mean `{suggestion}`?"), Some(suggestion));
        }
        self.diagnostics.push(diagnostic);
        None
    }

    /// Look up a referenced name. Kahn ordering guarantees defined names
    /// are already resolved; the failure paths are honest about WHY a
    /// name has no binding (disabled, broken, or truly unknown).
    fn resolve_name(
        &mut self,
        line: usize,
        node: &str,
        name: &str,
        span: crate::ast::LineSpan,
    ) -> Option<BindingKind<'a>> {
        if let Some(&definition) = self.definitions.get(name) {
            return self
                .resolved
                .get(&definition)
                .and_then(|bindings| bindings.get(name))
                .cloned();
        }
        let file_span = line_span_to_file(line, span);
        if self.disabled.contains(&name) {
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::UnknownName,
                    file_span,
                    format!("`{name}` is disabled (`#off`) — re-enable it to solve downstream"),
                )
                .with_node(node.to_owned()),
            );
            return None;
        }
        if self.broken.contains(&name) {
            self.diagnostics.push(
                Diagnostic::new(
                    DiagnosticKind::UnknownName,
                    file_span,
                    format!("`{name}` failed to parse — fix that statement first"),
                )
                .with_node(node.to_owned()),
            );
            return None;
        }
        let mut diagnostic = Diagnostic::new(
            DiagnosticKind::UnknownName,
            file_span,
            format!("nothing binds `{name}`"),
        )
        .with_node(node.to_owned());
        if let Some(suggestion) = did_you_mean(name, self.definitions.keys().copied()) {
            diagnostic =
                diagnostic.with_fix(format!("did you mean `{suggestion}`?"), Some(suggestion));
        }
        self.diagnostics.push(diagnostic);
        None
    }
}

// --------------------------------------------------------------- helpers --

/// Do this expression's LITERALS and OPERATORS preserve integrality?
/// (Variables are checked against their resolved types separately.)
fn expr_preserves_integer(expr: &Expr) -> bool {
    match expr {
        Expr::Number { integer, .. } => *integer,
        Expr::Var(_) => true,
        Expr::Neg { operand, .. } => expr_preserves_integer(operand),
        Expr::Binary { op, lhs, rhs, .. } => {
            matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul)
                && expr_preserves_integer(lhs)
                && expr_preserves_integer(rhs)
        }
    }
}

fn port_names(ports: &[PortSpec]) -> String {
    ports.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
}

fn kwarg_value_name(kwarg: &Kwarg) -> String {
    match kwarg.value.unlifted() {
        ValueExpr::Ref(port_ref) => port_ref.binding.name.clone(),
        _ => kwarg.name.name.clone(),
    }
}

fn plural<'w>(count: usize, one: &'w str, many: &'w str) -> &'w str {
    if count == 1 { one } else { many }
}

/// `an Integer`, `a Number`, `a [Point]` — English articles for the
/// AI-facing messages.
fn indefinite(noun: &str) -> String {
    let article = if noun
        .chars()
        .next()
        .is_some_and(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u'))
    {
        "an"
    } else {
        "a"
    };
    format!("{article} {noun}")
}

/// Smallest edit-distance candidate within 2 edits.
fn did_you_mean<'c>(input: &str, candidates: impl Iterator<Item = &'c str>) -> Option<String> {
    candidates
        .map(|candidate| (edit_distance(input, candidate), candidate))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, candidate)| (*distance, candidate.to_owned()))
        .map(|(_, candidate)| candidate.to_owned())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}
