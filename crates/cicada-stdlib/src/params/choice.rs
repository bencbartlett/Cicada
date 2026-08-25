//! The `choice` node — the dropdown param (docs/10 §3, DECISIONS.md
//! dialect-grammar row: `slider` / `toggle` / `choice` / bare literals are
//! the constructor bindings the canvas renders as widgets).

use cicada_macros::{Ports, node};

/// Inputs for [`choice`].
#[derive(Ports, Clone, Debug)]
pub struct ChoiceIn {
    /// The chosen option — one of `options`, as written.
    pub value: String,
    /// The options to choose from, in the order the dropdown lists them.
    pub options: Vec<String>,
}

/// Value List — a text parameter chosen from a fixed list of options (a
/// dropdown on the canvas and in the params panel).
///
/// The widget writes the chosen option into `value` as one text literal;
/// the engine sees a plain node whose output is that text. Grasshopper's
/// Value List carries a value per label; here the option IS the value —
/// a node downstream reads the text.
///
/// # Returns
///
/// The chosen option.
///
/// # Panics
///
/// Panics when `value` is not one of `options` (an option renamed or
/// removed in the text leaves a drifted value — a loud red, never a silent
/// fallback to the first option) — an empty `options` list included.
///
/// # Examples
///
/// ```cic
/// mode = choice(value="fast", options=["fast", "exact"])
/// ```
#[node(
    category = "Params & input",
    tier = "1",
    version = 1,
    gh = "Value List"
)]
#[must_use]
pub fn choice(input: ChoiceIn) -> String {
    assert!(
        input.options.contains(&input.value),
        "choice: value {:?} is not one of the options [{}]",
        input.value,
        input
            .options
            .iter()
            .map(|option| format!("{option:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    input.value
}

#[cfg(test)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    fn pick(value: &str, options: &[&str]) -> String {
        choice(ChoiceIn {
            value: value.to_owned(),
            options: options.iter().map(|o| (*o).to_owned()).collect(),
        })
    }

    #[test]
    fn choice_table_cases() {
        assert_eq!(pick("fast", &["fast", "exact"]), "fast");
        assert_eq!(pick("exact", &["fast", "exact"]), "exact");
        // The first, the last, a lone option, a duplicated one.
        assert_eq!(pick("a", &["a", "b", "c"]), "a");
        assert_eq!(pick("c", &["a", "b", "c"]), "c");
        assert_eq!(pick("only", &["only"]), "only");
        assert_eq!(pick("twice", &["twice", "twice"]), "twice");
        // Options are compared as written: case and spaces count.
        assert_eq!(pick("Fast", &["Fast", "fast"]), "Fast");
    }

    #[test]
    #[should_panic(
        expected = "choice: value \"slow\" is not one of the options [\"fast\", \"exact\"]"
    )]
    fn choice_value_outside_the_options_is_red() {
        let _ = pick("slow", &["fast", "exact"]);
    }

    #[test]
    #[should_panic(expected = "choice: value \"fast\" is not one of the options []")]
    fn choice_with_no_options_is_red() {
        let _ = pick("fast", &[]);
    }

    #[test]
    #[should_panic(expected = "is not one of the options")]
    fn choice_compares_options_as_written() {
        // `Fast` is not `fast`: no case folding, no trimming.
        let _ = pick("fast ", &["fast", "exact"]);
    }

    proptest::proptest! {
        // Any option chosen from any list comes back as itself; any text
        // not in the list is red.
        #[test]
        fn choice_property_is_the_identity_on_a_member_and_red_otherwise(
            options in proptest::collection::vec("[a-z ]{0,8}", 1..6),
            index in 0usize..6,
            stray in "[A-Z]{1,4}",
        ) {
            let chosen = options[index % options.len()].clone();
            let out = choice(ChoiceIn { value: chosen.clone(), options: options.clone() });
            proptest::prop_assert_eq!(out, chosen);
            // Upper-case text never matches a lower-case option.
            let refused = std::panic::catch_unwind(|| {
                choice(ChoiceIn { value: stray.clone(), options: options.clone() })
            });
            proptest::prop_assert!(refused.is_err());
        }
    }

    #[test]
    fn choice_determinism_golden_hash() {
        let out = pick("exact", &["fast", "exact"]);
        assert_eq!(
            HashedValue::new(ValueData::Text(out.into()))
                .unwrap()
                .hash()
                .to_hex(),
            "61b1559d0ef8c88e28d319689961b3916f22b1b6ecd40abd67078629f3715058"
        );
    }
}
