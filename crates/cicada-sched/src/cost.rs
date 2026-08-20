//! Cost samples and the (deliberately naive) estimator (docs/12 §Long-
//! running nodes: "cost sampling (recorded, even if the estimator stays
//! naive)", doc 15 stage 3). Samples persist in the memo log per operation;
//! the estimate feeds chunk sizing today and grows into the doc-12
//! regression estimator with stage 4's real workloads.

use serde::{Deserialize, Serialize};

/// One computation's measured cost: how many elements it executed and the
/// work nanoseconds they took (summed across parallel chunks — CPU cost,
/// not wall clock). Recorded beside a node-level memo entry so a cache hit
/// still knows what the computation cost when it last ran (docs/12 §Progress
/// and ETA: "cost samples per `NodeKey` persist in the memo table") — the
/// cost model stays complete across a warm reopen, where nothing computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostSample {
    /// Elements the computation processed (1 for scalar nodes).
    pub elements: u64,
    /// Work nanoseconds (CPU, summed across chunks).
    pub nanos: u64,
}

/// Accumulated cost observations for one operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostStats {
    /// Recorded calls (a fanned node records once per solve, not per
    /// element).
    pub calls: u64,
    /// Total elements across calls (scalar calls count 1).
    pub elements: u64,
    /// Total work nanoseconds across calls — summed across parallel
    /// chunks, so it is CPU cost, not wall clock.
    pub nanos: u64,
}

impl CostStats {
    /// Fold in one observation.
    pub fn record(&mut self, elements: u64, nanos: u64) {
        self.calls += 1;
        self.elements += elements;
        self.nanos += nanos;
    }

    /// Mean nanoseconds per element, when any element has been seen.
    #[must_use]
    pub fn per_element_nanos(&self) -> Option<u64> {
        (self.elements > 0).then(|| self.nanos / self.elements)
    }
}

/// Elements per chunk for an `each()` fan-out of `n` elements: aim for
/// `target_nanos` of work per chunk (docs/12: ~10–50 ms — small enough to
/// cancel responsively, big enough to amortize overhead), but never fewer
/// chunks than `workers` when the elements exist to go around — a cold
/// estimate must not serialize a big map.
#[must_use]
pub fn chunk_elements(
    per_element_nanos: Option<u64>,
    cold_element_nanos: u64,
    target_nanos: u64,
    n: usize,
    workers: usize,
) -> usize {
    let estimate = per_element_nanos.unwrap_or(cold_element_nanos).max(1);
    let by_cost = usize::try_from(target_nanos / estimate).unwrap_or(usize::MAX);
    let spread_cap = n.div_ceil(workers.max(1));
    by_cost.clamp(1, spread_cap.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_accumulate_and_average() {
        let mut stats = CostStats::default();
        assert_eq!(stats.per_element_nanos(), None);
        stats.record(10, 1_000);
        stats.record(10, 3_000);
        assert_eq!(stats.calls, 2);
        assert_eq!(stats.per_element_nanos(), Some(200));
    }

    #[test]
    fn chunking_targets_cost_and_never_starves_workers() {
        // 1 ms/element, 25 ms target → 25 elements/chunk…
        assert_eq!(chunk_elements(Some(1_000_000), 0, 25_000_000, 1_500, 6), 25);
        // …unless that would leave workers idle: 100 elements / 6 workers
        // caps chunks at ceil(100/6) = 17.
        assert_eq!(chunk_elements(Some(1_000_000), 0, 25_000_000, 100, 6), 17);
        // Expensive elements chunk singly.
        assert_eq!(chunk_elements(Some(60_000_000), 0, 25_000_000, 1_500, 6), 1);
        // Cold estimates use the default, and the result is never zero.
        assert_eq!(chunk_elements(None, 25_000_000, 25_000_000, 10, 4), 1);
        assert_eq!(chunk_elements(Some(u64::MAX), 0, 1, 0, 4), 1);
    }
}
