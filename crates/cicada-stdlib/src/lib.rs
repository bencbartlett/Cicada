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

/// The most slots one node may EMIT from a count, all its outputs
/// together: 2^22 (4,194,304). A count above it is a red node, never an
/// attempt to allocate it — an unbounded `count` once aborted the whole
/// engine on allocation failure (`series(count=100000000000)`: "memory
/// allocation of 800000000000 bytes failed", which is not a panic, so the
/// scheduler could not turn it red and `cicada serve` would have died with
/// it; C1 review). Why 2^22: the slot ceiling is the one that bounds what
/// a slot costs BEYOND the node's own buffer — the value model hashes
/// every slot on its way out, the memo log serialises it and zstd
/// compresses it — and that cost was measured, not guessed (v0.1
/// follow-up 2, headless `cicada run`, fresh cache, peak working set;
/// release engine unless said otherwise): `series` at 2^24 slots peaked
/// at 9,763 MiB and wrote 1.4 GB to the cache (a 128 MiB `Vec<f64>` became
/// ~580 bytes a slot end to end); at 2^22 it peaks at 2,478 MiB in 4.1 s
/// and writes 348 MB (2^21: 1,249 MiB — the cost is linear), and the last
/// allowed sphere (2,897 segments, 4.19M vertices) peaks at 650 MiB. 2^22
/// is the largest power of two whose end-to-end footprint an 8 GB machine
/// survives with room for the rest of the pipeline; the earlier 2^24
/// (15112fb) bounded the buffer and let the process reach the allocator
/// failure a few million slots under its own ceiling. The ceiling is
/// charged on what the node EMITS, because that is what the value model
/// hashes: a node with several list outputs charges every one of them
/// (`divide_curve` emits `count + 1` points AND tangents AND parameters —
/// charged per port, its cap admitted 3 × 2^22 slots and measured
/// 5,332 MiB / 1,172 MB of cache at `count = 2^22`, 2.15× the footprint
/// the ceiling is justified by; charged on the total, its last allowed
/// count on a closed curve — 1,398,101, 4,194,303 slots — peaks at
/// 2,039 MiB and writes 410 MB, against `series` at 2^22 measured in the
/// same run at 2,482 MiB / 365 MB; both the branch's debug engine,
/// 2026-08-21), and a fence-post node charges the length it emits
/// (`range` emits `steps + 1`). No design needs more
/// elements in ONE list (the production wall is 1,200 parts; an 8M point
/// grid is a fan-out or a mesh, not a list the value model hashes slot by
/// slot). This is a SLOT ceiling — it keeps absurd counts loud and the
/// per-slot overhead bounded; the bytes a fat slot allocates are the other
/// half, [`MAX_BYTES`]. The served path (`cicada serve` encoding a node's
/// display frames on top of the memo) is not what these numbers measure;
/// docs/17 names it with the frame follow-up. DECISIONS.md row of
/// 2026-08-21 is the binding record of both halves.
pub const MAX_SLOTS: i64 = 1 << 22;

/// The most bytes one count-driven buffer may take up front: 1 GiB. The
/// second half of the cap ([`MAX_SLOTS`] is the first; whichever bites
/// first refuses): it is what makes fat slots honest — thirty copies of a
/// 36 MB mesh are already a GiB the allocator may refuse, and an
/// allocation failure aborts the engine instead of going red. 1 GiB is the
/// largest single buffer a node may build eagerly without the scheduler's
/// cost model knowing about it; what the process commits on top of that
/// buffer is bounded by the slot half (see its measurements) — the two
/// halves are read together. Under a 2^22 slot ceiling only a slot above
/// 256 bytes can reach this half: the bare element types (`f64`,
/// `ElemSlot`, the 112-byte `Transformable`, a 96-byte prism vertex) never
/// do, a slot with a PAYLOAD does — so `bytes_per_slot` at every call site
/// is what a slot really makes the node allocate (`linear_array` charges
/// each copy its mesh or polyline), not the element's `size_of` alone.
pub const MAX_BYTES: u64 = 1 << 30;

/// What one vertex costs in a closed triangle mesh built from a count (a
/// sphere's `segments × rings` vertices): its position (three `f64`) and
/// its share of the triangles — a closed mesh has about two triangles per
/// vertex, three `u32` each.
pub(crate) const MESH_BYTES_PER_VERTEX: usize = 3 * 8 + 2 * 3 * 4;

/// What one profile vertex costs in a capped prism (`extrude`, `loft`,
/// `text_solids`): two mesh vertices (top and bottom), two wall triangles
/// and one cap triangle per cap.
pub(crate) const PRISM_BYTES_PER_PROFILE_VERTEX: usize = 2 * 3 * 8 + 4 * 3 * 4;

/// The floor every count port shares: red below `least` (`0` for a count,
/// `1` for a step count, `3` for a tessellation) with the port's name and
/// value in the message. [`checked_count`] applies it before the ceilings;
/// a node whose emitted length is not the port's value (`range`,
/// `divide_curve`) applies it itself and takes the emitted total through
/// [`checked_size`].
pub(crate) fn checked_floor(node: &str, port: &str, value: i64, least: i64) -> u128 {
    assert!(
        value >= least,
        "{node}: {port} must be >= {least}, got {value}"
    );
    u128::from(value.unsigned_abs())
}

/// A count port as the `usize` a node may allocate: red below `least`
/// ([`checked_floor`]), red above [`MAX_SLOTS`], and red when `count ×
/// bytes_per_slot` is above [`MAX_BYTES`] — the port's name and value in
/// the message either way, and never an allocation. `bytes_per_slot` is
/// what one slot makes the node allocate: the element the buffer holds
/// (`size_of::<f64>()` for a number list, a prism's cost per profile
/// vertex) PLUS whatever the node builds per slot behind it
/// (`linear_array` transforms a fresh copy of its geometry per slot, so a
/// copy costs the `Transformable` and its mesh or polyline payload — see
/// [`transform::support::payload_bytes`]); a node whose slots share one
/// value (`duplicate`) counts the slot alone. A node that emits exactly
/// `value` slots on its one list output goes through here (`series` is
/// the original pattern); a node whose emitted total is NOT the port's
/// value — a fence-post (`range`: `steps + 1`), several list outputs
/// (`divide_curve`: `count + 1` on each of three), or a PRODUCT of inputs
/// (the sphere, the text nodes) — computes the total it emits and goes
/// through [`checked_size`], so the ceiling is always on what the value
/// model will hash.
pub(crate) fn checked_count(
    node: &str,
    port: &str,
    value: i64,
    least: i64,
    bytes_per_slot: usize,
) -> usize {
    let _ = checked_floor(node, port, value, least);
    assert!(
        value <= MAX_SLOTS,
        "{node}: {port} is {value} — above the {MAX_SLOTS} (2^22) slot ceiling of one node \
         output"
    );
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // 0 <= value <= 2^22
    let count = value as usize;
    let bytes = bytes_of(count, bytes_per_slot);
    assert!(
        bytes <= u128::from(MAX_BYTES),
        "{node}: {port} is {value} — {bytes} bytes at {bytes_per_slot} bytes a slot, above the \
         {MAX_BYTES}-byte (1 GiB) ceiling of one node allocation"
    );
    count
}

/// A derived slot count — what the node will emit when that is not one
/// port's value: the sphere's `segments × rings` vertices, a text's
/// `spans × segments` outline vertices, `range`'s `steps + 1` values,
/// `divide_curve`'s `count + 1` samples on each of its three outputs —
/// checked against both ceilings ([`MAX_SLOTS`] and, at `bytes_per_slot`
/// each, [`MAX_BYTES`]) before the allocation it sizes; `what` names the
/// quantity in the message ("vertices would be …", "values at steps=N
/// (steps + 1) would be …" — a port-driven total names the port and its
/// value there, so the red text still says which count to lower). The
/// caller has already refused the negative and too-small ports with the
/// node's own floor message ([`checked_floor`] or the kernel's); this is
/// the ceiling.
pub(crate) fn checked_size(node: &str, what: &str, slots: u128, bytes_per_slot: usize) -> usize {
    assert!(
        slots <= u128::from(MAX_SLOTS.unsigned_abs()),
        "{node}: {what} would be {slots} — above the {MAX_SLOTS} (2^22) slot ceiling of one \
         node output"
    );
    #[allow(clippy::cast_possible_truncation)] // slots <= 2^22
    let count = slots as usize;
    let bytes = bytes_of(count, bytes_per_slot);
    assert!(
        bytes <= u128::from(MAX_BYTES),
        "{node}: {what} would be {slots} — {bytes} bytes at {bytes_per_slot} bytes each, above \
         the {MAX_BYTES}-byte (1 GiB) ceiling of one node allocation"
    );
    count
}

/// `count × bytes_per_slot` without overflow (both fit `u64`; the product
/// fits `u128`).
fn bytes_of(count: usize, bytes_per_slot: usize) -> u128 {
    (count as u128) * (bytes_per_slot as u128)
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

    /// Volatile Fixture — `#[node(volatile)]` must register uncached
    /// (docs/12 §Volatile nodes; the flag `clock` wears since item 4). The
    /// fixture keeps the macro → spec path honest independently of any
    /// shipped node.
    ///
    /// # Returns
    ///
    /// The truthy value.
    #[node(category = "Maths & logic", tier = "S", version = 1, gh = none, volatile)]
    pub fn fixture_volatile(input: FixtureIn) -> f64 {
        input.r#true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The ceilings every count port shares: both bounds inclusive, the
    // refusals name the port (or the derived quantity), the value, and the
    // ceiling that bit.
    #[test]
    fn checked_count_accepts_the_bounds_and_refuses_beyond_them() {
        assert_eq!(checked_count("series", "count", 0, 0, 8), 0);
        assert_eq!(checked_count("range", "steps", 1, 1, 8), 1);
        assert_eq!(
            checked_count("duplicate", "count", MAX_SLOTS, 0, 8),
            4_194_304,
            "the slot ceiling itself is allowed (2^22 × 8 bytes is 32 MiB)"
        );
        assert_eq!(
            checked_count("linear_array", "count", 1 << 20, 1, 1024),
            1 << 20,
            "the byte ceiling itself is allowed (2^20 × 1 KiB is exactly 1 GiB)"
        );
        let below = std::panic::catch_unwind(|| checked_count("series", "count", -1, 0, 8))
            .expect_err("below least refuses");
        assert_eq!(message(below), "series: count must be >= 0, got -1");
        let above =
            std::panic::catch_unwind(|| checked_count("repeat", "count", MAX_SLOTS + 1, 0, 8))
                .expect_err("above the slot ceiling refuses");
        assert_eq!(
            message(above),
            "repeat: count is 4194305 — above the 4194304 (2^22) slot ceiling of one node \
             output"
        );
        let fat = std::panic::catch_unwind(|| {
            checked_count("linear_array", "count", (1 << 20) + 1, 1, 1024)
        })
        .expect_err("above the byte ceiling refuses before the slot ceiling");
        assert_eq!(
            message(fat),
            "linear_array: count is 1048577 — 1073742848 bytes at 1024 bytes a slot, above the \
             1073741824-byte (1 GiB) ceiling of one node allocation"
        );
    }

    // The floor on its own — what `range` and `divide_curve` apply before
    // taking their EMITTED total through `checked_size`: the same message
    // `checked_count` gives, and the value back as the u128 the total is
    // computed in (so `steps + 1` or `3 × (count + 1)` cannot overflow at
    // any i64).
    #[test]
    fn checked_floor_refuses_below_least_with_the_shared_message() {
        assert_eq!(checked_floor("range", "steps", 1, 1), 1);
        assert_eq!(
            checked_floor("divide_curve", "count", i64::MAX, 1),
            u128::from(i64::MAX.unsigned_abs())
        );
        let below = std::panic::catch_unwind(|| checked_floor("range", "steps", 0, 1))
            .expect_err("below least refuses");
        assert_eq!(message(below), "range: steps must be >= 1, got 0");
        let negative = std::panic::catch_unwind(|| checked_floor("divide_curve", "count", -7, 1))
            .expect_err("negative refuses");
        assert_eq!(
            message(negative),
            "divide_curve: count must be >= 1, got -7"
        );
    }

    #[test]
    fn checked_size_refuses_products_at_either_ceiling() {
        assert_eq!(checked_size("sphere", "vertices", 0, 48), 0);
        assert_eq!(
            checked_size("sphere", "vertices", 1 << 22, 48),
            4_194_304,
            "2^22 vertices × 48 bytes is 192 MiB: under both ceilings"
        );
        let slots =
            std::panic::catch_unwind(|| checked_size("sphere", "vertices", (1 << 22) + 1, 48))
                .expect_err("above the slot ceiling refuses");
        assert_eq!(
            message(slots),
            "sphere: vertices would be 4194305 — above the 4194304 (2^22) slot ceiling of one \
             node output"
        );
        // A product far beyond u64 (a 2^40-segment sphere) is refused with
        // the true product in the message, never an overflow.
        let huge = std::panic::catch_unwind(|| {
            checked_size("sphere", "vertices", (1u128 << 40) * (1u128 << 39), 48)
        })
        .expect_err("an astronomic product refuses");
        let huge = message(huge);
        assert!(
            huge.starts_with("sphere: vertices would be 604462909807314587353088 —"),
            "{huge}"
        );
        // The byte half, with a slot fat enough that it bites under the slot
        // ceiling (1 GiB / 1024 bytes = 2^20 slots).
        let bytes = std::panic::catch_unwind(|| {
            checked_size("linear_array", "copies", (1 << 20) + 1, 1024)
        })
        .expect_err("above the byte ceiling refuses");
        assert_eq!(
            message(bytes),
            "linear_array: copies would be 1048577 — 1073742848 bytes at 1024 bytes each, \
             above the 1073741824-byte (1 GiB) ceiling of one node allocation"
        );
        assert_eq!(
            checked_size("linear_array", "copies", 1 << 20, 1024),
            1 << 20,
            "the byte ceiling itself is allowed"
        );
        // Under the slot ceiling, a 96-byte prism vertex never reaches the
        // byte half: 2^22 × 96 is 384 MiB.
        assert_eq!(
            checked_size("text_solids", "outline vertices", 1 << 22, 96),
            1 << 22
        );
    }

    /// The `String` payload of a caught `assert!` panic (by value: a
    /// `&Box<dyn Any>` would unsize to the Box's own type id).
    fn message(panic: Box<dyn std::any::Any + Send>) -> String {
        panic
            .downcast::<String>()
            .map_or_else(|_| panic!("a formatted message"), |s| *s)
    }

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
    fn volatile_attribute_registers_and_every_shipped_node_is_not() {
        let specs = registry();
        let fixture = specs
            .iter()
            .find(|s| s.name == "fixture_volatile")
            .expect("the test-only volatile fixture registers");
        assert!(fixture.volatile, "#[node(volatile)] sets the flag");
        assert!(fixture.pure, "volatile is not effectful");
        // Exporters are effectful; the ONE shipped volatile node is
        // `clock` (DECISIONS.md time row: uncached by design, item 4).
        let shipped: Vec<&str> = specs
            .iter()
            .filter(|s| s.volatile && s.name != "fixture_volatile")
            .map(|s| s.name)
            .collect();
        assert_eq!(shipped, ["clock"], "volatile nodes shipped");
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
