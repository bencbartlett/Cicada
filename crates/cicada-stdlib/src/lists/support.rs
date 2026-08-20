//! Test helpers shared by the list nodes' tests: slot builders, hole
//! helpers, the sealed-hash shorthand, and the holed-list strategy.

use cicada_core::marshal::{ElemSlot, IntoValue};
use cicada_core::value::{HashedValue, ValueData};

pub(crate) fn number(x: f64) -> ElemSlot {
    ElemSlot(Some(HashedValue::new(ValueData::Number(x)).unwrap()))
}

pub(crate) fn hole() -> ElemSlot {
    ElemSlot(None)
}

pub(crate) fn numbers(values: &[f64]) -> Vec<ElemSlot> {
    values.iter().map(|&x| number(x)).collect()
}

/// A list from `Some(x)` = number, `None` = absent slot.
pub(crate) fn slots(values: &[Option<f64>]) -> Vec<ElemSlot> {
    values.iter().map(|v| v.map_or_else(hole, number)).collect()
}

pub(crate) fn data(slot: &ElemSlot) -> Option<&ValueData> {
    slot.0.as_deref().map(HashedValue::data)
}

pub(crate) fn hex<V: IntoValue>(value: V) -> String {
    value.into_value().unwrap().hash().to_hex()
}

// Proptest strategy: a list with holes, drawn as Option<f64> per slot.
pub(crate) fn holed_list(
    max: usize,
) -> impl proptest::strategy::Strategy<Value = Vec<Option<f64>>> {
    proptest::collection::vec(proptest::option::of(-1.0e6..1.0e6_f64), 0..max)
}
