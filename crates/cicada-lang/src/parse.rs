//! The statement parser — boring to parse, total error recovery (docs/10
//! constraint 4): one statement per line, and a statement that fails to
//! parse reds ITS node, never the file. Errors are pointed and
//! domain-quality ("name the intermediate"), never parser jargon.

use crate::ast::{
    BinOp, Call, Expr, Ident, Kwarg, LineSpan, Lit, LitWithSpan, PortRef, Rhs, Statement, ValueExpr,
};
use crate::diag::DiagnosticKind;

/// A parse failure, line-local (the document layer adds line numbers and
/// the red node's name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIssue {
    /// Diagnostic kind (`ParseError`, `PositionalArgument`, `NestedCall`).
    pub kind: DiagnosticKind,
    /// Where in the line.
    pub span: LineSpan,
    /// The pointed message.
    pub message: String,
}

impl ParseIssue {
    fn new(kind: DiagnosticKind, span: LineSpan, message: impl Into<String>) -> Self {
        Self {
            kind,
            span,
            message: message.into(),
        }
    }
}

/// Statement keywords that signal someone writing a program, not a graph.
const CONTROL_KEYWORDS: &[&str] = &[
    "for", "while", "if", "else", "import", "from", "def", "class", "return", "with", "try",
];

/// Names a binding may never take: literals and lift syntax would shadow
/// them unreferenceably (`x = f(a=True)` always means the literal).
const RESERVED_NAMES: &[&str] = &["True", "False", "each"];

/// Nesting ceiling for values and expressions. Recursion is bounded so one
/// adversarial line can never overflow the stack (docs/10 constraint 4:
/// the file — and the process — always survives); doc 09 puts the sane
/// lift ceiling near ×2, so 64 is generous.
const MAX_NESTING: u32 = 64;

/// Is `name` a legal binding identifier (charset + not a reserved word)?
/// The writer validates rename/place inputs with this.
pub(crate) fn is_valid_binding_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !CONTROL_KEYWORDS.contains(&name)
        && !RESERVED_NAMES.contains(&name)
}

fn too_deep(span: LineSpan) -> ParseIssue {
    ParseIssue::new(
        DiagnosticKind::ParseError,
        span,
        "nesting too deep — name intermediate bindings instead",
    )
}

/// Parse one statement line (caller has already excluded blank/comment
/// lines). Total: every failure is a `ParseIssue`, never a panic.
///
/// # Errors
///
/// A [`ParseIssue`] with a pointed, domain-quality message — the document
/// layer turns it into a `Broken` line that reds one node.
pub fn parse_statement(raw: &str) -> Result<Statement, ParseIssue> {
    let mut cursor = Cursor::new(raw);
    cursor.skip_ws();

    // Targets: ident (',' ident)* '='
    let first = cursor.expect_ident("a binding name")?;
    if CONTROL_KEYWORDS.contains(&first.name.as_str()) {
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            first.span,
            format!(
                "`{}` is not a binding — multi-statement logic goes in a script node; \
                 loops are `each()`",
                first.name
            ),
        ));
    }
    let mut targets = vec![first];
    cursor.skip_ws();
    while cursor.eat(b',') {
        cursor.skip_ws();
        targets.push(cursor.expect_ident("a binding name after `,`")?);
        cursor.skip_ws();
    }
    for target in &targets {
        if RESERVED_NAMES.contains(&target.name.as_str()) {
            return Err(ParseIssue::new(
                DiagnosticKind::ParseError,
                target.span,
                format!(
                    "`{}` is reserved — a binding by that name could never be referenced",
                    target.name
                ),
            ));
        }
    }
    // Axis annotations (`cells: parts = …`) are locked grammar
    // (DECISIONS.md dialect row) deferred beyond stage 2 — refuse loudly
    // for the right reason, not "expected `=`".
    if cursor.peek() == Some(b':') {
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            cursor.here(),
            "axis annotations (`cells: parts = …`) arrive in a later spike stage",
        ));
    }
    if !cursor.eat(b'=') {
        // A bare call (`export_dxf(…)`) lands here: ident then `(`.
        if cursor.peek() == Some(b'(') {
            return Err(ParseIssue::new(
                DiagnosticKind::ParseError,
                targets[0].span,
                "bare calls are not statements — bind the result: `name = fn(…)`",
            ));
        }
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            cursor.here(),
            "expected `=` — the dialect is bindings only, one node per line",
        ));
    }
    cursor.skip_ws();

    // RHS.
    let rhs = parse_rhs(&mut cursor)?;
    if targets.len() > 1 && !matches!(rhs, Rhs::Call(_)) {
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            targets[1].span,
            "multi-output unpack needs a node call on the right-hand side",
        ));
    }

    // Only whitespace or a trailing comment may remain.
    cursor.skip_ws();
    let trailing_comment = if cursor.peek() == Some(b'#') {
        Some(LineSpan {
            start: cursor.pos,
            end: raw.len(),
        })
    } else if cursor.at_end() {
        None
    } else {
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            cursor.here(),
            "unexpected trailing tokens after the binding",
        ));
    };

    Ok(Statement {
        targets,
        rhs,
        trailing_comment,
    })
}

fn parse_rhs(cursor: &mut Cursor<'_>) -> Result<Rhs, ParseIssue> {
    match cursor.peek() {
        Some(b'"') => {
            let lit = parse_string(cursor)?;
            end_of_rhs(cursor)?;
            Ok(Rhs::Literal(lit))
        }
        Some(b'[') => {
            let lit = parse_list(cursor)?;
            end_of_rhs(cursor)?;
            Ok(Rhs::Literal(lit))
        }
        _ => {
            // Boolean literals, calls, expressions, and plain numbers all
            // start ident-or-number-like; look ahead.
            if let Some(lit) = try_parse_bool(cursor) {
                end_of_rhs(cursor)?;
                return Ok(Rhs::Literal(lit));
            }
            // A call is `ident(` at the very start of the RHS with nothing
            // after the close paren.
            let checkpoint = cursor.pos;
            if let Some(ident) = cursor.try_ident() {
                cursor.skip_ws();
                if cursor.peek() == Some(b'(') {
                    if ident.name == "each" {
                        return Err(ParseIssue::new(
                            DiagnosticKind::ParseError,
                            ident.span,
                            "each() marks iteration on a call's argument — it is not a node",
                        ));
                    }
                    let call = parse_call(cursor, ident)?;
                    cursor.skip_ws();
                    if cursor.at_end() || cursor.peek() == Some(b'#') {
                        return Ok(Rhs::Call(call));
                    }
                    return Err(ParseIssue::new(
                        DiagnosticKind::NestedCall,
                        cursor.here(),
                        "node calls don't mix with expressions — name the intermediate",
                    ));
                }
                cursor.pos = checkpoint;
            }
            // Expression (or a single literal/variable, classified after).
            let expr = parse_expr(cursor, 0)?;
            end_of_rhs(cursor)?;
            match expr {
                Expr::Number {
                    value,
                    integer,
                    span,
                } => Ok(Rhs::Literal(LitWithSpan {
                    lit: Lit::Number { value, integer },
                    span,
                })),
                Expr::Neg { operand, span } => match *operand {
                    Expr::Number { value, integer, .. } => Ok(Rhs::Literal(LitWithSpan {
                        lit: Lit::Number {
                            value: -value,
                            integer,
                        },
                        span,
                    })),
                    other => Ok(Rhs::Expression(Expr::Neg {
                        operand: Box::new(other),
                        span,
                    })),
                },
                Expr::Var(ident) => Err(ParseIssue::new(
                    DiagnosticKind::ParseError,
                    ident.span,
                    format!(
                        "`= {}` is an alias binding, which is not a statement — \
                         reference `{0}` directly where it is used",
                        ident.name
                    ),
                )),
                expr @ Expr::Binary { .. } => Ok(Rhs::Expression(expr)),
            }
        }
    }
}

fn end_of_rhs(cursor: &mut Cursor<'_>) -> Result<(), ParseIssue> {
    cursor.skip_ws();
    if cursor.at_end() || cursor.peek() == Some(b'#') {
        Ok(())
    } else {
        Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            cursor.here(),
            "unexpected tokens after the value",
        ))
    }
}

// ---------------------------------------------------------------- calls --

fn parse_call(cursor: &mut Cursor<'_>, func: Ident) -> Result<Call, ParseIssue> {
    assert!(cursor.eat(b'('), "caller checked");
    let mut kwargs = Vec::new();
    cursor.skip_ws();
    if !cursor.eat(b')') {
        loop {
            cursor.skip_ws();
            let kwarg = parse_kwarg(cursor)?;
            if kwargs
                .iter()
                .any(|k: &Kwarg| k.name.name == kwarg.name.name)
            {
                return Err(ParseIssue::new(
                    DiagnosticKind::ParseError,
                    kwarg.name.span,
                    format!("kwarg `{}` given twice", kwarg.name.name),
                ));
            }
            kwargs.push(kwarg);
            cursor.skip_ws();
            if cursor.eat(b',') {
                cursor.skip_ws();
                if cursor.peek() == Some(b')') {
                    return Err(ParseIssue::new(
                        DiagnosticKind::ParseError,
                        cursor.here(),
                        "expected `port=value` after `,` — remove the trailing comma",
                    ));
                }
                continue;
            }
            if cursor.eat(b')') {
                break;
            }
            return Err(ParseIssue::new(
                DiagnosticKind::ParseError,
                cursor.here(),
                "expected `,` or `)` in the argument list",
            ));
        }
    }
    Ok(Call {
        func,
        kwargs,
        close_paren: cursor.pos - 1,
    })
}

fn parse_kwarg(cursor: &mut Cursor<'_>) -> Result<Kwarg, ParseIssue> {
    let start = cursor.pos;
    let Some(name) = cursor.try_ident() else {
        return Err(ParseIssue::new(
            DiagnosticKind::PositionalArgument,
            cursor.here(),
            "ports are named — write `port=value`",
        ));
    };
    cursor.skip_ws();
    if !cursor.eat(b'=') {
        // `f(x)` — a bare name where `port=value` was needed.
        return Err(ParseIssue::new(
            DiagnosticKind::PositionalArgument,
            name.span,
            "ports are named — write `port=value`",
        ));
    }
    cursor.skip_ws();
    let value = parse_value(cursor)?;
    let span = LineSpan {
        start,
        end: value.span().end,
    };
    Ok(Kwarg { name, value, span })
}

fn parse_value(cursor: &mut Cursor<'_>) -> Result<ValueExpr, ParseIssue> {
    match cursor.peek() {
        Some(b'"') => Ok(ValueExpr::Literal(parse_string(cursor)?)),
        Some(b'[') => Ok(ValueExpr::Literal(parse_list(cursor)?)),
        Some(c) if c.is_ascii_digit() || c == b'-' || c == b'.' => {
            Ok(ValueExpr::Literal(parse_number(cursor)?))
        }
        _ => {
            if let Some(lit) = try_parse_bool(cursor) {
                return Ok(ValueExpr::Literal(lit));
            }
            let start = cursor.pos;
            let Some(ident) = cursor.try_ident() else {
                return Err(ParseIssue::new(
                    DiagnosticKind::ParseError,
                    cursor.here(),
                    "expected a value: literal, reference, or each(…)",
                ));
            };
            if ident.name == "each" {
                cursor.skip_ws();
                if !cursor.eat(b'(') {
                    return Err(ParseIssue::new(
                        DiagnosticKind::ParseError,
                        ident.span,
                        "each() wraps an argument — write `each(value)`",
                    ));
                }
                cursor.descend(ident.span)?;
                cursor.skip_ws();
                let inner = parse_value(cursor)?;
                cursor.ascend();
                cursor.skip_ws();
                if !cursor.eat(b')') {
                    return Err(ParseIssue::new(
                        DiagnosticKind::ParseError,
                        cursor.here(),
                        "expected `)` to close each(…)",
                    ));
                }
                return Ok(ValueExpr::Each {
                    inner: Box::new(inner),
                    span: LineSpan {
                        start,
                        end: cursor.pos,
                    },
                });
            }
            if cursor.peek() == Some(b'(') {
                return Err(ParseIssue::new(
                    DiagnosticKind::NestedCall,
                    LineSpan {
                        start,
                        end: cursor.pos,
                    },
                    format!(
                        "nested node call `{}(…)` — name the intermediate",
                        ident.name
                    ),
                ));
            }
            let port = if cursor.eat(b'.') {
                Some(cursor.expect_ident("a port name after `.`")?)
            } else {
                None
            };
            let end = port.as_ref().map_or(ident.span.end, |p| p.span.end);
            Ok(ValueExpr::Ref(PortRef {
                binding: ident,
                port,
                span: LineSpan { start, end },
            }))
        }
    }
}

// ------------------------------------------------------------- literals --

fn parse_number(cursor: &mut Cursor<'_>) -> Result<LitWithSpan, ParseIssue> {
    let start = cursor.pos;
    cursor.eat(b'-');
    let digits_start = cursor.pos;
    while cursor.peek().is_some_and(|c| c.is_ascii_digit()) {
        cursor.pos += 1;
    }
    if cursor.peek() == Some(b'.') {
        cursor.pos += 1;
        while cursor.peek().is_some_and(|c| c.is_ascii_digit()) {
            cursor.pos += 1;
        }
    }
    if matches!(cursor.peek(), Some(b'e' | b'E')) {
        cursor.pos += 1;
        if matches!(cursor.peek(), Some(b'+' | b'-')) {
            cursor.pos += 1;
        }
        while cursor.peek().is_some_and(|c| c.is_ascii_digit()) {
            cursor.pos += 1;
        }
    }
    let span = LineSpan {
        start,
        end: cursor.pos,
    };
    if cursor.pos == digits_start {
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            span,
            "expected a number",
        ));
    }
    let text = span.slice(cursor.raw);
    let Ok(value) = text.parse::<f64>() else {
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            span,
            format!("`{text}` is not a valid number"),
        ));
    };
    if !value.is_finite() {
        return Err(ParseIssue::new(
            DiagnosticKind::ParseError,
            span,
            format!("`{text}` is too large for the Number type"),
        ));
    }
    Ok(LitWithSpan {
        lit: Lit::Number {
            value,
            integer: !text.contains(['.', 'e', 'E']),
        },
        span,
    })
}

fn parse_string(cursor: &mut Cursor<'_>) -> Result<LitWithSpan, ParseIssue> {
    let start = cursor.pos;
    assert!(cursor.eat(b'"'), "caller checked");
    // Scan to the closing quote, validating escapes byte-wise (UTF-8
    // content bytes are all >= 0x80 and pass through untouched).
    loop {
        match cursor.bump() {
            Some(b'"') => break,
            Some(b'\\') => match cursor.bump() {
                Some(b'"' | b'\\') => {}
                _ => {
                    return Err(ParseIssue::new(
                        DiagnosticKind::ParseError,
                        cursor.here(),
                        r#"unknown escape — only `\"` and `\\` exist"#,
                    ));
                }
            },
            Some(_) => {}
            None => {
                return Err(ParseIssue::new(
                    DiagnosticKind::ParseError,
                    LineSpan {
                        start,
                        end: cursor.pos,
                    },
                    "unterminated string",
                ));
            }
        }
    }
    let span = LineSpan {
        start,
        end: cursor.pos,
    };
    // Unescape from the source slice.
    let inner = &cursor.raw[start + 1..cursor.pos - 1];
    let mut contents = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => contents.push('"'),
                Some('\\') => contents.push('\\'),
                _ => unreachable!("validated above"),
            }
        } else {
            contents.push(c);
        }
    }
    Ok(LitWithSpan {
        lit: Lit::Text(contents),
        span,
    })
}

fn parse_list(cursor: &mut Cursor<'_>) -> Result<LitWithSpan, ParseIssue> {
    let start = cursor.pos;
    assert!(cursor.eat(b'['), "caller checked");
    let mut items = Vec::new();
    cursor.skip_ws();
    if !cursor.eat(b']') {
        loop {
            cursor.skip_ws();
            let item = match cursor.peek() {
                Some(b'"') => parse_string(cursor)?,
                Some(c) if c.is_ascii_digit() || c == b'-' || c == b'.' => parse_number(cursor)?,
                _ => {
                    if let Some(lit) = try_parse_bool(cursor) {
                        lit
                    } else if cursor.peek() == Some(b'[') {
                        return Err(ParseIssue::new(
                            DiagnosticKind::ParseError,
                            cursor.here(),
                            "nested lists arrive in a later spike stage — \
                             list elements are scalar literals for now",
                        ));
                    } else {
                        return Err(ParseIssue::new(
                            DiagnosticKind::ParseError,
                            cursor.here(),
                            "list elements are scalar literals (numbers, strings, booleans) \
                             in the spike subset",
                        ));
                    }
                }
            };
            items.push(item);
            cursor.skip_ws();
            if cursor.eat(b',') {
                continue;
            }
            if cursor.eat(b']') {
                break;
            }
            return Err(ParseIssue::new(
                DiagnosticKind::ParseError,
                cursor.here(),
                "expected `,` or `]` in the list",
            ));
        }
    }
    Ok(LitWithSpan {
        lit: Lit::List(items),
        span: LineSpan {
            start,
            end: cursor.pos,
        },
    })
}

fn try_parse_bool(cursor: &mut Cursor<'_>) -> Option<LitWithSpan> {
    let checkpoint = cursor.pos;
    let ident = cursor.try_ident()?;
    match ident.name.as_str() {
        "True" => Some(LitWithSpan {
            lit: Lit::Boolean(true),
            span: ident.span,
        }),
        "False" => Some(LitWithSpan {
            lit: Lit::Boolean(false),
            span: ident.span,
        }),
        _ => {
            cursor.pos = checkpoint;
            None
        }
    }
}

// ---------------------------------------------------------- expressions --

/// Precedence-climbing expression parser. `^`/`**` bind tightest and
/// right-associate; unary minus next; `*`/`/`; `+`/`-`.
fn parse_expr(cursor: &mut Cursor<'_>, min_prec: u8) -> Result<Expr, ParseIssue> {
    cursor.skip_ws();
    let mut lhs = parse_expr_atom(cursor)?;
    loop {
        cursor.skip_ws();
        let Some((op, prec, right_assoc)) = peek_operator(cursor) else {
            break;
        };
        if prec < min_prec {
            break;
        }
        consume_operator(cursor, op);
        let next_min = if right_assoc { prec } else { prec + 1 };
        cursor.descend(expr_span(&lhs))?;
        let rhs = parse_expr(cursor, next_min)?;
        cursor.ascend();
        let span = LineSpan {
            start: expr_span(&lhs).start,
            end: expr_span(&rhs).end,
        };
        lhs = Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };
    }
    Ok(lhs)
}

fn parse_expr_atom(cursor: &mut Cursor<'_>) -> Result<Expr, ParseIssue> {
    cursor.skip_ws();
    match cursor.peek() {
        Some(b'(') => {
            cursor.eat(b'(');
            cursor.descend(cursor.here())?;
            let inner = parse_expr(cursor, 0)?;
            cursor.ascend();
            cursor.skip_ws();
            if !cursor.eat(b')') {
                return Err(ParseIssue::new(
                    DiagnosticKind::ParseError,
                    cursor.here(),
                    "expected `)`",
                ));
            }
            Ok(inner)
        }
        Some(b'-') => {
            let start = cursor.pos;
            cursor.eat(b'-');
            cursor.descend(cursor.here())?;
            let operand = parse_expr(cursor, 30)?;
            cursor.ascend();
            let span = LineSpan {
                start,
                end: expr_span(&operand).end,
            };
            Ok(Expr::Neg {
                operand: Box::new(operand),
                span,
            })
        }
        Some(c) if c.is_ascii_digit() || c == b'.' => {
            let lit = parse_number(cursor)?;
            let Lit::Number { value, integer } = lit.lit else {
                unreachable!("parse_number returns numbers")
            };
            Ok(Expr::Number {
                value,
                integer,
                span: lit.span,
            })
        }
        _ => {
            let Some(ident) = cursor.try_ident() else {
                return Err(ParseIssue::new(
                    DiagnosticKind::ParseError,
                    cursor.here(),
                    "expected a number, name, or parenthesized expression",
                ));
            };
            if cursor.peek() == Some(b'(') {
                return Err(ParseIssue::new(
                    DiagnosticKind::NestedCall,
                    ident.span,
                    format!(
                        "`{}(…)` — node calls don't nest in expressions; name the intermediate",
                        ident.name
                    ),
                ));
            }
            if cursor.peek() == Some(b'.')
                && cursor
                    .peek_at(cursor.pos + 1)
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            {
                return Err(ParseIssue::new(
                    DiagnosticKind::ParseError,
                    ident.span,
                    format!(
                        "expressions take single-value bindings — unpack `{}`'s outputs \
                         first (`a, b = …`) or wire the port into a node input",
                        ident.name
                    ),
                ));
            }
            Ok(Expr::Var(ident))
        }
    }
}

fn expr_span(expr: &Expr) -> LineSpan {
    match expr {
        Expr::Number { span, .. } | Expr::Neg { span, .. } | Expr::Binary { span, .. } => *span,
        Expr::Var(ident) => ident.span,
    }
}

fn peek_operator(cursor: &Cursor<'_>) -> Option<(BinOp, u8, bool)> {
    match cursor.peek()? {
        b'+' => Some((BinOp::Add, 10, false)),
        b'-' => Some((BinOp::Sub, 10, false)),
        b'*' => {
            if cursor.peek_at(cursor.pos + 1) == Some(b'*') {
                Some((BinOp::Pow, 40, true))
            } else {
                Some((BinOp::Mul, 20, false))
            }
        }
        b'/' => Some((BinOp::Div, 20, false)),
        b'^' => Some((BinOp::Pow, 40, true)),
        _ => None,
    }
}

fn consume_operator(cursor: &mut Cursor<'_>, op: BinOp) {
    match op {
        BinOp::Pow if cursor.peek() == Some(b'*') => {
            cursor.pos += 2;
        }
        _ => {
            cursor.pos += 1;
        }
    }
}

// -------------------------------------------------------------- cursor --

struct Cursor<'a> {
    raw: &'a str,
    pos: usize,
    depth: u32,
}

impl<'a> Cursor<'a> {
    fn new(raw: &'a str) -> Self {
        Self {
            raw,
            pos: 0,
            depth: 0,
        }
    }

    /// Enter one nesting level; errors past [`MAX_NESTING`] so adversarial
    /// nesting reds the statement instead of overflowing the stack.
    fn descend(&mut self, span: LineSpan) -> Result<(), ParseIssue> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(too_deep(span));
        }
        Ok(())
    }

    fn ascend(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn peek(&self) -> Option<u8> {
        self.raw.as_bytes().get(self.pos).copied()
    }

    fn peek_at(&self, pos: usize) -> Option<u8> {
        self.raw.as_bytes().get(pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.raw.len()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    /// A one-character span at the cursor, clamped to char boundaries so
    /// diagnostic columns are always sliceable (doc-11 consumers slice).
    fn here(&self) -> LineSpan {
        let mut start = self.pos.min(self.raw.len());
        while start > 0 && !self.raw.is_char_boundary(start) {
            start -= 1;
        }
        let end = if start < self.raw.len() {
            start + self.raw[start..].chars().next().map_or(1, char::len_utf8)
        } else {
            start
        };
        LineSpan { start, end }
    }

    fn try_ident(&mut self) -> Option<Ident> {
        let start = self.pos;
        let first = self.peek()?;
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return None;
        }
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            self.pos += 1;
        }
        Some(Ident {
            name: self.raw[start..self.pos].to_owned(),
            span: LineSpan {
                start,
                end: self.pos,
            },
        })
    }

    fn expect_ident(&mut self, what: &str) -> Result<Ident, ParseIssue> {
        self.try_ident().ok_or_else(|| {
            ParseIssue::new(
                DiagnosticKind::ParseError,
                self.here(),
                format!("expected {what}"),
            )
        })
    }
}
