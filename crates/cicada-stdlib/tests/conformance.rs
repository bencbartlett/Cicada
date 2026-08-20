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
//! check — that an example exists and actually calls the node, and that
//! the node's FILE is where the format says and holds its three tests —
//! is the part that matters. Whether the examples SOLVE is the
//! `node_examples` test in `cicada-cli` (the solver lives above this crate;
//! stdlib never depends on sched).

#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use cicada_core::spec::NodeSpec;
use cicada_stdlib::registry;

/// The crate's `src/` — the source layout is part of the format, so the
/// test reads it (the crate's own files, deterministic; no network, no
/// clock).
fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The directory a category's nodes live in (docs/14 §node file format;
/// the add-stdlib-node skill's table). A new category is a design addition:
/// add its row here in the same commit as docs/08.
fn category_dir(category: &str) -> Option<&'static str> {
    Some(match category {
        "Params & input" => "params",
        "Sequences & random" => "sequences",
        "Maths & logic" => "maths",
        "List & axis" => "lists",
        "Point · Vector · Plane" => "points",
        "Curve" => "curves",
        "Surface & solid" => "solids",
        "Mesh & field" => "meshes",
        "Intersect & regions" => "intersect",
        "Transform" => "transform",
        "Output, display & export" => "output",
        _ => return None,
    })
}

/// The file a node's `module_path!()` names, relative to `src/`:
/// `cicada_stdlib::solids::box` → `solids/box.rs` (a raw-identifier module
/// such as `r#box`/`r#move` renders without the `r#`).
fn module_file(spec: &NodeSpec) -> PathBuf {
    let mut path = PathBuf::new();
    for segment in spec
        .module
        .strip_prefix("cicada_stdlib::")
        .unwrap_or(spec.module)
        .split("::")
    {
        path.push(segment.strip_prefix("r#").unwrap_or(segment));
    }
    path.set_extension("rs");
    path
}

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

#[test]
fn every_node_is_one_file_in_its_category_directory() {
    // One node per file, `src/<category>/<node>.rs`, named after the
    // DIALECT name (`solids/box.rs` for `fn box_`): the layout that keeps
    // parallel agents conflict-free and an edit's context to one file
    // (DECISIONS.md stdlib row). The macro records `module_path!()`; the
    // file it names must exist, sit in the category's directory, carry the
    // node's name, and declare exactly one node.
    let src = src_dir();
    let failures = collect_failures(|spec| {
        let mut problems = Vec::new();
        let relative = module_file(spec);
        let Some(dir) = category_dir(spec.category) else {
            problems.push(format!(
                "category `{}` has no directory in this test's table (docs/14)",
                spec.category
            ));
            return problems;
        };
        let expected = Path::new(dir).join(format!("{}.rs", spec.name));
        if relative != expected {
            problems.push(format!(
                "defined in `{}` — the format puts it in `{}`",
                relative.display(),
                expected.display()
            ));
        }
        match std::fs::read_to_string(src.join(&relative)) {
            Ok(source) => {
                let nodes = source.matches("#[node(").count();
                if nodes != 1 {
                    problems.push(format!(
                        "`{}` declares {nodes} nodes — one node per file",
                        relative.display()
                    ));
                }
            }
            Err(error) => problems.push(format!(
                "cannot read `{}`: {error}",
                src.join(&relative).display()
            )),
        }
        problems
    });
    assert_no_failures("layout", &failures);
}

#[test]
fn every_node_file_holds_its_three_tests() {
    // Table cases, a property test, and a determinism test IN THE NODE'S
    // FILE (docs/14 §node file format; doc 17 "every node: the three
    // tests"). A test that spans two nodes may live with the primary node
    // in addition — never instead: the C0 review found four files whose
    // coverage was only a sibling's joint test. Machine-checked proxies,
    // stable under rustfmt: a plain `#[test]` at module-test indentation
    // (proptest's inner `#[test]` sits deeper), a `proptest!` block, and a
    // test fn named `*determinism*` (a golden blake3 hash, golden bytes
    // for an exporter, or hash identity for a pass-through). A sink
    // returning `()` has no output to hash and is exempt from the third.
    let src = src_dir();
    let failures = collect_failures(|spec| {
        let mut problems = Vec::new();
        let Ok(source) = std::fs::read_to_string(src.join(module_file(spec))) else {
            // Reported by the layout test; one failure per cause.
            return problems;
        };
        if !source.contains("\n    #[test]\n") {
            problems.push("no table/case test (`#[test]` fn in `mod tests`)".to_owned());
        }
        if !source.contains("proptest! {") {
            problems.push("no property test (`proptest! { … }` block)".to_owned());
        }
        let has_determinism_test = source.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("fn ")
                && line
                    .split('(')
                    .next()
                    .is_some_and(|head| head.contains("determinism"))
        });
        if !spec.outputs.is_empty() && !has_determinism_test {
            problems.push(
                "no determinism test (a `fn …determinism…` golden hash / golden bytes / \
                 hash-identity test)"
                    .to_owned(),
            );
        }
        problems
    });
    assert_no_failures("three tests", &failures);
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
