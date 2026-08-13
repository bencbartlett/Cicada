//! Checker rules (doc 15 stage-2 DoD): positive and negative wiring cases
//! per rule, diagnostics snapshot-tested as their doc-11 JSON (insta;
//! bless via `cargo insta review` or `INSTA_UPDATE=always`, never by hand).
//!
//! The catalog here is hand-built fake nodes — checker behavior must not
//! churn when the real stdlib grows.

// Tests are exempt from the expect/unwrap denial (clippy.toml), but that
// exemption only recognizes #[test] fns — not helpers in integration tests.
#![allow(clippy::expect_used)]

use cicada_core::spec::{NodeSpec, PortSpec, PortType, Tier};
use cicada_lang::{Catalog, Document};

const fn port(name: &'static str, base: &'static str, depth: u8) -> PortSpec {
    PortSpec {
        name,
        ty: PortType {
            base,
            list_depth: depth,
            optional: false,
        },
        default: None,
        doc: "",
        dimension: None,
    }
}

const fn opt_port(
    name: &'static str,
    base: &'static str,
    depth: u8,
    default: &'static str,
) -> PortSpec {
    PortSpec {
        name,
        ty: PortType {
            base,
            list_depth: depth,
            optional: false,
        },
        default: Some(default),
        doc: "",
        dimension: None,
    }
}

const fn node(
    name: &'static str,
    inputs: &'static [PortSpec],
    outputs: &'static [PortSpec],
) -> NodeSpec {
    NodeSpec {
        name,
        title: "Fake",
        description: "checker fixture.",
        category: "Maths & logic",
        tier: Tier::S,
        version: 1,
        pure: true,
        uses_tolerance: false,
        inputs,
        outputs,
        module: "fake",
        line: 0,
    }
}

static OUT_NUMBER: &[PortSpec] = &[port("out", "Number", 0)];

static SLIDER: NodeSpec = node(
    "slider",
    &[
        opt_port("value", "Number", 0, "0.0"),
        opt_port("min", "Number", 0, "0.0"),
        opt_port("max", "Number", 0, "1.0"),
        opt_port("step", "Number", 0, "0.1"),
    ],
    OUT_NUMBER,
);
static CIRCLE: NodeSpec = node(
    "circle",
    &[port("radius", "Number", 0)],
    &[port("out", "Curve", 0)],
);
static SCATTER: NodeSpec = node(
    "scatter",
    &[port("count", "Integer", 0), port("seed", "Integer", 0)],
    &[port("out", "Point", 1)],
);
static MOVE: NodeSpec = node(
    "move",
    &[port("geometry", "Point", 0), port("motion", "Vector", 0)],
    &[port("out", "Point", 0)],
);
static EXTRUDE: NodeSpec = node(
    "extrude",
    &[
        port("profile", "Closed<Curve>", 0),
        port("height", "Number", 0),
    ],
    &[port("out", "Mesh", 0)],
);
static DIVIDE: NodeSpec = node(
    "divide_curve",
    &[
        port("curve", "Curve", 0),
        opt_port("count", "Integer", 0, "10"),
    ],
    &[
        port("points", "Point", 1),
        port("tangents", "Vector", 1),
        port("parameters", "Number", 1),
    ],
);
static ADD: NodeSpec = node(
    "add",
    &[port("a", "Number", 0), port("b", "Number", 0)],
    OUT_NUMBER,
);
static LOOP_A: NodeSpec = node("loop_a", &[port("x", "Number", 0)], OUT_NUMBER);
static LOOP_B: NodeSpec = node("loop_b", &[port("x", "Number", 0)], OUT_NUMBER);
static AS_CLOSED: NodeSpec = node(
    "as_closed",
    &[port("curve", "Curve", 0)],
    &[port("out", "Closed<Curve>", 0)],
);
static SUM: NodeSpec = node("sum_list", &[port("values", "Number", 1)], OUT_NUMBER);
static NESTED: NodeSpec = node("nested", &[], &[port("out", "Number", 2)]);

fn catalog_specs() -> Vec<&'static NodeSpec> {
    vec![
        &SLIDER, &CIRCLE, &SCATTER, &MOVE, &EXTRUDE, &DIVIDE, &ADD, &LOOP_A, &LOOP_B, &AS_CLOSED,
        &SUM, &NESTED,
    ]
}

fn check(source: &str) -> serde_json::Value {
    let specs = catalog_specs();
    let catalog = Catalog::new(&specs);
    let document = Document::parse(source);
    let diagnostics = cicada_lang::diagnostics(&document, &catalog);
    serde_json::to_value(diagnostics).expect("diagnostics serialize")
}

#[test]
fn clean_pipeline_is_green() {
    let diagnostics = check(
        "# cicada 1\n\
         amps = slider(value=12.0)\n\
         c = circle(radius=amps)\n\
         pts = scatter(count=100, seed=7)\n\
         moved = move(geometry=each(pts), motion=motion_v)\n\
         points, tangents, t = divide_curve(curve=c)\n\
         first_t = add(a=1, b=2.5)\n\
         h = first_t * 2 + amps\n\
         motion_v = vector_stub(x=1)\n",
    );
    // One expected red: `vector_stub` is not in the catalog. Everything
    // else — widening (Integer literal into Number), lifts, unpack, port
    // math — is green.
    let list = diagnostics.as_array().unwrap();
    assert_eq!(list.len(), 1, "{diagnostics:#}");
    assert_eq!(list[0]["kind"], "unknown_node");
}

#[test]
fn fully_green_file_has_zero_diagnostics() {
    let diagnostics = check(
        "# cicada 1\n\
         amps = slider(value=12.0)\n\
         doubled = amps * 2\n\
         total = add(a=amps, b=doubled)\n",
    );
    assert_eq!(diagnostics.as_array().unwrap().len(), 0, "{diagnostics:#}");
}

#[test]
fn wrong_wires_produce_the_specified_diagnostics() {
    insta::assert_json_snapshot!(check(
        "# cicada 1\n\
         pts = scatter(count=100, seed=7)\n\
         bad_scalar = move(geometry=pts, motion=pts)\n\
         c = circle(radius=1.0)\n\
         needs_refined = extrude(profile=c, height=2.0)\n\
         wrong = circle(radius=c)\n"
    ));
}

#[test]
fn unknown_names_nodes_kwargs_with_did_you_mean() {
    insta::assert_json_snapshot!(check(
        "# cicada 1\n\
         amps = slider(valu=12.0)\n\
         c = circel(radius=amps)\n\
         m = move(geometry=ptz, motion=amps)\n\
         pts = scatter(count=100, seed=7)\n"
    ));
}

#[test]
fn structure_errors_rebinding_cycle_unpack_ports() {
    insta::assert_json_snapshot!(check(
        "# cicada 1\n\
         a = slider(value=1.0)\n\
         a = slider(value=2.0)\n\
         x = loop_a(x=y)\n\
         y = loop_b(x=x)\n\
         p, t = divide_curve(curve=c)\n\
         c = circle(radius=1.0)\n\
         d = divide_curve(curve=c)\n\
         solo = move(geometry=d, motion=d.nope)\n"
    ));
}

#[test]
fn each_and_zip_errors() {
    insta::assert_json_snapshot!(check(
        "# cicada 1\n\
         amps = slider(value=1.0)\n\
         bad_each = move(geometry=each(amps), motion=amps)\n\
         zip_bad = move(geometry=each([1, 2, 3]), motion=each([1, 2]))\n"
    ));
}

#[test]
fn expression_errors() {
    insta::assert_json_snapshot!(check(
        "# cicada 1\n\
         pts = scatter(count=10, seed=1)\n\
         c = circle(radius=1.0)\n\
         d = divide_curve(curve=c)\n\
         bad = pts * 2 + missing_var - d\n"
    ));
}

#[test]
fn broken_statement_reds_downstream_honestly() {
    insta::assert_json_snapshot!(check(
        "# cicada 1\n\
         glitch = slider(value=)\n\
         after = add(a=glitch, b=1.0)\n"
    ));
}

#[test]
fn pragma_diagnostics() {
    insta::assert_json_snapshot!(check("amps = slider(value=1.0)\n"));
    insta::assert_json_snapshot!(check("# cicada 99\namps = slider(value=1.0)\n"));
}

#[test]
fn optional_elements_into_required_port_is_flagged() {
    // No optional-producing fixture node yet — covered via a literal once
    // compact/Optional-producing nodes exist (stage 4). The lattice path
    // is unit-tested through WireType rendering in the meantime.
    let diagnostics = check("# cicada 1\namps = slider(value=1.0)\n");
    assert_eq!(diagnostics.as_array().unwrap().len(), 0);
}

// The doc-10 §Errors red-before-solve list, snapshot-pinned: positional
// args, nested calls (kwarg + expression position), control flow, bare
// calls, missing required kwargs, mismatched each() depths.
#[test]
fn specified_parse_and_call_errors() {
    insta::assert_json_snapshot!(check(
        "# cicada 1\n\
         p1 = circle(1.0)\n\
         n1 = move(geometry=circle(radius=1.0), motion=v)\n\
         n2 = 1 + circle(radius=2.0)\n\
         for x in range\n\
         export_dxf(path=\"out.dxf\")\n\
         missing = circle()\n\
         zip_depth = move(geometry=each(each(nn)), motion=each(nn))\n\
         nn = nested()\n"
    ));
}

#[test]
fn refined_values_widen_into_base_ports() {
    // Dropping a refinement is a free total upcast (doc 02): the adapter's
    // own output must feed base-typed ports, or `insert as_closed` would
    // be self-defeating.
    let diagnostics = check(
        "# cicada 1\n\
         c = circle(radius=1.0)\n\
         cc = as_closed(curve=c)\n\
         d = divide_curve(curve=cc)\n",
    );
    assert_eq!(diagnostics.as_array().unwrap().len(), 0, "{diagnostics:#}");
}

#[test]
fn empty_lists_unify_with_any_list_port() {
    // `[]` is data (docs/09), wireable anywhere a list fits.
    let diagnostics = check("# cicada 1\nxs = []\ns = sum_list(values=xs)\n");
    assert_eq!(diagnostics.as_array().unwrap().len(), 0, "{diagnostics:#}");
}

#[test]
fn integer_expressions_feed_integer_ports() {
    // `+ - *` over Integer leaves stays Integer (docs/08 Expression row);
    // `/` produces Number and reds an Integer port honestly.
    let green = check("# cicada 1\nn = 2 * 3 + 1\npts = scatter(count=n, seed=7)\n");
    assert_eq!(green.as_array().unwrap().len(), 0, "{green:#}");
    insta::assert_json_snapshot!(check(
        "# cicada 1\nn = 6 / 2\npts = scatter(count=n, seed=7)\n"
    ));
}

#[test]
fn dot_out_selects_the_single_output() {
    // The ABI names every single output `out` (DECISIONS.md) — selecting
    // it is the bare reference; other selections get a fix.
    let green = check("# cicada 1\ns = slider(value=1.0)\nc = circle(radius=s.out)\n");
    assert_eq!(green.as_array().unwrap().len(), 0, "{green:#}");
    insta::assert_json_snapshot!(check(
        "# cicada 1\ns = slider(value=1.0)\nc = circle(radius=s.value)\n"
    ));
}

#[test]
fn disabled_bindings_red_downstream_honestly() {
    // Never generic unknown-name for a `#off` binding (DECISIONS.md
    // node-disable row).
    insta::assert_json_snapshot!(check(
        "# cicada 1\n#off amps = slider(value=1.0)\nc = circle(radius=amps)\n"
    ));
}

#[test]
fn residual_lift_still_offers_each() {
    // One each() on [[Number]] into a scalar port: the fix is MORE lift,
    // not a dead-end mismatch.
    insta::assert_json_snapshot!(check(
        "# cicada 1\nnn = nested()\nr = add(a=each(nn), b=1.0)\n"
    ));
}

#[test]
fn duplicate_unpack_targets_poison_not_cascade() {
    let diagnostics = check(
        "# cicada 1\n\
         c = circle(radius=1.0)\n\
         p, p, p = divide_curve(curve=c)\n\
         use_p = sum_list(values=p)\n",
    );
    let list = diagnostics.as_array().unwrap();
    // Two rebindings for the duplicate targets — and NOTHING on use_p:
    // `p` is poisoned, downstream stays quiet.
    assert_eq!(list.len(), 2, "{diagnostics:#}");
    assert!(list.iter().all(|d| d["kind"] == "rebinding"));
}

#[test]
fn deep_forward_reference_chains_resolve_iteratively() {
    // Forward references are legal at any length (docs/10); resolution is
    // iterative and must survive thousands of use-before-def hops.
    use std::fmt::Write as _;
    let mut source = String::from("# cicada 1\n");
    for i in 0..2000 {
        // write! to a String is infallible.
        let _ = writeln!(source, "x{i} = x{} + 1", i + 1);
    }
    source.push_str("x2000 = 1\n");
    let diagnostics = check(&source);
    assert_eq!(diagnostics.as_array().unwrap().len(), 0, "chain is green");
}

#[test]
fn empty_file_needs_a_pragma_too() {
    let diagnostics = check("");
    let list = diagnostics.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["kind"], "missing_pragma");
}
