//! The Cicada standard library: the node catalog (docs/08).
//!
//! Nodes are pure functions — the scheduler calls *them*; this crate never
//! depends on `cicada-sched` (doc 14, enforced by the dependency-DAG check).
//! Every node ships with table + property + determinism-hash tests and doc
//! comments that feed the generated catalog (DECISIONS.md).
//!
//! Nodes register at compile time via `#[node]` + `#[derive(Ports)]`
//! (cicada-macros); the registry is queried through [`registry`].
//!
//! Layout (DECISIONS.md stdlib row, revised 2026-08-19): one node per
//! file, `src/<category>/<node>.rs`, where the categories are the ribbon
//! tabs (docs/08 §Catalog); a category's `mod.rs` lists its nodes and a
//! `support.rs` holds whatever several of them share. Catalog order never
//! depends on this layout (name order within a category).

use cicada_core::spec::NodeSpec;

pub mod curves;
pub mod intersect;
pub mod lists;
pub mod maths;
pub mod meshes;
pub mod output;
pub mod params;
pub mod points;
pub mod sequences;
pub mod solids;
pub mod transform;

/// Unwrap a geometry result, turning the error into a node panic — the
/// scheduler catches panics into red nodes carrying this message
/// (docs/12; `series` is the original pattern).
pub(crate) fn red<T>(result: Result<T, cicada_geom::GeomError>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{error}"),
    }
}

/// Every node registered in this binary, in canonical catalog order
/// (docs/08 category order, then dialect name within a category — the
/// order never depends on the source layout).
#[must_use]
pub fn registry() -> &'static [&'static NodeSpec] {
    cicada_core::spec::registered()
}

// Naming-contract fixtures: registered only in test builds, asserting the
// macro behaviors the manuals document (keyword-dodging underscore strip,
// raw-ident unraw, explicit name override). They never reach the shipped
// catalog — `cicada catalog` runs without cfg(test).
#[cfg(test)]
#[allow(dead_code)] // fixtures exist to be REGISTERED (inventory), not called
mod naming_fixtures {
    use cicada_macros::{Ports, node};

    /// Inputs for the naming fixtures.
    #[derive(Ports, Clone, Copy, Debug)]
    pub struct FixtureIn {
        /// Truthy value (raw identifier — must register as port `true`).
        #[port(default = 1.5)]
        pub r#true: f64,
    }

    /// Loop Fixture — keyword-dodging fn name must register as `loop`.
    ///
    /// # Returns
    ///
    /// The truthy value.
    #[node(category = "Maths & logic", tier = "S", version = 1, gh = none)]
    pub fn loop_(input: FixtureIn) -> f64 {
        input.r#true
    }

    /// Renamed Fixture — the explicit name override must win.
    ///
    /// # Returns
    ///
    /// The truthy value.
    #[node(
        category = "Maths & logic",
        tier = "S",
        version = 1,
        gh = none,
        name = "fixture_renamed"
    )]
    pub fn some_other_ident(input: FixtureIn) -> f64 {
        input.r#true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_nonempty_and_names_unique() {
        // registered() panics on duplicate names; reaching the assert means
        // uniqueness held.
        let specs = registry();
        assert!(!specs.is_empty());
    }

    #[test]
    fn registry_categories_are_known() {
        for spec in registry() {
            assert!(
                cicada_core::catalog::CATEGORY_ORDER.contains(&spec.category),
                "node `{}` uses unknown category `{}` (docs/08 §Catalog)",
                spec.name,
                spec.category
            );
        }
    }

    // Stage-1 DoD (doc 15): a #[node] function round-trips into the catalog
    // with ports, defaults, and doc lines intact.
    #[test]
    fn add_spec_roundtrips_ports_and_docs() {
        let specs = registry();
        let add = specs
            .iter()
            .find(|s| s.name == "add")
            .expect("add registered");
        assert_eq!(add.title, "Add");
        // Lowercase after the em dash, per docs/08's own doc-comment style
        // ("Move — translate geometry along a vector.").
        assert_eq!(add.description, "sum of two numbers.");
        assert_eq!(add.category, "Maths & logic");
        assert_eq!(add.version, 1);
        assert!(add.pure && !add.uses_tolerance);
        let [a, b] = add.inputs else {
            panic!("add has two inputs")
        };
        assert_eq!(
            (a.name, a.ty.render().as_str(), a.default),
            ("a", "Number", None)
        );
        assert_eq!(
            (b.name, b.ty.render().as_str(), b.default),
            ("b", "Number", None)
        );
        assert_eq!(a.doc, "First addend.");
        assert_eq!(add.signature(), "add(a: Number, b: Number) → Number");
        // The bare single output carries the `# Returns` line as its doc
        // (one doc line per port — the output included).
        let [out] = add.outputs else {
            panic!("add has one output")
        };
        assert_eq!((out.name, out.ty.render().as_str()), ("out", "Number"));
        assert_eq!(out.doc, "The sum `a + b`.");
    }

    #[test]
    fn series_spec_roundtrips_defaults_and_list_output() {
        let specs = registry();
        let series = specs
            .iter()
            .find(|s| s.name == "series")
            .expect("series registered");
        assert_eq!(
            series.signature(),
            "series(start: Number = 0.0, step: Number = 1.0, count: Integer) → [Number]"
        );
        let [start, _, count] = series.inputs else {
            panic!("series has three inputs")
        };
        assert_eq!(start.default, Some("0.0"));
        assert_eq!(start.doc, "First value.");
        assert_eq!(count.default, None);
    }

    // A wrapped field doc is ONE port doc: the macro joins the first
    // paragraph's source lines (C1 review: it once kept the first line only,
    // truncating 28 port docs mid-sentence in catalog.json).
    #[test]
    fn wrapped_port_docs_keep_their_whole_first_paragraph() {
        let specs = registry();
        let jitter = specs
            .iter()
            .find(|s| s.name == "jitter")
            .expect("jitter registered");
        let strength = jitter
            .inputs
            .iter()
            .find(|p| p.name == "strength")
            .expect("jitter has a strength port");
        assert_eq!(
            strength.doc,
            "How far from the original order to stray, `0.0` (unchanged) to `1.0` (a fully \
             random order)."
        );
    }

    // C1: the first optional-element VARIABLE port (`Vec<Option<ElemSlot>>`
    // → `[E?]`) renders the documented `compact` signature — the port takes
    // the holes, the outputs are the bare `[E]` plus the IndexMap.
    #[test]
    fn compact_spec_renders_the_optional_element_port() {
        let specs = registry();
        let compact = specs
            .iter()
            .find(|s| s.name == "compact")
            .expect("compact registered");
        assert_eq!(
            compact.signature(),
            "compact(list: [E?]) → (values: [E], map: IndexMap)"
        );
        let [list] = compact.inputs else {
            panic!("compact has one input")
        };
        assert!(list.ty.optional && list.ty.list_depth == 1 && list.ty.base == "E");
        assert!(
            !compact.outputs[0].ty.optional,
            "values is the present list"
        );
    }

    // The node format's two newest pieces through the real macro: the
    // `gh` attribute and the `# Examples` ```cic fence, extracted as the
    // snippet text without the header (the runner adds `# cicada 1`).
    #[test]
    fn gh_and_examples_roundtrip_into_the_spec() {
        let specs = registry();
        let series = specs
            .iter()
            .find(|s| s.name == "series")
            .expect("series registered");
        assert_eq!(series.gh, Some("Series"));
        assert_eq!(
            series.examples,
            &["xs = series(start=0.0, step=2.5, count=4)"]
        );
        // Multi-line fences keep their line structure, `\n`-joined, no
        // trailing newline.
        let remap = specs
            .iter()
            .find(|s| s.name == "remap")
            .expect("remap registered");
        assert_eq!(remap.gh, Some("Remap Numbers"));
        assert_eq!(
            remap.examples,
            &["unit = construct_domain(start=0.0, end=1.0)\n\
               percent = construct_domain(start=0.0, end=100.0)\n\
               scaled = remap(value=0.25, source=unit, target=percent)"]
        );
        // `gh = none` is an explicit answer, carried as None.
        let as_closed = specs
            .iter()
            .find(|s| s.name == "as_closed")
            .expect("as_closed registered");
        assert_eq!(as_closed.gh, None);
        // The `# Panics` contract still ends where `# Examples` begins.
        let panics = series.panics.expect("series has a contract");
        assert!(!panics.contains("```"), "{panics}");
        assert!(!panics.contains("Examples"), "{panics}");
    }

    #[test]
    fn deconstruct_domain_spec_roundtrips_multi_output() {
        let specs = registry();
        let node = specs
            .iter()
            .find(|s| s.name == "deconstruct_domain")
            .expect("deconstruct_domain registered");
        assert_eq!(
            node.signature(),
            "deconstruct_domain(domain: Domain) → (start: Number, end: Number)"
        );
        assert_eq!(node.outputs.len(), 2);
        assert_eq!(node.outputs[0].doc, "Interval start.");
    }

    // The naming contracts the manuals document, exercised through the
    // real macro (docs/10: the dialect name never carries Rust's
    // keyword-dodging underscore; DECISIONS.md one-nomenclature row).
    #[test]
    fn naming_contracts_roundtrip() {
        let specs = registry();
        let loop_node = specs
            .iter()
            .find(|s| s.name == "loop")
            .expect("fn loop_ registers as `loop`");
        assert!(
            !specs.iter().any(|s| s.name == "loop_"),
            "trailing underscore must not reach the dialect name"
        );
        let [port] = loop_node.inputs else {
            panic!("fixture has one input")
        };
        assert_eq!(
            port.name, "true",
            "r#true field must register as port `true`"
        );
        assert_eq!(port.default, Some("1.5"));
        assert!(
            specs.iter().any(|s| s.name == "fixture_renamed"),
            "name override must win over the fn ident"
        );
        assert!(!specs.iter().any(|s| s.name == "some_other_ident"));
    }

    // Stage 3: the erased invoke shim — registry dispatch with wire values,
    // no hand-written glue (the scheduler's calling convention).
    #[test]
    fn erased_invoke_dispatches_with_defaults_and_multi_outputs() {
        use cicada_core::config::ProjectConfig;
        use cicada_core::marshal::InvokeError;
        use cicada_core::value::{HashedValue, ValueData};

        let config = ProjectConfig::default();
        let number = |x: f64| Some(HashedValue::new(ValueData::Number(x)).expect("valid number"));
        let integer =
            |i: i64| Some(HashedValue::new(ValueData::Integer(i)).expect("valid integer"));

        let add = cicada_core::spec::invoker("add").expect("add invoker registered");
        let out = add(&config, &[number(1.5), number(2.25)]).expect("add invokes");
        assert_eq!(*out[0].data(), ValueData::Number(3.75));

        // Absent slots take the TYPED defaults (start=0.0, step=1.0).
        let series = cicada_core::spec::invoker("series").expect("series invoker");
        let out = series(&config, &[None, None, integer(3)]).expect("series invokes");
        let ValueData::List(list) = out[0].data() else {
            panic!("series returns a list")
        };
        assert_eq!(list.slots.len(), 3);

        // Multi-output nodes return one value per output port, spec order.
        let construct = cicada_core::spec::invoker("construct_domain").expect("construct");
        let domain =
            construct(&config, &[number(2.0), number(5.0)]).expect("constructs")[0].clone();
        let deconstruct = cicada_core::spec::invoker("deconstruct_domain").expect("deconstruct");
        let out = deconstruct(&config, &[Some(domain)]).expect("deconstructs");
        assert_eq!(*out[0].data(), ValueData::Number(2.0));
        assert_eq!(*out[1].data(), ValueData::Number(5.0));

        // A required port with no value refuses loudly.
        let err = add(&config, &[number(1.0), None]).expect_err("missing b must refuse");
        assert_eq!(err, InvokeError::Missing { port: "b" });

        // Unknown names resolve to nothing — never a silent fallback.
        assert!(cicada_core::spec::invoker("no_such_node").is_none());
    }

    #[test]
    fn registry_order_is_category_then_name() {
        let specs = registry();
        let names: Vec<&str> = specs.iter().map(|s| s.name).collect();
        let position = |name: &str| names.iter().position(|n| *n == name).expect(name);
        assert!(
            position("series") < position("add"),
            "Sequences & random (docs/08 §2) precedes Maths & logic (§3): {names:?}"
        );
        // Within a category the tie-break is the dialect name, not the
        // source position: `construct_plane` is defined LAST in its module
        // but sorts first; `subtract` is defined before `divide` but sorts
        // after it.
        assert!(position("construct_plane") < position("construct_point"));
        assert!(position("divide") < position("subtract"));
        for pair in specs.windows(2) {
            if pair[0].category == pair[1].category {
                assert!(
                    pair[0].name < pair[1].name,
                    "`{}` must precede `{}` inside `{}`",
                    pair[0].name,
                    pair[1].name,
                    pair[0].category
                );
            }
        }
    }
}
