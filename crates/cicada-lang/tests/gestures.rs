//! Gesture fixtures (doc 15 stage-2 DoD): every writer gesture has a
//! before/after pair, asserted byte-exact — the round-trip contract's
//! writer half (docs/10 §Round-trip contract). Plus the loud-failure
//! contract: a gesture that would corrupt the document errors and leaves
//! it untouched.

use cicada_lang::Document;
use cicada_lang::diag::Span;
use cicada_lang::writer::{self, WriterError};

fn apply(before: &str, ops: impl FnOnce(&mut Document)) -> String {
    let mut document = Document::parse(before);
    ops(&mut document);
    document.emit()
}

#[test]
fn place_appends_after_last_dependency_or_eof() {
    let before = include_str!("fixtures/gestures/place/before.cic");
    let after = include_str!("fixtures/gestures/place/after.cic");
    let emitted = apply(before, |doc| {
        // Wired placement: lands right after its dependency.
        let name = writer::place(doc, "add", &["amps"]).unwrap();
        assert_eq!(name, "add_1");
        // Bare palette drop: lands at EOF.
        let name = writer::place(doc, "panel", &[]).unwrap();
        assert_eq!(name, "panel_1");
    });
    assert_eq!(emitted, after);
}

#[test]
fn place_auto_names_numbered_never_shadowing_the_callable() {
    // A binding named bare `slider` would shadow the node for later calls
    // (docs/10 §5 resolution order) — auto-names start at `_1`.
    let mut document = Document::parse("# cicada 1\n");
    assert_eq!(
        writer::place(&mut document, "slider", &[]).unwrap(),
        "slider_1"
    );
    assert_eq!(
        writer::place(&mut document, "slider", &[]).unwrap(),
        "slider_2"
    );
    assert_eq!(
        writer::place(&mut document, "slider", &[]).unwrap(),
        "slider_3"
    );
}

#[test]
fn place_refuses_unknown_dependencies_loudly() {
    // Never a silent EOF fallback on a bad dependency name.
    let mut document = Document::parse("# cicada 1\na = 1\n");
    let error = writer::place(&mut document, "add", &["ghost"]).unwrap_err();
    assert_eq!(error, WriterError::UnknownBinding("ghost".to_owned()));
    assert_eq!(document.emit(), "# cicada 1\na = 1\n", "untouched");
}

#[test]
fn place_restores_the_eof_newline() {
    let mut document = Document::parse("# cicada 1\na = 1");
    writer::place(&mut document, "panel", &[]).unwrap();
    assert!(document.emit().ends_with("panel_1 = panel()\n"));
}

#[test]
fn wire_rewrites_one_kwarg_and_inserts_in_spec_order() {
    let before = include_str!("fixtures/gestures/wire/before.cic");
    let after = include_str!("fixtures/gestures/wire/after.cic");
    let emitted = apply(before, |doc| {
        // Replace an existing kwarg's value.
        writer::set_kwarg(doc, "m", "geometry", "pts", None).unwrap();
        // Append into empty parens.
        writer::set_kwarg(doc, "empty", "items", "pts", None).unwrap();
        // Insert BEFORE an existing later port when spec order is known
        // ("kwargs in spec order", docs/10 writer discipline).
        writer::set_kwarg(
            doc,
            "late",
            "geometry",
            "pts",
            Some(&["geometry", "direction"]),
        )
        .unwrap();
    });
    assert_eq!(emitted, after);
}

#[test]
fn unwire_removes_one_kwarg_and_one_separator() {
    let before = include_str!("fixtures/gestures/unwire/before.cic");
    let after = include_str!("fixtures/gestures/unwire/after.cic");
    let emitted = apply(before, |doc| {
        // First kwarg: the FOLLOWING separator goes with it.
        writer::remove_kwarg(doc, "m", "geometry").unwrap();
        // Only kwarg: empty parens remain.
        writer::remove_kwarg(doc, "solo", "items").unwrap();
        // Middle kwarg: its following separator goes; the trailing comment
        // and everything else stay byte-identical.
        writer::remove_kwarg(doc, "late", "direction").unwrap();
    });
    assert_eq!(emitted, after);
    // Last kwarg: the PRECEDING separator goes.
    let mut document = Document::parse(
        "# cicada 1
x = f(a=1, b=2)
",
    );
    writer::remove_kwarg(&mut document, "x", "b").unwrap();
    assert_eq!(
        document.emit(),
        "# cicada 1
x = f(a=1)
"
    );
    // Unknown kwarg / non-call: loud, untouched.
    let source = "# cicada 1
x = f(a=1)
y = 2
";
    let mut document = Document::parse(source);
    assert_eq!(
        writer::remove_kwarg(&mut document, "x", "zzz").unwrap_err(),
        WriterError::UnknownKwarg {
            binding: "x".to_owned(),
            kwarg: "zzz".to_owned()
        }
    );
    assert_eq!(
        writer::remove_kwarg(&mut document, "y", "a").unwrap_err(),
        WriterError::NotACall("y".to_owned())
    );
    assert_eq!(document.emit(), source);
}

#[test]
fn lift_wraps_the_kwarg_value_in_each() {
    let before = include_str!("fixtures/gestures/lift/before.cic");
    let after = include_str!("fixtures/gestures/lift/after.cic");
    let emitted = apply(before, |doc| {
        writer::wrap_each(doc, "labeled", "solid").unwrap();
        writer::wrap_each(doc, "labeled", "text").unwrap();
    });
    assert_eq!(emitted, after);
}

#[test]
fn set_param_rewrites_one_numeric_literal() {
    let before = include_str!("fixtures/gestures/set_param/before.cic");
    let after = include_str!("fixtures/gestures/set_param/after.cic");
    let emitted = apply(before, |doc| {
        writer::set_param(doc, "amps", "value", "14.5").unwrap();
        writer::set_literal(doc, "count", "56").unwrap();
    });
    assert_eq!(emitted, after);
}

#[test]
fn delete_removes_the_statement_never_cascades() {
    let before = include_str!("fixtures/gestures/delete/before.cic");
    let after = include_str!("fixtures/gestures/delete/after.cic");
    let emitted = apply(before, |doc| {
        writer::delete(doc, "cells").unwrap();
    });
    // The downstream `frustum(profile=each(cells))` line is untouched —
    // it goes red at check time, it is NOT deleted (docs/10).
    assert_eq!(emitted, after);
}

#[test]
fn rename_updates_binding_and_references_not_kwarg_names() {
    let before = include_str!("fixtures/gestures/rename/before.cic");
    let after = include_str!("fixtures/gestures/rename/after.cic");
    let emitted = apply(before, |doc| {
        writer::rename(doc, "seeds", "sites").unwrap();
        writer::rename(doc, "cells", "regions").unwrap();
    });
    // Note what must NOT change: the kwarg NAME `seeds=` (a port label),
    // and `seeds_rate` (a different identifier).
    assert_eq!(emitted, after);
}

#[test]
fn rename_refuses_taken_and_invalid_names() {
    let source = "# cicada 1\na = 1\nb = 2\n";
    let mut document = Document::parse(source);
    assert_eq!(
        writer::rename(&mut document, "a", "b").unwrap_err(),
        WriterError::NameTaken("b".to_owned())
    );
    // Non-identifiers, keywords, and reserved literal names would break or
    // silently rewire the file (`x=True` becomes a literal) — refused.
    for bad in ["9lives", "1 + 1", "for", "True", "each", ""] {
        assert_eq!(
            writer::rename(&mut document, "a", bad).unwrap_err(),
            WriterError::InvalidName(bad.to_owned()),
            "`{bad}` must be refused"
        );
    }
    assert_eq!(document.emit(), source, "document untouched after refusals");
}

#[test]
fn corrupting_edits_are_reverted_and_reported() {
    let source = "# cicada 1\nb = frustum(profile=old)\n";
    let mut document = Document::parse(source);
    let error = writer::set_kwarg(&mut document, "b", "profile", "((", None).unwrap_err();
    assert!(
        matches!(error, WriterError::ProducedBrokenStatement { .. }),
        "{error:?}"
    );
    assert_eq!(document.emit(), source, "document untouched after revert");
}

#[test]
fn future_version_files_are_not_edited() {
    let source = "# cicada 99\na = 1\n";
    let mut document = Document::parse(source);
    let error = writer::set_literal(&mut document, "a", "2").unwrap_err();
    assert_eq!(error, WriterError::FutureVersion { found: 99 });
    assert_eq!(document.emit(), source);
}

#[test]
fn apply_fix_splices_a_diagnostic_replacement() {
    // The doc-11 loop's apply step: a did-you-mean replacement lands as a
    // validated splice at the diagnostic's span.
    let mut document = Document::parse("# cicada 1\nt = add(aa=1.0, b=2.0)\n");
    // Span of `aa` (line 2, cols 8..10) — what an unknown_kwarg carries.
    writer::apply_fix(
        &mut document,
        Span {
            line: 2,
            col_start: 8,
            col_end: 10,
        },
        "a",
    )
    .unwrap();
    assert_eq!(document.emit(), "# cicada 1\nt = add(a=1.0, b=2.0)\n");
    // Out-of-range spans are refused.
    assert!(matches!(
        writer::apply_fix(
            &mut document,
            Span {
                line: 99,
                col_start: 0,
                col_end: 1
            },
            "x"
        ),
        Err(WriterError::InvalidSpan { .. })
    ));
}

#[test]
fn gestures_error_loudly_on_unknown_bindings() {
    let mut document = Document::parse("# cicada 1\na = f(x=1)\n");
    assert!(writer::set_kwarg(&mut document, "ghost", "x", "a", None).is_err());
    assert!(writer::delete(&mut document, "ghost").is_err());
    assert!(writer::wrap_each(&mut document, "ghost", "x").is_err());
}
