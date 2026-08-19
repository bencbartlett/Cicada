//! Maths & logic nodes (docs/08 §Catalog 3).

use cicada_core::scalar::Domain;
use cicada_macros::{Ports, node};

/// Inputs for [`add`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct AddIn {
    /// First addend.
    pub a: f64,
    /// Second addend.
    pub b: f64,
}

/// Add — sum of two numbers.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn add(input: AddIn) -> f64 {
    input.a + input.b
}

/// Inputs for [`construct_domain`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructDomainIn {
    /// Interval start.
    pub start: f64,
    /// Interval end.
    pub end: f64,
}

/// Construct Domain — a numeric interval from its endpoints.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn construct_domain(input: ConstructDomainIn) -> Domain {
    Domain::new(input.start, input.end)
}

/// Inputs for [`deconstruct_domain`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructDomainIn {
    /// The interval.
    pub domain: Domain,
}

/// Outputs of [`deconstruct_domain`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructDomainOut {
    /// Interval start.
    pub start: f64,
    /// Interval end.
    pub end: f64,
}

/// Deconstruct Domain — the endpoints of an interval.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn deconstruct_domain(input: DeconstructDomainIn) -> DeconstructDomainOut {
    DeconstructDomainOut {
        start: input.domain.start,
        end: input.domain.end,
    }
}

/// Inputs shared by the two-number arithmetic nodes.
#[derive(Ports, Clone, Copy, Debug)]
pub struct BinaryIn {
    /// Left operand.
    pub a: f64,
    /// Right operand.
    pub b: f64,
}

/// Subtract — difference of two numbers.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn subtract(input: BinaryIn) -> f64 {
    input.a - input.b
}

/// Multiply — product of two numbers.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn multiply(input: BinaryIn) -> f64 {
    input.a * input.b
}

/// Divide — quotient of two numbers (IEEE: dividing by zero yields ±∞).
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn divide(input: BinaryIn) -> f64 {
    input.a / input.b
}

/// Modulo — IEEE remainder of `a / b` (sign follows `a`).
///
/// # Panics
///
/// `a % 0` is NaN, which value construction refuses — the node goes red.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn modulo(input: BinaryIn) -> f64 {
    input.a % input.b
}

/// Power — `a` raised to `b` (`^` in expressions).
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn power(input: BinaryIn) -> f64 {
    input.a.powf(input.b)
}

/// Inputs for [`remap`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RemapIn {
    /// The value to remap.
    pub value: f64,
    /// The domain the value lives in.
    pub source: Domain,
    /// The domain to map it into.
    pub target: Domain,
}

/// Remap — map a value linearly from a source domain to a target domain.
/// Values outside the source domain extrapolate linearly (no clamping).
///
/// # Panics
///
/// Panics when the source domain is empty (`start == end`) — the map is
/// undefined there.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
#[allow(clippy::float_cmp)] // exact emptiness IS the undefined case
pub fn remap(input: RemapIn) -> f64 {
    let span = input.source.end - input.source.start;
    assert!(
        span != 0.0,
        "remap: source domain {}..{} is empty",
        input.source.start,
        input.source.end
    );
    let t = (input.value - input.source.start) / span;
    (input.target.end - input.target.start).mul_add(t, input.target.start)
}

// Test standards per doc 14: table cases + a proptest property + a golden
// blake3 determinism hash, for every stdlib node. Exact float `==` is
// sanctioned here because these nodes' contract IS exact IEEE arithmetic
// (geometry tests use tolerance-aware asserts instead).
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn add_table_cases() {
        let cases: &[(f64, f64, f64)] = &[
            (1.0, 2.0, 3.0),
            (0.0, 0.0, 0.0),
            (-1.5, 1.5, 0.0),
            (2.5e300, 2.5e300, 5.0e300),
            (0.1, 0.2, 0.300_000_000_000_000_04), // IEEE 754, exactly
        ];
        for &(a, b, want) in cases {
            assert_eq!(add(AddIn { a, b }), want, "add({a}, {b})");
        }
    }

    #[test]
    fn domain_table_cases() {
        let domain = construct_domain(ConstructDomainIn {
            start: 2.0,
            end: -3.5,
        });
        assert_eq!(
            domain,
            Domain::new(2.0, -3.5),
            "decreasing domains are legal"
        );
        let out = deconstruct_domain(DeconstructDomainIn { domain });
        assert_eq!((out.start, out.end), (2.0, -3.5));
    }

    proptest::proptest! {
        // IEEE 754 addition is commutative (for non-NaN inputs).
        #[test]
        fn add_property_commutative(a in -1.0e12..1.0e12_f64, b in -1.0e12..1.0e12_f64) {
            proptest::prop_assert_eq!(add(AddIn { a, b }), add(AddIn { a: b, b: a }));
        }

        // Zero is the additive identity.
        #[test]
        fn add_property_zero_identity(a in -1.0e12..1.0e12_f64) {
            proptest::prop_assert_eq!(add(AddIn { a, b: 0.0 }), a);
        }

        // Construct then deconstruct is the identity on endpoints.
        #[test]
        fn domain_property_roundtrip(start in -1.0e12..1.0e12_f64, end in -1.0e12..1.0e12_f64) {
            let out = deconstruct_domain(DeconstructDomainIn {
                domain: construct_domain(ConstructDomainIn { start, end }),
            });
            proptest::prop_assert_eq!((out.start, out.end), (start, end));
        }
    }

    #[test]
    fn arithmetic_table_cases() {
        let two_three = BinaryIn { a: 2.0, b: 3.0 };
        assert_eq!(subtract(two_three), -1.0);
        assert_eq!(multiply(two_three), 6.0);
        assert_eq!(divide(BinaryIn { a: 7.0, b: 2.0 }), 3.5);
        assert_eq!(divide(BinaryIn { a: 1.0, b: 0.0 }), f64::INFINITY);
        assert_eq!(modulo(BinaryIn { a: 7.5, b: 2.0 }), 1.5);
        assert_eq!(modulo(BinaryIn { a: -7.5, b: 2.0 }), -1.5, "sign follows a");
        assert_eq!(power(BinaryIn { a: 2.0, b: 10.0 }), 1024.0);
        assert_eq!(power(BinaryIn { a: 9.0, b: 0.5 }), 3.0);
    }

    #[test]
    fn modulo_by_zero_output_is_refused_at_value_construction() {
        // The node returns NaN; the value model refuses it — red, not a
        // silent NaN in a cache key.
        let out = modulo(BinaryIn { a: 1.0, b: 0.0 });
        assert!(out.is_nan());
        assert!(HashedValue::new(ValueData::Number(out)).is_err());
    }

    #[test]
    fn remap_table_cases() {
        let unit_to_percent = RemapIn {
            value: 0.25,
            source: Domain::new(0.0, 1.0),
            target: Domain::new(0.0, 100.0),
        };
        assert_eq!(remap(unit_to_percent), 25.0);
        // Decreasing target — the wall used these constantly.
        assert_eq!(
            remap(RemapIn {
                value: 0.25,
                source: Domain::new(0.0, 1.0),
                target: Domain::new(100.0, 0.0),
            }),
            75.0
        );
        // Outside the source: extrapolates, never clamps.
        assert_eq!(
            remap(RemapIn {
                value: 2.0,
                source: Domain::new(0.0, 1.0),
                target: Domain::new(0.0, 10.0),
            }),
            20.0
        );
    }

    #[test]
    #[should_panic(expected = "source domain")]
    fn remap_empty_source_is_red() {
        let _ = remap(RemapIn {
            value: 1.0,
            source: Domain::new(3.0, 3.0),
            target: Domain::new(0.0, 1.0),
        });
    }

    proptest::proptest! {
        // IEEE subtraction is exactly anti-symmetric (rounding is
        // sign-symmetric), and zero is its right identity.
        #[test]
        fn property_subtract_antisymmetry(a in -1.0e9..1.0e9_f64, b in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(
                subtract(BinaryIn { a, b }),
                -subtract(BinaryIn { a: b, b: a })
            );
            proptest::prop_assert_eq!(subtract(BinaryIn { a, b: 0.0 }), a);
        }

        // IEEE multiplication is commutative, and one is its identity.
        #[test]
        fn property_multiply_commutative(a in -1.0e9..1.0e9_f64, b in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(
                multiply(BinaryIn { a, b }),
                multiply(BinaryIn { a: b, b: a })
            );
            proptest::prop_assert_eq!(multiply(BinaryIn { a, b: 1.0 }), a);
        }

        // Dividing by one is exact, and divide-then-multiply lands within
        // one rounding step of `a` (two correctly-rounded IEEE ops).
        #[test]
        fn property_divide_roundtrip(a in -1.0e9..1.0e9_f64, b in 0.001f64..1.0e9) {
            proptest::prop_assert_eq!(divide(BinaryIn { a, b: 1.0 }), a);
            let back = multiply(BinaryIn { a: divide(BinaryIn { a, b }), b });
            proptest::prop_assert!((back - a).abs() <= 1e-12 * a.abs().max(1.0));
        }

        // fmod is exact: the remainder never reaches |b|, and its sign
        // follows `a` (or it is zero).
        #[test]
        fn property_modulo_range_and_sign(a in -1.0e9..1.0e9_f64, b in 0.001f64..1.0e6) {
            let r = modulo(BinaryIn { a, b });
            proptest::prop_assert!(r.abs() < b);
            proptest::prop_assert!(r == 0.0 || (r < 0.0) == (a < 0.0));
        }

        // IEEE 754 pins pow(x, 0) = 1 and pow(x, 1) = x exactly, on every
        // platform's libm.
        #[test]
        fn property_power_pinned_exponents(a in 0.001f64..1.0e9) {
            proptest::prop_assert_eq!(power(BinaryIn { a, b: 0.0 }), 1.0);
            proptest::prop_assert_eq!(power(BinaryIn { a, b: 1.0 }), a);
        }

        // Remap endpoints land exactly on the target endpoints.
        #[test]
        fn property_remap_endpoints(
            s0 in -1.0e3..1.0e3_f64, span in 0.001f64..1.0e3,
            t0 in -1.0e3..1.0e3_f64, t1 in -1.0e3..1.0e3_f64,
        ) {
            let source = Domain::new(s0, s0 + span);
            let target = Domain::new(t0, t1);
            proptest::prop_assert_eq!(
                remap(RemapIn { value: s0, source, target }),
                t0
            );
            let end = remap(RemapIn { value: s0 + span, source, target });
            proptest::prop_assert!((end - t1).abs() <= 1e-9 * t1.abs().max(1.0));
        }
    }

    #[test]
    fn binary_nodes_determinism_golden_hashes() {
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        // Arithmetic-exact inputs only: every output below is an exact
        // dyadic value, so the bit pattern (and hash) is platform-free.
        assert_eq!(
            hash(subtract(BinaryIn { a: 7.5, b: 2.25 })),
            "ca9cbb2b358bc696b112f70b6377e8e54a72fabc1b4a7603655a0cc8d37f406d"
        );
        assert_eq!(
            hash(multiply(BinaryIn { a: 1.5, b: 2.5 })),
            "8fb16814dd81aecf4fb62272ff268ffa7cac28cc1997dfaf1b5b85d39e464f76"
        );
        assert_eq!(
            hash(divide(BinaryIn { a: 7.0, b: 2.0 })),
            "b69e5ba382a20ddf8b8873de846ca57fb12935d86345d12513254b517aec8037"
        );
        assert_eq!(
            hash(modulo(BinaryIn { a: 7.5, b: 2.0 })),
            "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03"
        );
        assert_eq!(
            // 2^10: exactly representable AND on the IEEE-pinned powf
            // path — never a platform-libm hash (adversarial review,
            // stage 4).
            hash(power(BinaryIn { a: 2.0, b: 10.0 })),
            "ed155f8b6d76336f8458372211c5918a5ee4d7f5bda82394c99260a55f1cb0a8"
        );
        assert_eq!(
            hash(remap(RemapIn {
                value: 0.3,
                source: Domain::new(0.0, 1.0),
                target: Domain::new(10.0, 20.0),
            })),
            "ea0bcd90a9ec4e1d49641c9e5b8503cb7ff24e682c11855cee8aa099de23476b"
        );
    }

    // Golden output hashes through the value model: byte-identical across
    // runs and platforms is a unit test (DECISIONS.md determinism row).
    // Blessed via the run-once path; update only with the diff explained.
    #[test]
    fn add_determinism_golden_hash() {
        let out = HashedValue::new(ValueData::Number(add(AddIn { a: 1.5, b: 2.25 }))).unwrap();
        assert_eq!(
            out.hash().to_hex(),
            "8fb16814dd81aecf4fb62272ff268ffa7cac28cc1997dfaf1b5b85d39e464f76"
        );
    }

    #[test]
    fn domain_determinism_golden_hash() {
        let domain = construct_domain(ConstructDomainIn {
            start: 0.25,
            end: 4.0,
        });
        let out = HashedValue::new(ValueData::Domain(domain)).unwrap();
        assert_eq!(
            out.hash().to_hex(),
            "53346c4847e95756894eabdce84e548c9cf3aec000de291d944a8f7713b99494"
        );
        // deconstruct_domain: both outputs through the value model.
        let ends = deconstruct_domain(DeconstructDomainIn { domain });
        let number_hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        assert_eq!(
            number_hash(ends.start),
            "71b099e9be5351c658523316836088b7b65d8d393e485cc825e0ce991ef90f01"
        );
        assert_eq!(
            number_hash(ends.end),
            "bbdac5dc7660e2a6d1f490af14375cb2a0eb92a909a7a7d30730f0ff206775f9"
        );
    }
}
