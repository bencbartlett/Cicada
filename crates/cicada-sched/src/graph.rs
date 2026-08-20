//! The solve graph: the scheduler's own model of a pipeline — nodes with
//! typed-erased run functions, value/port inputs, and per-port `each()`
//! fan depths (the pairing shape, docs/12 §Cache keys).
//!
//! Deliberately independent of `cicada-lang`: the lowering from a checked
//! `.cic` document lives with the consumers (cicada-cli today, the server
//! at stage 5), and tests drive the scheduler with fake nodes directly.
//! The `.cic` file IS the graph (docs/12 §Why not salsa) — this is its
//! executable projection, validated once at construction: bad edges and
//! cycles refuse loudly here, never mid-solve.

use std::collections::BTreeSet;
use std::sync::Arc;

use cicada_core::hash::ValueHash;
use cicada_core::value::HashedValue;

use crate::cancel::NodeCtx;

/// A node's index in its [`SolveGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

/// Why one node failed at run time. The scheduler wraps this with the node
/// name and (for element fan-out) the offending element IDs — red with IDs,
/// docs/12 §Element failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct NodeError {
    /// What went wrong, in domain terms.
    pub message: String,
}

impl NodeError {
    /// A new error from any message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// The type-erased run function of one node: the generation's [`NodeCtx`]
/// (its cancel handle — a long call polls it at safe points, a host bridge
/// hooks it), per-port inputs in spec order (`None` = use the port
/// default), per-port outputs. For `each()` nodes this is the
/// **element-level** function — the executor owns iteration, chunking, and
/// slot assembly. Panics are caught by the executor and become red nodes
/// (docs/12).
pub type NodeFn = Arc<
    dyn Fn(&NodeCtx<'_>, &[Option<Arc<HashedValue>>]) -> Result<Vec<Arc<HashedValue>>, NodeError>
        + Send
        + Sync,
>;

/// One input slot of a node, in spec port order.
#[derive(Clone)]
pub enum Input {
    /// A literal/param value, content-addressed like everything else.
    Value(Arc<HashedValue>),
    /// A wire from an upstream node's output port.
    Port {
        /// The upstream node.
        node: NodeId,
        /// Which of its outputs.
        output: usize,
    },
    /// No value — the node's default for this port applies (and the cache
    /// key records the absence, so a changed default is covered by the
    /// node's version bump).
    Absent,
}

/// One node of the solve graph.
#[derive(Clone)]
pub struct NodeDecl {
    /// Display name (the `.cic` binding) — diagnostics and progress only,
    /// never part of the cache key: renames must not recompute (docs/10).
    pub name: String,
    /// The operation identity in cache keys (`add`, `expr`, …).
    pub op: String,
    /// Semantic version of the operation (doc 12 cache keys).
    pub version: u32,
    /// Extra key material for body-carrying ops: normalized expression IR
    /// hash today, script source hashes at stage 4. `None` for stdlib ops.
    pub body_hash: Option<ValueHash>,
    /// The project tolerance hash, for ops declaring `uses_tolerance`
    /// (DECISIONS.md tolerance row).
    pub tolerance: Option<ValueHash>,
    /// Inputs, one per port in spec order.
    pub inputs: Vec<Input>,
    /// `each()` depth per input, parallel to `inputs` — the pairing shape.
    /// Depth 1 = element fan-out; multiple depth-1 ports zip strictly.
    pub fan: Vec<u8>,
    /// Output port count.
    pub output_count: usize,
    /// Effectful (exporters, doc 10 §7): NEVER served from or recorded to
    /// the memo table — a "cache hit" that skipped writing the file would
    /// be a silent lie. Effectful nodes run every time they are targeted.
    pub effectful: bool,
    /// Volatile (DECISIONS.md time row: `Clock` is "uncached by design"):
    /// the memo is neither read nor written for this node's outputs, at
    /// node AND element granularity, so it executes in every generation
    /// whose cone holds it — and inside an `each()` fan-out, once per
    /// element. Downstream nodes are ordinary: their keys include the
    /// volatile output's fresh value hash, so they recompute exactly when
    /// that value changed and hit the memo when it did not (docs/12
    /// §Volatile nodes). Exclusive with `effectful` (the macro refuses
    /// both).
    pub volatile: bool,
    /// The run function.
    pub run: NodeFn,
}

/// Graph construction errors — every one is a lowering bug surfaced loudly
/// at build time, never a mid-solve surprise.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// An input wires a node index that does not exist.
    #[error("node `{node}` input {input} wires missing node index {upstream}")]
    EdgeOutOfRange {
        /// Referring node.
        node: String,
        /// Input slot.
        input: usize,
        /// The dangling index.
        upstream: usize,
    },
    /// An input selects an output port the upstream node does not have.
    #[error(
        "node `{node}` input {input} selects output {output} of `{upstream}` \
         (which has {available})"
    )]
    OutputOutOfRange {
        /// Referring node.
        node: String,
        /// Input slot.
        input: usize,
        /// Upstream node name.
        upstream: String,
        /// Selected output index.
        output: usize,
        /// How many outputs exist.
        available: usize,
    },
    /// `fan` and `inputs` lengths differ.
    #[error("node `{node}` has {inputs} inputs but {fan} fan depths")]
    FanArity {
        /// The node.
        node: String,
        /// Input count.
        inputs: usize,
        /// Fan count.
        fan: usize,
    },
    /// A fan depth above 1 — no S-tier node needed nested lifts, so
    /// nested execution waits for a node set that does (v0.1); refusing
    /// beats a wrong fan-out.
    #[error("node `{node}` input {input} has each() depth {depth}; only depth 1 executes today")]
    FanDepthUnsupported {
        /// The node.
        node: String,
        /// Input slot.
        input: usize,
        /// The unsupported depth.
        depth: u8,
    },
    /// A fanned input with no value to fan over.
    #[error("node `{node}` input {input} is each()-lifted but has no value wired")]
    FanOnAbsent {
        /// The node.
        node: String,
        /// Input slot.
        input: usize,
    },
    /// The nodes form a cycle — the semantics are a DAG (docs/10).
    #[error("cycle — the graph must be a DAG: {}", members.join(" → "))]
    Cycle {
        /// Names of the cycle members, in index order.
        members: Vec<String>,
    },
}

/// A validated, topologically ordered solve graph.
pub struct SolveGraph {
    nodes: Vec<NodeDecl>,
    /// Downstream adjacency: `dependents[i]` = nodes with an input wired to
    /// node `i` (deduplicated).
    dependents: Vec<Vec<NodeId>>,
    /// Upstream adjacency: distinct nodes each node wires from.
    upstream: Vec<Vec<NodeId>>,
    /// One valid topological order (deterministic: smallest index first).
    order: Vec<NodeId>,
}

impl SolveGraph {
    /// Validate and build.
    ///
    /// # Errors
    ///
    /// [`GraphError`] on dangling edges, bad output indices, fan-shape
    /// problems, or cycles.
    pub fn new(nodes: Vec<NodeDecl>) -> Result<Self, GraphError> {
        let count = nodes.len();
        let mut upstream: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); count];
        for (index, node) in nodes.iter().enumerate() {
            if node.fan.len() != node.inputs.len() {
                return Err(GraphError::FanArity {
                    node: node.name.clone(),
                    inputs: node.inputs.len(),
                    fan: node.fan.len(),
                });
            }
            for (slot, (input, &fan)) in node.inputs.iter().zip(&node.fan).enumerate() {
                if fan > 1 {
                    return Err(GraphError::FanDepthUnsupported {
                        node: node.name.clone(),
                        input: slot,
                        depth: fan,
                    });
                }
                match input {
                    Input::Value(_) => {}
                    Input::Absent => {
                        if fan > 0 {
                            return Err(GraphError::FanOnAbsent {
                                node: node.name.clone(),
                                input: slot,
                            });
                        }
                    }
                    Input::Port { node: from, output } => {
                        let Some(producer) = nodes.get(from.0) else {
                            return Err(GraphError::EdgeOutOfRange {
                                node: node.name.clone(),
                                input: slot,
                                upstream: from.0,
                            });
                        };
                        if *output >= producer.output_count {
                            return Err(GraphError::OutputOutOfRange {
                                node: node.name.clone(),
                                input: slot,
                                upstream: producer.name.clone(),
                                output: *output,
                                available: producer.output_count,
                            });
                        }
                        upstream[index].insert(*from);
                    }
                }
            }
        }

        let mut dependents: Vec<BTreeSet<NodeId>> = vec![BTreeSet::new(); count];
        for (index, ups) in upstream.iter().enumerate() {
            for up in ups {
                dependents[up.0].insert(NodeId(index));
            }
        }

        // Kahn, deterministic (smallest ready index first). Leftovers are
        // cycle members.
        let mut pending: Vec<usize> = upstream.iter().map(BTreeSet::len).collect();
        let mut ready: BTreeSet<NodeId> = pending
            .iter()
            .enumerate()
            .filter(|&(_, &degree)| degree == 0)
            .map(|(index, _)| NodeId(index))
            .collect();
        let mut order = Vec::with_capacity(count);
        while let Some(&next) = ready.iter().next() {
            ready.remove(&next);
            order.push(next);
            for dependent in &dependents[next.0] {
                pending[dependent.0] -= 1;
                if pending[dependent.0] == 0 {
                    ready.insert(*dependent);
                }
            }
        }
        if order.len() != count {
            let members = pending
                .iter()
                .enumerate()
                .filter(|&(_, &degree)| degree > 0)
                .map(|(index, _)| nodes[index].name.clone())
                .collect();
            return Err(GraphError::Cycle { members });
        }

        Ok(Self {
            nodes,
            dependents: dependents
                .into_iter()
                .map(|set| set.into_iter().collect())
                .collect(),
            upstream: upstream
                .into_iter()
                .map(|set| set.into_iter().collect())
                .collect(),
            order,
        })
    }

    /// Node count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the graph has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// One node.
    ///
    /// # Panics
    ///
    /// Panics when `id` is out of range — `NodeId`s come from this graph.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &NodeDecl {
        &self.nodes[id.0]
    }

    /// All nodes, index = `NodeId`.
    #[must_use]
    pub fn nodes(&self) -> &[NodeDecl] {
        &self.nodes
    }

    /// The `NodeId` bound to a display name, if any.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .position(|node| node.name == name)
            .map(NodeId)
    }

    /// Direct downstream dependents of one node.
    #[must_use]
    pub fn dependents(&self, id: NodeId) -> &[NodeId] {
        &self.dependents[id.0]
    }

    /// Distinct upstream nodes one node wires from.
    #[must_use]
    pub fn upstream(&self, id: NodeId) -> &[NodeId] {
        &self.upstream[id.0]
    }

    /// A valid topological order (deterministic).
    #[must_use]
    pub fn topo_order(&self) -> &[NodeId] {
        &self.order
    }

    /// The **upstream cone**: `targets` plus everything they transitively
    /// wire from — the set a pull-solve must consider. Returned as a mask
    /// indexed by `NodeId`.
    #[must_use]
    pub fn ancestors(&self, targets: &[NodeId]) -> Vec<bool> {
        self.cone(targets, |id| &self.upstream[id.0])
    }

    /// The **downstream cone**: `seeds` plus everything transitively fed by
    /// them — the dirty set of an edit at `seeds` (docs/12 §Solve
    /// generations: "the dirty set is their downstream cone").
    #[must_use]
    pub fn downstream_cone(&self, seeds: &[NodeId]) -> Vec<bool> {
        self.cone(seeds, |id| &self.dependents[id.0])
    }

    fn cone<'graph>(
        &'graph self,
        from: &[NodeId],
        edges: impl Fn(NodeId) -> &'graph [NodeId],
    ) -> Vec<bool> {
        let mut mask = vec![false; self.nodes.len()];
        let mut stack: Vec<NodeId> = from.to_vec();
        while let Some(id) = stack.pop() {
            if mask[id.0] {
                continue;
            }
            mask[id.0] = true;
            stack.extend_from_slice(edges(id));
        }
        mask
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;

    fn noop_fn() -> NodeFn {
        Arc::new(|_ctx, _inputs| Ok(vec![]))
    }

    fn decl(name: &str, inputs: Vec<Input>) -> NodeDecl {
        let fan = vec![0; inputs.len()];
        NodeDecl {
            name: name.to_owned(),
            op: "fake".to_owned(),
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs,
            fan,
            output_count: 1,
            effectful: false,
            volatile: false,
            run: noop_fn(),
        }
    }

    fn port(node: usize) -> Input {
        Input::Port {
            node: NodeId(node),
            output: 0,
        }
    }

    /// a → b → d, a → c → d (diamond).
    fn diamond() -> SolveGraph {
        SolveGraph::new(vec![
            decl("a", vec![]),
            decl("b", vec![port(0)]),
            decl("c", vec![port(0)]),
            decl("d", vec![port(1), port(2)]),
        ])
        .expect("diamond is valid")
    }

    #[test]
    fn cones_are_exact_on_the_diamond() {
        let graph = diamond();
        assert_eq!(
            graph.downstream_cone(&[NodeId(1)]),
            vec![false, true, false, true],
            "b's dirty cone is b, d"
        );
        assert_eq!(
            graph.ancestors(&[NodeId(1)]),
            vec![true, true, false, false],
            "b pulls a, b"
        );
        assert_eq!(graph.downstream_cone(&[NodeId(0)]), vec![true; 4]);
        assert_eq!(graph.ancestors(&[NodeId(3)]), vec![true; 4]);
    }

    #[test]
    fn topo_order_is_deterministic_and_valid() {
        let graph = diamond();
        assert_eq!(
            graph.topo_order(),
            &[NodeId(0), NodeId(1), NodeId(2), NodeId(3)]
        );
    }

    #[test]
    fn cycle_is_refused_with_members() {
        let result = SolveGraph::new(vec![
            decl("a", vec![port(1)]),
            decl("b", vec![port(0)]),
            decl("free", vec![]),
        ]);
        let Err(GraphError::Cycle { members }) = result else {
            panic!("cycle must refuse")
        };
        assert_eq!(members, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn dangling_edge_and_bad_output_are_refused() {
        assert!(matches!(
            SolveGraph::new(vec![decl("a", vec![port(7)])]),
            Err(GraphError::EdgeOutOfRange { upstream: 7, .. })
        ));
        assert!(matches!(
            SolveGraph::new(vec![
                decl("a", vec![]),
                decl(
                    "b",
                    vec![Input::Port {
                        node: NodeId(0),
                        output: 3
                    }]
                ),
            ]),
            Err(GraphError::OutputOutOfRange {
                output: 3,
                available: 1,
                ..
            })
        ));
    }

    #[test]
    fn fan_shape_problems_are_refused() {
        let value = HashedValue::new(ValueData::Number(1.0)).unwrap();
        let mut bad_arity = decl("a", vec![Input::Value(value)]);
        bad_arity.fan = vec![];
        assert!(matches!(
            SolveGraph::new(vec![bad_arity]),
            Err(GraphError::FanArity { .. })
        ));

        let mut deep = decl("a", vec![]);
        deep.inputs = vec![Input::Absent];
        deep.fan = vec![2];
        assert!(matches!(
            SolveGraph::new(vec![deep]),
            Err(GraphError::FanDepthUnsupported { depth: 2, .. })
        ));

        let mut fan_absent = decl("a", vec![]);
        fan_absent.inputs = vec![Input::Absent];
        fan_absent.fan = vec![1];
        assert!(matches!(
            SolveGraph::new(vec![fan_absent]),
            Err(GraphError::FanOnAbsent { .. })
        ));
    }
}
