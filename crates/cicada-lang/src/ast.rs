//! The dialect AST (docs/10 §Statement forms), spike subset: bindings,
//! kwargs-only calls, literals, `each()`, expression RHS, multi-output
//! unpack, port selection.
//!
//! Every node carries **line-local byte spans** into the statement's raw
//! text — the minimal-edit writer splices at spans and never reformats, so
//! spans are the round-trip contract's currency. Axis annotations and
//! `#off` disabled bindings arrive with later stages (doc 15 scope).

/// Byte range within one line's raw text (`end` exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    /// Start byte offset.
    pub start: usize,
    /// End byte offset, exclusive.
    pub end: usize,
}

impl LineSpan {
    /// The spanned slice of `raw`.
    #[must_use]
    pub fn slice<'a>(&self, raw: &'a str) -> &'a str {
        &raw[self.start..self.end]
    }
}

/// An identifier with its span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    /// The name text.
    pub name: String,
    /// Where it sits in the line.
    pub span: LineSpan,
}

/// A reference to a binding, optionally selecting one output port
/// (`field.dirs`). Port selection is a reference, not a node (docs/10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRef {
    /// The binding name.
    pub binding: Ident,
    /// The selected output port, if any.
    pub port: Option<Ident>,
    /// Span of the whole reference.
    pub span: LineSpan,
}

/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    /// A number. The source text is recoverable via the span into the
    /// line's raw text — the Document owns round-trip fidelity (`12.0`
    /// stays `12.0` because untouched lines re-emit their raw bytes).
    Number {
        /// Parsed value.
        value: f64,
        /// True when written without `.`/exponent (an Integer literal).
        integer: bool,
    },
    /// A string literal (contents, unescaped).
    Text(String),
    /// `True` / `False`.
    Boolean(bool),
    /// A literal list `[a, b, c]` (literals only — node references in
    /// lists are not part of the spike subset).
    List(Vec<LitWithSpan>),
}

/// A literal plus its span.
#[derive(Debug, Clone, PartialEq)]
pub struct LitWithSpan {
    /// The literal.
    pub lit: Lit,
    /// Where.
    pub span: LineSpan,
}

/// A kwarg value: literal, reference, or `each(…)` lift (docs/10 §2 —
/// `each` is syntax, not a node; nesting = deeper map).
#[derive(Debug, Clone, PartialEq)]
pub enum ValueExpr {
    /// Literal.
    Literal(LitWithSpan),
    /// Reference to a binding / port.
    Ref(PortRef),
    /// `each(inner)` — one lift level.
    Each {
        /// The lifted value.
        inner: Box<ValueExpr>,
        /// Span of the whole `each(…)` expression.
        span: LineSpan,
    },
}

impl ValueExpr {
    /// Span of the whole value expression.
    #[must_use]
    pub fn span(&self) -> LineSpan {
        match self {
            Self::Literal(l) => l.span,
            Self::Ref(r) => r.span,
            Self::Each { span, .. } => *span,
        }
    }

    /// How many `each(…)` levels wrap this value. Saturating (the parser
    /// caps nesting far below `u8::MAX`; this is defense in depth).
    #[must_use]
    pub fn each_depth(&self) -> u8 {
        match self {
            Self::Each { inner, .. } => 1u8.saturating_add(inner.each_depth()),
            _ => 0,
        }
    }

    /// The value inside all `each(…)` wrappers.
    #[must_use]
    pub fn unlifted(&self) -> &ValueExpr {
        match self {
            Self::Each { inner, .. } => inner.unlifted(),
            other => other,
        }
    }
}

/// One keyword argument (`port=value`). All arguments are kwargs — the
/// dialect has no positional arguments (DECISIONS.md grammar row).
#[derive(Debug, Clone, PartialEq)]
pub struct Kwarg {
    /// The port name.
    pub name: Ident,
    /// The value.
    pub value: ValueExpr,
    /// Span from the name's first byte to the value's last.
    pub span: LineSpan,
}

/// A node call (`fn(a=x, b=y)`).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The node function name.
    pub func: Ident,
    /// The kwargs, in written order.
    pub kwargs: Vec<Kwarg>,
    /// Byte offset of the closing `)` — where the writer appends kwargs.
    pub close_paren: usize,
}

/// Binary operators of the expression language (`^` is power; `**` is
/// accepted and preserved as written).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `^` or `**`
    Pow,
}

/// An expression-node AST (docs/10 §4): language-neutral math; free
/// variables become input ports in order of first appearance.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal.
    Number {
        /// Parsed value.
        value: f64,
        /// Written without `.`/exponent (an Integer literal) — feeds the
        /// checker's integer-preserving expression inference.
        integer: bool,
        /// Where.
        span: LineSpan,
    },
    /// Free variable (a binding reference).
    Var(Ident),
    /// Unary negation.
    Neg {
        /// Operand.
        operand: Box<Expr>,
        /// Whole span.
        span: LineSpan,
    },
    /// Binary operation.
    Binary {
        /// Operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
        /// Whole span.
        span: LineSpan,
    },
}

impl Expr {
    /// Free variables in order of first appearance — the Expression node's
    /// input ports (docs/10 §4).
    #[must_use]
    pub fn free_vars(&self) -> Vec<&Ident> {
        let mut seen: Vec<&Ident> = Vec::new();
        self.collect_vars(&mut seen);
        seen
    }

    fn collect_vars<'a>(&'a self, seen: &mut Vec<&'a Ident>) {
        match self {
            Self::Number { .. } => {}
            Self::Var(ident) => {
                if !seen.iter().any(|known| known.name == ident.name) {
                    seen.push(ident);
                }
            }
            Self::Neg { operand, .. } => operand.collect_vars(seen),
            Self::Binary { lhs, rhs, .. } => {
                lhs.collect_vars(seen);
                rhs.collect_vars(seen);
            }
        }
    }
}

/// The right-hand side of a binding.
#[derive(Debug, Clone, PartialEq)]
pub enum Rhs {
    /// One node call.
    Call(Call),
    /// A bare literal — a constant param node (docs/10 §3).
    Literal(LitWithSpan),
    /// An operator expression — one Expression node (docs/10 §4).
    Expression(Expr),
}

/// One parsed binding statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    /// Bound names: one, or several for multi-output unpack (spec order).
    pub targets: Vec<Ident>,
    /// The right-hand side.
    pub rhs: Rhs,
    /// Trailing comment span (from `#` to end of line), if present.
    pub trailing_comment: Option<LineSpan>,
}

impl Statement {
    /// The primary binding name (first target).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.targets[0].name
    }

    /// Every binding reference the statement makes (kwarg refs and
    /// expression free vars), for rename and unknown-name checks.
    #[must_use]
    pub fn references(&self) -> Vec<&Ident> {
        let mut refs = Vec::new();
        match &self.rhs {
            Rhs::Call(call) => {
                for kwarg in &call.kwargs {
                    collect_value_refs(&kwarg.value, &mut refs);
                }
            }
            Rhs::Expression(expr) => {
                refs.extend(expr.free_vars());
            }
            Rhs::Literal(_) => {}
        }
        refs
    }
}

fn collect_value_refs<'a>(value: &'a ValueExpr, refs: &mut Vec<&'a Ident>) {
    match value {
        ValueExpr::Ref(port_ref) => refs.push(&port_ref.binding),
        ValueExpr::Each { inner, .. } => collect_value_refs(inner, refs),
        ValueExpr::Literal(_) => {}
    }
}
