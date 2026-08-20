//! The node-format conformance test (DECISIONS.md stdlib row, revised
//! 2026-08-19; docs/14 §The node file format): every registered stdlib
//! node carries the pieces the catalog, the canvas, the docs, and the AI
//! read — a title line, a description, a doc line per port, a `gh`
//! answer, and at least one runnable example. An integration test on
//! purpose: it sees exactly the shipped registry (the crate's cfg(test)
//! naming fixtures never register here), so "every registered node" has
//! no exceptions.
//!
//! What the compiler already guarantees (the macro refuses a node without
//! a `Title — description` line, an undocumented input port, a bare single
//! output without a `# Returns` line, a missing `gh`, or an untagged
//! example fence) is asserted again here cheaply:
//! the test is the single place that states the format, and it would catch
//! a macro regression that let one piece through. What the compiler cannot
//! check — that an example exists and actually calls the node — is the
//! part that matters. Whether the examples SOLVE is the
//! `node_examples` test in `cicada-cli` (the solver lives above this crate;
//! stdlib never depends on sched).

#![allow(clippy::expect_used)]

use cicada_stdlib::registry;

/// One failing node is a list entry, not an early exit: a batch of new
/// nodes gets one report, not one failure per rerun.
fn collect_failures(check: impl Fn(&cicada_core::spec::NodeSpec) -> Vec<String>) -> Vec<String> {
    let mut failures = Vec::new();
    for spec in registry() {
        for problem in check(spec) {
            failures.push(format!("`{}`: {problem}", spec.name));
        }
    }
    failures
}

/// Does the snippet call `name(`, as a whole identifier? A raw substring
/// match would let `polyline(` satisfy `line` and `bounding_box(` satisfy
/// `box` (adversarial review of C0): the character before the call must
/// not be an identifier character.
fn calls_node(example: &str, name: &str) -> bool {
    let call = format!("{name}(");
    example.match_indices(&call).any(|(at, _)| {
        at == 0
            || !example[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn assert_no_failures(what: &str, failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} node(s) fail the {what} rule:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[test]
fn the_registry_is_the_shipped_catalog() {
    // The spike shipped 57 nodes; the count only grows. A drop means a
    // module stopped being compiled in (a `mod` line lost in a move).
    assert!(
        registry().len() >= 57,
        "{} nodes registered",
        registry().len()
    );
}

#[test]
fn every_node_has_a_title_line_and_description() {
    let failures = collect_failures(|spec| {
        let mut problems = Vec::new();
        if spec.title.trim().is_empty() {
            problems.push("empty title".to_owned());
        }
        if !spec.title.chars().next().is_some_and(char::is_uppercase) {
            problems.push(format!("title `{}` must start with a capital", spec.title));
        }
        if spec.description.trim().is_empty() {
            problems.push("empty description".to_owned());
        }
        // A sentence, not a fragment: it ends with a full stop (or a
        // parenthesised aside that contains one).
        if !spec.description.ends_with(['.', ')']) {
            problems.push(format!(
                "description must end with a full stop: `{}`",
                spec.description
            ));
        }
        problems
    });
    assert_no_failures("title line", &failures);
}

#[test]
fn every_port_is_documented() {
    let failures = collect_failures(|spec| {
        let mut problems = Vec::new();
        for port in spec.inputs {
            if port.doc.trim().is_empty() {
                problems.push(format!("input port `{}` has no doc line", port.name));
            }
        }
        // EVERY output too: a bare single `out` carries the node's
        // `# Returns` line (the macro refuses a single-output node without
        // one), named outputs carry their struct fields' docs. No
        // exemption — the C0 review caught the first version of this rule
        // skipping `out`, which left 47 output ports undocumented in
        // catalog.json.
        for port in spec.outputs {
            if port.doc.trim().is_empty() {
                problems.push(format!("output port `{}` has no doc line", port.name));
            }
        }
        problems
    });
    assert_no_failures("port docs", &failures);
}

#[test]
fn every_node_answers_the_grasshopper_question() {
    // `gh` is required at compile time; what remains is that a given name
    // is a real name (non-empty, trimmed). `None` is the explicit `none`.
    let failures = collect_failures(|spec| match spec.gh {
        Some(name) if name.trim().is_empty() || name.trim() != name => {
            vec![format!(
                "gh name {name:?} is empty or has surrounding whitespace"
            )]
        }
        Some(_) | None => Vec::new(),
    });
    assert_no_failures("gh", &failures);
}

#[test]
fn every_node_has_a_runnable_example_that_calls_it() {
    let failures = collect_failures(|spec| {
        let mut problems = Vec::new();
        if spec.examples.is_empty() {
            problems.push("no `# Examples` ```cic fence".to_owned());
        }
        for (index, example) in spec.examples.iter().enumerate() {
            if example.trim().is_empty() {
                problems.push(format!("example {index} is empty"));
            }
            if !calls_node(example, spec.name) {
                problems.push(format!(
                    "example {index} never calls `{}` — it must exercise the node it documents",
                    spec.name
                ));
            }
            if example
                .lines()
                .any(|line| line.trim_start().starts_with('#'))
            {
                // The runner adds the `# cicada 1` header; a header or a
                // `# comment` inside the fence would also read as a rustdoc
                // heading to anyone skimming the source.
                problems.push(format!(
                    "example {index} has a `#` line — no header, no comments inside the fence"
                ));
            }
        }
        problems
    });
    assert_no_failures("examples", &failures);
}

#[test]
fn every_node_with_a_refusal_states_its_contract() {
    // Not every node refuses anything (`add` is total). But a `# Panics`
    // section that exists must be a sentence the catalog can render.
    let failures = collect_failures(|spec| match spec.panics {
        Some(contract) if contract.trim().is_empty() => vec!["empty `# Panics`".to_owned()],
        Some(contract) if contract.starts_with("Panics") => vec![format!(
            "`# Panics` should state the condition (the macro strips `Panics when`): {contract}"
        )],
        _ => Vec::new(),
    });
    assert_no_failures("contract", &failures);
}

// The helper's own contract: a sibling name that CONTAINS the node's name
// is not a call of the node.
#[test]
fn calls_node_needs_an_identifier_boundary() {
    assert!(calls_node("segment = line(a=start, b=end)", "line"));
    assert!(calls_node("line(a=start, b=end)", "line"));
    assert!(!calls_node(
        "outline = polyline(vertices=corners, closed=True)",
        "line"
    ));
    assert!(!calls_node("bb = bounding_box(geometry=g)", "box"));
    assert!(calls_node(
        "outline = polyline(vertices=corners)\nseg = line(a=p, b=q)",
        "line"
    ));
}
