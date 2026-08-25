//! Scrub caching — the pure half (docs/12 §Speculative warming; DECISIONS.md
//! row 39, revised 2026-08-24; v0.1 item 5, package S1): a slider the user
//! opts in (`scrub=True` in the TEXT — never the sidecar) gets its
//! step-quantized positions pre-solved while the app is idle, nearest the
//! committed value first, so dragging it later is a cache read. This module
//! holds everything that needs no session: **eligibility** as a pure
//! function of the slider's literals, the **positions** and how they are
//! spelled (the same decimal snap the canvas widget uses, so a warmed
//! literal and a later tick build the same `NodeKey`), the
//! **nearest-first, alternating-sides order**, and the **warm queue** —
//! generic over (param, ordered value list, order), so the transport's
//! playhead-ahead warming can reuse it later. The worker that drives the
//! queues through the idle class lives in [`crate::session`].

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;

use cicada_lang::Document;
use cicada_lang::ast::{Call, Lit, Rhs, ValueExpr};

use crate::protocol::ScrubView;

/// The most positions a slider may have to scrub-cache (the ledger's
/// constant — DECISIONS.md row 39, revised 2026-08-24: 32 unless Ben says
/// otherwise). 0…1 by 0.1 is 11 positions and qualifies; 0…10 by 0.5 is
/// 21; 0…1 by 0.02 is 51 and is refused.
pub const SCRUB_MAX_POSITIONS: usize = 32;

/// The per-slider byte cap on what the warming may put in the store —
/// 256 MiB of memo entries attributed to one slider's warming, counted
/// deep from the compressed blobs it stored ([`cicada_sched::DiskStore::
/// stored_bytes`]). A queue that crosses it stops ("capped"); the
/// positions warmed before it stay warm. A session constant
/// (`SessionConfig::scrub_byte_cap` defaults to it; tests lower it).
pub const SCRUB_BYTE_CAP: u64 = 256 * 1024 * 1024;

/// The kwarg a slider's scrub queue warms — the one the ticks spell.
pub const SCRUB_PORT: &str = "value";

/// The ports that must be literals for a slider to scrub-cache: the
/// positions are a function of them.
const RANGE_PORTS: [&str; 3] = ["min", "max", "step"];

/// Why a slider cannot scrub-cache — the `set_scrub` refusal's reason and
/// the view's `ineligible`, spelled once here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ineligible {
    /// The binding is not a `slider` call (or not a binding at all).
    NotASlider,
    /// `min` / `max` / `step` are fed by wires: the positions are not a
    /// function of the text.
    Wired(Vec<String>),
    /// `step` is 0 — a continuous slider has no positions to warm.
    StepZero,
    /// `max` is below `min` (the node itself is red).
    InvertedBounds,
    /// More positions than [`SCRUB_MAX_POSITIONS`].
    TooManyPositions {
        /// `floor((max − min) / step) + 1`.
        positions: usize,
    },
}

impl fmt::Display for Ineligible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotASlider => f.write_str("not a slider — only sliders scrub-cache"),
            Self::Wired(ports) => {
                let verb = if ports.len() == 1 { "is" } else { "are" };
                write!(
                    f,
                    "{} {verb} wired — the positions are a function of literal min, max and step",
                    ports.join(" and ")
                )
            }
            Self::StepZero => {
                f.write_str("step is 0 — a continuous slider has no positions to warm")
            }
            Self::InvertedBounds => f.write_str("max is below min"),
            Self::TooManyPositions { positions } => write!(
                f,
                "too many positions ({positions} > {SCRUB_MAX_POSITIONS})"
            ),
        }
    }
}

/// The step-quantized positions of an eligible slider: `min + k × step`
/// for `k` in `0..count`, spelled the way the canvas snaps them.
#[derive(Debug, Clone, PartialEq)]
pub struct Positions {
    /// The slider's `min` literal.
    pub min: f64,
    /// The slider's `step` literal (> 0).
    pub step: f64,
    /// `floor((max − min) / step) + 1`.
    pub count: usize,
    /// Decimal places the snap keeps — the canvas widget's rule
    /// (`web/src/canvas/grid.ts::snapToStep`: the larger of `step`'s and
    /// `min`'s decimal places), so a position's value is bit-identical to
    /// the one the widget sends for that notch and the two build the same
    /// `NodeKey`.
    pub decimals: usize,
}

impl Positions {
    /// The position count for a range, or why there is none.
    ///
    /// # Errors
    ///
    /// [`Ineligible::StepZero`], [`Ineligible::InvertedBounds`],
    /// [`Ineligible::TooManyPositions`] — also for a non-finite quotient.
    pub fn for_range(min: f64, max: f64, step: f64) -> Result<Self, Ineligible> {
        // `partial_cmp`: a NaN literal is neither positive nor ordered — refused
        // like a zero step / inverted bounds, never a silent position count.
        if step.partial_cmp(&0.0) != Some(Ordering::Greater) {
            return Err(Ineligible::StepZero);
        }
        if !matches!(
            max.partial_cmp(&min),
            Some(Ordering::Greater | Ordering::Equal)
        ) {
            return Err(Ineligible::InvertedBounds);
        }
        // The contract's `floor((max − min) / step) + 1`, with the quotient
        // nudged by 1e-9 before the floor: 0…0.3 by 0.1 is 2.9999999999999996
        // in IEEE arithmetic and would count 0.3 out of its own range.
        let quotient = (max - min) / step + 1e-9;
        if !quotient.is_finite() || quotient > 1.0e9 {
            return Err(Ineligible::TooManyPositions {
                positions: usize::MAX,
            });
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 0 ≤ q ≤ 1e9
        let count = quotient.floor() as usize + 1;
        if count > SCRUB_MAX_POSITIONS {
            return Err(Ineligible::TooManyPositions { positions: count });
        }
        Ok(Self {
            min,
            step,
            count,
            decimals: decimals_of(step).max(decimals_of(min)).min(20),
        })
    }

    /// The value at position `index` — `min + index × step`, rounded to
    /// the snap's decimals exactly as the canvas does.
    #[must_use]
    pub fn value(&self, index: usize) -> f64 {
        #[allow(clippy::cast_precision_loss)] // index ≤ 32
        let raw = self.min + index as f64 * self.step;
        let snapped = format!("{raw:.prec$}", prec = self.decimals);
        snapped.parse().unwrap_or(raw)
    }

    /// The value at `index` as a dialect literal — always with a decimal
    /// point (`2.0`, not `2`: an integer literal lowers to an `Integer`
    /// value, and the widgets spell their releases `2.0`).
    #[must_use]
    pub fn literal(&self, index: usize) -> String {
        number_literal(self.value(index))
    }

    /// The position nearest `value` (the committed literal, which may sit
    /// off the grid) — the warming's starting point.
    #[must_use]
    pub fn nearest(&self, value: f64) -> usize {
        if self.count == 0 {
            return 0;
        }
        let k = ((value - self.min) / self.step).round();
        if !k.is_finite() || k <= 0.0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 0 < k, clamped below
        let k = k as usize;
        k.min(self.count - 1)
    }
}

/// A finite number as the dialect spells it: the shortest round-trip form,
/// with a decimal point kept for integral values (`numberLiteral` in
/// `tools/measure/lib.mjs` and the widgets spell them the same way).
#[must_use]
pub fn number_literal(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1.0e15 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

/// Decimal places a literal needs (`0.25` → 2, `1` → 0, `1e-4` → 4) —
/// `web/src/canvas/grid.ts::stepDecimals`, read off the shortest
/// round-trip spelling as the widget reads it off `String(step)`.
fn decimals_of(value: f64) -> usize {
    let text = format!("{value}");
    if let Some((_, exponent)) = text.split_once("e-") {
        return exponent.parse().unwrap_or(0);
    }
    text.split_once('.').map_or(0, |(_, frac)| frac.len())
}

/// The literals of an eligible slider's call, read off the document.
#[derive(Debug, Clone, PartialEq)]
pub struct SliderLiterals {
    /// `value`.
    pub value: f64,
    /// `scrub` (the opt-in), `false` when absent.
    pub scrub: bool,
    /// The positions `min`/`max`/`step` define.
    pub positions: Positions,
}

/// Eligibility of the binding `name`, read off the DOCUMENT (as
/// `viewmodel::collapse_refusal` does — inside a `batch` the view lags
/// the document): the binding's call must be a `slider`, `min`/`max`/
/// `step` literals (an absent kwarg is its default), `step > 0`, and the
/// range at most [`SCRUB_MAX_POSITIONS`] positions. `value` must be a
/// number literal too (a wired `value` is no param widget at all).
///
/// # Errors
///
/// [`Ineligible`] with the reason `set_scrub` refuses with.
pub fn eligibility(document: &Document, name: &str) -> Result<SliderLiterals, Ineligible> {
    let call = document
        .statements_including_disabled()
        .find(|(_, statement, _, _)| statement.targets.iter().any(|t| t.name == name))
        .and_then(|(_, statement, _, _)| match &statement.rhs {
            Rhs::Call(call) if call.func.name == "slider" => Some(call),
            _ => None,
        })
        .ok_or(Ineligible::NotASlider)?;
    eligibility_of_call(call)
}

/// [`eligibility`] for a `slider` call already in hand (the view-model's
/// slider arm).
///
/// # Errors
///
/// As [`eligibility`].
pub fn eligibility_of_call(call: &Call) -> Result<SliderLiterals, Ineligible> {
    let wired: Vec<String> = RANGE_PORTS
        .into_iter()
        .filter(|port| {
            call.kwargs.iter().any(|kwarg| {
                kwarg.name.name == *port && !matches!(kwarg.value.unlifted(), ValueExpr::Literal(_))
            })
        })
        .map(str::to_owned)
        .collect();
    if !wired.is_empty() {
        return Err(Ineligible::Wired(wired));
    }
    let Some(value) = number_kwarg(call, "value") else {
        return Err(Ineligible::NotASlider);
    };
    let min = number_kwarg(call, "min").unwrap_or(0.0);
    let max = number_kwarg(call, "max").unwrap_or(10.0);
    let step = number_kwarg(call, "step").unwrap_or(0.0);
    let positions = Positions::for_range(min, max, step)?;
    Ok(SliderLiterals {
        value,
        scrub: bool_kwarg(call, "scrub").unwrap_or(false),
        positions,
    })
}

/// Does the call carry the `scrub` kwarg at all (the `set_scrub off`
/// gesture removes it — the default says the same thing)?
#[must_use]
pub fn has_scrub_kwarg(document: &Document, name: &str) -> bool {
    document
        .statements_including_disabled()
        .find(|(_, statement, _, _)| statement.targets.iter().any(|t| t.name == name))
        .is_some_and(|(_, statement, _, _)| match &statement.rhs {
            Rhs::Call(call) => call.kwargs.iter().any(|k| k.name.name == "scrub"),
            _ => false,
        })
}

fn number_kwarg(call: &Call, name: &str) -> Option<f64> {
    call.kwargs
        .iter()
        .find(|k| k.name.name == name)
        .and_then(|k| match k.value.unlifted() {
            ValueExpr::Literal(lit) => match lit.lit {
                Lit::Number { value, .. } => Some(value),
                _ => None,
            },
            _ => None,
        })
}

fn bool_kwarg(call: &Call, name: &str) -> Option<bool> {
    call.kwargs
        .iter()
        .find(|k| k.name.name == name)
        .and_then(|k| match k.value.unlifted() {
            ValueExpr::Literal(lit) => match lit.lit {
                Lit::Boolean(flag) => Some(flag),
                _ => None,
            },
            _ => None,
        })
}

/// The visiting order over `count` positions starting at `center`:
/// the center, then alternating sides outward — above first (`center +
/// 1`, `center − 1`, `center + 2`, …); when one side runs out the other
/// continues. A permutation of `0..count` whose distances from `center`
/// never decrease.
#[must_use]
pub fn nearest_first(count: usize, center: usize) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    let center = center.min(count - 1);
    let mut order = Vec::with_capacity(count);
    order.push(center);
    let mut distance = 1;
    while order.len() < count {
        if let Some(above) = center.checked_add(distance)
            && above < count
        {
            order.push(above);
        }
        if let Some(below) = center.checked_sub(distance) {
            order.push(below);
        }
        distance += 1;
    }
    order
}

/// One param's warm queue: an ordered value list (as dialect literals), a
/// visiting order over it, and what the warming has done so far. Generic
/// over the param: a slider's step positions today; a `cycle`'s frames
/// (playhead-ahead first) when the transport reuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmQueue {
    /// A monotonic id the session hands out: a result that comes back for
    /// an id no live queue carries belongs to a dropped queue and is
    /// discarded.
    pub id: u64,
    /// The param's binding.
    pub node: String,
    /// Its kwarg.
    pub port: String,
    /// The values, one literal per position, in position order.
    pub values: Vec<String>,
    /// The visiting order over `values`' indices.
    pub order: Vec<usize>,
    /// Positions verified warm: a memo hit on the hash-only dry run, or
    /// solved to completion by the warming.
    pub warmed: BTreeSet<usize>,
    /// Positions the warming is done with — `warmed`, plus positions whose
    /// solve went red (never retried within this queue's life: the red is
    /// not memoized and a retry would be the same red).
    pub visited: BTreeSet<usize>,
    /// Bytes of memo entries the warming stored for this queue, deep.
    pub bytes: u64,
    /// The byte cap stopped this queue.
    pub capped: bool,
    /// The position in flight, if one is.
    pub in_flight: Option<usize>,
}

impl WarmQueue {
    /// A fresh queue over `values` in `order`.
    #[must_use]
    pub fn new(id: u64, node: &str, port: &str, values: Vec<String>, order: Vec<usize>) -> Self {
        Self {
            id,
            node: node.to_owned(),
            port: port.to_owned(),
            values,
            order,
            warmed: BTreeSet::new(),
            visited: BTreeSet::new(),
            bytes: 0,
            capped: false,
            in_flight: None,
        }
    }

    /// The next position to warm — the first of `order` not yet visited —
    /// or `None` when the queue is finished or capped.
    #[must_use]
    pub fn next(&self) -> Option<usize> {
        if self.capped {
            return None;
        }
        self.order
            .iter()
            .copied()
            .find(|index| !self.visited.contains(index))
    }

    /// Nothing left to do: every position visited, or the cap hit.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.next().is_none()
    }

    /// Work remains (the view's `warming`): not finished, not capped — the
    /// worker will get to it (it may be parked or blocked right now).
    #[must_use]
    pub fn warming(&self) -> bool {
        !self.finished()
    }

    /// Record a position as warm (`bytes` the memo entries its solve
    /// stored — 0 for a dry-run hit); applies the cap.
    pub fn record_warm(&mut self, index: usize, bytes: u64, cap: u64) {
        self.in_flight = None;
        self.visited.insert(index);
        self.warmed.insert(index);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes >= cap {
            self.capped = true;
        }
    }

    /// Record a position whose solve went red or was refused: done with,
    /// not warm.
    pub fn record_red(&mut self, index: usize) {
        self.in_flight = None;
        self.visited.insert(index);
    }

    /// The position came back pre-empted (cancelled): nothing recorded, it
    /// is next again.
    pub fn record_preempted(&mut self) {
        self.in_flight = None;
    }

    /// The wire view of this queue's progress over an eligible slider's
    /// `on` flag.
    #[must_use]
    pub fn view(&self, on: bool) -> ScrubView {
        ScrubView {
            on,
            positions: self.values.len(),
            warmed: self.warmed.iter().copied().collect(),
            warming: self.warming(),
            bytes: self.bytes,
            capped: self.capped,
            ineligible: None,
        }
    }
}

/// The view of a slider that is eligible but has no queue (the text says
/// `scrub=False`): positions known, nothing warm, nothing warming.
#[must_use]
pub fn idle_view(positions: usize, on: bool) -> ScrubView {
    ScrubView {
        on,
        positions,
        warmed: Vec::new(),
        warming: false,
        bytes: 0,
        capped: false,
        ineligible: None,
    }
}

/// The view of an ineligible slider: the reason, and what the text says.
#[must_use]
pub fn ineligible_view(on: bool, why: &Ineligible) -> ScrubView {
    ScrubView {
        on,
        positions: 0,
        warmed: Vec::new(),
        warming: false,
        bytes: 0,
        capped: false,
        ineligible: Some(why.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(source: &str) -> Document {
        Document::parse(source)
    }

    // The contract's three worked examples and its boundaries: 32 is the
    // last eligible count, 33 the first refused.
    #[test]
    fn position_counts_follow_the_contract() {
        assert_eq!(Positions::for_range(0.0, 1.0, 0.1).unwrap().count, 11);
        assert_eq!(Positions::for_range(0.0, 10.0, 0.5).unwrap().count, 21);
        assert_eq!(
            Positions::for_range(0.0, 1.0, 0.02),
            Err(Ineligible::TooManyPositions { positions: 51 })
        );
        assert_eq!(Positions::for_range(0.0, 31.0, 1.0).unwrap().count, 32);
        assert_eq!(
            Positions::for_range(0.0, 32.0, 1.0),
            Err(Ineligible::TooManyPositions { positions: 33 })
        );
        assert_eq!(
            Positions::for_range(0.0, 1.0, 0.0),
            Err(Ineligible::StepZero)
        );
        assert_eq!(
            Positions::for_range(0.0, 1.0, -0.5),
            Err(Ineligible::StepZero)
        );
        assert_eq!(
            Positions::for_range(1.0, 0.0, 0.5),
            Err(Ineligible::InvertedBounds)
        );
        // A range of one position (min == max) is eligible.
        assert_eq!(Positions::for_range(2.0, 2.0, 0.5).unwrap().count, 1);
        // The IEEE nudge: 0…0.3 by 0.1 counts 0.3 (the quotient is
        // 2.9999999999999996 without it).
        assert_eq!(Positions::for_range(0.0, 0.3, 0.1).unwrap().count, 4);
        // 02-solids' cone slider: 0.5…5.0 by 0.25.
        assert_eq!(Positions::for_range(0.5, 5.0, 0.25).unwrap().count, 19);
    }

    // The spelled positions are the canvas widget's snapped values
    // (grid.ts: `Number((min + k·step).toFixed(decimals))`), so a warmed
    // literal and the widget's tick build the same key.
    #[test]
    fn positions_are_spelled_as_the_widget_snaps_them() {
        let p = Positions::for_range(0.0, 1.0, 0.1).unwrap();
        let spelled: Vec<String> = (0..p.count).map(|i| p.literal(i)).collect();
        assert_eq!(
            spelled,
            [
                "0.0", "0.1", "0.2", "0.3", "0.4", "0.5", "0.6", "0.7", "0.8", "0.9", "1.0"
            ]
        );
        let p = Positions::for_range(0.5, 5.0, 0.25).unwrap();
        assert_eq!(p.decimals, 2);
        assert_eq!(p.literal(0), "0.5");
        assert_eq!(p.literal(1), "0.75");
        assert_eq!(p.literal(6), "2.0");
        assert_eq!(p.literal(18), "5.0");
        // min's decimals win when they are finer than step's.
        let p = Positions::for_range(0.125, 1.125, 0.5).unwrap();
        assert_eq!(p.decimals, 3);
        assert_eq!(p.literal(1), "0.625");
        // The snap kills the multiply's noise: 0.1 × 3 is 0.30000000000000004.
        let p = Positions::for_range(0.0, 3.0, 0.1).unwrap();
        assert_eq!(p.value(3).to_bits(), 0.3_f64.to_bits());
        assert_eq!(number_literal(2.0), "2.0");
        assert_eq!(number_literal(-3.0), "-3.0");
        assert_eq!(number_literal(0.75), "0.75");
    }

    #[test]
    fn nearest_position_clamps_into_the_range() {
        let p = Positions::for_range(0.5, 5.0, 0.25).unwrap();
        assert_eq!(p.nearest(2.0), 6);
        assert_eq!(p.nearest(2.1), 6, "off-grid rounds to the nearest notch");
        assert_eq!(p.nearest(2.13), 7);
        assert_eq!(p.nearest(-4.0), 0);
        assert_eq!(p.nearest(99.0), 18);
        assert_eq!(p.nearest(f64::NAN), 0);
    }

    #[test]
    fn nearest_first_alternates_sides_and_finishes_the_longer_side() {
        assert_eq!(nearest_first(10, 3), [3, 4, 2, 5, 1, 6, 0, 7, 8, 9]);
        assert_eq!(nearest_first(5, 0), [0, 1, 2, 3, 4]);
        assert_eq!(nearest_first(5, 4), [4, 3, 2, 1, 0]);
        assert_eq!(nearest_first(1, 0), [0]);
        assert_eq!(nearest_first(0, 0), Vec::<usize>::new());
        assert_eq!(
            nearest_first(3, 99),
            [2, 1, 0],
            "a center beyond the end clamps"
        );
    }

    proptest::proptest! {
        // Nearest-first over random ranges: the order is a permutation of
        // the positions, starts at the committed value's notch, never
        // steps to a farther position before a nearer one, and alternates
        // sides while both have positions left.
        #[test]
        fn nearest_first_is_a_nearest_first_permutation(
            min in -1000.0..1000.0_f64,
            span in 0.0..100.0_f64,
            step_index in 0_usize..6,
            t in 0.0..=1.0_f64,
        ) {
            let step = [0.1, 0.25, 0.5, 1.0, 2.5, 5.0][step_index];
            let max = min + span;
            let Ok(positions) = Positions::for_range(min, max, step) else {
                // Too many positions for this span/step: refused, nothing to order.
                return Ok(());
            };
            let value = min + span * t;
            let center = positions.nearest(value);
            let order = nearest_first(positions.count, center);
            let mut sorted = order.clone();
            sorted.sort_unstable();
            sorted.dedup();
            proptest::prop_assert_eq!(sorted.len(), positions.count, "a permutation: {:?}", order);
            proptest::prop_assert_eq!(order[0], center);
            let distance = |i: usize| i.abs_diff(center);
            for pair in order.windows(2) {
                proptest::prop_assert!(distance(pair[0]) <= distance(pair[1]), "{:?}", order);
                if distance(pair[0]) == distance(pair[1]) && distance(pair[0]) > 0 {
                    proptest::prop_assert!(
                        (pair[0] > center) != (pair[1] > center),
                        "equal distances sit on opposite sides: {:?}",
                        order
                    );
                }
            }
            // The center is the notch nearest the value.
            let best = (0..positions.count)
                .min_by(|&a, &b| {
                    (positions.value(a) - value).abs()
                        .total_cmp(&(positions.value(b) - value).abs())
                })
                .unwrap();
            proptest::prop_assert!(
                (positions.value(best) - value).abs() >= (positions.value(center) - value).abs() - step * 1e-9,
                "center {} is not nearest {} (best {})", center, value, best
            );
        }
    }

    #[test]
    fn eligibility_reads_the_slider_off_the_document() {
        let d = doc("# cicada 1\n\
             n = 4.0\n\
             a = slider(value=2.0, min=0.5, max=5.0, step=0.25, scrub=True)\n\
             b = slider(value=0.5, min=0.0, max=1.0, step=0.02)\n\
             c = slider(value=0.5, min=0.0, max=n, step=0.1)\n\
             d = slider(value=0.5)\n\
             e = slider(value=0.5, min=n, max=n, step=0.1)\n\
             f = box(x=n, y=n, z=n)\n");
        let a = eligibility(&d, "a").unwrap();
        assert_eq!((a.value, a.scrub, a.positions.count), (2.0, true, 19));
        let b = eligibility(&d, "b").unwrap_err();
        assert_eq!(b, Ineligible::TooManyPositions { positions: 51 });
        assert_eq!(b.to_string(), "too many positions (51 > 32)");
        let c = eligibility(&d, "c").unwrap_err();
        assert_eq!(c, Ineligible::Wired(vec!["max".into()]));
        assert_eq!(
            c.to_string(),
            "max is wired — the positions are a function of literal min, max and step"
        );
        assert_eq!(eligibility(&d, "d"), Err(Ineligible::StepZero));
        assert_eq!(
            eligibility(&d, "e").unwrap_err().to_string(),
            "min and max are wired — the positions are a function of literal min, max and step"
        );
        assert_eq!(eligibility(&d, "f"), Err(Ineligible::NotASlider));
        assert_eq!(eligibility(&d, "nope"), Err(Ineligible::NotASlider));
        assert_eq!(eligibility(&d, "n"), Err(Ineligible::NotASlider));
        assert!(has_scrub_kwarg(&d, "a"));
        assert!(!has_scrub_kwarg(&d, "b"));
    }

    #[test]
    fn the_queue_walks_its_order_applies_the_cap_and_views_itself() {
        let values: Vec<String> = (0..5).map(|i| format!("{i}.0")).collect();
        let mut q = WarmQueue::new(7, "size", "value", values, nearest_first(5, 2));
        assert_eq!(q.next(), Some(2));
        assert!(q.warming());
        q.record_warm(2, 10, 100);
        assert_eq!(q.next(), Some(3));
        q.record_red(3);
        assert_eq!(q.next(), Some(1));
        assert_eq!(q.warmed.iter().copied().collect::<Vec<_>>(), [2]);
        q.in_flight = Some(1);
        q.record_preempted();
        assert_eq!(q.in_flight, None);
        assert_eq!(q.next(), Some(1), "a pre-empted position is next again");
        q.record_warm(1, 95, 100);
        assert!(q.capped, "10 + 95 ≥ 100");
        assert_eq!(q.next(), None);
        assert!(q.finished() && !q.warming());
        let view = q.view(true);
        assert_eq!(
            view,
            ScrubView {
                on: true,
                positions: 5,
                warmed: vec![1, 2],
                warming: false,
                bytes: 105,
                capped: true,
                ineligible: None,
            }
        );
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({"on": true, "positions": 5, "warmed": [1, 2], "warming": false, "bytes": 105, "capped": true})
        );
        assert_eq!(
            serde_json::to_value(idle_view(19, false)).unwrap(),
            serde_json::json!({"on": false, "positions": 19, "warmed": [], "warming": false, "bytes": 0})
        );
        assert_eq!(
            serde_json::to_value(ineligible_view(true, &Ineligible::StepZero)).unwrap(),
            serde_json::json!({"on": true, "positions": 0, "warmed": [], "warming": false, "bytes": 0,
                "ineligible": "step is 0 — a continuous slider has no positions to warm"})
        );
    }
}
