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
//! Three rules, because a message names a node in three shapes:
//!
//! 1. A backticked identifier (`` `compact` ``, `` `each()` ``) must be a
//!    node or listed vocabulary — the convention the messages follow.
//! 2. Any `snake_case` word, backticked or not (`pad_last` — the shape that
//!    first slipped through), must be a node or listed vocabulary.
//! 3. A PHANTOM — a single word docs/09's combinator inventory or an old
//!    diagnostic used for something that is NOT a node (`cycle` the zip
//!    policy, `cross`, `squeeze`, …) — may not appear as a whole word at
//!    all, backticked or bare. Rule 1 alone let a bare `cycle` through (the
//!    2026-08-19 wording that motivated this test). The graph-cycle messages
//!    ("the graph must be a DAG") are exempted by their wording, listed
//!    below.
//!
//! Scanned: the checker, parser and diagnostics; the scheduler's executor
//! and graph; the server's lowering, compile and session sources; and every
//! stdlib node file — a node's panic message is the red text a user reads
//! on the canvas. Comments, rustdoc and `#[cfg(test)]` modules are skipped
//! — only what can reach a user counts. The script host's Solid refusal
//! (`cicada-script/src/value.rs`, the one literal there that names a node)
//! is held to rule 1 on its own: the rest of that file is wire-protocol
//! vocabulary (`k`, `v`, `kind`, "does not cross the boundary").
//!
//! A fourth rule for the kernel seam (`cicada-geom`'s error type, value
//! level and OCCT module — whose `GeomError` texts are a Solid node's red
//! text): no string literal may carry a C++ glue identifier (`cicada_…`,
//! the fork's bridge-function prefix) or a `TopAbs_ShapeEnum` number
//! ("shape type N") — the 2026-08-21 review read `kernel refused: OCCT cut:
//! cicada_single_solid: expected exactly one solid, found 2 (shape type 0)`
//! where the catalog promised "a cut that splits the solid in two is red".
//! The seam sources are not held to the node-name rules (their
//! `BadParameter` names are seam-level parameters, not nodes).

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The sources whose string literals can reach a user as diagnostics, node
/// failures, lowering errors, or session refusals.
const DIAGNOSTIC_SOURCES: &[&str] = &[
    "cicada-lang/src/check.rs",
    "cicada-lang/src/diag.rs",
    "cicada-lang/src/parse.rs",
    "cicada-sched/src/exec.rs",
    "cicada-sched/src/graph.rs",
    "cicada-server/src/lower.rs",
    "cicada-server/src/compile.rs",
    "cicada-server/src/session.rs",
];

/// The script host's value boundary: its Solid refusal names the node to
/// use, and that node must exist (2026-08-21: the text said `tessellate`
/// was "arriving with WP-C" months after it shipped — tense pinned by a
/// test; now the test pins existence instead).
const SCRIPT_BOUNDARY: &str = "cicada-script/src/value.rs";

/// The kernel seam's sources, scanned for glue identifiers only.
const SEAM_SOURCES: &[&str] = &[
    "cicada-geom/src/lib.rs",
    "cicada-geom/src/solid.rs",
    "cicada-geom/src/occt/mod.rs",
];

/// Substrings that mark a kernel diagnostic as leaked glue: the fork's
/// bridge-function prefix, and OCCT's shape-type enum rendered as a number.
const GLUE_MARKERS: &[&str] = &["cicada_", "shape type", "expected exactly one solid"];

/// Every `.rs` under this directory is scanned too: a stdlib node's panic
/// message is user-facing red text — the strict-zip refusal of `cull`
/// names the adapters `pad_last` / `repeat` / `truncate`.
const STDLIB_SOURCES: &str = "cicada-stdlib/src";

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
    // Session protocol vocabulary (docs/13): the canvas gestures a `batch`
    // may hold and the whole-file edit route, quoted back to the client in
    // refusals; `insert_between` is the adapter-chip gesture the refusal
    // promises for v0.1. Intents, not nodes.
    "place_node",
    "accept_lift",
    "set_param",
    "delete_node",
    "toggle_disable",
    "move_node",
    "set_preview",
    "apply_text",
    "insert_between",
];

/// Names a user could read as nodes that are NOT nodes: the zip policy
/// docs/02/09 once spelled `cycle` (the node is `repeat`; the §1 time
/// param owns `cycle`), and docs/09's combinator inventory's unbuilt rows.
/// A diagnostic may not mention one as a whole word — backticked or bare —
/// because a user would go looking for it. Each entry says what it is
/// instead. When one ships as a node it leaves this list (asserted below).
const PHANTOMS: &[(&str, &str)] = &[
    (
        "cycle",
        "the zip policy's node is `repeat`; `cycle` is the §1 time param",
    ),
    (
        "cross",
        "the all-pairs combinator — two type variables, pending (docs/09)",
    ),
    (
        "squeeze",
        "drop singleton levels — data-dependent depth, pending (docs/09)",
    ),
    (
        "flatten_all",
        "flatten every level — data-dependent depth, pending (docs/09)",
    ),
    ("unzip", "list disassembly — pending (docs/09)"),
    (
        "longest_list",
        "GH's Longest List — the node form is `pad_last`",
    ),
    (
        "shortest_list",
        "GH's Shortest List — the node form is `truncate`",
    ),
];

/// Wording that makes `cycle` the GRAPH sense (a dependency cycle), which
/// is not the phantom zip policy: "cycle — the graph must be a DAG",
/// "would create a cycle", "part of a cycle", "cycle in the target cone".
const GRAPH_CYCLE_WORDING: &[&str] = &["DAG", "a cycle", "cycle in the"];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/cicada-cli has a parent")
        .to_path_buf()
}

fn rust_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files_under(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out.sort();
}

/// The source with its `#[cfg(test)]` MODULES cut off: a test module's
/// strings (`should_panic(expected = …)`, proptest regexes, assertion
/// messages) never reach a user. Files keep their tests at the bottom, so
/// the first test module ends the user-facing text.
fn without_test_modules(source: &str) -> &str {
    const MARKER: &str = "#[cfg(test)]";
    let mut search_from = 0;
    while let Some(found) = source[search_from..].find(MARKER) {
        let marker = search_from + found;
        let after = &source[marker + MARKER.len()..];
        let item = after
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("#["))
            .unwrap_or("");
        if item.contains("mod ") && item.ends_with('{') {
            return &source[..marker];
        }
        search_from = marker + 1;
    }
    source
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

/// A literal that is not a message: a bare Rust path (`Option::is_none` —
/// serde attribute values), or a single identifier with no spaces at all
/// (`"text_hash"`, `"stale_base"` — the session's JSON keys and refusal
/// codes). A diagnostic that names a node is a sentence.
fn is_code_path(raw: &str) -> bool {
    !raw.contains(' ') && (raw.contains("::") || raw.chars().all(is_ident_char))
}

/// Escape pairs replaced by a space, so `"\n_probe"` splits as `_probe`,
/// not `n_probe` (the scanner keeps escapes verbatim).
fn without_escapes(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    let mut chars = literal.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            chars.next();
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// The node-like tokens of one literal: every backticked identifier (with a
/// trailing `()` allowed) and every `snake_case` word anywhere in the text.
fn node_like_tokens(raw: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    if is_code_path(raw) {
        return tokens;
    }
    let literal = without_placeholders(&without_escapes(raw));
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

/// The phantoms a literal mentions as WHOLE words (identifier boundaries on
/// both sides — `cycle` is found in "`cycle`", "cycle," and "/ cycle /",
/// not in "recycled"), minus the graph-cycle wording.
fn phantom_mentions(raw: &str) -> Vec<&'static str> {
    if is_code_path(raw) {
        return Vec::new();
    }
    let literal = without_placeholders(&without_escapes(raw));
    let words: BTreeSet<&str> = literal
        .split(|c: char| !is_ident_char(c))
        .filter(|word| !word.is_empty())
        .collect();
    PHANTOMS
        .iter()
        .filter(|(name, _)| words.contains(name))
        .filter(|(name, _)| {
            *name != "cycle" || !GRAPH_CYCLE_WORDING.iter().any(|w| literal.contains(w))
        })
        .map(|(name, _)| *name)
        .collect()
}

/// `(crates-relative path, user-facing source)` for every scanned file.
fn scanned_sources() -> Vec<(String, String)> {
    let crates = crates_dir();
    let mut files: Vec<PathBuf> = DIAGNOSTIC_SOURCES.iter().map(|r| crates.join(r)).collect();
    rust_files_under(&crates.join(STDLIB_SOURCES), &mut files);
    files
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            let relative = path
                .strip_prefix(&crates)
                .expect("under crates/")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, without_test_modules(&source).to_owned())
        })
        .collect()
}

#[test]
fn diagnostics_name_only_registered_nodes() {
    let registered: BTreeSet<&str> = cicada_stdlib::registry()
        .iter()
        .map(|spec| spec.name)
        .collect();
    for (phantom, what) in PHANTOMS {
        assert!(
            !registered.contains(phantom),
            "`{phantom}` is a registered node now — drop it from PHANTOMS ({what})"
        );
    }
    let mut offences = Vec::new();
    let mut literals_seen = 0usize;
    let mut files_seen = 0usize;
    for (relative, source) in scanned_sources() {
        files_seen += 1;
        for literal in string_literals(&source) {
            literals_seen += 1;
            for token in node_like_tokens(&literal) {
                if !registered.contains(token.as_str()) && !VOCABULARY.contains(&token.as_str()) {
                    offences.push(format!(
                        "{relative}: `{token}` is not a registered node (in \"{literal}\")"
                    ));
                }
            }
            for phantom in phantom_mentions(&literal) {
                let (_, what) = PHANTOMS
                    .iter()
                    .find(|(name, _)| *name == phantom)
                    .expect("listed");
                offences.push(format!(
                    "{relative}: `{phantom}` is not a node — {what} (in \"{literal}\")"
                ));
            }
        }
    }
    assert!(
        files_seen > 100 && literals_seen > 300,
        "the scanner saw only {files_seen} files / {literals_seen} string literals — it is \
         broken, not the sources"
    );
    assert!(
        offences.is_empty(),
        "{} diagnostic string(s) name a node that does not exist:\n  {}",
        offences.len(),
        offences.join("\n  ")
    );
}

#[test]
fn the_script_hosts_solid_refusal_names_a_node_that_exists() {
    let registered: BTreeSet<&str> = cicada_stdlib::registry()
        .iter()
        .map(|spec| spec.name)
        .collect();
    let path = crates_dir().join(SCRIPT_BOUNDARY);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let refusal = string_literals(without_test_modules(&source))
        .into_iter()
        .find(|literal| literal.starts_with("no Solid ABI exists"))
        .expect("the Solid refusal literal is in the script host");
    let named = node_like_tokens(&refusal);
    assert!(
        named.contains("tessellate"),
        "the refusal names the way forward: {refusal}"
    );
    for token in &named {
        assert!(
            registered.contains(token.as_str()),
            "`{token}` is not a registered node (in \"{refusal}\")"
        );
    }
    // No tense, no work-package name: wording bound to a milestone is wrong
    // the day the milestone lands.
    for stale in ["arriving", "WP-", "until it lands", "not yet"] {
        assert!(
            !refusal.contains(stale),
            "stale wording `{stale}`: {refusal}"
        );
    }
}

#[test]
fn kernel_diagnostics_never_leak_glue_identifiers() {
    let crates = crates_dir();
    let mut offences = Vec::new();
    let mut literals_seen = 0usize;
    for relative in SEAM_SOURCES
        .iter()
        .chain(DIAGNOSTIC_SOURCES)
        .chain(std::iter::once(&SCRIPT_BOUNDARY))
    {
        let path = crates.join(relative);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        for literal in string_literals(without_test_modules(&source)) {
            literals_seen += 1;
            for marker in GLUE_MARKERS {
                if literal.contains(marker) {
                    offences.push(format!("{relative}: \"{literal}\" carries `{marker}`"));
                }
            }
        }
    }
    assert!(
        literals_seen > 200,
        "the scanner saw only {literals_seen} string literals — it is broken, not the sources"
    );
    assert!(
        offences.is_empty(),
        "{} diagnostic string(s) leak a glue identifier:\n  {}",
        offences.len(),
        offences.join("\n  ")
    );
    // The rule is live: the seam's own test strings are skipped, and a
    // literal shaped like the reviewed message would be caught.
    assert!(GLUE_MARKERS.iter().any(|m| {
        "OCCT cut: cicada_single_solid: expected exactly one solid, found 2 (shape \
                      type 0)"
            .contains(m)
    }));
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
    assert!(
        node_like_tokens("text_hash").is_empty(),
        "a lone identifier is a protocol key, not a message"
    );
    assert_eq!(
        node_like_tokens("# cicada 1\\n_probe = f(x={value})\\n"),
        BTreeSet::new(),
        "an escape is a separator: `\\n_probe` is not the word `n_probe`"
    );
}

// The phantom rule catches the ORIGINAL defect shape — a bare, unbackticked
// single word — and leaves the graph-cycle wording alone.
#[test]
fn the_phantom_rule_catches_bare_words_and_spares_graph_cycles() {
    assert_eq!(
        phantom_mentions("zip is strict: 3 vs 4 — `pad_last` / cycle / `truncate`"),
        vec!["cycle"],
        "the 2026-08-19 wording: a bare `cycle` is the phantom"
    );
    assert_eq!(
        phantom_mentions("zip is strict — `pad_last` / `cycle` / `truncate` are the adapters"),
        vec!["cycle"]
    );
    assert_eq!(
        phantom_mentions(
            "use `cross` for all pairs, `squeeze` to drop levels, flatten_all for all"
        ),
        vec!["cross", "squeeze", "flatten_all"]
    );
    assert!(
        phantom_mentions("cycle — the graph must be a DAG: {} → {}").is_empty(),
        "the graph sense is exempt"
    );
    assert!(phantom_mentions("would create a cycle").is_empty());
    assert!(phantom_mentions("part of a cycle").is_empty());
    assert!(phantom_mentions("cycle in the target cone").is_empty());
    assert!(
        phantom_mentions("the recycled buffer crosses nothing").is_empty(),
        "whole words only"
    );
    assert!(
        phantom_mentions("crate::cycle::Policy").is_empty(),
        "a bare Rust path is code"
    );
}

// Test modules are cut off at the `#[cfg(test)] mod … {` marker — and only
// there: a `#[cfg(test)]` on a single item leaves the production text intact.
#[test]
fn test_modules_are_skipped() {
    let with_module = "fn f() { panic!(\"`ghost`\") }\n\
                       #[cfg(test)]\n\
                       #[allow(dead_code)]\n\
                       mod tests {\n    const S: &str = \"`phantom_in_test`\";\n}\n";
    let kept = without_test_modules(with_module);
    assert!(kept.contains("`ghost`") && !kept.contains("phantom_in_test"));
    let with_item = "#[cfg(test)]\npub(crate) mod support;\nfn f() { panic!(\"`ghost`\") }\n";
    assert_eq!(without_test_modules(with_item), with_item);
}
