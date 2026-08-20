//! Diagnostics may only name nodes that exist (docs/17 track C, C1).
//!
//! The checker's strict-zip message once told users to reach for
//! `pad_last` / `cycle` / `truncate`, and two messages named `compact`,
//! before any of those nodes existed — a user following the advice found
//! nothing to place. This test reads the diagnostic-emitting sources and
//! asserts that every node-like identifier inside a STRING LITERAL is a
//! registered stdlib node or a listed piece of dialect vocabulary. It lives
//! in `cicada-cli` because that is the one crate that sees both the
//! sources' crates and the shipped registry (dependency law: `lang` and
//! `sched` never depend on `stdlib`).
//!
//! Two token shapes are checked, because both are how messages name nodes:
//! backticked identifiers (`` `compact` ``, `` `each()` ``) and any
//! `snake_case` word, backticked or not (`pad_last` — the shape that slipped
//! through). Single-word node names mentioned without backticks are not
//! detectable without a false-positive flood (`item`, `length`, `line`),
//! so the convention the messages follow is: a node name in a diagnostic is
//! backticked. Comments and rustdoc are skipped — only what can reach a
//! user counts.

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The sources whose string literals can reach a user as diagnostics, node
/// failures, or lowering errors.
const DIAGNOSTIC_SOURCES: &[&str] = &[
    "cicada-lang/src/check.rs",
    "cicada-lang/src/diag.rs",
    "cicada-lang/src/parse.rs",
    "cicada-sched/src/exec.rs",
    "cicada-sched/src/graph.rs",
    "cicada-server/src/lower.rs",
    "cicada-server/src/compile.rs",
];

/// Identifier-shaped words in diagnostics that are not nodes and never will
/// be. Each entry says why.
const VOCABULARY: &[&str] = &[
    // Dialect syntax: the lift is `each(...)`, not a node (docs/10).
    "each",
    // The default output port name of a single-output node.
    "out",
    // The naming rule's own wording ("must be snake_case").
    "snake_case",
    // An internal invariant message in the parser, never user-facing.
    "parse_number",
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/cicada-cli has a parent")
        .to_path_buf()
}

/// Every string literal in a Rust source, with comments and rustdoc skipped,
/// escape pairs kept verbatim, and `\`-newline continuations joined (the
/// shape `format!` strings take in this codebase). Char literals are
/// skipped so `'"'` cannot open a string.
fn string_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut literals = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &source[i..];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            i += rest.find("*/").map_or(rest.len(), |end| end + 2);
        } else if bytes[i] == b'\'' {
            // A char literal is `'x'`, `'\n'`, `'\''`, `'\x41'`, `'\u{…}'`; a
            // lifetime (`'a`) has no closing quote right after one char.
            i += char_literal_len(rest).unwrap_or(1);
        } else if let Some(raw) = raw_string(rest) {
            literals.push(raw.0);
            i += raw.1;
        } else if bytes[i] == b'"' {
            let (literal, consumed) = plain_string(rest);
            literals.push(literal);
            i += consumed;
        } else {
            i += 1;
        }
    }
    literals
}

fn char_literal_len(rest: &str) -> Option<usize> {
    let mut chars = rest.char_indices().skip(1);
    let (_, first) = chars.next()?;
    if first == '\\' {
        // Escape: find the closing quote after the escape body.
        let close = rest[2..].find('\'')? + 2;
        // `'\''` is the one escape whose body contains a quote: its close
        // is the third quote, not the second.
        return if rest[2..].starts_with('\'') {
            Some(4)
        } else {
            Some(close + 1)
        };
    }
    let (end, next) = chars.next()?;
    (next == '\'').then_some(end + 1)
}

fn raw_string(rest: &str) -> Option<(String, usize)> {
    if !rest.starts_with('r') {
        return None;
    }
    let hashes = rest[1..].chars().take_while(|&c| c == '#').count();
    if !rest[1 + hashes..].starts_with('"') {
        return None;
    }
    let body_start = 2 + hashes;
    let closer = format!("\"{}", "#".repeat(hashes));
    let end = rest[body_start..].find(&closer)? + body_start;
    Some((rest[body_start..end].to_owned(), end + closer.len()))
}

fn plain_string(rest: &str) -> (String, usize) {
    let bytes = rest.as_bytes();
    let mut out = String::new();
    let mut j = 1;
    while j < bytes.len() && bytes[j] != b'"' {
        if bytes[j] == b'\\' {
            if bytes.get(j + 1) == Some(&b'\n') {
                // Continuation: skip the newline and the next line's indent.
                j += 2;
                while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
                    j += 1;
                }
                continue;
            }
            out.push_str(&rest[j..(j + 2).min(rest.len())]);
            j += 2;
            continue;
        }
        let ch = rest[j..].chars().next().expect("in bounds");
        out.push(ch);
        j += ch.len_utf8();
    }
    (out, j + 1)
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A literal with its `format!` placeholders removed: `{first_len}` and
/// `{key_ms:.3}` are argument names, never user text.
fn without_placeholders(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    let mut depth = 0usize;
    for c in literal.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The node-like tokens of one literal: every backticked identifier (with a
/// trailing `()` allowed) and every `snake_case` word anywhere in the text.
/// A bare Rust path (`Option::is_none` — serde attribute values) is code,
/// not a message, and yields nothing.
fn node_like_tokens(raw: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    if raw.contains("::") && !raw.contains(' ') {
        return tokens;
    }
    let literal = without_placeholders(raw);
    let literal = literal.as_str();
    // Backticked identifiers.
    let mut parts = literal.split('`');
    let _prefix = parts.next();
    while let (Some(inside), Some(_after)) = (parts.next(), parts.next()) {
        let ident = inside.strip_suffix("()").unwrap_or(inside);
        if ident.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && ident.chars().all(is_ident_char)
        {
            tokens.insert(ident.to_owned());
        }
    }
    // snake_case words, backticked or not: split on non-identifier chars,
    // keep words with an interior underscore and a lowercase first char.
    for word in literal.split(|c: char| !is_ident_char(c)) {
        let interior_underscore = word.trim_matches('_').contains('_');
        if interior_underscore
            && word.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && word
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            tokens.insert(word.to_owned());
        }
    }
    tokens
}

#[test]
fn diagnostics_name_only_registered_nodes() {
    let registered: BTreeSet<&str> = cicada_stdlib::registry()
        .iter()
        .map(|spec| spec.name)
        .collect();
    let mut offences = Vec::new();
    let mut literals_seen = 0usize;
    for relative in DIAGNOSTIC_SOURCES {
        let path = crates_dir().join(relative);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        for literal in string_literals(&source) {
            literals_seen += 1;
            for token in node_like_tokens(&literal) {
                if !registered.contains(token.as_str()) && !VOCABULARY.contains(&token.as_str()) {
                    offences.push(format!(
                        "{relative}: `{token}` is not a registered node (in \"{literal}\")"
                    ));
                }
            }
        }
    }
    assert!(
        literals_seen > 100,
        "the scanner found only {literals_seen} string literals — it is broken, not the sources"
    );
    assert!(
        offences.is_empty(),
        "{} diagnostic string(s) name a node that does not exist:\n  {}",
        offences.len(),
        offences.join("\n  ")
    );
}

// The scanner itself, on the shapes it must and must not see.
#[test]
fn the_scanner_reads_literals_and_skips_comments() {
    let source = r##"
        // a comment naming `ghost_node` is not a diagnostic
        /// nor is rustdoc: `other_ghost`
        fn f() {
            let c = '"';
            let q = '\'';
            let s = "zip is strict — `pad_last` / `repeat` are the \
                     opt-in adapter nodes";
            let raw = r#"raw "quoted" `compact`"#;
            let plain = "snake_case words like no_such_node count too";
            let placeholders = "zip is strict: {first_len} vs {len} ({key_ms:.3} ms)";
            #[serde(skip_serializing_if = "Option::is_none")]
            let _ = ();
        }
    "##;
    let literals = string_literals(source);
    assert_eq!(literals.len(), 5, "{literals:?}");
    assert_eq!(
        literals[0],
        "zip is strict — `pad_last` / `repeat` are the opt-in adapter nodes"
    );
    let tokens: BTreeSet<String> = literals.iter().flat_map(|l| node_like_tokens(l)).collect();
    let expected: BTreeSet<String> = [
        "pad_last",
        "repeat",
        "compact",
        "snake_case",
        "no_such_node",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(tokens, expected);
    assert!(
        !tokens.contains("ghost_node") && !tokens.contains("other_ghost"),
        "comments and rustdoc are skipped"
    );
    assert!(
        !tokens.contains("first_len") && !tokens.contains("key_ms"),
        "format placeholders are not message text"
    );
    assert!(!tokens.contains("is_none"), "a bare Rust path is code");
}
