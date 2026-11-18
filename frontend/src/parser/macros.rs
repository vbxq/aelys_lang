use super::Parser;
use super::block::method_call;
use super::expr::is_pure;
use aelys_common::Result;
use aelys_syntax::{
    BinaryOp, Expr, ExprKind, FMT_SPEC_MARKER, FmtStringPart, Span, Stmt, StmtKind, TokenKind,
    UnaryOp,
};

const MACROS: &str = "println!, print!, eprintln!, eprint!, format!, write!, writeln!, vec!, assert!, \
     assert_eq!, assert_ne!, panic!, todo!, unreachable!";

enum ArgRef {
    Next,
    Index(usize),
    Name(String),
}

enum Piece {
    Text(String),
    Arg(ArgRef, Option<String>),
}

struct FmtArgs {
    template: String,
    span: Span,
    positional: Vec<Expr>,
    named: Vec<(String, Expr)>,
}

fn expr(kind: ExprKind, span: Span) -> Expr {
    Expr::new(kind, span)
}

fn call(name: &str, args: Vec<Expr>, span: Span) -> Expr {
    expr(
        ExprKind::Call {
            callee: Box::new(expr(ExprKind::Identifier(name.to_string()), span)),
            args,
        },
        span,
    )
}

impl Parser {
    pub(super) fn macro_call(&mut self, name: String, start: Span) -> Result<Expr> {
        self.advance();
        let close = match self.advance().kind {
            TokenKind::LParen => TokenKind::RParen,
            TokenKind::LBracket => TokenKind::RBracket,
            _ => TokenKind::RBrace,
        };
        match name.as_str() {
            "vec" => self.vec_macro(close, start),
            "println" | "print" => {
                let args = self.fmt_args(close, name == "println", start)?;
                let span = start.merge(self.previous().span);
                self.print_call(&name, args, span)
            }
            "format" => {
                let args = self.fmt_args(close, false, start)?;
                let span = start.merge(self.previous().span);
                self.format(args, span)
            }
            "eprintln" | "eprint" | "write" | "writeln" => {
                self.call_args(close)?;
                self.fail(
                    396,
                    &format!("`{name}!` is not available yet; print to stdout with `println!`"),
                    start.merge(self.previous().span),
                )
            }
            "panic" | "todo" | "unreachable" => {
                let args = self.fmt_args(close, true, start)?;
                let span = start.merge(self.previous().span);
                let prefix = match name.as_str() {
                    "todo" => "not yet implemented",
                    "unreachable" => "internal error: entered unreachable code",
                    _ => "explicit panic",
                };
                let message = self.static_message(args, prefix, name != "panic", span)?;
                Ok(self.panic_expr(message, span))
            }
            "assert" => self.assert_macro(close, start),
            "assert_eq" | "assert_ne" => self.assert_eq_macro(close, start, name == "assert_eq"),
            other => {
                self.skip_to(&close);
                self.fail_help(
                    392,
                    &format!("cannot find macro `{other}!`: Aelys has no user macros"),
                    &format!("the macros are {MACROS}"),
                    start.merge(self.previous().span),
                )
            }
        }
    }

    fn skip_to(&mut self, close: &TokenKind) {
        let mut depth = 0usize;
        while !self.is_at_end() {
            match self.peek().kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                ref k if k == close && depth == 0 => {
                    self.advance();
                    return;
                }
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.advance();
        }
    }

    fn vec_macro(&mut self, close: TokenKind, start: Span) -> Result<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let result = (|| {
            let mut elements = Vec::new();
            while !self.check(&close) {
                elements.push(self.expression()?);
                if self.check(&TokenKind::Semicolon) && elements.len() == 1 {
                    self.advance();
                    self.expression()?;
                    self.consume(&close, &close.to_string())?;
                    return self.fail(
                        396,
                        "`vec![x; n]` is not available yet; build it with `Vec::new()` and `push`",
                        start.merge(self.previous().span),
                    );
                }
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&close, &close.to_string())?;
            Ok(expr(
                ExprKind::VecLiteral {
                    element_type: None,
                    elements,
                },
                start.merge(self.previous().span),
            ))
        })();
        self.no_struct = saved;
        result
    }

    fn fmt_args(&mut self, close: TokenKind, optional: bool, start: Span) -> Result<Option<FmtArgs>> {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let result = self.fmt_args_inner(close, optional, start);
        self.no_struct = saved;
        result
    }

    fn fmt_args_inner(&mut self, close: TokenKind, optional: bool, start: Span) -> Result<Option<FmtArgs>> {
        if self.match_token(&close) {
            if optional {
                return Ok(None);
            }
            return self.fail(395, "this macro needs a format string literal", start.merge(self.previous().span));
        }
        let span = self.peek().span;
        let template = match self.peek().kind.clone() {
            TokenKind::String(s) => {
                self.advance();
                s
            }
            _ => {
                let value = self.expression()?;
                return self.fail_help(
                    395,
                    "format argument must be a string literal",
                    "write `\"{}\", value` to format a value",
                    value.span,
                );
            }
        };
        let mut positional = Vec::new();
        let mut named: Vec<(String, Expr)> = Vec::new();
        while self.match_token(&TokenKind::Comma) {
            if self.check(&close) {
                break;
            }
            if let TokenKind::Identifier(n) = self.peek().kind.clone()
                && self.peek_at(1).kind == TokenKind::Eq
            {
                let name_span = self.peek().span;
                self.advance();
                self.advance();
                let value = self.expression()?;
                if named.iter().any(|(m, _)| *m == n) {
                    return self.fail(394, &format!("duplicate argument named `{n}`"), name_span);
                }
                named.push((n, value));
                continue;
            }
            let value = self.expression()?;
            if !named.is_empty() {
                return self.fail(394, "positional arguments cannot follow named arguments", value.span);
            }
            positional.push(value);
        }
        self.consume(&close, &close.to_string())?;
        Ok(Some(FmtArgs {
            template,
            span,
            positional,
            named,
        }))
    }

    fn pieces(&self, args: &FmtArgs) -> Result<Vec<Piece>> {
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut chars = args.template.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    text.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    text.push('}');
                }
                '}' => {
                    return self.fail_help(
                        393,
                        "invalid format string: unmatched `}` found",
                        "write `}}` for a literal brace",
                        args.span,
                    );
                }
                '{' => {
                    let mut inner = String::new();
                    let mut closed = false;
                    for d in chars.by_ref() {
                        if d == '}' {
                            closed = true;
                            break;
                        }
                        inner.push(d);
                    }
                    if !closed {
                        return self.fail_help(
                            393,
                            "invalid format string: unmatched `{` in format string",
                            "write `{{` for a literal brace",
                            args.span,
                        );
                    }
                    if !text.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut text)));
                    }
                    let (arg, spec) = match inner.split_once(':') {
                        Some((a, s)) => (a, Some(s.to_string())),
                        None => (inner.as_str(), None),
                    };
                    let arg = if arg.is_empty() {
                        ArgRef::Next
                    } else if arg.chars().all(|c| c.is_ascii_digit()) {
                        ArgRef::Index(arg.parse().unwrap_or(usize::MAX))
                    } else if arg.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
                        && arg.chars().all(|c| c.is_alphanumeric() || c == '_')
                    {
                        ArgRef::Name(arg.to_string())
                    } else {
                        let what = if arg.contains('.') && !arg.contains('(') {
                            "field access isn't supported"
                        } else {
                            "expressions in format strings are not supported"
                        };
                        return self.fail_help(
                            393,
                            &format!("invalid format string: {what}"),
                            &format!("write `{{}}` and pass `{arg}` as an argument"),
                            args.span,
                        );
                    };
                    pieces.push(Piece::Arg(arg, spec));
                }
                other => text.push(other),
            }
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        Ok(pieces)
    }

    // explicit arguments are evaluated once, left to right, before any of them is rendered
    fn render(&mut self, args: FmtArgs, span: Span) -> Result<(Vec<Stmt>, Vec<FmtStringPart>)> {
        let pieces = self.pieces(&args)?;
        let FmtArgs {
            positional, named, ..
        } = args;
        let mut stmts = Vec::new();
        let all_pure = positional.iter().chain(named.iter().map(|(_, e)| e)).all(is_pure);
        let mut bind = |this: &mut Self, e: Expr| -> Expr {
            if all_pure {
                return e;
            }
            let name = this.temp("fmt");
            let at = e.span;
            stmts.push(Stmt::new(
                StmtKind::Let {
                    name: name.clone(),
                    mutable: false,
                    type_annotation: None,
                    initializer: e,
                    is_pub: false,
                },
                at,
            ));
            expr(ExprKind::Identifier(name), at)
        };
        let positional: Vec<Expr> = positional.into_iter().map(|e| bind(self, e)).collect();
        let named: Vec<(String, Expr)> = named.into_iter().map(|(n, e)| (n, bind(self, e))).collect();
        let mut used_pos = vec![false; positional.len()];
        let mut used_named = vec![false; named.len()];
        let mut next = 0;
        let mut parts = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Text(t) => parts.push(FmtStringPart::Literal(t)),
                Piece::Arg(arg, spec) => {
                    let value = match arg {
                        ArgRef::Next | ArgRef::Index(_) => {
                            let i = match arg {
                                ArgRef::Index(i) => i,
                                _ => {
                                    next += 1;
                                    next - 1
                                }
                            };
                            let Some(v) = positional.get(i) else {
                                let n = positional.len();
                                let plural = if n == 1 { "argument" } else { "arguments" };
                                return self.fail(
                                    394,
                                    &format!("invalid reference to positional argument {i} (there is {n} {plural})"),
                                    span,
                                );
                            };
                            used_pos[i] = true;
                            v.clone()
                        }
                        ArgRef::Name(n) => match named.iter().position(|(m, _)| *m == n) {
                            Some(i) => {
                                used_named[i] = true;
                                named[i].1.clone()
                            }
                            None => expr(ExprKind::Identifier(n), span),
                        },
                    };
                    let value = match spec {
                        Some(spec) => expr(
                            ExprKind::Call {
                                callee: Box::new(expr(ExprKind::Identifier(FMT_SPEC_MARKER.to_string()), span)),
                                args: vec![value, expr(ExprKind::String(spec), span)],
                            },
                            span,
                        ),
                        None => value,
                    };
                    parts.push(FmtStringPart::Expr(Box::new(value)));
                }
            }
        }
        if let Some(i) = used_pos.iter().position(|u| !u) {
            return self.fail(394, &format!("argument never used: positional argument {i}"), positional[i].span);
        }
        if let Some(i) = used_named.iter().position(|u| !u) {
            return self.fail(394, &format!("named argument never used: `{}`", named[i].0), named[i].1.span);
        }
        Ok((stmts, parts))
    }

    fn wrap(stmts: Vec<Stmt>, value: Expr, span: Span) -> Expr {
        if stmts.is_empty() {
            return value;
        }
        expr(
            ExprKind::Block {
                stmts,
                tail: Box::new(value),
            },
            span,
        )
    }

    fn format_value(parts: Vec<FmtStringPart>, span: Span) -> Expr {
        match parts.as_slice() {
            [] => expr(ExprKind::String(String::new()), span),
            [FmtStringPart::Literal(s)] => expr(ExprKind::String(s.clone()), span),
            _ => expr(ExprKind::FmtString(parts), span),
        }
    }

    fn format(&mut self, args: Option<FmtArgs>, span: Span) -> Result<Expr> {
        let Some(args) = args else {
            return Ok(expr(ExprKind::String(String::new()), span));
        };
        let (stmts, parts) = self.render(args, span)?;
        Ok(Self::wrap(stmts, Self::format_value(parts, span), span))
    }

    fn print_call(&mut self, name: &str, args: Option<FmtArgs>, span: Span) -> Result<Expr> {
        let Some(args) = args else {
            return Ok(call(name, vec![expr(ExprKind::String(String::new()), span)], span));
        };
        let (stmts, mut parts) = self.render(args, span)?;
        let value = match parts.as_slice() {
            [FmtStringPart::Expr(e)] if !is_spec(e) => {
                let Some(FmtStringPart::Expr(e)) = parts.pop() else { unreachable!() };
                *e
            }
            _ => Self::format_value(parts, span),
        };
        Ok(Self::wrap(stmts, call(name, vec![value], span), span))
    }

    fn static_message(&mut self, args: Option<FmtArgs>, prefix: &str, joined: bool, span: Span) -> Result<String> {
        let Some(args) = args else {
            return Ok(prefix.to_string());
        };
        let (stmts, parts) = self.render(args, span)?;
        let text = match parts.as_slice() {
            [] => String::new(),
            [FmtStringPart::Literal(s)] if stmts.is_empty() => s.clone(),
            _ => {
                return self.fail(
                    396,
                    "a formatted panic message is not supported yet; write a literal message",
                    span,
                );
            }
        };
        Ok(if joined { format!("{prefix}: {text}") } else { text })
    }

    pub(super) fn panic_expr(&mut self, message: String, span: Span) -> Expr {
        let err = expr(
            ExprKind::EnumVariant {
                enum_name: "Result".to_string(),
                variant: "Err".to_string(),
                args: vec![expr(ExprKind::Int(0), span)],
                unqualified: true,
            },
            span,
        );
        let value = method_call(err, "expect", vec![expr(ExprKind::String(message), span)], span);
        self.panics.insert((span.start, span.end));
        value
    }

    fn assert_macro(&mut self, close: TokenKind, start: Span) -> Result<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let cond = self.expression();
        self.no_struct = saved;
        let cond = cond?;
        let source = self.source.content[cond.span.start..cond.span.end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let message = if self.match_token(&TokenKind::Comma) && !self.check(&close) {
            let args = self.fmt_args(close, true, start)?;
            let span = start.merge(self.previous().span);
            self.static_message(args, "", false, span)?
        } else {
            self.consume(&close, &close.to_string())?;
            format!("assertion failed: {source}")
        };
        let span = start.merge(self.previous().span);
        Ok(self.assert_failing_when(negate(cond), message, span))
    }

    fn assert_eq_macro(&mut self, close: TokenKind, start: Span, equal: bool) -> Result<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let operands = (|| {
            let left = self.expression()?;
            self.consume(&TokenKind::Comma, "`,`")?;
            let right = self.expression()?;
            Ok::<_, aelys_common::error::AelysError>((left, right))
        })();
        self.no_struct = saved;
        let (left, right) = operands?;
        let op = if equal { "==" } else { "!=" };
        let mut message = format!("assertion `left {op} right` failed");
        if self.match_token(&TokenKind::Comma) && !self.check(&close) {
            let args = self.fmt_args(close, true, start)?;
            let span = start.merge(self.previous().span);
            let extra = self.static_message(args, "", false, span)?;
            message = format!("{message}: {extra}");
        } else {
            self.consume(&close, &close.to_string())?;
        }
        let span = start.merge(self.previous().span);
        let failing = expr(
            ExprKind::Binary {
                left: Box::new(left),
                op: if equal { BinaryOp::Ne } else { BinaryOp::Eq },
                right: Box::new(right),
            },
            span,
        );
        Ok(self.assert_failing_when(failing, message, span))
    }

    fn assert_failing_when(&mut self, failing: Expr, message: String, span: Span) -> Expr {
        let panic = self.panic_expr(message, span);
        let panic = self.semi_stmt(panic);
        self.panics.remove(&(span.start, span.end));
        let check = Stmt::new(
            StmtKind::If {
                condition: failing,
                then_branch: Box::new(Stmt::new(StmtKind::Block(vec![panic]), span)),
                else_branch: None,
            },
            span,
        );
        expr(
            ExprKind::Block {
                stmts: vec![check],
                tail: Box::new(expr(ExprKind::Null, span)),
            },
            span,
        )
    }
}

fn is_spec(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Call { callee, .. } if matches!(&callee.kind, ExprKind::Identifier(n) if n == FMT_SPEC_MARKER))
}

fn negate(cond: Expr) -> Expr {
    let span = cond.span;
    expr(
        ExprKind::Unary {
            op: UnaryOp::Not,
            operand: Box::new(expr(ExprKind::Grouping(Box::new(cond)), span)),
        },
        span,
    )
}
