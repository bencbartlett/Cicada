//! Auto-layout on the unit grid (docs/10 §The layout sidecar: primary,
//! deterministic, snappy; the sidecar stores manual overrides only).
//!
//! The spike's layout is the simplest honest one: **layer by dependency
//! depth, stack in definition order** — column = longest path from a
//! source, row = cumulative height of the nodes above it in that column.
//! Same graph → same cells (a unit test); hundreds of nodes lay out in
//! microseconds. Overridden nodes sit exactly where the sidecar says.
//! Smarter layout (crossing reduction, compaction) is v0.1 polish.

use std::collections::HashMap;

/// Grid columns between layers (in units).
pub const COLUMN_GAP: i64 = 3;
/// Grid rows between stacked nodes.
pub const ROW_GAP: i64 = 1;
/// Default node width in units.
pub const NODE_WIDTH: u32 = 8;

/// One node's layout input.
#[derive(Debug, Clone)]
pub struct LayoutNode<'a> {
    /// Binding name.
    pub name: &'a str,
    /// Names this node reads from.
    pub deps: Vec<&'a str>,
    /// Size in grid units `[w, h]`.
    pub size: [u32; 2],
    /// Manual override cell, if any.
    pub manual: Option<[i64; 2]>,
}

/// Cells for every node, by name. Nodes are given in definition order —
/// the tie-breaker within a column.
#[must_use]
pub fn auto_layout(nodes: &[LayoutNode<'_>]) -> HashMap<String, [i64; 2]> {
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.name, i))
        .collect();
    // Longest-path depth, iterative over definition order with a fixpoint
    // (forward references exist; the graph is a DAG apart from cycles,
    // which the checker already flags — a cycle here just stops deepening
    // once every member has been visited enough times).
    let mut depth = vec![0_usize; nodes.len()];
    let bound = nodes.len().max(1);
    for _ in 0..bound {
        let mut changed = false;
        for (i, node) in nodes.iter().enumerate() {
            let mut best = 0;
            for dep in &node.deps {
                if let Some(&j) = index.get(dep)
                    && j != i
                {
                    best = best.max(depth[j] + 1);
                }
            }
            if best > depth[i] && best < bound {
                depth[i] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Column x positions: each column as wide as its widest node.
    let column_count = depth.iter().copied().max().map_or(0, |d| d + 1);
    let mut column_width = vec![0_i64; column_count];
    for (i, node) in nodes.iter().enumerate() {
        if node.manual.is_none() {
            column_width[depth[i]] = column_width[depth[i]].max(i64::from(node.size[0]));
        }
    }
    let mut column_x = vec![0_i64; column_count];
    let mut x = 0;
    for (column, width) in column_width.iter().enumerate() {
        column_x[column] = x;
        x += (*width).max(i64::from(NODE_WIDTH)) + COLUMN_GAP;
    }
    let mut cursor = vec![0_i64; column_count];
    let mut cells = HashMap::with_capacity(nodes.len());
    for (i, node) in nodes.iter().enumerate() {
        if let Some(cell) = node.manual {
            cells.insert(node.name.to_owned(), cell);
            continue;
        }
        let column = depth[i];
        let y = cursor[column];
        cursor[column] = y + i64::from(node.size[1]) + ROW_GAP;
        cells.insert(node.name.to_owned(), [column_x[column], y]);
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node<'a>(name: &'a str, deps: &[&'a str], h: u32) -> LayoutNode<'a> {
        LayoutNode {
            name,
            deps: deps.to_vec(),
            size: [NODE_WIDTH, h],
            manual: None,
        }
    }

    #[test]
    fn layers_by_depth_and_stacks_in_definition_order() {
        let nodes = vec![
            node("a", &[], 3),
            node("b", &["a"], 2),
            node("c", &[], 2),
            node("d", &["b", "c"], 4),
        ];
        let cells = auto_layout(&nodes);
        assert_eq!(cells["a"], [0, 0]);
        assert_eq!(cells["c"], [0, 3 + ROW_GAP], "stacked under a");
        let col1 = i64::from(NODE_WIDTH) + COLUMN_GAP;
        assert_eq!(cells["b"], [col1, 0]);
        assert_eq!(cells["d"], [2 * col1, 0]);
    }

    #[test]
    fn manual_overrides_win_and_are_skipped_in_stacking() {
        let mut nodes = vec![node("a", &[], 3), node("b", &[], 2)];
        nodes[0].manual = Some([40, 9]);
        let cells = auto_layout(&nodes);
        assert_eq!(cells["a"], [40, 9]);
        assert_eq!(cells["b"], [0, 0], "b takes the top slot a vacated");
    }

    #[test]
    fn deterministic_and_cycle_safe() {
        let nodes = vec![
            node("a", &["b"], 1),
            node("b", &["a"], 1),
            node("c", &["zzz"], 1),
        ];
        let first = auto_layout(&nodes);
        let second = auto_layout(&nodes);
        assert_eq!(first, second);
        assert!(
            first.contains_key("c"),
            "unknown deps are ignored, node still placed"
        );
    }
}
