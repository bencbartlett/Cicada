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

use crate::geometry::{Closed, Curve, GeometryValue, Mesh, Solid, Transformable, Watertight};
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
    /// The value's kind fits but its data fails the port's refinement
    /// predicate (an open curve into `Closed<Curve>`, a leaky mesh into
    /// `Watertight<Mesh>`). The checker prevents this wiring statically;
    /// hitting it means a producer broke its contract — refused loudly.
    #[error("value does not satisfy {refinement}: {reason}")]
    Unrefined {
        /// The refinement's catalog name.
        refinement: &'static str,
        /// Why the predicate failed.
        reason: String,
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

    /// Convert from the sealed `Arc` — the calling convention of generated
    /// marshalling code. Defaults to [`Self::from_value`]; pass-through
    /// types ([`AnyValue`], [`ElemSlot`]) override it to clone the `Arc`,
    /// so an `Any`/`E` port never re-hashes a mesh-sized payload.
    ///
    /// # Errors
    ///
    /// As [`Self::from_value`].
    fn from_arc(value: &Arc<HashedValue>) -> Result<Self, FromValueError> {
        Self::from_value(value)
    }

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

/// A type-erased node invocation: project config + per-port inputs in,
/// per-port outputs out. Registered by `#[node]` alongside the spec; the
/// scheduler's dispatch currency.
///
/// The config parameter is how `uses_tolerance` nodes read tolerance —
/// explicit state passed explicitly, never ambient (DECISIONS.md tolerance
/// row). Nodes not declaring `uses_tolerance` never see it (their shim
/// ignores it), so a node cannot consult tolerance without also folding
/// the tolerance hash into its `NodeKey`.
pub type ErasedInvoke = fn(
    &crate::config::ProjectConfig,
    &[Option<Arc<HashedValue>>],
) -> Result<Vec<Arc<HashedValue>>, InvokeError>;

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
impl_marshal_leaf!(Curve, Curve, "Curve");
impl_marshal_leaf!(Mesh, Mesh, "Mesh");
impl_marshal_leaf!(Solid, Solid, "Solid");

// ------------------------------------------------- refinement wrappers --
//
// On the wire a refined value is the PLAIN base kind (a circle and a
// `Closed<Curve>` circle are one value, one hash — interning and early
// cutoff depend on that). The wrapper exists at the port-type level; the
// predicate is re-verified on the way IN (the wire boundary takes no one's
// word) and debug-asserted on the way OUT (our own constructors are the
// tested parties). `is_closed` is O(1); `is_watertight` is O(edges) —
// measured negligible next to any node that consumes a mesh; revisit only
// on profiling evidence.

impl FromValue for Closed<Curve> {
    fn expected() -> String {
        "Closed<Curve>".to_owned()
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        let curve = Curve::from_value(value).map_err(|_| FromValueError::Kind {
            expected: "Closed<Curve>".to_owned(),
            got: value.data().kind_name(),
        })?;
        if curve.is_closed() {
            Ok(Self(curve))
        } else {
            Err(FromValueError::Unrefined {
                refinement: "Closed<Curve>",
                reason: format!("{} is open", curve.variant_name()),
            })
        }
    }
}

impl IntoValue for Closed<Curve> {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        debug_assert!(
            self.0.is_closed(),
            "a node produced Closed<Curve> around an open {}",
            self.0.variant_name()
        );
        HashedValue::new(ValueData::Curve(self.0))
    }
}

impl FromValue for Watertight<Mesh> {
    fn expected() -> String {
        "Watertight<Mesh>".to_owned()
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        let mesh = Mesh::from_value(value).map_err(|_| FromValueError::Kind {
            expected: "Watertight<Mesh>".to_owned(),
            got: value.data().kind_name(),
        })?;
        if mesh.is_watertight() {
            Ok(Self(mesh))
        } else {
            Err(FromValueError::Unrefined {
                refinement: "Watertight<Mesh>",
                reason: format!(
                    "mesh ({} triangles) has open or inconsistently oriented edges",
                    mesh.triangle_count()
                ),
            })
        }
    }
}

impl IntoValue for Watertight<Mesh> {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        debug_assert!(
            self.0.is_watertight(),
            "a node produced Watertight<Mesh> around a leaky mesh"
        );
        HashedValue::new(ValueData::Mesh(self.0))
    }
}

impl IntoValues for Closed<Curve> {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.into_value().map_err(|source| {
            InvokeError::Output {
                port: "out",
                source,
            }
        })?])
    }
}

impl IntoValues for Watertight<Mesh> {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.into_value().map_err(|source| {
            InvokeError::Output {
                port: "out",
                source,
            }
        })?])
    }
}

// ------------------------------------------- runtime-polymorphic ports --

/// A pass-through wire value for `Any` ports (docs/08 Panel): accepts
/// every kind, converts nothing. Distinct from [`ElemSlot`]: `Any` never
/// binds a type variable.
#[derive(Debug, Clone, PartialEq)]
pub struct AnyValue(pub Arc<HashedValue>);

/// Re-seal a value's data into a fresh `Arc` — the direct-call fallback of
/// the pass-through types (generated marshalling goes through `from_arc`,
/// which just clones the `Arc`). Re-sealing already-canonical data cannot
/// fail.
fn reseal(value: &HashedValue) -> Arc<HashedValue> {
    HashedValue::new(value.data().clone())
        .unwrap_or_else(|_| unreachable!("re-sealing an already-sealed value cannot fail"))
}

impl FromValue for AnyValue {
    fn expected() -> String {
        "Any".to_owned()
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        Ok(Self(reseal(value)))
    }

    fn from_arc(value: &Arc<HashedValue>) -> Result<Self, FromValueError> {
        Ok(Self(Arc::clone(value)))
    }
}

impl IntoValue for AnyValue {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        Ok(self.0)
    }
}

impl IntoValues for AnyValue {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.0])
    }
}

/// One slot of an `E`-variable port (list nodes: `item(list: [E]) → E`,
/// `flatten(list: [[E]]) → [E]`): a pass-through wire value of ANY kind, or
/// an absent slot (`None`). `E` is the element type *including its
/// optionality* — the CHECKER binds `E` per call from the wired value's kind
/// AND its `?`, so a `[Point?]` flows through the list combinators as
/// `[Point?]` and a `[Point]` as `[Point]` (slot-preserving nulls, docs/08
/// rule 6), while nodes stay total over holes at runtime. An absent slot
/// marshals out as `Nothing`, which list canonicalization folds back into a
/// hole.
#[derive(Debug, Clone, PartialEq)]
pub struct ElemSlot(pub Option<Arc<HashedValue>>);

impl ElemSlot {
    /// Is the slot present?
    #[must_use]
    pub const fn is_present(&self) -> bool {
        self.0.is_some()
    }
}

impl FromValue for ElemSlot {
    fn expected() -> String {
        "E".to_owned()
    }

    fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
        Ok(Self(match value.data() {
            ValueData::Nothing => None,
            _ => Some(reseal(value)),
        }))
    }

    fn from_arc(value: &Arc<HashedValue>) -> Result<Self, FromValueError> {
        Ok(Self(match value.data() {
            ValueData::Nothing => None,
            _ => Some(Arc::clone(value)),
        }))
    }

    fn from_absent() -> Option<Self> {
        Some(Self(None))
    }
}

impl IntoValue for ElemSlot {
    fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
        match self.0 {
            // Inside a list, HashedValue canonicalization folds a sealed
            // Nothing element into a None slot — one spelling of absent.
            None => HashedValue::new(ValueData::Nothing),
            Some(value) => Ok(value),
        }
    }
}

impl IntoValues for ElemSlot {
    fn into_values(self) -> Result<Vec<Arc<HashedValue>>, InvokeError> {
        Ok(vec![self.into_value().map_err(|source| {
            InvokeError::Output {
                port: "out",
                source,
            }
        })?])
    }
}

/// One marshalling table for the two runtime-dispatched geometry enums.
macro_rules! impl_marshal_geometry_enum {
    ($ty:ident, $name:literal, [$(($variant:ident, $data:ident)),+ $(,)?]) => {
        impl FromValue for $ty {
            fn expected() -> String {
                $name.to_owned()
            }

            fn from_value(value: &HashedValue) -> Result<Self, FromValueError> {
                match value.data() {
                    $(ValueData::$data(x) => Ok(Self::$variant(x.clone())),)+
                    other => Err(FromValueError::Kind {
                        expected: $name.to_owned(),
                        got: other.kind_name(),
                    }),
                }
            }
        }

        impl IntoValue for $ty {
            fn into_value(self) -> Result<Arc<HashedValue>, ValueError> {
                match self {
                    $(Self::$variant(x) => HashedValue::new(ValueData::$data(x)),)+
                }
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

impl_marshal_geometry_enum!(
    Transformable,
    "T",
    [
        (Point, Point),
        (Vector, Vector),
        (Plane, Plane),
        (Curve, Curve),
        (Mesh, Mesh),
        (Solid, Solid),
    ]
);
impl_marshal_geometry_enum!(
    GeometryValue,
    "Geometry",
    [(Point, Point), (Curve, Curve), (Mesh, Mesh), (Solid, Solid)]
);

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
                Some(element) => T::from_arc(element).map_err(|source| FromValueError::Element {
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

    fn from_arc(value: &Arc<HashedValue>) -> Result<Self, FromValueError> {
        match value.data() {
            ValueData::Nothing => Ok(None),
            _ => T::from_arc(value).map(Some),
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

    fn pseudo_solid() -> Solid {
        let mut bytes = crate::geometry::SOLID_CANONICAL_HEADER.to_vec();
        bytes.extend_from_slice(b"\nmarshal test");
        Solid::from_canonical_bytes(bytes).unwrap()
    }

    // A Solid marshals as a leaf, rides the `T` and `Geometry` enums (the
    // checker admits it there — TRANSFORMABLE_KINDS / GEOMETRY_KINDS), and
    // never passes for a Mesh: `tessellate` is the only bridge.
    #[test]
    fn solid_marshals_as_leaf_and_through_the_geometry_enums() {
        let solid = pseudo_solid();
        let v = solid.clone().into_value().unwrap();
        assert_eq!(Solid::from_value(&v).unwrap(), solid);
        assert!(
            Arc::ptr_eq(
                Solid::from_value(&v).unwrap().shared_bytes(),
                solid.shared_bytes()
            ),
            "the bytes are shared, never copied, through marshalling"
        );
        assert_eq!(
            Transformable::from_value(&v).unwrap(),
            Transformable::Solid(solid.clone())
        );
        assert_eq!(
            GeometryValue::from_value(&v).unwrap(),
            GeometryValue::Solid(solid.clone())
        );
        assert_eq!(
            Transformable::Solid(solid).into_value().unwrap().hash(),
            v.hash()
        );
        assert_eq!(
            Mesh::from_value(&v),
            Err(FromValueError::Kind {
                expected: "Mesh".to_owned(),
                got: "Solid"
            })
        );
        assert!(matches!(
            Watertight::<Mesh>::from_value(&v),
            Err(FromValueError::Kind { got: "Solid", .. })
        ));
        assert_eq!(Solid::expected(), "Solid");
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

    // `E` slots are total over holes: a `[E]` port takes a holed list
    // without re-hashing the present payloads, and emits holes back as
    // holes (stage 6: the slot-preserving list combinators).
    #[test]
    fn elem_slots_pass_holes_through_both_ways() {
        let one = number(1.0);
        let holed = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(Arc::clone(&one)), None],
        }))
        .unwrap();
        let slots = Vec::<ElemSlot>::from_value(&holed).unwrap();
        assert_eq!(slots.len(), 2);
        assert!(slots[0].is_present() && !slots[1].is_present());
        assert!(
            Arc::ptr_eq(slots[0].0.as_ref().unwrap(), &one),
            "pass-through clones the Arc"
        );
        // Depth 2: inner holes survive, an OUTER hole is a loud refusal
        // (optional lists have no representation — docs/09).
        let nested = HashedValue::new(ValueData::List(List {
            axis: None,
            slots: vec![Some(Arc::clone(&holed)), None],
        }))
        .unwrap();
        assert_eq!(
            Vec::<Vec<ElemSlot>>::from_value(&nested),
            Err(FromValueError::Hole { index: 1 })
        );
        // Back out: an absent slot becomes a hole, present slots keep
        // their hash — the round trip is the identity on the sealed list.
        let back = vec![ElemSlot(Some(one)), ElemSlot(None)]
            .into_value()
            .unwrap();
        assert_eq!(back.hash(), holed.hash());
        // A bare absent slot (single `E` output) is the Nothing value.
        let bare = ElemSlot(None).into_value().unwrap();
        assert!(matches!(bare.data(), ValueData::Nothing));
        assert!(!ElemSlot::from_arc(&bare).unwrap().is_present());
    }
}
