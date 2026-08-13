//! Time, injected: the scheduler measures cost through a [`Clock`] so
//! production reads a monotonic wall clock and tests run on **virtual
//! time** — fake nodes advance a counter, and cost/cancellation assertions
//! run in milliseconds of real time regardless of the virtual workload
//! (doc 14 §Testing standards: no sleeps, no wall-clock).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// A monotonic nanosecond source with an arbitrary epoch. Only differences
/// are meaningful.
pub trait Clock: Send + Sync {
    /// Nanoseconds since the clock's epoch.
    fn now_nanos(&self) -> u64;
}

/// Production clock: monotonic, anchored at construction.
#[derive(Debug)]
pub struct MonotonicClock {
    start: Instant,
}

impl MonotonicClock {
    /// A clock anchored now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now_nanos(&self) -> u64 {
        // 2^64 ns ≈ 584 years of uptime; saturate rather than wrap.
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// Test clock: virtual nanoseconds, advanced explicitly by fake nodes.
/// Thread-safe — parallel fake chunks advance it concurrently, and the sum
/// of advances is exact regardless of interleaving.
#[derive(Debug, Default)]
pub struct VirtualClock {
    nanos: AtomicU64,
}

impl VirtualClock {
    /// A clock at virtual zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance virtual time by `nanos` (a fake node "doing work").
    pub fn advance(&self, nanos: u64) {
        self.nanos.fetch_add(nanos, Ordering::SeqCst);
    }
}

impl Clock for VirtualClock {
    fn now_nanos(&self) -> u64 {
        self.nanos.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_clock_advances_exactly() {
        let clock = VirtualClock::new();
        assert_eq!(clock.now_nanos(), 0);
        clock.advance(25);
        clock.advance(17);
        assert_eq!(clock.now_nanos(), 42);
    }

    #[test]
    fn monotonic_clock_is_monotonic() {
        let clock = MonotonicClock::new();
        let a = clock.now_nanos();
        let b = clock.now_nanos();
        assert!(b >= a);
    }
}
