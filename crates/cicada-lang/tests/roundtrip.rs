//! The golden round-trip corpus (doc 15 stage-2 DoD): parse → emit is
//! byte-identical for every fixture — comments, spacing, broken lines,
//! and all. Guaranteed by construction (the document stores raw lines);
//! these tests keep the construction honest.

use cicada_lang::Document;

fn roundtrip(name: &str, source: &str) {
    let document = Document::parse(source);
    let emitted = document.emit();
    assert_eq!(
        source, emitted,
        "round-trip must be byte-identical for {name}"
    );
}

#[test]
fn corpus_fixtures_parse_fully() {
    // Byte-identical emission is true by construction even for garbage, so
    // the corpus ALSO pins that its statements actually parse — a parser
    // regression cannot hide behind the round-trip.
    let pipeline = Document::parse(include_str!("fixtures/corpus/pipeline.cic"));
    assert_eq!(pipeline.statements().count(), 12);
    assert!(
        !pipeline
            .lines()
            .iter()
            .any(|l| matches!(l, cicada_lang::Line::Broken { .. })),
        "pipeline.cic must parse clean"
    );
    let oddities = Document::parse(include_str!("fixtures/corpus/oddities.cic"));
    assert_eq!(oddities.statements().count(), 6);
    let broken: Vec<_> = oddities
        .lines()
        .iter()
        .filter(|l| matches!(l, cicada_lang::Line::Broken { .. }))
        .collect();
    assert_eq!(broken.len(), 1, "exactly the one intended broken line");
    // The `#off` line: parsed behind the prefix (ports + wiring for the
    // ghost), but not a statement — `statements()` and `find_binding`
    // skip it.
    assert_eq!(oddities.statements_including_disabled().count(), 7);
    assert!(oddities.find_binding("ghost").is_none());
    assert!(oddities.find_disabled("ghost").is_some());
}

#[test]
fn crlf_files_parse_and_roundtrip() {
    // Windows editors default to CRLF; the '\r' is part of the line
    // terminator, never a parse error (and emission stays byte-exact).
    let source = "# cicada 1\r\namps = slider(value=1.0)\r\ncount = 40\r\n";
    let document = Document::parse(source);
    assert_eq!(document.emit(), source);
    assert_eq!(document.version(), Some(1));
    assert_eq!(document.statements().count(), 2);
    assert!(
        document.parse_diagnostics().is_empty(),
        "{:?}",
        document.parse_diagnostics()
    );
}

#[test]
fn bom_does_not_defeat_the_pragma() {
    let source = "\u{feff}# cicada 1\na = 1\n";
    let document = Document::parse(source);
    assert_eq!(document.emit(), source);
    assert_eq!(document.version(), Some(1));
    assert!(document.parse_diagnostics().is_empty());
}

#[test]
fn disabled_bindings_are_recognized_not_swallowed() {
    let source = "# cicada 1\n#off frusta = frustum(profile=c)\nf2 = g(profile=frusta)\n";
    let document = Document::parse(source);
    assert_eq!(document.emit(), source);
    let disabled = document
        .lines()
        .iter()
        .find_map(|l| match l {
            cicada_lang::Line::Disabled {
                name: Some(n),
                statement,
                raw,
            } if n == "frusta" => Some((statement, raw)),
            _ => None,
        })
        .expect("#off line must classify as Disabled with its name");
    // The body parses in place: spans index the raw line, prefix included.
    let (Some(statement), raw) = disabled else {
        panic!("a parseable #off body keeps its parse");
    };
    assert_eq!(statement.targets[0].span.slice(raw), "frusta");
    assert_eq!(statement.references()[0].span.slice(raw), "c");
    // A body that does not parse still yields the name (first identifier)
    // and no statement — the prefix hides the parse error until enabled.
    let junk = Document::parse("# cicada 1\n#off bad = (((\n");
    assert!(matches!(
        &junk.lines()[1],
        cicada_lang::Line::Disabled { name: Some(n), statement: None, .. } if n == "bad"
    ));
    assert!(
        junk.parse_diagnostics().is_empty(),
        "disabled text is not a diagnostic"
    );
}

#[test]
fn adversarial_nesting_reds_the_statement_not_the_process() {
    // docs/10 constraint 4: one hostile line reds one node; the process
    // survives. These used to overflow the stack.
    let deep_each = format!(
        "# cicada 1\na = f(x={}1{})\n",
        "each(".repeat(300),
        ")".repeat(300)
    );
    let deep_parens = format!(
        "# cicada 1\na = {}1{}\n",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    let deep_neg = format!("# cicada 1\na = {}3\n", "-".repeat(5000));
    let deep_pow = format!("# cicada 1\na = 1{}\n", "**1".repeat(5000));
    for source in [&deep_each, &deep_parens, &deep_neg, &deep_pow] {
        let document = Document::parse(source);
        assert_eq!(document.emit(), *source, "still byte-identical");
        let diagnostics = document.parse_diagnostics();
        assert_eq!(diagnostics.len(), 1, "one red node");
        assert!(
            diagnostics[0].message.contains("nesting too deep"),
            "{}",
            diagnostics[0].message
        );
    }
}

#[test]
fn corpus_roundtrips_byte_identical() {
    // Fixture files: the realistic shapes.
    for (name, source) in [
        ("pipeline.cic", include_str!("fixtures/corpus/pipeline.cic")),
        ("oddities.cic", include_str!("fixtures/corpus/oddities.cic")),
    ] {
        roundtrip(name, source);
    }
}

#[test]
fn edge_shapes_roundtrip() {
    // Byte-level edge cases the fixture files can't hold naturally.
    for source in [
        "",                    // empty file
        "\n",                  // single newline
        "# cicada 1",          // no trailing newline
        "# cicada 1\n",        // pragma only
        "a = 1",               // statement, no trailing newline
        "a = 1\n\n\n",         // trailing blank lines
        "  \t \n# c\n",        // whitespace-only line
        "totally broken ((\n", // unparseable, still byte-exact
    ] {
        roundtrip("edge", source);
    }
}

#[test]
fn broken_statement_reds_one_node_not_the_file() {
    let source =
        "# cicada 1\ngood = slider(value=1.0)\nbad bad bad\nalso_good = slider(value=2.0)\n";
    let document = Document::parse(source);
    let diagnostics = document.parse_diagnostics();
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the broken line: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].span.line, 3);
    assert_eq!(diagnostics[0].node.as_deref(), Some("bad"));
    // The good statements parsed fine.
    assert_eq!(document.statements().count(), 2);
    // And the file still emits byte-identical.
    assert_eq!(document.emit(), source);
}

#[test]
fn pragma_version_is_read() {
    assert_eq!(Document::parse("# cicada 1\n").version(), Some(1));
    assert_eq!(Document::parse("# cicada 7\n").version(), Some(7));
    assert_eq!(Document::parse("# not a pragma\n").version(), None);
}
