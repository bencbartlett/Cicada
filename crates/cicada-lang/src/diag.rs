//! Diagnostics — the typed, machine-readable errors of docs/11: kind, span,
//! expected/actual, suggested fix. The checker is the AI's inner loop
//! (milliseconds, no solve), so this shape IS the product surface; it
//! serializes to the doc-11 JSON the agents consume. User-facing problems
//! are these, never bare strings (doc 14).

use serde::Serialize;

/// What went wrong. Serialized as `snake_case` strings; append-only —
/// consumers match on these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    /// A statement failed to parse (reds one node, not the file).
    ParseError,
    /// Missing or malformed `# cicada N` first line.
    MissingPragma,
    /// The file's pragma demands a newer Cicada.
    FutureVersion,
    /// A name bound more than once (single assignment).
    Rebinding,
    /// Reference to a name no binding introduces.
    UnknownName,
    /// Call to a node the catalog does not know.
    UnknownNode,
    /// Positional arguments are parse errors — ports are named.
    PositionalArgument,
    /// Nested node calls — name the intermediate.
    NestedCall,
    /// Required kwarg not supplied.
    MissingKwarg,
    /// Kwarg that matches no port.
    UnknownKwarg,
    /// Wrong wire: value type does not fit the port and no lift fixes it.
    TypeMismatch,
    /// A list is wired into a scalar port — a `map` lift would fix it.
    NeedsLift,
    /// A value is wired into a checked-refinement port — the validating
    /// adapter would fix it (`as_closed`, `as_watertight`).
    NeedsAdapter,
    /// `each()` around a value that is not a list at that depth.
    EachOnScalar,
    /// Strict-zip length mismatch, counts included (statically known
    /// lengths only; runtime rechecks the rest).
    ZipLengthMismatch,
    /// Unpack target count ≠ node output count.
    UnpackArity,
    /// Port selection names a port the node does not have.
    UnknownPort,
    /// The bindings form a cycle — the semantics are a DAG.
    Cycle,
}

/// Where in the file, 1-based line, 0-based UTF-8 byte columns within the
/// line (`col_end` exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    /// 1-based line number.
    pub line: usize,
    /// 0-based byte column of the first offending byte.
    pub col_start: usize,
    /// 0-based byte column one past the last offending byte.
    pub col_end: usize,
}

/// A machine-applicable suggested fix (docs/11: "wrap in `each()`",
/// "insert `as_closed`", "name the intermediate").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fix {
    /// Human-readable action, e.g. `wrap in each()`.
    pub label: String,
    /// Replacement text for the diagnostic's span, when the fix is a pure
    /// splice; `None` when the fix needs a gesture (insert a binding).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
}

/// One diagnostic, the doc-11 JSON shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// What kind of problem.
    pub kind: DiagnosticKind,
    /// The binding (node) the problem reds, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Where.
    pub span: Span,
    /// Human-readable message. Domain-quality, never rustc-style.
    pub message: String,
    /// What the port/context wanted, catalog notation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// What the wire actually carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    /// Machine-applicable suggestion, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
}

impl Diagnostic {
    /// Minimal constructor for the common fields; extend with the
    /// builder-style setters.
    #[must_use]
    pub fn new(kind: DiagnosticKind, span: Span, message: impl Into<String>) -> Self {
        Self {
            kind,
            node: None,
            span,
            message: message.into(),
            expected: None,
            actual: None,
            fix: None,
        }
    }

    /// Attach the red node's binding name.
    #[must_use]
    pub fn with_node(mut self, node: impl Into<String>) -> Self {
        self.node = Some(node.into());
        self
    }

    /// Attach expected/actual types (catalog notation).
    #[must_use]
    pub fn with_types(mut self, expected: impl Into<String>, actual: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self.actual = Some(actual.into());
        self
    }

    /// Attach a suggested fix.
    #[must_use]
    pub fn with_fix(mut self, label: impl Into<String>, replacement: Option<String>) -> Self {
        self.fix = Some(Fix {
            label: label.into(),
            replacement,
        });
        self
    }
}
