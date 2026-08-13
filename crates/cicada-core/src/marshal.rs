//! Value marshalling for node invocation (stage 3): the bridge between
//! [`HashedValue`]s on wires and the typed structs of the node ABI
//! (struct-in/struct-out, DECISIONS.md).
//!
//! `#[derive(Ports)]` generates [`FromValues`]/[`IntoValues`] impls for port
//! structs (defaults inlined as typed Rust expressions — never re-parsed
//! from catalog strings); `#[node]` combines them into one [`ErasedInvoke`]
//! registered next to the `NodeSpec`, so the scheduler dispatches any node
//! by name with zero hand-written glue.
//!
//! Failing loudly is the contract: a wrong kind, an absent `Optional` slot
//! feeding a present-only port, or an `Integer` that does not convert
//! exactly to `Number` all refuse with a typed error — never a silent
//! coercion (Integer → Number is the checker's sanctioned widening, so it
//! converts here, but only when the conversion is exact).

use std::sync::Arc;

use crate::scalar::{Color, Domain, IndexMap};
use crate::spatial::{Plane, Point, Vector, Xform};
use crate::value::{HashedValue, List, ValueData, ValueError};

/// Why one value refused to convert to a Rust type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FromValueError {
    /// The value's kind does not fit the requested type.
    #[error("expected {expected}, got {got}")]
    Kind {
        /// Catalog notation of the requested type.
        expected: String,
        /// Kind name of the value actually present.
        got: &'static str,
    },
    /// A list slot is an absent `Optional` element, but the requested type
    /// wants elements present (`compact` removes the holes, docs/09).
    #[error("list slot {index} is an absent Optional element")]
    Hole {
        /// Slot index of the hole.
        index: usize,
    },
    /// One list element refused to convert.
    #[error("element {index}: {source}")]
    Element {
        /// Slot index of the offending element.
        index: usize,
        /// Why it refused.
        source: Box<FromValueError>,
    },
    /// An `Integer` fed a `Number` port but does not convert exactly —
    /// beyond ±2^53 the widening would silently lose precision, and a
    /// wrong answer is worse than a loud refusal.
    #[error("Integer {value} does not convert exactly to Number")]
    LossyInteger {
        /// The unconvertible value.
        value: i64,
    },
}

/// Converts a wire value into a Rust port type. Implemented for the core
/// leaf kinds and composed structurally (`Vec<T>` = list level, `Option<T>`
/// = element optionality) — mirroring [`PortTyped`](crate::spec::PortTyped).
pub trait FromValue: Sized {
    /// Catalog notation for error messages (`Number`, `[Point]`).
    fn expected() -> String;

    /// Convert, refusing loudly on any mismatch.
    ///
    /// # Errors
    ///
    /// [`FromValueError`] when the value's kind, shape, or precision does
    /// not fit.
    fn from_value(value: &HashedValue) -> Result<Self, FromValueError>;

    /// What an absent list slot converts to, when this type accepts
    /// absence. `None` for every type except `Option<T>` — a hole feeding
    /// a present-only element type is a loud [`FromValueError::Hole`].
    #[must_use]
    fn from_absent() -> Option<Self> {
        None
    }
}

/// Converts a Rust port value into a wire value.
pub trait IntoValue {
    /// Convert. The only failure is value construction itself refusing
    /// (a NaN produced by a node — docs/12 loud refusal).
    ///
    /// # Errors
    ///
    /// [`ValueError`] when the produced value is invalid (NaN).
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError>;
}

/// Why a node invocation failed to marshal. The scheduler attaches the
/// producing node's name; ports are named here.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvokeError {
    /// The caller supplied the wrong number of input slots — a lowering
    /// bug, surfaced loudly rather than misaligning ports.
    #[error("node given {got} input slots; its spec has {want} ports")]
    Arity {
        /// Ports in the spec.
        want: usize,
        /// Slots supplied.
        got: usize,
    },
    /// A required port (no default) received no value. The checker reports
    /// this before any solve; reaching it here is a lowering bug.
    #[error("required port `{port}` has no value")]
    Missing {
        /// The port.
        port: &'static str,
    },
    /// An input value refused to convert.
    #[error("port `{port}`: {source}")]
    Input {
        /// The port.
        port: &'static str,
        /// Why.
        source: FromValueError,
    },
    /// An output value refused construction (NaN — docs/12 loud refusal).
    #[error("output `{port}`: {source}")]
    Output {
        /// The output port.
        port: &'static str,
        /// Why.
        source: ValueError,
    },
}

/// Builds a node's input struct from per-port wire values, in spec port
/// order. `None` means "use the port's default" (required ports refuse).
/// Generated by `#[derive(Ports)]`.
pub trait FromValues: Sized {
    /// Convert one slot per port, defaults filled in.
    ///
    /// # Errors
    ///
    /// [`InvokeError`] on arity/kind/missing-value problems.
    fn from_values(values: &[Option<Arc<HashedValue>>]) -> Result<Self, InvokeError>;
}

/// Converts a node's return value into per-port wire values, in spec output
/// order. Generated by `#[derive(Ports)]` for output structs; implemented
/// here for single-`out` returns and `()` sinks (mirroring
/// [`AsOutputs`](crate::spec::AsOutputs)).
pub trait IntoValues {
    /// Convert to one value per output port.
    ///
    /// # Errors
    ///
    /// [`InvokeError::Output`] when a produced value is invalid (NaN).
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError>;
}

/// A type-erased node invocation: per-port inputs in, per-port outputs out.
/// Registered by `#[node]` alongside the spec; the scheduler's dispatch
/// currency.
pub type ErasedInvoke =
    fn(&[Option<Arc<HashedValue>>]) -> Result<Vec<Arc<HashedValue>>, InvokeError>;

// ---------------------------------------------------------------- leaves --

/// Implements [`FromValue`]/[`IntoValue`] (and the single-`out`
/// [`IntoValues`]) for one leaf kind backed by one `ValueData` variant.
macro_rules! impl_marshal_leaf {
    ($ty:ty, $variant:ident, $name:literal) => {
        impl FromValue for $ty {
            fn expected() -> String {
                $name.to_owned()
            }

            fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
                match value.data() {
                    // The macro serves Copy and non-Copy kinds alike.
                    #[allow(clippy::clone_on_copy)]
                    ValueData::$variant(x) => Ok(x.clone()),
                    other => Err(FromValueError::Kind {
                        expected: $name.to_owned(),
                        got: other.kind_name(),
                    }),
                }
            }
        }

        impl IntoValue for $ty {
            fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
                HashedValue::new(ValueData::$variant(self))
            }
        }

        impl IntoValues for $ty {
            fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
                Ok(vec![self.into_value().map_err(|source| {
                    InvokeError::Output {
                        port: "out",
                        source,
                    }
                })?])
            }
        }
    };
}

impl_marshal_leaf!(i64, Integer, "Integer");
impl_marshal_leaf!(bool, Boolean, "Boolean");
impl_marshal_leaf!(Color, Color, "Color");
impl_marshal_leaf!(Domain, Domain, "Domain");
impl_marshal_leaf!(IndexMap, IndexMap, "IndexMap");
impl_marshal_leaf!(Point, Point, "Point");
impl_marshal_leaf!(Vector, Vector, "Vector");
impl_marshal_leaf!(Plane, Plane, "Plane");
impl_marshal_leaf!(Xform, Xform, "Xform");

/// Exact Integer → Number widening, or `None` when the conversion would
/// shift the value. The naive roundtrip test `(i as f64) as i64 == i` is
/// WRONG at exactly one value: `i64::MAX` rounds up to 2^63, and the
/// saturating f64→i64 cast maps 2^63 back to `i64::MAX`, masquerading as
/// exact. The upper-bound guard closes that hole (−2^63 is a power of two,
/// exactly representable — the bottom end never saturates). The single
/// widening rule for every call site: forking this check is how the bug
/// happened twice.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn integer_to_number_exact(i: i64) -> Option<f64> {
    let x = i as f64;
    if x < 9_223_372_036_854_775_808.0 && x as i64 == i {
        Some(x)
    } else {
        None
    }
}

// f64 by hand: it also accepts Integer, the checker's sanctioned implicit
// widening (docs/02) — exact conversions only.
impl FromValue for f64 {
    fn expected() -> String {
        "Number".to_owned()
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        match value.data() {
            ValueData::Number(x) => Ok(*x),
            // Loud refusal beats a silently shifted number.
            ValueData::Integer(i) => {
                integer_to_number_exact(*i).ok_or(FromValueError::LossyInteger { value: *i })
            }
            other => Err(FromValueError::Kind {
                expected: "Number".to_owned(),
                got: other.kind_name(),
            }),
        }
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        HashedValue::new(ValueData::Number(self))
    }
}

impl IntoValues for f64 {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.into_value().map_err(|source| {
            InvokeError::Output {
                port: "out",
                source,
            }
        })?])
    }
}

// String by hand: the value model stores Arc<str>.
impl FromValue for String {
    fn expected() -> String {
        "Text".to_owned()
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        match value.data() {
            ValueData::Text(s) => Ok(s.as_ref().to_owned()),
            other => Err(FromValueError::Kind {
                expected: "Text".to_owned(),
                got: other.kind_name(),
            }),
        }
    }
}

impl IntoValue for String {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        HashedValue::new(ValueData::Text(Arc::from(self.as_str())))
    }
}

impl IntoValues for String {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.into_value().map_err(|source| {
            InvokeError::Output {
                port: "out",
                source,
            }
        })?])
    }
}

// ------------------------------------------------------------ structural --

impl<T: FromValue> FromValue for Vec<T> {
    fn expected() -> String {
        format!("[{}]", T::expected())
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        let ValueData::List(list) = value.data() else {
            return Err(FromValueError::Kind {
                expected: Self::expected(),
                got: value.data().kind_name(),
            });
        };
        list.slots
            .iter()
            .enumerate()
            .map(|(index, slot)| match slot {
                None => T::from_absent().ok_or(FromValueError::Hole { index }),
                Some(element) => T::from_value(element).map_err(|source| FromValueError::Element {
                    index,
                    source: Box::new(source),
                }),
            })
            .collect()
    }
}

impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        let slots = self
            .into_iter()
            .map(|element| element.into_value().map(Some))
            .collect::<Result<_, _>>()?;
        HashedValue::new(ValueData::List(List { axis: None, slots }))
    }
}

impl<T: IntoValue> IntoValues for Vec<T> {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.into_value().map_err(|source| {
            InvokeError::Output {
                port: "out",
                source,
            }
        })?])
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn expected() -> String {
        format!("{}?", T::expected())
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        match value.data() {
            ValueData::Nothing => Ok(None),
            _ => T::from_value(value).map(Some),
        }
    }

    fn from_absent() -> Option<Self> {
        Some(None)
    }
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        match self {
            // Inside a list, HashedValue canonicalization folds a sealed
            // Nothing element into a None slot — one spelling of absent.
            None => HashedValue::new(ValueData::Nothing),
            Some(inner) => inner.into_value(),
        }
    }
}

impl<T: IntoValue> IntoValues for Option<T> {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.into_value().map_err(|source| {
            InvokeError::Output {
                port: "out",
                source,
            }
        })?])
    }
}

impl IntoValues for () {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
// Exact float `==` is sanctioned here: marshalling's contract is exact
// bit-preserving pass-through (ledger revision 2026-08-12).
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn number(x: f64) -> Arc<HashedValue> {
        HashedValue::new(ValueData::Number(x)).unwrap()
    }

    fn integer(i: i64) -> Arc<HashedValue> {
        HashedValue::new(ValueData::Integer(i)).unwrap()
    }

    #[test]
    fn leaf_roundtrips() {
        let v = 4.25_f64.into_value().unwrap();
        assert_eq!(f64::from_value(&v).unwrap(), 4.25);
        let v = 7_i64.into_value().unwrap();
        assert_eq!(i64::from_value(&v).unwrap(), 7);
        let v = String::from("cicada").into_value().unwrap();
        assert_eq!(String::from_value(&v).unwrap(), "cicada");
        let v = Point::new(1.0, 2.0, 3.0).into_value().unwrap();
        assert_eq!(Point::from_value(&v).unwrap(), Point::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn integer_widens_to_number_exactly() {
        assert_eq!(f64::from_value(&integer(10)).unwrap(), 10.0);
        // 2^53 + 1 does not convert exactly — loud refusal.
        let big = (1_i64 << 53) + 1;
        assert_eq!(
            f64::from_value(&integer(big)),
            Err(FromValueError::LossyInteger { value: big })
        );
    }

    // Regression (adversarial review, stage 3): i64::MAX rounds up to 2^63
    // and the SATURATING f64→i64 cast maps it back, so a naive roundtrip
    // check accepts a value that shifted by one. Every boundary pinned.
    #[test]
    fn integer_widening_boundaries_are_exact() {
        // i64::MAX must refuse — its widening is off by one.
        assert_eq!(
            f64::from_value(&integer(i64::MAX)),
            Err(FromValueError::LossyInteger { value: i64::MAX })
        );
        assert_eq!(integer_to_number_exact(i64::MAX), None);
        // i64::MIN is −2^63, a power of two: exactly representable.
        assert_eq!(
            f64::from_value(&integer(i64::MIN)).unwrap(),
            -(2f64.powi(63))
        );
        // Non-representable neighbors refuse; representable powers pass.
        assert_eq!(integer_to_number_exact(i64::MIN + 1), None);
        assert_eq!(integer_to_number_exact(i64::MAX - 1), None);
        assert_eq!(integer_to_number_exact(1 << 62), Some(2f64.powi(62)));
        assert_eq!(integer_to_number_exact(1 << 53), Some(2f64.powi(53)));
        assert_eq!(integer_to_number_exact((1 << 53) + 1), None);
    }

    #[test]
    fn number_does_not_narrow_to_integer() {
        assert!(matches!(
            i64::from_value(&number(1.0)),
            Err(FromValueError::Kind { .. })
        ));
    }

    #[test]
    fn vec_roundtrip_and_hole_refusal() {
        let v = vec![1.0, 2.5].into_value().unwrap();
        assert_eq!(Vec::<f64>::from_value(&v).unwrap(), vec![1.0, 2.5]);

        let holed = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(number(1.0)), None],
        }))
        .unwrap();
        assert_eq!(
            Vec::<f64>::from_value(&holed),
            Err(FromValueError::Hole { index: 1 })
        );
        // The optional-element form accepts the hole.
        assert_eq!(
            Vec::<Option<f64>>::from_value(&holed).unwrap(),
            vec![Some(1.0), None]
        );
    }

    #[test]
    fn optional_output_none_becomes_a_hole_in_lists() {
        let v = vec![Some(1.0_f64), None].into_value().unwrap();
        let ValueData::List(list) = v.data() else {
            panic!("wrong variant")
        };
        assert!(list.slots[1].is_none(), "canonicalized to a None slot");
    }

    #[test]
    fn nan_output_is_refused() {
        assert!(f64::NAN.into_value().is_err());
        assert!(matches!(
            f64::NAN.into_values(),
            Err(InvokeError::Output { port: "out", .. })
        ));
    }

    #[test]
    fn element_error_carries_the_index() {
        let mixed = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(number(1.0)), Some(integer((1 << 53) + 1))],
        }))
        .unwrap();
        assert!(matches!(
            Vec::<f64>::from_value(&mixed),
            Err(FromValueError::Element { index: 1, .. })
        ));
    }
}
