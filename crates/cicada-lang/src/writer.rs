//! The minimal-edit writer (docs/10 §Round-trip contract): every canvas
//! gesture is a specified text edit that touches only what the gesture
//! implies — never reformatting, reordering, or realigning existing lines.
//! Stage-2 gestures (doc 15): place, wire, lift, set-param, delete, rename,
//! plus `apply_fix` for machine-applicable diagnostic fixes (docs/11);
//! stage 5 added `remove_kwarg` (deleting a wire on the canvas); v0.1 added
//! `toggle_disable` (the `#off` prefix, DECISIONS.md node-disable row).
//!
//! Gestures fail loudly and leave the document UNTOUCHED on failure: an
//! edit that would turn a parsed statement into a broken one is reverted
//! and reported, never silently committed.

use crate::ast::{LineSpan, Rhs, Statement};
use crate::diag::Span;
use crate::document::{DIALECT_VERSION, Document, Line};
use crate::parse::is_valid_binding_name;

/// Writer failures — loud, typed, never silent no-ops.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriterError {
    /// No statement binds this name.
    #[error("no binding named `{0}`")]
    UnknownBinding(String),
    /// The binding is `#off`-disabled and the gesture needs a live one
    /// (wire, unwire, lift, set-param): enable it first.
    #[error("`{0}` is disabled (`#off`) — enable it to edit")]
    Disabled(String),
    /// The binding's call has no such kwarg (and the gesture required an
    /// existing one).
    #[error("binding `{binding}` has no kwarg `{kwarg}`")]
    UnknownKwarg {
        /// The binding.
        binding: String,
        /// The kwarg.
        kwarg: String,
    },
    /// The target name is already bound (single assignment).
    #[error("`{0}` is already bound — names bind once per file")]
    NameTaken(String),
    /// The gesture needed a call RHS (wire/lift target).
    #[error("binding `{0}` is not a node call")]
    NotACall(String),
    /// The gesture needed a bare-literal RHS (constant param).
    #[error("binding `{0}` is not a bare literal")]
    NotALiteral(String),
    /// The new name is not a legal binding identifier.
    #[error("`{0}` is not a valid binding name")]
    InvalidName(String),
    /// The edit would have produced text that no longer parses; the
    /// document was left untouched.
    #[error("edit rejected — `{text}` would break the statement: {why}")]
    ProducedBrokenStatement {
        /// The text the gesture tried to splice.
        text: String,
        /// The parse failure it would have caused.
        why: String,
    },
    /// The file declares a newer dialect than this build reads; editing
    /// constructs we cannot parse would corrupt them.
    #[error("file declares dialect {found}; this build writes {DIALECT_VERSION} — not editing")]
    FutureVersion {
        /// The declared version.
        found: u32,
    },
    /// A span (from a diagnostic) does not lie inside the document.
    #[error("span out of range: line {line}")]
    InvalidSpan {
        /// The 1-based line.
        line: usize,
    },
}

/// Place a node (docs/10: append `name = fn(…)` after the last dependency,
/// or at EOF; auto-name `fn_1`/`fn_2`-style — a binding never takes a bare
/// callable name, which would shadow the node for later calls, docs/10 §5).
/// `dependencies` are the bindings the placement is wired from (empty for
/// a bare palette drop). Returns the chosen name. Required ports start
/// unwired — the node is red until wired, exactly like the canvas.
///
/// # Errors
///
/// [`WriterError::FutureVersion`] / [`WriterError::UnknownBinding`] (a
/// named dependency that no statement binds — never a silent EOF fallback).
pub fn place(
    document: &mut Document,
    func: &str,
    dependencies: &[&str],
) -> Result<String, WriterError> {
    guard_version(document)?;
    let mut insert_at = document.line_count();
    if !dependencies.is_empty() {
        let mut last_dependency = 0;
        for dependency in dependencies {
            // A disabled dependency is still a line to place after: the
            // new wire to it is red ("disabled") until it is re-enabled.
            let line = document
                .find_binding(dependency)
                .or_else(|| document.find_disabled(dependency))
                .ok_or_else(|| WriterError::UnknownBinding((*dependency).to_owned()))?;
            last_dependency = last_dependency.max(line);
        }
        insert_at = last_dependency + 1;
    }
    let name = auto_name(document, func);
    let raw = format!("{name} = {func}()");
    document.insert_line(insert_at, &raw);
    Ok(name)
}

/// Draw a wire (docs/10: rewrite one kwarg in the target binding). Sets
/// `port=value_text`, replacing the existing kwarg value or inserting the
/// kwarg — at its spec-order position when `spec_order` (the port names in
/// catalog order) is given, else appended. `value_text` is a reference
/// (`source` / `source.port`).
///
/// # Errors
///
/// [`WriterError::UnknownBinding`] / [`WriterError::NotACall`] /
/// [`WriterError::ProducedBrokenStatement`] / [`WriterError::FutureVersion`].
pub fn set_kwarg(
    document: &mut Document,
    binding: &str,
    port: &str,
    value_text: &str,
    spec_order: Option<&[&str]>,
) -> Result<(), WriterError> {
    guard_version(document)?;
    let (line, statement) = call_statement(document, binding)?;
    let Rhs::Call(call) = &statement.rhs else {
        unreachable!("call_statement checked")
    };
    if let Some(kwarg) = call.kwargs.iter().find(|k| k.name.name == port) {
        let span = kwarg.value.span();
        return checked_splice(document, line, span, value_text);
    }
    // Insert. Position: before the first present kwarg that comes AFTER
    // this port in spec order ("kwargs in spec order", docs/10 writer
    // discipline); unknown spec → append.
    let insert_before = spec_order.and_then(|order| {
        let position = order.iter().position(|p| *p == port)?;
        call.kwargs.iter().find(|k| {
            order
                .iter()
                .position(|p| *p == k.name.name)
                .is_some_and(|kp| kp > position)
        })
    });
    let (at, insertion) = match insert_before {
        Some(next_kwarg) => (next_kwarg.span.start, format!("{port}={value_text}, ")),
        None => (
            call.close_paren,
            if call.kwargs.is_empty() {
                format!("{port}={value_text}")
            } else {
                format!(", {port}={value_text}")
            },
        ),
    };
    let span = LineSpan { start: at, end: at };
    checked_splice(document, line, span, &insertion)
}

/// Remove a wire / clear a kwarg (stage 5: deleting an edge on the canvas
/// — the inverse of [`set_kwarg`]). Removes `port=value` together with ONE
/// adjacent separator, touching nothing else: `f(a=x, b=y)` → `f(b=y)` /
/// `f(a=x)`; `f(a=x)` → `f()`. A required port left unwired reds the node
/// (missing-kwarg), never a silent default — the honest state of "this
/// wire is gone".
///
/// # Errors
///
/// [`WriterError::UnknownBinding`] / [`WriterError::NotACall`] /
/// [`WriterError::UnknownKwarg`] / [`WriterError::FutureVersion`].
pub fn remove_kwarg(document: &mut Document, binding: &str, port: &str) -> Result<(), WriterError> {
    guard_version(document)?;
    let (line, statement) = call_statement(document, binding)?;
    let Rhs::Call(call) = &statement.rhs else {
        unreachable!("call_statement checked")
    };
    let position = call
        .kwargs
        .iter()
        .position(|k| k.name.name == port)
        .ok_or_else(|| WriterError::UnknownKwarg {
            binding: binding.to_owned(),
            kwarg: port.to_owned(),
        })?;
    let kwarg = &call.kwargs[position];
    let raw = document.lines()[line].raw();
    // Span to cut: the kwarg plus the separator that joins it to a
    // neighbour — the FOLLOWING `, ` when one exists (so the first kwarg's
    // removal leaves the second flush against `(`), else the PRECEDING one.
    let (start, end) = if let Some(next) = call.kwargs.get(position + 1) {
        (kwarg.span.start, next.span.start)
    } else if position > 0 {
        (call.kwargs[position - 1].span.end, kwarg.span.end)
    } else {
        (kwarg.span.start, kwarg.span.end)
    };
    debug_assert!(start <= end && end <= raw.len());
    checked_splice(document, line, LineSpan { start, end }, "")
}

/// Accept a lift chip (docs/10: wrap that kwarg's value in `each(…)`).
///
/// # Errors
///
/// [`WriterError::UnknownBinding`] / [`WriterError::NotACall`] /
/// [`WriterError::UnknownKwarg`] / [`WriterError::FutureVersion`].
pub fn wrap_each(document: &mut Document, binding: &str, port: &str) -> Result<(), WriterError> {
    guard_version(document)?;
    let (line, statement) = call_statement(document, binding)?;
    let Rhs::Call(call) = &statement.rhs else {
        unreachable!("call_statement checked")
    };
    let kwarg = call
        .kwargs
        .iter()
        .find(|k| k.name.name == port)
        .ok_or_else(|| WriterError::UnknownKwarg {
            binding: binding.to_owned(),
            kwarg: port.to_owned(),
        })?;
    let span = kwarg.value.span();
    let raw_line = document.lines()[line].raw();
    let wrapped = format!("each({})", span.slice(raw_line));
    checked_splice(document, line, span, &wrapped)
}

/// Drag a slider / edit a param on a call binding (docs/10: rewrite one
/// numeric literal), or — wave 4 B3, typing a value into an unconnected
/// port of a placed node — ADD the kwarg the call lacks
/// (`construct_domain()` → `construct_domain(end=40.0)`), inserted at its
/// spec-order position when `spec_order` (the port names in catalog
/// order) is given, else appended, exactly as [`set_kwarg`] inserts a
/// wire. `literal_text` is the new literal (shortest round-trip repr).
///
/// # Errors
///
/// As [`set_kwarg`].
pub fn set_param(
    document: &mut Document,
    binding: &str,
    port: &str,
    literal_text: &str,
    spec_order: Option<&[&str]>,
) -> Result<(), WriterError> {
    set_kwarg(document, binding, port, literal_text, spec_order)
}

/// Edit a bare-literal constant binding (`count = 40` → `count = 56`).
///
/// # Errors
///
/// [`WriterError::UnknownBinding`] / [`WriterError::NotALiteral`] /
/// [`WriterError::ProducedBrokenStatement`] / [`WriterError::FutureVersion`].
pub fn set_literal(
    document: &mut Document,
    binding: &str,
    literal_text: &str,
) -> Result<(), WriterError> {
    guard_version(document)?;
    let line = live_binding(document, binding)?;
    let Line::Statement { statement, .. } = &document.lines()[line] else {
        return Err(WriterError::UnknownBinding(binding.to_owned()));
    };
    let Rhs::Literal(lit) = &statement.rhs else {
        return Err(WriterError::NotALiteral(binding.to_owned()));
    };
    let span = lit.span;
    checked_splice(document, line, span, literal_text)
}

/// Delete a node (docs/10: delete its statement; downstream references
/// become red unknown-name errors — NEVER cascade deletion).
///
/// # Errors
///
/// [`WriterError::UnknownBinding`] / [`WriterError::FutureVersion`].
pub fn delete(document: &mut Document, binding: &str) -> Result<(), WriterError> {
    guard_version(document)?;
    // A `#off`-disabled ghost is deletable too — it is the user's line.
    let line = document
        .find_binding(binding)
        .or_else(|| document.find_disabled(binding))
        .ok_or_else(|| WriterError::UnknownBinding(binding.to_owned()))?;
    document.remove_line(line);
    Ok(())
}

/// The state [`toggle_disable`] left the binding in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableState {
    /// The statement now carries the `#off ` prefix (a ghost; skipped in
    /// solves; downstream references red as "disabled").
    Disabled,
    /// The prefix is gone: a live statement again (or a Broken line, when
    /// the body behind the prefix never parsed — enabling surfaces it).
    Enabled,
}

/// The `#off ` prefix the writer puts on a disabled statement (docs/10
/// gesture table: "prefix / unprefix the statement with `#off `").
const OFF_PREFIX: &str = "#off ";

/// Toggle disable on a binding (docs/10 §1 + gesture table; DECISIONS.md
/// node-disable row): a live statement gets `#off ` in front of its text and
/// becomes a [`Line::Disabled`] whose parse is kept (ports and wiring
/// intact on the canvas); a disabled line loses exactly that prefix and is
/// a statement again. The rest of the line — spacing, comment, CRLF — is
/// untouched either way, so disable → enable is byte-identical. Re-enabling
/// a line whose body does not parse yields a `Broken` line: the honest
/// state of text the prefix was hiding, never a silent refusal.
///
/// # Errors
///
/// [`WriterError::UnknownBinding`] (neither a live nor a disabled line
/// binds the name) / [`WriterError::FutureVersion`].
pub fn toggle_disable(document: &mut Document, binding: &str) -> Result<DisableState, WriterError> {
    guard_version(document)?;
    if let Some(line) = document.find_binding(binding) {
        document.splice(line, LineSpan { start: 0, end: 0 }, OFF_PREFIX);
        debug_assert!(
            matches!(document.lines()[line], Line::Disabled { .. }),
            "`#off ` in front of a parsed statement classifies as Disabled"
        );
        return Ok(DisableState::Disabled);
    }
    let line = document
        .find_disabled(binding)
        .ok_or_else(|| WriterError::UnknownBinding(binding.to_owned()))?;
    let raw = document.lines()[line].raw();
    // The prefix sits after any leading whitespace / BOM (`classify_line`
    // trims exactly that before it looks for `#`).
    let start = raw
        .find('#')
        .filter(|&at| raw[at..].starts_with(OFF_PREFIX))
        .ok_or_else(|| WriterError::UnknownBinding(binding.to_owned()))?;
    document.splice(
        line,
        LineSpan {
            start,
            end: start + OFF_PREFIX.len(),
        },
        "",
    );
    Ok(DisableState::Enabled)
}

/// Rename a node (docs/10: rename binding + all references atomically; the
/// layout sidecar key moves with it at stage 5). Caches key on content
/// hashes, so a rename never invalidates results. References inside Broken
/// lines cannot be renamed (they have no parse); the definition — live or
/// `#off`-disabled — and every parsed reference move together.
///
/// # Errors
///
/// [`WriterError::UnknownBinding`] / [`WriterError::NameTaken`] /
/// [`WriterError::InvalidName`] / [`WriterError::FutureVersion`].
pub fn rename(document: &mut Document, old: &str, new: &str) -> Result<(), WriterError> {
    guard_version(document)?;
    if !is_valid_binding_name(new) {
        return Err(WriterError::InvalidName(new.to_owned()));
    }
    // Taken by a live statement, or held by a `#off`/broken line the user
    // may re-enable or fix next — either way the rename would plant a
    // rebinding error.
    if document.find_binding(new).is_some() || name_shadowed(document, new) {
        return Err(WriterError::NameTaken(new.to_owned()));
    }
    let definition = document
        .find_binding(old)
        .or_else(|| document.find_disabled(old))
        .ok_or_else(|| WriterError::UnknownBinding(old.to_owned()))?;

    // Collect every span to rewrite, per line: the defining target plus
    // all references (kwarg refs, expression free vars) — inside
    // `#off`-disabled lines too, so re-enabling one never meets a stale
    // name. References inside Broken lines have no parse to rename.
    let mut edits: Vec<(usize, LineSpan)> = Vec::new();
    for (line, statement, _, _) in document.statements_including_disabled() {
        if line == definition {
            for target in &statement.targets {
                if target.name == old {
                    edits.push((line, target.span));
                }
            }
        }
        for reference in statement.references() {
            if reference.name == old {
                edits.push((line, reference.span));
            }
        }
    }
    // Splice back-to-front within each line so earlier spans stay valid.
    // A valid-identifier rename cannot break a parsed line, so no revert
    // path is needed here.
    edits.sort_by(|a, b| (a.0, a.1.start).cmp(&(b.0, b.1.start)).reverse());
    for (line, span) in edits {
        document.splice(line, span, new);
    }
    Ok(())
}

/// Apply a machine-applicable diagnostic fix: replace `span` (a doc-11
/// diagnostic span) with `replacement` — the doc-11 "iterate until green"
/// loop's apply step (docs/11).
///
/// # Errors
///
/// [`WriterError::InvalidSpan`] / [`WriterError::ProducedBrokenStatement`]
/// (a fix must repair or preserve the statement, never break it) /
/// [`WriterError::FutureVersion`].
pub fn apply_fix(
    document: &mut Document,
    span: Span,
    replacement: &str,
) -> Result<(), WriterError> {
    guard_version(document)?;
    let line = span
        .line
        .checked_sub(1)
        .filter(|l| *l < document.line_count())
        .ok_or(WriterError::InvalidSpan { line: span.line })?;
    let raw = document.lines()[line].raw();
    if span.col_end > raw.len() || span.col_start > span.col_end {
        return Err(WriterError::InvalidSpan { line: span.line });
    }
    let local = LineSpan {
        start: span.col_start,
        end: span.col_end,
    };
    checked_splice(document, line, local, replacement)
}

// ------------------------------------------------------------- helpers --

/// Splice, then verify the edited line still parses; revert and error if
/// the gesture would have broken it (fail loudly, leave the file intact).
fn checked_splice(
    document: &mut Document,
    line: usize,
    span: LineSpan,
    text: &str,
) -> Result<(), WriterError> {
    let was_parsed = !matches!(document.lines()[line], Line::Broken { .. });
    let old_raw = document.lines()[line].raw().to_owned();
    document.splice(line, span, text);
    if was_parsed && let Line::Broken { issue, .. } = &document.lines()[line] {
        let why = issue.message.clone();
        let full = LineSpan {
            start: 0,
            end: document.lines()[line].raw().len(),
        };
        document.splice(line, full, &old_raw);
        return Err(WriterError::ProducedBrokenStatement {
            text: text.to_owned(),
            why,
        });
    }
    Ok(())
}

fn guard_version(document: &Document) -> Result<(), WriterError> {
    match document.version() {
        Some(found) if found > DIALECT_VERSION => Err(WriterError::FutureVersion { found }),
        _ => Ok(()),
    }
}

fn auto_name(document: &Document, func: &str) -> String {
    let mut counter = 1;
    loop {
        let candidate = format!("{func}_{counter}");
        if document.find_binding(&candidate).is_none() && !name_shadowed(document, &candidate) {
            return candidate;
        }
        counter += 1;
    }
}

/// Names held by Broken or Disabled lines still block auto-naming and
/// renames — the user fixing/re-enabling their line must not inherit a
/// rebinding error.
fn name_shadowed(document: &Document, name: &str) -> bool {
    document.find_disabled(name).is_some()
        || document.lines().iter().any(|line| match line {
            Line::Broken {
                node: Some(node), ..
            } => node == name,
            _ => false,
        })
}

/// The line of the LIVE statement binding `binding` — a `#off`-disabled
/// one is refused by name ("enable it to edit"), never as "no binding".
fn live_binding(document: &Document, binding: &str) -> Result<usize, WriterError> {
    if let Some(line) = document.find_binding(binding) {
        return Ok(line);
    }
    if document.find_disabled(binding).is_some() {
        return Err(WriterError::Disabled(binding.to_owned()));
    }
    Err(WriterError::UnknownBinding(binding.to_owned()))
}

fn call_statement<'a>(
    document: &'a Document,
    binding: &str,
) -> Result<(usize, &'a Statement), WriterError> {
    let line = live_binding(document, binding)?;
    let Line::Statement { statement, .. } = &document.lines()[line] else {
        return Err(WriterError::UnknownBinding(binding.to_owned()));
    };
    if matches!(statement.rhs, Rhs::Call(_)) {
        Ok((line, statement))
    } else {
        Err(WriterError::NotACall(binding.to_owned()))
    }
}
