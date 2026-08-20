//! Lowering: a checked `.cic` document → an executable
//! [`SolveGraph`](cicada_sched::SolveGraph). Lives in the server (its
//! hydration path, doc 15 stage 5 — moved here from `cicada-cli`, which
//! still drives it for `cicada run`), not in `cicada-sched`, which stays
//! language-agnostic behind fake-node tests.
//!
//! Two entry points, one algorithm:
//!
//! - [`lower`] — the strict, target-scoped form `cicada run` uses: only the
//!   statements REACHABLE from the requested targets are lowered (docs/12:
//!   red cones are excluded from scheduling, everything else proceeds — so
//!   an unrelated broken statement never blocks a run). Callers gate on
//!   checker diagnostics for the same reachable set first; anything
//!   unresolved that survives to lowering is refused loudly.
//! - [`lower_partial`] — the live-session form: lowers every statement that
//!   CAN lower and reports the rest as excluded with the honest reason (its
//!   own diagnostic, a lowering refusal, or "fed by" an excluded upstream),
//!   so the canvas shows red + blocked cones while everything else solves.
//!
//! Cache-key material produced here (docs/12 §Cache keys): stdlib calls
//! key on the spec's dialect name + explicit semantic version; expression
//! nodes key on `expr` v1 plus a **normalized IR hash** — variables hash by
//! ordinal of first appearance, so renaming a binding never recomputes
//! (docs/10 round-trip discipline).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use cicada_core::config::ProjectConfig;
use cicada_core::hash::{KindTag, ValueHash, ValueHasher};
use cicada_core::spec::NodeSpec;
use cicada_core::value::{HashedValue, List, ValueData, ValueError};
use cicada_lang::ast::{BinOp, Expr, Lit, LitWithSpan, Rhs, Statement, ValueExpr};
use cicada_lang::check::{BindingType, Resolution};
use cicada_lang::diag::Diagnostic;
use cicada_lang::document::{Document, Line};
use cicada_sched::{Input, NodeDecl, NodeError, NodeFn, NodeId, SolveGraph};

use crate::scripts::ScriptNode;

/// Where a binding's value lives after lowering.
#[derive(Debug, Clone)]
pub enum LoweredBinding {
    /// A literal — content-addressed value, no node at all (constant param
    /// nodes are free; an edit changes the hash and dirties downstream).
    Value(Arc<HashedValue>),
    /// One output port of a node.
    Port {
        /// The node.
        node: NodeId,
        /// Output index.
        output: usize,
    },
    /// A whole multi-output node (references select ports).
    Node {
        /// The node.
        node: NodeId,
    },
}

/// A lowered pipeline: the graph plus name → binding and per-node output
/// port names (for display), and — for [`lower_partial`] — the bindings
/// that did NOT lower, each with its reason.
pub struct Lowered {
    /// The executable graph.
    pub graph: SolveGraph,
    /// Every lowered binding by name.
    pub bindings: HashMap<String, LoweredBinding>,
    /// Output port names per node, parallel to the graph's nodes.
    pub output_names: Vec<Vec<String>>,
    /// Bindings excluded from the graph, with why (ordered by name for
    /// deterministic reporting). Always empty from [`lower`].
    pub excluded: BTreeMap<String, Exclusion>,
}

/// Why a binding is not in the solve graph (docs/12: red cones are
/// excluded from scheduling; the reason is always spelled out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exclusion {
    /// The binding's own statement has checker/parse diagnostics.
    Diagnostics,
    /// The statement is `#off`-disabled (DECISIONS.md node-disable row).
    Disabled,
    /// Lowering itself refused (message from [`LowerError`]).
    Lowering(String),
    /// An upstream binding is excluded (or unknown); this one is blocked
    /// by it — the name is the culprit.
    FedBy(String),
}

impl Exclusion {
    /// The status-vocabulary word (docs/16): `red` or `blocked`.
    #[must_use]
    pub fn status(&self) -> &'static str {
        match self {
            Self::FedBy(_) => "blocked",
            Self::Diagnostics | Self::Disabled | Self::Lowering(_) => "red",
        }
    }

    /// The honest reason as a user reads it — the text the canvas shows on
    /// the node and `cicada mcp`'s `check` reports per excluded binding
    /// (one rendering, so the two never disagree).
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Diagnostics => "has diagnostics".to_owned(),
            Self::Disabled => "disabled (`#off`)".to_owned(),
            Self::Lowering(message) => message.clone(),
            Self::FedBy(upstream) => format!("fed by red `{upstream}`"),
        }
    }
}

/// Why lowering refused. Most cases are pre-empted by the checker gate;
/// each is loud rather than silently mis-lowered.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LowerError {
    /// Nested `each()` — refused until a node set needs it (v0.1).
    #[error(
        "`{node}`: nested each() (depth {depth}) is not yet executable — the S-tier set needs depth 1 only (nested lifts: v0.1)"
    )]
    EachDepth {
        /// The binding.
        node: String,
        /// The unsupported depth.
        depth: u8,
    },
    /// A name did not resolve to a lowerable binding (checker gate bug or
    /// caller skipped the gate).
    #[error("`{name}` is not lowerable — the checker must pass before running")]
    Unresolved {
        /// The name.
        name: String,
    },
    /// A catalog node without a registered invoker — a registry bug.
    #[error("node `{name}` has a spec but no invoker — registry bug")]
    NoInvoker {
        /// The dialect name.
        name: String,
    },
    /// An integer literal at or beyond 2^53 (the parser carries literals
    /// as f64; drifted digits must not become cache keys — and an f64 of
    /// exactly ±2^53 is indistinguishable from a drifted ±(2^53+1), so the
    /// boundary itself is refused).
    #[error("`{node}`: integer literal {value} is outside the exact range (|v| < 2^53)")]
    IntegerRange {
        /// The binding.
        node: String,
        /// The literal as parsed.
        value: f64,
    },
    /// A literal value refused construction.
    #[error("`{node}`: {message}")]
    BadLiteral {
        /// The binding.
        node: String,
        /// Why.
        message: String,
    },
    /// The requested target name binds nothing.
    #[error("no binding named `{name}` in the pipeline")]
    NoSuchTarget {
        /// The name.
        name: String,
    },
    /// Graph assembly failed (should be unreachable after checking).
    #[error("graph assembly: {0}")]
    Graph(#[from] cicada_sched::GraphError),
}

/// The statement set reachable from `targets` (names), or every statement
/// when `targets` is empty. Returns binding names in the closure — the set
/// the caller gates diagnostics on.
#[must_use]
pub fn reachable_bindings(document: &Document, targets: &[String]) -> HashSet<String> {
    let by_name: HashMap<&str, &Statement> = document
        .statements()
        .flat_map(|(_, statement, _)| {
            statement
                .targets
                .iter()
                .map(move |target| (target.name.as_str(), statement))
        })
        .collect();
    let mut queue: VecDeque<&str> = if targets.is_empty() {
        by_name.keys().copied().collect()
    } else {
        targets.iter().map(String::as_str).collect()
    };
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(name) = queue.pop_front() {
        if !seen.insert(name.to_owned()) {
            continue;
        }
        if let Some(statement) = by_name.get(name) {
            // The whole statement resolves together: all its targets and
            // everything it references join the closure.
            for target in &statement.targets {
                queue.push_back(&target.name);
            }
            for reference in statement.references() {
                queue.push_back(&reference.name);
            }
        }
    }
    seen
}

/// Lower the statements reachable from `targets` (all statements when
/// empty). The caller has already gated checker diagnostics on the same
/// closure.
///
/// # Errors
///
/// [`LowerError`] — see the variants; all loud.
#[allow(clippy::implicit_hasher)] // one internal call site; genericity buys nothing
pub fn lower(
    document: &Document,
    resolution: &Resolution,
    specs: &[&'static NodeSpec],
    config: &ProjectConfig,
    targets: &[String],
    scripts: &HashMap<String, ScriptNode>,
) -> Result<Lowered, LowerError> {
    for target in targets {
        if document.find_binding(target).is_none() {
            return Err(LowerError::NoSuchTarget {
                name: target.clone(),
            });
        }
    }
    let needed = reachable_bindings(document, targets);
    let spec_by_name: HashMap<&str, &'static NodeSpec> =
        specs.iter().map(|spec| (spec.name, *spec)).collect();

    let mut lowering = Lowering {
        resolution,
        spec_by_name,
        config,
        scripts,
        nodes: Vec::new(),
        output_names: Vec::new(),
        bindings: HashMap::new(),
    };

    // Kahn over the needed statements (iterative — forward references of
    // any length, never a stack overflow; same discipline as the checker).
    let statements: Vec<(usize, &Statement)> = document
        .lines()
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Line::Statement { statement, .. }
                if statement
                    .targets
                    .iter()
                    .any(|target| needed.contains(&target.name)) =>
            {
                Some((index, statement))
            }
            _ => None,
        })
        .collect();
    let line_of: HashMap<&str, usize> = statements
        .iter()
        .flat_map(|&(line, statement)| {
            statement
                .targets
                .iter()
                .map(move |target| (target.name.as_str(), line))
        })
        .collect();
    let mut pending: HashMap<usize, usize> = HashMap::new();
    let mut dependents: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(line, statement) in &statements {
        let mut upstream: HashSet<usize> = HashSet::new();
        for reference in statement.references() {
            // Self-edges included (mirroring the checker): a
            // self-referential statement must never lower half-resolved —
            // it stays a Kahn leftover and errors below.
            if let Some(&definition) = line_of.get(reference.name.as_str()) {
                upstream.insert(definition);
            }
        }
        pending.insert(line, upstream.len());
        for definition in upstream {
            dependents.entry(definition).or_default().push(line);
        }
    }
    let statement_at: HashMap<usize, &Statement> = statements.iter().copied().collect();
    let mut ready: VecDeque<usize> = {
        let mut roots: Vec<usize> = pending
            .iter()
            .filter(|&(_, &count)| count == 0)
            .map(|(&line, _)| line)
            .collect();
        roots.sort_unstable();
        roots.into_iter().collect()
    };
    let mut lowered_count = 0;
    while let Some(line) = ready.pop_front() {
        lowering.lower_statement(statement_at[&line])?;
        lowered_count += 1;
        for &dependent in dependents.get(&line).into_iter().flatten() {
            if let Some(count) = pending.get_mut(&dependent) {
                *count -= 1;
                if *count == 0 {
                    ready.push_back(dependent);
                }
            }
        }
    }
    if lowered_count != statements.len() {
        // Leftovers = a cycle the gate should have refused.
        return Err(LowerError::Unresolved {
            name: "cycle in the target cone".to_owned(),
        });
    }

    Ok(Lowered {
        graph: SolveGraph::new(lowering.nodes)?,
        bindings: lowering.bindings,
        output_names: lowering.output_names,
        excluded: BTreeMap::new(),
    })
}

/// Lower everything that can lower (the live-session form). Statements
/// with diagnostics, disabled statements, and statements whose lowering
/// refuses are excluded with their reason; everything downstream of an
/// exclusion (or of an unknown name) is excluded as `FedBy`. Kahn order,
/// iterative — forward references of any length, never a stack overflow.
///
/// # Errors
///
/// Only [`LowerError::Graph`] — graph assembly over the lowered subset
/// failing is a bug, never a user problem.
#[allow(clippy::implicit_hasher, clippy::too_many_lines)] // one Kahn pass; genericity buys nothing
pub fn lower_partial(
    document: &Document,
    resolution: &Resolution,
    specs: &[&'static NodeSpec],
    config: &ProjectConfig,
    scripts: &HashMap<String, ScriptNode>,
) -> Result<Lowered, LowerError> {
    let spec_by_name: HashMap<&str, &'static NodeSpec> =
        specs.iter().map(|spec| (spec.name, *spec)).collect();
    let mut lowering = Lowering {
        resolution,
        spec_by_name,
        config,
        scripts,
        nodes: Vec::new(),
        output_names: Vec::new(),
        bindings: HashMap::new(),
    };
    let mut excluded: BTreeMap<String, Exclusion> = BTreeMap::new();

    // Bindings with their own diagnostics (parse or semantic) are red.
    let mut red: HashSet<&str> = HashSet::new();
    for diagnostic in &resolution.diagnostics {
        if let Some(node) = &diagnostic.node {
            red.insert(node.as_str());
        }
    }
    let statements: Vec<(usize, &Statement)> = document
        .lines()
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Line::Statement { statement, .. } => Some((index, statement)),
            _ => None,
        })
        .collect();
    for line in document.lines() {
        if let Line::Disabled {
            name: Some(name), ..
        } = line
        {
            excluded.insert(name.clone(), Exclusion::Disabled);
        }
    }
    let line_of: HashMap<&str, usize> = statements
        .iter()
        .flat_map(|&(line, statement)| {
            statement
                .targets
                .iter()
                .map(move |target| (target.name.as_str(), line))
        })
        .collect();
    let mut pending: HashMap<usize, usize> = HashMap::new();
    let mut dependents: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(line, statement) in &statements {
        let mut upstream: HashSet<usize> = HashSet::new();
        for reference in statement.references() {
            if let Some(&definition) = line_of.get(reference.name.as_str()) {
                upstream.insert(definition);
            }
        }
        pending.insert(line, upstream.len());
        for definition in upstream {
            dependents.entry(definition).or_default().push(line);
        }
    }
    let statement_at: HashMap<usize, &Statement> = statements.iter().copied().collect();
    let mut ready: VecDeque<usize> = {
        let mut roots: Vec<usize> = pending
            .iter()
            .filter(|&(_, &count)| count == 0)
            .map(|(&line, _)| line)
            .collect();
        roots.sort_unstable();
        roots.into_iter().collect()
    };
    let mut visited = 0_usize;
    while let Some(line) = ready.pop_front() {
        let statement = statement_at[&line];
        visited += 1;
        // This statement's fate: red by its own diagnostics; blocked by an
        // excluded (or unknown) upstream — the first such reference names
        // the culprit; else lowered, where a refusal is a red of its own.
        let own_red = statement
            .targets
            .iter()
            .any(|target| red.contains(target.name.as_str()));
        let fed_by = statement.references().into_iter().find_map(|reference| {
            let name = reference.name.as_str();
            let missing = !line_of.contains_key(name) && !lowering.bindings.contains_key(name);
            (excluded.contains_key(name) || missing).then(|| reference.name.clone())
        });
        let outcome: Option<Exclusion> = if own_red {
            Some(Exclusion::Diagnostics)
        } else if let Some(upstream) = fed_by {
            Some(Exclusion::FedBy(upstream))
        } else {
            match lowering.lower_statement(statement) {
                Ok(()) => None,
                Err(error) => Some(Exclusion::Lowering(error.to_string())),
            }
        };
        if let Some(reason) = outcome {
            for target in &statement.targets {
                excluded.insert(target.name.clone(), reason.clone());
            }
        }
        for &dependent in dependents.get(&line).into_iter().flatten() {
            if let Some(count) = pending.get_mut(&dependent) {
                *count -= 1;
                if *count == 0 {
                    ready.push_back(dependent);
                }
            }
        }
    }
    if visited != statements.len() {
        // Kahn leftovers = a cycle. The checker already red-flagged its
        // members; name them explicitly rather than losing them.
        for &(line, statement) in &statements {
            if pending.get(&line).is_some_and(|&count| count > 0) {
                for target in &statement.targets {
                    excluded
                        .entry(target.name.clone())
                        .or_insert_with(|| Exclusion::Lowering("part of a cycle".to_owned()));
                }
            }
        }
    }
    Ok(Lowered {
        graph: SolveGraph::new(lowering.nodes)?,
        bindings: lowering.bindings,
        output_names: lowering.output_names,
        excluded,
    })
}

/// The diagnostics that name bindings inside `cone` (or name no binding at
/// all — file-level problems block everything), split from those outside
/// it: `(blocking, outside)`. The `cicada run` gate and the session share
/// this one rule.
#[must_use]
#[allow(clippy::implicit_hasher)] // one internal call site
pub fn split_diagnostics<'d>(
    diagnostics: &'d [Diagnostic],
    cone: &HashSet<String>,
) -> (Vec<&'d Diagnostic>, Vec<&'d Diagnostic>) {
    diagnostics.iter().partition(|diagnostic| {
        diagnostic
            .node
            .as_ref()
            .is_none_or(|node| cone.contains(node))
    })
}

struct Lowering<'a> {
    resolution: &'a Resolution,
    spec_by_name: HashMap<&'static str, &'static NodeSpec>,
    config: &'a ProjectConfig,
    scripts: &'a HashMap<String, ScriptNode>,
    nodes: Vec<NodeDecl>,
    output_names: Vec<Vec<String>>,
    bindings: HashMap<String, LoweredBinding>,
}

impl Lowering<'_> {
    fn push_node(&mut self, decl: NodeDecl, outputs: Vec<String>) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(decl);
        self.output_names.push(outputs);
        id
    }

    fn lower_statement(&mut self, statement: &Statement) -> Result<(), LowerError> {
        let name = statement.name().to_owned();
        match &statement.rhs {
            Rhs::Literal(lit) => {
                let value = literal_value(&name, lit)?;
                self.bindings.insert(name, LoweredBinding::Value(value));
                Ok(())
            }
            Rhs::Expression(expr) => self.lower_expression(&name, expr),
            Rhs::Call(call) => self.lower_call(statement, call),
        }
    }

    // ------------------------------------------------------- literals --

    // ---------------------------------------------------- expressions --

    fn lower_expression(&mut self, name: &str, expr: &Expr) -> Result<(), LowerError> {
        let variables: Vec<String> = expr
            .free_vars()
            .into_iter()
            .map(|ident| ident.name.clone())
            .collect();
        let inputs = variables
            .iter()
            .map(|variable| self.input_for_name(variable))
            .collect::<Result<Vec<_>, _>>()?;

        // Integer mode exactly when the checker typed this binding Integer
        // (literals and ops preserve integrality, every var is Integer).
        let integer_mode = matches!(
            self.resolution.bindings.get(name),
            Some(BindingType::Value { ty, .. }) if ty.base == "Integer"
        );

        let ir_hash = expression_ir_hash(expr, &variables);
        let run = expression_fn(name, expr.clone(), integer_mode);
        let fan = vec![0; inputs.len()];
        let id = self.push_node(
            NodeDecl {
                name: name.to_owned(),
                op: "expr".to_owned(),
                version: 1,
                body_hash: Some(ir_hash),
                tolerance: None,
                inputs,
                fan,
                output_count: 1,
                effectful: false,
                run,
            },
            vec!["out".to_owned()],
        );
        self.bindings.insert(
            name.to_owned(),
            LoweredBinding::Port {
                node: id,
                output: 0,
            },
        );
        Ok(())
    }

    // ----------------------------------------------------------- calls --

    fn lower_call(
        &mut self,
        statement: &Statement,
        call: &cicada_lang::ast::Call,
    ) -> Result<(), LowerError> {
        let name = statement.name().to_owned();
        let Some(spec) = self.spec_by_name.get(call.func.name.as_str()).copied() else {
            return Err(LowerError::Unresolved { name });
        };
        // Script nodes carry their own run fn + source hash (docs/12:
        // script node_version = source hash, in the body_hash slot);
        // stdlib nodes dispatch through the registry's erased invoker.
        let (run, body_hash): (NodeFn, Option<ValueHash>) =
            if let Some(script) = self.scripts.get(spec.name) {
                (Arc::clone(&script.run), Some(script.body_hash))
            } else {
                let Some(invoke) = cicada_core::spec::invoker(spec.name) else {
                    return Err(LowerError::NoInvoker {
                        name: spec.name.to_owned(),
                    });
                };
                // The closure captures a config COPY: the invoke ABI takes
                // it explicitly (tolerance is explicit state), and the
                // NodeKey already folds the tolerance hash for
                // uses_tolerance nodes.
                let config = *self.config;
                let run: NodeFn = Arc::new(move |values| {
                    invoke(&config, values).map_err(|error| NodeError::new(error.to_string()))
                });
                (run, None)
            };

        let mut inputs = Vec::with_capacity(spec.inputs.len());
        let mut fan = Vec::with_capacity(spec.inputs.len());
        for port in spec.inputs {
            let kwarg = call
                .kwargs
                .iter()
                .find(|kwarg| kwarg.name.name == port.name);
            match kwarg {
                None => {
                    inputs.push(Input::Absent);
                    fan.push(0);
                }
                Some(kwarg) => {
                    let depth = kwarg.value.each_depth();
                    if depth > 1 {
                        return Err(LowerError::EachDepth { node: name, depth });
                    }
                    inputs.push(self.input_for_value(&name, kwarg.value.unlifted())?);
                    fan.push(depth);
                }
            }
        }

        let outputs: Vec<String> = spec
            .outputs
            .iter()
            .map(|port| port.name.to_owned())
            .collect();
        let id = self.push_node(
            NodeDecl {
                name: name.clone(),
                op: spec.name.to_owned(),
                version: spec.version,
                body_hash,
                tolerance: spec.uses_tolerance.then(|| self.config.tolerance_hash()),
                inputs,
                fan,
                output_count: spec.outputs.len(),
                effectful: !spec.pure,
                run,
            },
            outputs,
        );

        // Targets → bindings (mirrors the checker's binding shapes).
        if statement.targets.len() > 1 {
            for (index, target) in statement.targets.iter().enumerate() {
                self.bindings.insert(
                    target.name.clone(),
                    LoweredBinding::Port {
                        node: id,
                        output: index,
                    },
                );
            }
        } else if spec.outputs.len() == 1 {
            self.bindings.insert(
                name,
                LoweredBinding::Port {
                    node: id,
                    output: 0,
                },
            );
        } else {
            self.bindings
                .insert(name, LoweredBinding::Node { node: id });
        }
        Ok(())
    }

    // ------------------------------------------------------ references --

    fn input_for_value(&self, node: &str, value: &ValueExpr) -> Result<Input, LowerError> {
        match value {
            ValueExpr::Literal(lit) => Ok(Input::Value(literal_value(node, lit)?)),
            ValueExpr::Each { .. } => unreachable!("caller unlifts"),
            ValueExpr::Ref(port_ref) => {
                let binding = self.bindings.get(&port_ref.binding.name).ok_or_else(|| {
                    LowerError::Unresolved {
                        name: port_ref.binding.name.clone(),
                    }
                })?;
                match (binding, &port_ref.port) {
                    // A port selection on a Value can only be `.out` on a
                    // single-output call — the bare reference
                    // (checker-verified); anything else was gated as a
                    // diagnostic before lowering.
                    (LoweredBinding::Value(value), _) => Ok(Input::Value(Arc::clone(value))),
                    (LoweredBinding::Port { node, output }, _) => Ok(Input::Port {
                        node: *node,
                        output: *output,
                    }),
                    (LoweredBinding::Node { node }, Some(port)) => {
                        let output = self.output_names[node.0]
                            .iter()
                            .position(|name| name == &port.name)
                            .ok_or_else(|| LowerError::Unresolved {
                                name: format!("{}.{}", port_ref.binding.name, port.name),
                            })?;
                        Ok(Input::Port {
                            node: *node,
                            output,
                        })
                    }
                    (LoweredBinding::Node { .. }, None) => Err(LowerError::Unresolved {
                        name: port_ref.binding.name.clone(),
                    }),
                }
            }
        }
    }

    fn input_for_name(&self, name: &str) -> Result<Input, LowerError> {
        match self.bindings.get(name) {
            Some(LoweredBinding::Value(value)) => Ok(Input::Value(Arc::clone(value))),
            Some(LoweredBinding::Port { node, output }) => Ok(Input::Port {
                node: *node,
                output: *output,
            }),
            _ => Err(LowerError::Unresolved {
                name: name.to_owned(),
            }),
        }
    }
}

/// One literal binding's sealed value.
fn literal_value(node: &str, lit: &LitWithSpan) -> Result<Arc<HashedValue>, LowerError> {
    let data = literal_data(node, &lit.lit)?;
    HashedValue::new(data).map_err(|error: ValueError| LowerError::BadLiteral {
        node: node.to_owned(),
        message: error.to_string(),
    })
}

/// One literal's `ValueData` (lists recurse; the parser caps nesting).
fn literal_data(node: &str, lit: &Lit) -> Result<ValueData, LowerError> {
    Ok(match lit {
        Lit::Number { value, integer } => {
            if *integer {
                ValueData::Integer(exact_i64(node, *value)?)
            } else {
                ValueData::Number(*value)
            }
        }
        Lit::Text(text) => ValueData::Text(Arc::from(text.as_str())),
        Lit::Boolean(flag) => ValueData::Boolean(*flag),
        Lit::List(items) => {
            let mut slots = Vec::with_capacity(items.len());
            for item in items {
                let data = literal_data(node, &item.lit)?;
                let element = HashedValue::new(data).map_err(|error| LowerError::BadLiteral {
                    node: node.to_owned(),
                    message: error.to_string(),
                })?;
                slots.push(Some(element));
            }
            ValueData::List(List { axis: None, slots })
        }
    })
}

/// Exact i64 from a parsed integer literal (carried as f64 by the parser).
/// The bound is HALF-OPEN (`>=`): the source text `9007199254740993`
/// (2^53+1) parses (ties-to-even) to exactly 2^53, so an f64 equal to
/// ±2^53 may be a silently drifted digit — refusing the boundary is the
/// only loud option (regression: adversarial review, stage 3).
fn exact_i64(node: &str, value: f64) -> Result<i64, LowerError> {
    let range = LowerError::IntegerRange {
        node: node.to_owned(),
        value,
    };
    if value.fract() != 0.0 || value.abs() >= 9_007_199_254_740_992.0 {
        return Err(range);
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i64)
}

/// The normalized expression IR hash (docs/12): structure, operator codes,
/// and literal bits, with variables hashed by ordinal of first appearance —
/// renames never change the hash; whitespace never existed in the AST.
fn expression_ir_hash(expr: &Expr, variables: &[String]) -> ValueHash {
    fn walk(hasher: ValueHasher, expr: &Expr, variables: &[String]) -> ValueHasher {
        match expr {
            Expr::Number { value, integer, .. } => {
                hasher.byte(1).f64(*value).byte(u8::from(*integer))
            }
            Expr::Var(ident) => {
                let ordinal = variables
                    .iter()
                    .position(|name| name == &ident.name)
                    .unwrap_or(usize::MAX);
                hasher.byte(2).u64(ordinal as u64)
            }
            Expr::Neg { operand, .. } => walk(hasher.byte(3), operand, variables),
            Expr::Binary { op, lhs, rhs, .. } => {
                let code = match op {
                    BinOp::Add => 1,
                    BinOp::Sub => 2,
                    BinOp::Mul => 3,
                    BinOp::Div => 4,
                    BinOp::Pow => 5,
                };
                let hasher = walk(hasher.byte(4).byte(code), lhs, variables);
                walk(hasher, rhs, variables)
            }
        }
    }
    walk(ValueHasher::new(KindTag::ExprIr), expr, variables).finish()
}

/// The run function of an expression node: evaluate over the free-variable
/// values (in first-appearance order). Integer mode uses checked i64
/// arithmetic (overflow is a red node, never a wrap); float mode is f64
/// with `^` = `powf`. NaN results refuse at value construction (docs/12).
fn expression_fn(name: &str, expr: Expr, integer_mode: bool) -> NodeFn {
    let name = name.to_owned();
    Arc::new(move |values| {
        let mut variables: Vec<(String, &Arc<HashedValue>)> = Vec::new();
        let mut ordered: Vec<&Arc<HashedValue>> = Vec::with_capacity(values.len());
        for value in values {
            let value = value.as_ref().ok_or_else(|| {
                NodeError::new("expression input missing — lowering bug".to_owned())
            })?;
            ordered.push(value);
        }
        for (ordinal, ident) in expr.free_vars().into_iter().enumerate() {
            let value = ordered.get(ordinal).ok_or_else(|| {
                NodeError::new("expression arity mismatch — lowering bug".to_owned())
            })?;
            variables.push((ident.name.clone(), value));
        }
        if integer_mode {
            let out = eval_integer(&expr, &variables)?;
            HashedValue::new(ValueData::Integer(out))
                .map(|value| vec![value])
                .map_err(|error| NodeError::new(error.to_string()))
        } else {
            let out = eval_float(&expr, &variables)?;
            HashedValue::new(ValueData::Number(out))
                .map(|value| vec![value])
                .map_err(|error| NodeError::new(format!("`{name}` produced {out}: {error}")))
        }
    })
}

fn variable_value<'v>(
    variables: &[(String, &'v Arc<HashedValue>)],
    name: &str,
) -> Result<&'v Arc<HashedValue>, NodeError> {
    variables
        .iter()
        .find(|(variable, _)| variable == name)
        .map(|(_, value)| *value)
        .ok_or_else(|| NodeError::new(format!("unbound expression variable `{name}`")))
}

fn eval_integer(expr: &Expr, variables: &[(String, &Arc<HashedValue>)]) -> Result<i64, NodeError> {
    match expr {
        Expr::Number { value, .. } => {
            exact_i64("expr", *value).map_err(|error| NodeError::new(error.to_string()))
        }
        Expr::Var(ident) => match variable_value(variables, &ident.name)?.data() {
            ValueData::Integer(i) => Ok(*i),
            other => Err(NodeError::new(format!(
                "integer expression got a {} for `{}` — checker bug",
                other.kind_name(),
                ident.name
            ))),
        },
        Expr::Neg { operand, .. } => eval_integer(operand, variables)?
            .checked_neg()
            .ok_or_else(|| NodeError::new("integer overflow in `-`".to_owned())),
        Expr::Binary { op, lhs, rhs, .. } => {
            let a = eval_integer(lhs, variables)?;
            let b = eval_integer(rhs, variables)?;
            let (result, symbol) = match op {
                BinOp::Add => (a.checked_add(b), "+"),
                BinOp::Sub => (a.checked_sub(b), "-"),
                BinOp::Mul => (a.checked_mul(b), "*"),
                // The checker types `/` and `^` expressions Number.
                BinOp::Div | BinOp::Pow => {
                    return Err(NodeError::new(
                        "non-integer operator in integer mode — checker bug".to_owned(),
                    ));
                }
            };
            result.ok_or_else(|| NodeError::new(format!("integer overflow in `{a} {symbol} {b}`")))
        }
    }
}

fn eval_float(expr: &Expr, variables: &[(String, &Arc<HashedValue>)]) -> Result<f64, NodeError> {
    match expr {
        Expr::Number { value, .. } => Ok(*value),
        Expr::Var(ident) => match variable_value(variables, &ident.name)?.data() {
            ValueData::Number(x) => Ok(*x),
            // ONE widening rule for the whole system (the saturating-cast
            // trap at i64::MAX lives in the helper's doc).
            ValueData::Integer(i) => {
                cicada_core::marshal::integer_to_number_exact(*i).ok_or_else(|| {
                    NodeError::new(format!("Integer {i} does not convert exactly to Number"))
                })
            }
            other => Err(NodeError::new(format!(
                "expression got a {} for `{}`",
                other.kind_name(),
                ident.name
            ))),
        },
        Expr::Neg { operand, .. } => Ok(-eval_float(operand, variables)?),
        Expr::Binary { op, lhs, rhs, .. } => {
            let a = eval_float(lhs, variables)?;
            let b = eval_float(rhs, variables)?;
            Ok(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                BinOp::Pow => a.powf(b),
            })
        }
    }
}
