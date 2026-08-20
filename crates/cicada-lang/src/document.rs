//! The lossless document model: a `.cic` file as lines, each line owning
//! its raw text. Emission concatenates raw lines — byte-identical
//! round-trips hold by construction, and the minimal-edit writer touches
//! only the raw text of the lines a gesture implies (docs/10 §Round-trip
//! contract).

use crate::ast::{LineSpan, Statement};
use crate::diag::{Diagnostic, DiagnosticKind, Span};
use crate::parse::{ParseIssue, parse_statement};

/// The dialect version this build reads and writes.
pub const DIALECT_VERSION: u32 = 1;

/// One line of a `.cic` file.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// Whitespace-only.
    Blank {
        /// Raw text (preserved exactly).
        raw: String,
    },
    /// A comment line (attaches to the following binding as its canvas
    /// note — the attachment is computed by consumers, stage 5).
    Comment {
        /// Raw text.
        raw: String,
    },
    /// The `# cicada N` version pragma (line 1).
    Pragma {
        /// Raw text.
        raw: String,
        /// Declared version.
        version: u32,
    },
    /// A parsed binding statement.
    Statement {
        /// Raw text — spans in `statement` index into this.
        raw: String,
        /// The parse.
        statement: Statement,
    },
    /// A `#off `-disabled binding (docs/10 §1, DECISIONS.md node-disable
    /// row): a comment to generic tools, but Cicada parses the binding
    /// behind the prefix so the canvas can ghost the node WITH its ports
    /// and wiring, downstream references error as "disabled" (never
    /// unknown-name), and the solve skips it. Not a definition: the
    /// checker, `statements()` and `find_binding` do not see it.
    Disabled {
        /// Raw text (prefix included).
        raw: String,
        /// The disabled binding's name: the parsed statement's, else the
        /// first identifier after the prefix, else `None`.
        name: Option<String>,
        /// The parse of the text after `#off `, when it parses — its spans
        /// index `raw` exactly like a `Statement`'s do. `None` when the
        /// body does not parse (re-enabling it yields a `Broken` line).
        statement: Option<Statement>,
    },
    /// A statement that failed to parse. Reds ITS node; the rest of the
    /// file is unaffected (docs/10 constraint 4).
    Broken {
        /// Raw text, preserved untouched.
        raw: String,
        /// What went wrong.
        issue: ParseIssue,
        /// Best-effort binding name (first identifier), so the diagnostic
        /// can name the red node.
        node: Option<String>,
    },
}

impl Line {
    /// The line's raw text.
    #[must_use]
    pub fn raw(&self) -> &str {
        match self {
            Self::Blank { raw }
            | Self::Comment { raw }
            | Self::Pragma { raw, .. }
            | Self::Statement { raw, .. }
            | Self::Disabled { raw, .. }
            | Self::Broken { raw, .. } => raw,
        }
    }
}

/// A parsed `.cic` document. Parsing is total — every input produces a
/// document; problems surface as diagnostics, never failures.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    lines: Vec<Line>,
    trailing_newline: bool,
}

impl Document {
    /// Parse a source text. Total: never fails.
    #[must_use]
    pub fn parse(source: &str) -> Self {
        let trailing_newline = source.ends_with('\n');
        let body = source.strip_suffix('\n').unwrap_or(source);
        let lines = if body.is_empty() && source.is_empty() {
            Vec::new()
        } else {
            body.split('\n')
                .enumerate()
                .map(|(index, raw)| classify_line(raw, index == 0))
                .collect()
        };
        Self {
            lines,
            trailing_newline,
        }
    }

    /// Emit the document — byte-identical for untouched lines by
    /// construction.
    #[must_use]
    pub fn emit(&self) -> String {
        let mut out = self
            .lines
            .iter()
            .map(Line::raw)
            .collect::<Vec<_>>()
            .join("\n");
        if self.trailing_newline {
            out.push('\n');
        }
        out
    }

    /// The declared dialect version, if a pragma exists.
    #[must_use]
    pub fn version(&self) -> Option<u32> {
        self.lines.iter().find_map(|line| match line {
            Line::Pragma { version, .. } => Some(*version),
            _ => None,
        })
    }

    /// The lines.
    #[must_use]
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Parsed statements with their 0-based line indices.
    pub fn statements(&self) -> impl Iterator<Item = (usize, &Statement, &str)> {
        self.lines.iter().enumerate().filter_map(|(index, line)| {
            if let Line::Statement { statement, raw } = line {
                Some((index, statement, raw.as_str()))
            } else {
                None
            }
        })
    }

    /// The 0-based line index of the statement binding `name` (any unpack
    /// target counts). A `#off`-disabled binding is not one — see
    /// [`Self::find_disabled`].
    #[must_use]
    pub fn find_binding(&self, name: &str) -> Option<usize> {
        self.statements()
            .find(|(_, statement, _)| statement.targets.iter().any(|t| t.name == name))
            .map(|(index, _, _)| index)
    }

    /// The 0-based line index of the `#off`-disabled line holding `name`
    /// (any target of its parsed statement, else its extracted name).
    #[must_use]
    pub fn find_disabled(&self, name: &str) -> Option<usize> {
        self.lines.iter().position(|line| match line {
            Line::Disabled {
                statement: Some(statement),
                ..
            } => statement.targets.iter().any(|t| t.name == name),
            Line::Disabled {
                name: Some(held), ..
            } => held == name,
            _ => false,
        })
    }

    /// Every parsed statement — live AND `#off`-disabled — with its line
    /// index, raw text and disabled flag: for the gestures that must see
    /// through the prefix (rename rewrites references inside a disabled
    /// line too, so re-enabling it never meets a stale name).
    pub fn statements_including_disabled(
        &self,
    ) -> impl Iterator<Item = (usize, &Statement, &str, bool)> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| match line {
                Line::Statement { statement, raw } => Some((index, statement, raw.as_str(), false)),
                Line::Disabled {
                    statement: Some(statement),
                    raw,
                    ..
                } => Some((index, statement, raw.as_str(), true)),
                _ => None,
            })
    }

    /// Parse-level diagnostics: pragma problems and broken statements.
    #[must_use]
    pub fn parse_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        match self.lines.first() {
            Some(Line::Pragma { version, raw }) => {
                if *version > DIALECT_VERSION {
                    diagnostics.push(Diagnostic::new(
                        DiagnosticKind::FutureVersion,
                        Span {
                            line: 1,
                            col_start: 0,
                            col_end: raw.len(),
                        },
                        format!(
                            "this file needs a newer Cicada (dialect {version}; \
                             this build reads {DIALECT_VERSION})"
                        ),
                    ));
                }
            }
            Some(line) => diagnostics.push(Diagnostic::new(
                DiagnosticKind::MissingPragma,
                Span {
                    line: 1,
                    col_start: 0,
                    col_end: line.raw().len(),
                },
                format!("first line must be the version pragma `# cicada {DIALECT_VERSION}`"),
            )),
            // An empty file needs its pragma too — same rule as "\n".
            None => diagnostics.push(Diagnostic::new(
                DiagnosticKind::MissingPragma,
                Span {
                    line: 1,
                    col_start: 0,
                    col_end: 0,
                },
                format!("first line must be the version pragma `# cicada {DIALECT_VERSION}`"),
            )),
        }
        for (index, line) in self.lines.iter().enumerate() {
            if let Line::Broken { issue, node, .. } = line {
                let mut diagnostic = Diagnostic::new(
                    issue.kind,
                    line_span_to_file(index, issue.span),
                    issue.message.clone(),
                );
                if let Some(node) = node {
                    diagnostic = diagnostic.with_node(node.clone());
                }
                diagnostics.push(diagnostic);
            }
        }
        diagnostics
    }

    // ------------------------------------------------------ writer API --
    // pub(crate): gestures go through `writer`, which owns the round-trip
    // contract; these are its splicing primitives.

    /// Replace `span` within line `index` with `text`, re-parsing the line.
    pub(crate) fn splice(&mut self, index: usize, span: LineSpan, text: &str) {
        let raw = self.lines[index].raw();
        let mut new_raw = String::with_capacity(raw.len() + text.len());
        new_raw.push_str(&raw[..span.start]);
        new_raw.push_str(text);
        new_raw.push_str(&raw[span.end..]);
        self.lines[index] = classify_line(&new_raw, index == 0);
    }

    /// Insert a new line at `index` (existing lines shift down). Appending
    /// at EOF restores the trailing newline — written content always ends
    /// with one (docs/10 writer discipline: "newline at EOF").
    pub(crate) fn insert_line(&mut self, index: usize, raw: &str) {
        if index == self.lines.len() {
            self.trailing_newline = true;
        }
        self.lines.insert(index, classify_line(raw, index == 0));
    }

    /// Remove the line at `index`.
    pub(crate) fn remove_line(&mut self, index: usize) {
        self.lines.remove(index);
    }

    /// Number of lines.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

/// A line-local span placed in the file (1-based line).
#[must_use]
pub fn line_span_to_file(line_index: usize, span: LineSpan) -> Span {
    Span {
        line: line_index + 1,
        col_start: span.start,
        col_end: span.end,
    }
}

fn classify_line(raw: &str, is_first: bool) -> Line {
    // A trailing '\r' (CRLF file) is part of the line terminator, not the
    // statement: classify and parse without it, emit with it (raw stays
    // byte-exact; spans index the shared prefix, so both views agree).
    let parse_region = raw.strip_suffix('\r').unwrap_or(raw);

    // A leading BOM on line 1 (default for several Windows editors) must
    // not defeat pragma recognition; it stays in raw for emission.
    let trimmed = if is_first {
        parse_region.trim_start_matches('\u{feff}').trim_start()
    } else {
        parse_region.trim_start()
    };
    if trimmed.is_empty() {
        return Line::Blank {
            raw: raw.to_owned(),
        };
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        if is_first && let Some(version) = parse_pragma(rest) {
            return Line::Pragma {
                raw: raw.to_owned(),
                version,
            };
        }
        // `#off name = …` is doc-10 syntax for a disabled binding. The body
        // is parsed in place — padded with spaces where the prefix sits, so
        // every span indexes `raw` exactly as a live statement's would and
        // the writer's splices work unchanged on a disabled line.
        if let Some(disabled) = rest.strip_prefix("off ") {
            let offset = parse_region.len() - disabled.len();
            let padded = format!("{}{disabled}", " ".repeat(offset));
            let (name, statement) = match parse_statement(&padded) {
                Ok(statement) => (Some(statement.name().to_owned()), Some(statement)),
                Err(_) => (first_ident(disabled), None),
            };
            return Line::Disabled {
                raw: raw.to_owned(),
                name,
                statement,
            };
        }
        return Line::Comment {
            raw: raw.to_owned(),
        };
    }
    match parse_statement(parse_region) {
        Ok(statement) => Line::Statement {
            raw: raw.to_owned(),
            statement,
        },
        Err(issue) => {
            let node = first_ident(parse_region);
            Line::Broken {
                raw: raw.to_owned(),
                issue,
                node,
            }
        }
    }
}

/// ` cicada N` after the `#`.
fn parse_pragma(rest: &str) -> Option<u32> {
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("cicada")?;
    let rest = rest.trim();
    rest.parse().ok()
}

fn first_ident(raw: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    let end = trimmed
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(trimmed.len());
    let candidate = &trimmed[..end];
    let starts_ok = candidate
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    starts_ok.then(|| candidate.to_owned())
}
