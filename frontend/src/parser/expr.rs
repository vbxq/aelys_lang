use super::Parser;
use super::types::{canonical_type_name, is_primitive};
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::{
    BinaryOp, Expr, ExprKind, Parameter, Span, Stmt, StmtKind, StructFieldInit, TokenKind,
    TypeAnnotation, UnaryOp,
};
use std::sync::Arc;

const ASSIGN: u8 = 1;
const RANGE: u8 = 2;
const OR: u8 = 3;
const AND: u8 = 4;
const COMPARE: u8 = 5;
const CAST: u8 = 12;

pub(super) fn starts_expression(kind: &TokenKind, no_struct: bool) -> bool {
    match kind {
        TokenKind::LBrace => !no_struct,
        TokenKind::Int(_)
        | TokenKind::UInt(_)
        | TokenKind::Float(_)
        | TokenKind::SuffixedInt(..)
        | TokenKind::SuffixedFloat(..)
        | TokenKind::String(_)
        | TokenKind::Char(_)
        | TokenKind::Byte(_)
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Identifier(_)
        | TokenKind::Lifetime(_)
        | TokenKind::LParen
        | TokenKind::LBracket
        | TokenKind::Minus
        | TokenKind::Bang
        | TokenKind::Star
        | TokenKind::Ampersand
        | TokenKind::AndAnd
        | TokenKind::Pipe
        | TokenKind::OrOr
        | TokenKind::Move
        | TokenKind::If
        | TokenKind::Match
        | TokenKind::Loop
        | TokenKind::While
        | TokenKind::For
        | TokenKind::Unsafe
        | TokenKind::Return
        | TokenKind::Break
        | TokenKind::Continue
        | TokenKind::DotDot
        | TokenKind::DotDotEq
        | TokenKind::Crate => true,
        _ => false,
    }
}

enum Infix {
    Assign(Option<BinaryOp>),
    Range(bool),
    Or,
    And,
    Binary(BinaryOp, u8),
    Cast,
}

fn infix(kind: &TokenKind) -> Option<(Infix, u8)> {
    use BinaryOp::*;
    Some(match kind {
        TokenKind::Eq => (Infix::Assign(None), ASSIGN),
        TokenKind::PlusEq => (Infix::Assign(Some(Add)), ASSIGN),
        TokenKind::MinusEq => (Infix::Assign(Some(Sub)), ASSIGN),
        TokenKind::StarEq => (Infix::Assign(Some(Mul)), ASSIGN),
        TokenKind::SlashEq => (Infix::Assign(Some(Div)), ASSIGN),
        TokenKind::PercentEq => (Infix::Assign(Some(Mod)), ASSIGN),
        TokenKind::AmpEq => (Infix::Assign(Some(BitAnd)), ASSIGN),
        TokenKind::PipeEq => (Infix::Assign(Some(BitOr)), ASSIGN),
        TokenKind::CaretEq => (Infix::Assign(Some(BitXor)), ASSIGN),
        TokenKind::ShlEq => (Infix::Assign(Some(Shl)), ASSIGN),
        TokenKind::ShrEq => (Infix::Assign(Some(Shr)), ASSIGN),
        TokenKind::DotDot => (Infix::Range(false), RANGE),
        TokenKind::DotDotEq => (Infix::Range(true), RANGE),
        TokenKind::OrOr => (Infix::Or, OR),
        TokenKind::AndAnd => (Infix::And, AND),
        TokenKind::EqEq => (Infix::Binary(Eq, COMPARE), COMPARE),
        TokenKind::BangEq => (Infix::Binary(Ne, COMPARE), COMPARE),
        TokenKind::Lt => (Infix::Binary(Lt, COMPARE), COMPARE),
        TokenKind::LtEq => (Infix::Binary(Le, COMPARE), COMPARE),
        TokenKind::Gt => (Infix::Binary(Gt, COMPARE), COMPARE),
        TokenKind::GtEq => (Infix::Binary(Ge, COMPARE), COMPARE),
        TokenKind::Pipe => (Infix::Binary(BitOr, 6), 6),
        TokenKind::Caret => (Infix::Binary(BitXor, 7), 7),
        TokenKind::Ampersand => (Infix::Binary(BitAnd, 8), 8),
        TokenKind::Shl => (Infix::Binary(Shl, 9), 9),
        TokenKind::Shr => (Infix::Binary(Shr, 9), 9),
        TokenKind::Plus => (Infix::Binary(Add, 10), 10),
        TokenKind::Minus => (Infix::Binary(Sub, 10), 10),
        TokenKind::Star => (Infix::Binary(Mul, 11), 11),
        TokenKind::Slash => (Infix::Binary(Div, 11), 11),
        TokenKind::Percent => (Infix::Binary(Mod, 11), 11),
        TokenKind::As => (Infix::Cast, CAST),
        _ => return None,
    })
}

fn is_comparison(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::EqEq | TokenKind::BangEq | TokenKind::Lt | TokenKind::LtEq | TokenKind::Gt | TokenKind::GtEq
    )
}

fn expr(kind: ExprKind, span: Span) -> Expr {
    Expr::new(kind, span)
}

pub(super) fn is_pure(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Identifier(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::String(_)
        | ExprKind::Char(_)
        | ExprKind::Bool(_)
        | ExprKind::Null => true,
        ExprKind::Member { object, .. } => is_pure(object),
        ExprKind::Grouping(inner) | ExprKind::Deref(inner) => is_pure(inner),
        ExprKind::Unary { operand, .. } | ExprKind::Reference { operand, .. } => is_pure(operand),
        ExprKind::Cast { expr, .. } => is_pure(expr),
        ExprKind::Binary { left, right, .. } => is_pure(left) && is_pure(right),
        ExprKind::Index { object, index } => is_pure(object) && is_pure(index),
        ExprKind::EnumVariant { args, .. } => args.iter().all(is_pure),
        _ => false,
    }
}

impl Parser {
    pub(super) fn expression(&mut self) -> Result<Expr> {
        self.enter_recursion()?;
        let result = self.expr_bp(0);
        self.exit_recursion();
        result
    }

    pub(super) fn no_struct_expr(&mut self) -> Result<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, true);
        let result = self.expression();
        self.no_struct = saved;
        result
    }

    fn expr_bp(&mut self, min: u8) -> Result<Expr> {
        let start = self.peek().span;
        let lhs = match self.peek().kind {
            TokenKind::Pipe | TokenKind::OrOr | TokenKind::Move => return self.closure(),
            TokenKind::Return | TokenKind::Break | TokenKind::Continue => {
                let stmts = self.jump_statement()?;
                let span = start.merge(self.previous().span);
                return Ok(expr(
                    ExprKind::Block {
                        stmts,
                        tail: Box::new(expr(ExprKind::Null, span)),
                    },
                    span,
                ));
            }
            TokenKind::DotDot | TokenKind::DotDotEq => {
                let inclusive = self.advance().kind == TokenKind::DotDotEq;
                let end = if starts_expression(&self.peek().kind, self.no_struct) {
                    Some(Box::new(self.expr_bp(RANGE + 1)?))
                } else {
                    None
                };
                expr(
                    ExprKind::Range {
                        start: None,
                        end,
                        inclusive,
                    },
                    start.merge(self.previous().span),
                )
            }
            _ => self.unary()?,
        };
        self.binary_rest(lhs, min)
    }

    pub(super) fn binary_rest(&mut self, mut lhs: Expr, min: u8) -> Result<Expr> {
        loop {
            if let TokenKind::Identifier(word) = self.peek().kind.clone() {
                let operand_follows = starts_expression(&self.peek_at(1).kind, false);
                match word.as_str() {
                    "and" | "or" if operand_follows => {
                        let new = if word == "and" { "&&" } else { "||" };
                        return self.fail(112, &format!("`{word}` is not an operator; use `{new}`"), self.peek().span);
                    }
                    "catch" => {
                        return self.fail_help(
                            119,
                            "`catch` is not an operator; handle the error with `match`: `match e { Ok(v) => v, Err(x) => .. }`",
                            "or propagate it with `?`",
                            self.peek().span,
                        );
                    }
                    _ => {}
                }
            }
            let Some((op, prec)) = infix(&self.peek().kind) else {
                break;
            };
            if prec < min {
                break;
            }
            let op_span = self.peek().span;
            match op {
                Infix::Assign(compound) => {
                    self.advance();
                    let rhs = self.expr_bp(ASSIGN)?;
                    lhs = self.make_assign(lhs, compound, rhs)?;
                }
                Infix::Range(inclusive) => {
                    self.advance();
                    let end = if starts_expression(&self.peek().kind, self.no_struct) {
                        Some(Box::new(self.expr_bp(RANGE + 1)?))
                    } else {
                        None
                    };
                    let span = lhs.span.merge(self.previous().span);
                    lhs = expr(
                        ExprKind::Range {
                            start: Some(Box::new(lhs)),
                            end,
                            inclusive,
                        },
                        span,
                    );
                    if matches!(self.peek().kind, TokenKind::DotDot | TokenKind::DotDotEq) {
                        return self.fail(101, "ranges cannot be chained; use parentheses", self.peek().span);
                    }
                }
                Infix::Or | Infix::And => {
                    self.advance();
                    let rhs = self.expr_bp(prec + 1)?;
                    let span = lhs.span.merge(rhs.span);
                    let (left, right) = (Box::new(lhs), Box::new(rhs));
                    lhs = expr(
                        if matches!(op, Infix::Or) {
                            ExprKind::Or { left, right }
                        } else {
                            ExprKind::And { left, right }
                        },
                        span,
                    );
                }
                Infix::Binary(bin, level) => {
                    if matches!(bin, BinaryOp::Add | BinaryOp::Sub) && self.doubled_operator() {
                        let (op_text, fix) = if bin == BinaryOp::Add { ("++", "+= 1") } else { ("--", "-= 1") };
                        return self.fail_help(
                            113,
                            &format!("there is no `{op_text}` operator; write `x {fix}`"),
                            &format!("`x {fix}` changes `x` in place"),
                            op_span.merge(self.peek_at(1).span),
                        );
                    }
                    self.advance();
                    let rhs = self.expr_bp(level + 1)?;
                    let span = lhs.span.merge(rhs.span);
                    lhs = expr(
                        ExprKind::Binary {
                            left: Box::new(lhs),
                            op: bin,
                            right: Box::new(rhs),
                        },
                        span,
                    );
                    if level == COMPARE && is_comparison(&self.peek().kind) {
                        return self.fail_help(
                            131,
                            "comparison operators cannot be chained",
                            "use parentheses or `&&`: `0 < a && a < 3`",
                            span.merge(self.peek().span),
                        );
                    }
                }
                Infix::Cast => {
                    self.advance();
                    let target = self.parse_cast_target()?;
                    let span = lhs.span.merge(self.previous().span);
                    lhs = expr(
                        ExprKind::Cast {
                            expr: Box::new(lhs),
                            target,
                        },
                        span,
                    );
                }
            }
        }
        Ok(lhs)
    }

    fn doubled_operator(&self) -> bool {
        self.peek_at(1).kind == self.peek().kind
            && self.adjacent(0, 1)
            && !starts_expression(&self.peek_at(2).kind, false)
    }

    fn make_assign(&mut self, target: Expr, compound: Option<BinaryOp>, value: Expr) -> Result<Expr> {
        let target = Self::unwrap_grouping(target);
        let Some(op) = compound else {
            return self.plain_assign(target, value);
        };
        let mut lets = Vec::new();
        let place = self.stabilize(target, &mut lets);
        let span = place.span.merge(value.span);
        let combined = expr(
            ExprKind::Binary {
                left: Box::new(place.clone()),
                op,
                right: Box::new(value),
            },
            span,
        );
        let assignment = self.plain_assign(place, combined)?;
        if lets.is_empty() {
            return Ok(assignment);
        }
        lets.push(Stmt::new(StmtKind::Semi(assignment), span));
        Ok(expr(
            ExprKind::Block {
                stmts: lets,
                tail: Box::new(expr(ExprKind::Null, span)),
            },
            span,
        ))
    }

    fn plain_assign(&self, target: Expr, value: Expr) -> Result<Expr> {
        let span = target.span.merge(value.span);
        let value = Box::new(value);
        let kind = match target.kind {
            ExprKind::Identifier(name) => ExprKind::Assign { name, value },
            ExprKind::Index { object, index } => ExprKind::IndexAssign { object, index, value },
            ExprKind::Member { object, member } => ExprKind::FieldAssign {
                object,
                field: member,
                value,
            },
            ExprKind::Deref(target) => ExprKind::DerefAssign { target, value },
            _ => {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidAssignmentTarget,
                    target.span,
                    Arc::clone(&self.source),
                )
                .into());
            }
        };
        Ok(expr(kind, span))
    }

    // the place of a compound assignment is evaluated once: impure index and receiver expressions go to temporaries
    fn stabilize(&mut self, e: Expr, lets: &mut Vec<Stmt>) -> Expr {
        let span = e.span;
        match e.kind {
            ExprKind::Grouping(inner) => self.stabilize(*inner, lets),
            ExprKind::Index { object, index } => {
                let object = self.stabilize_object(*object, lets);
                let index = if is_pure(&index) { *index } else { self.bind(*index, lets) };
                expr(
                    ExprKind::Index {
                        object: Box::new(object),
                        index: Box::new(index),
                    },
                    span,
                )
            }
            ExprKind::Member { object, member } => {
                let object = self.stabilize_object(*object, lets);
                expr(
                    ExprKind::Member {
                        object: Box::new(object),
                        member,
                    },
                    span,
                )
            }
            ExprKind::Deref(inner) if !is_pure(&inner) => {
                let bound = self.bind(*inner, lets);
                expr(ExprKind::Deref(Box::new(bound)), span)
            }
            kind => expr(kind, span),
        }
    }

    fn stabilize_object(&mut self, e: Expr, lets: &mut Vec<Stmt>) -> Expr {
        match e.kind {
            ExprKind::Identifier(_)
            | ExprKind::Index { .. }
            | ExprKind::Member { .. }
            | ExprKind::Deref(_)
            | ExprKind::Grouping(_) => self.stabilize(e, lets),
            _ if is_pure(&e) => e,
            _ => self.bind(e, lets),
        }
    }

    fn bind(&mut self, e: Expr, lets: &mut Vec<Stmt>) -> Expr {
        let span = e.span;
        let name = self.temp("place");
        lets.push(Stmt::new(
            StmtKind::Let {
                name: name.clone(),
                mutable: false,
                type_annotation: None,
                initializer: e,
                is_pub: false,
            },
            span,
        ));
        expr(ExprKind::Identifier(name), span)
    }

    pub(super) fn unwrap_grouping(e: Expr) -> Expr {
        let span = e.span;
        let mut cur = e;
        while let ExprKind::Grouping(inner) = cur.kind {
            cur = *inner;
        }
        Expr::new(cur.kind, span)
    }

    fn unary(&mut self) -> Result<Expr> {
        self.enter_recursion()?;
        let result = self.unary_inner();
        self.exit_recursion();
        result
    }

    fn unary_inner(&mut self) -> Result<Expr> {
        let start = self.peek().span;
        match self.peek().kind.clone() {
            TokenKind::Minus => {
                self.advance();
                if let TokenKind::SuffixedInt(value, suffix) = self.peek().kind.clone()
                    && suffix.starts_with('i')
                    && !matches!(
                        self.peek_at(1).kind,
                        TokenKind::Dot | TokenKind::LParen | TokenKind::LBracket | TokenKind::Question
                    )
                {
                    let literal = self.advance().span;
                    return self.suffixed_int(value, &suffix, literal, true).map(|mut e| {
                        e.span = start.merge(literal);
                        e
                    });
                }
                let operand = self.unary()?;
                let span = start.merge(operand.span);
                Ok(match operand.kind {
                    ExprKind::Int(n) => expr(ExprKind::Int(n.wrapping_neg()), span),
                    ExprKind::Float(f) => expr(ExprKind::Float(-f), span),
                    kind => expr(
                        ExprKind::Unary {
                            op: UnaryOp::Neg,
                            operand: Box::new(expr(kind, operand.span)),
                        },
                        span,
                    ),
                })
            }
            TokenKind::Bang => {
                self.advance();
                let operand = self.unary()?;
                let span = start.merge(operand.span);
                Ok(expr(
                    ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                ))
            }
            TokenKind::Tilde => self.fail(112, "`~` is not an operator; use `!`, which is bitwise on integers", start),
            TokenKind::Star => {
                self.advance();
                let operand = self.unary()?;
                let span = start.merge(operand.span);
                Ok(expr(ExprKind::Deref(Box::new(operand)), span))
            }
            TokenKind::Ampersand | TokenKind::AndAnd => {
                let double = self.advance().kind == TokenKind::AndAnd;
                let mutable = self.match_token(&TokenKind::Mut);
                let operand = self.unary()?;
                let span = start.merge(operand.span);
                if matches!(operand.kind, ExprKind::Slice { .. }) && !double {
                    return Ok(expr(operand.kind, span));
                }
                let inner = expr(
                    ExprKind::Reference {
                        mutable,
                        operand: Box::new(operand),
                    },
                    span,
                );
                if !double {
                    return Ok(inner);
                }
                Ok(expr(
                    ExprKind::Reference {
                        mutable: false,
                        operand: Box::new(inner),
                    },
                    span,
                ))
            }
            TokenKind::Identifier(w)
                if w == "not"
                    && self.peek_at(1).kind != TokenKind::LParen
                    && starts_expression(&self.peek_at(1).kind, false)
                    && !matches!(self.peek_at(1).kind, TokenKind::Minus | TokenKind::Star | TokenKind::Ampersand | TokenKind::Pipe) =>
            {
                self.fail(112, "`not` is not an operator; use `!`", start)
            }
            _ => {
                let primary = self.primary()?;
                self.postfix_rest(primary)
            }
        }
    }

    pub(super) fn postfix_rest(&mut self, mut e: Expr) -> Result<Expr> {
        loop {
            match self.peek().kind {
                TokenKind::LParen => {
                    self.advance();
                    let args = self.call_args(TokenKind::RParen)?;
                    let span = e.span.merge(self.previous().span);
                    if let ExprKind::EnumVariant {
                        unqualified: true,
                        args: slot,
                        ..
                    } = &mut e.kind
                        && slot.is_empty()
                    {
                        *slot = args;
                        e.span = span;
                        continue;
                    }
                    e = expr(
                        ExprKind::Call {
                            callee: Box::new(e),
                            args,
                        },
                        span,
                    );
                }
                TokenKind::Dot => {
                    self.advance();
                    let member = match self.peek().kind {
                        TokenKind::Int(n) => {
                            self.advance();
                            n.to_string()
                        }
                        _ => self.consume_member_name("a field or method name")?,
                    };
                    let span = e.span.merge(self.previous().span);
                    if self.check(&TokenKind::ColonColon) && self.peek_at(1).kind == TokenKind::Lt {
                        return self.fail_help(
                            129,
                            "explicit type arguments `::<>` are not supported",
                            "annotate the binding that receives the value instead",
                            self.peek().span,
                        );
                    }
                    if let ExprKind::Identifier(namespace) = &e.kind
                        && self.check(&TokenKind::ColonColon)
                    {
                        let mut path = format!("{namespace}::{member}");
                        let mut i = 0;
                        while self.peek_at(i).kind == TokenKind::ColonColon {
                            if let TokenKind::Identifier(s) = &self.peek_at(i + 1).kind {
                                path.push_str("::");
                                path.push_str(s);
                            }
                            i += 2;
                        }
                        return self.fail(115, &format!("a module path is written with `::`: write `{path}`"), span);
                    }
                    e = expr(
                        ExprKind::Member {
                            object: Box::new(e),
                            member,
                        },
                        span,
                    );
                }
                TokenKind::LBracket => {
                    self.advance();
                    let saved = std::mem::replace(&mut self.no_struct, false);
                    let index = self.expression();
                    self.no_struct = saved;
                    let index = index?;
                    self.consume(&TokenKind::RBracket, "`]`")?;
                    let span = e.span.merge(self.previous().span);
                    e = if matches!(index.kind, ExprKind::Range { .. }) {
                        expr(
                            ExprKind::Slice {
                                object: Box::new(e),
                                range: Box::new(index),
                            },
                            span,
                        )
                    } else {
                        expr(
                            ExprKind::Index {
                                object: Box::new(e),
                                index: Box::new(index),
                            },
                            span,
                        )
                    };
                }
                TokenKind::Question => {
                    self.advance();
                    let span = e.span.merge(self.previous().span);
                    e = expr(ExprKind::Try(Box::new(e)), span);
                }
                _ => return Ok(e),
            }
        }
    }

    pub(super) fn call_args(&mut self, close: TokenKind) -> Result<Vec<Expr>> {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let result = (|| {
            let mut args = Vec::new();
            while !self.check(&close) {
                args.push(self.expression()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&close, &close.to_string())?;
            Ok(args)
        })();
        self.no_struct = saved;
        result
    }

    fn primary(&mut self) -> Result<Expr> {
        let span = self.peek().span;
        let kind = self.peek().kind.clone();
        let kind = match kind {
            TokenKind::Int(n) => {
                self.advance();
                ExprKind::Int(n)
            }
            TokenKind::UInt(n) => {
                self.advance();
                ExprKind::Cast {
                    expr: Box::new(expr(ExprKind::Int(n as i64), span)),
                    target: TypeAnnotation::new("u64".to_string(), span),
                }
            }
            TokenKind::Float(f) => {
                self.advance();
                ExprKind::Float(f)
            }
            TokenKind::SuffixedInt(value, suffix) => {
                self.advance();
                return self.suffixed_int(value, &suffix, span, false);
            }
            TokenKind::SuffixedFloat(value, suffix) => {
                self.advance();
                return match suffix.as_str() {
                    "f64" => Ok(expr(ExprKind::Float(value), span)),
                    "f32" => Ok(expr(
                        ExprKind::Cast {
                            expr: Box::new(expr(ExprKind::Float(value), span)),
                            target: TypeAnnotation::new("f32".to_string(), span),
                        },
                        span,
                    )),
                    other => self.fail(128, &format!("`{other}` is not a float suffix; write `f32` or `f64`"), span),
                };
            }
            TokenKind::Byte(b) => {
                self.advance();
                ExprKind::Cast {
                    expr: Box::new(expr(ExprKind::Int(b as i64), span)),
                    target: TypeAnnotation::new("u8".to_string(), span),
                }
            }
            TokenKind::String(s) => {
                self.advance();
                ExprKind::String(s)
            }
            TokenKind::Char(c) => {
                self.advance();
                ExprKind::Char(c)
            }
            TokenKind::True => {
                self.advance();
                ExprKind::Bool(true)
            }
            TokenKind::False => {
                self.advance();
                ExprKind::Bool(false)
            }
            TokenKind::Identifier(_) | TokenKind::Crate => return self.path_expression(),
            TokenKind::LBracket => {
                self.advance();
                return self.array_literal(span);
            }
            TokenKind::LParen => {
                self.advance();
                return self.paren_expression(span);
            }
            TokenKind::If
            | TokenKind::Match
            | TokenKind::Loop
            | TokenKind::While
            | TokenKind::For
            | TokenKind::LBrace
            | TokenKind::Unsafe
            | TokenKind::Lifetime(_) => {
                if let TokenKind::Lifetime(name) = &kind
                    && self.peek_at(1).kind != TokenKind::Colon
                {
                    return self.fail(123, &format!("Aelys has no lifetime annotations; remove `'{name}`"), span);
                }
                return self.block_like_value();
            }
            TokenKind::Fn => {
                return self.fail_help(
                    117,
                    "`fn(..)` does not start a closure; write `|x: i64| -> i64 { .. }`",
                    "a closure is written `|x| x + 1`",
                    span,
                );
            }
            TokenKind::At => {
                return self.fail(116, "`@` does not start an expression; attributes are written `#[name]`", span);
            }
            _ => {
                return Err(CompileError::new(CompileErrorKind::ExpectedExpression, span, Arc::clone(&self.source)).into());
            }
        };
        Ok(expr(kind, span))
    }

    fn suffixed_int(&mut self, value: u128, suffix: &str, span: Span, negated: bool) -> Result<Expr> {
        let (name, max): (&str, u128) = match suffix {
            "i8" => ("i8", 127),
            "i16" => ("i16", i16::MAX as u128),
            "i32" => ("i32", i32::MAX as u128),
            "i64" | "isize" => ("i64", i64::MAX as u128),
            "u8" => ("u8", u8::MAX as u128),
            "u16" => ("u16", u16::MAX as u128),
            "u32" => ("u32", u32::MAX as u128),
            "u64" | "usize" => ("u64", u64::MAX as u128),
            "f32" | "f64" => {
                let f = expr(ExprKind::Float(value as f64), span);
                if suffix == "f64" {
                    return Ok(f);
                }
                return Ok(expr(
                    ExprKind::Cast {
                        expr: Box::new(f),
                        target: TypeAnnotation::new("f32".to_string(), span),
                    },
                    span,
                ));
            }
            other => {
                return self.fail_help(
                    128,
                    &format!("`{other}` is not an Aelys integer type"),
                    "the integer suffixes are `i8` `i16` `i32` `i64` `u8` `u16` `u32` `u64` `usize` `isize`",
                    span,
                );
            }
        };
        let signed = name.starts_with('i');
        let limit = if signed && negated { max + 1 } else { max };
        if value > limit {
            let shown = &self.source.content[span.start..span.end];
            return self.fail(128, &format!("the literal `{shown}` is out of range for `{name}`"), span);
        }
        let value = if negated { (value as u64 as i64).wrapping_neg() } else { value as u64 as i64 };
        let int = expr(ExprKind::Int(value), span);
        if name == "i64" {
            return Ok(int);
        }
        Ok(expr(
            ExprKind::Cast {
                expr: Box::new(int),
                target: TypeAnnotation::new(name.to_string(), span),
            },
            span,
        ))
    }

    fn paren_expression(&mut self, span: Span) -> Result<Expr> {
        if self.match_token(&TokenKind::RParen) {
            return self.fail_help(
                102,
                "expected expression, found `()`: there is no unit value",
                "a function without a result leaves out its return type",
                span.merge(self.previous().span),
            );
        }
        let saved = std::mem::replace(&mut self.no_struct, false);
        let result = (|| {
            let inner = self.expression()?;
            if !self.match_token(&TokenKind::Comma) {
                self.consume(&TokenKind::RParen, "`)`")?;
                return Ok(expr(ExprKind::Grouping(Box::new(inner)), span.merge(self.previous().span)));
            }
            if self.check(&TokenKind::RParen) {
                return self.fail(902, "a one-element tuple is not supported; write the value itself", span.merge(self.peek().span));
            }
            let mut elems = vec![inner];
            loop {
                elems.push(self.expression()?);
                if !self.match_token(&TokenKind::Comma) || self.check(&TokenKind::RParen) {
                    break;
                }
            }
            self.consume(&TokenKind::RParen, "`)`")?;
            Ok(expr(ExprKind::Tuple(elems), span.merge(self.previous().span)))
        })();
        self.no_struct = saved;
        result
    }

    fn array_literal(&mut self, start: Span) -> Result<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let result = (|| {
            if self.match_token(&TokenKind::RBracket) {
                return Ok(expr(ExprKind::ArrayLiteral { elements: Vec::new() }, start.merge(self.previous().span)));
            }
            let first = self.expression()?;
            if self.match_token(&TokenKind::Semicolon) {
                let size = self.expression()?;
                self.consume(&TokenKind::RBracket, "`]`")?;
                return Ok(expr(
                    ExprKind::ArraySized {
                        size: Box::new(size),
                        fill_value: Some(Box::new(first)),
                    },
                    start.merge(self.previous().span),
                ));
            }
            let mut elements = vec![first];
            while self.match_token(&TokenKind::Comma) {
                if self.check(&TokenKind::RBracket) {
                    break;
                }
                elements.push(self.expression()?);
            }
            self.consume(&TokenKind::RBracket, "`]`")?;
            Ok(expr(ExprKind::ArrayLiteral { elements }, start.merge(self.previous().span)))
        })();
        self.no_struct = saved;
        result
    }

    fn path_expression(&mut self) -> Result<Expr> {
        let start = self.peek().span;
        if self.match_token(&TokenKind::Crate) {
            self.consume(&TokenKind::ColonColon, "`::` after `crate`")?;
        }
        let first = self.consume_identifier("an expression")?;
        if self.check(&TokenKind::Bang)
            && matches!(self.peek_at(1).kind, TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace)
        {
            return self.macro_call(first, start);
        }
        if (first == "vec" || first == "Vec") && self.check(&TokenKind::LBracket) {
            return self.fail_help(138, "`vec[..]` is not a literal; write `vec![..]`", "`vec![1, 2, 3]` builds a `Vec`", start);
        }
        if (first == "println" || first == "print") && self.check(&TokenKind::LParen) {
            return self.fail_help(
                392,
                &format!("`{first}` is a macro: write `{first}!(...)`"),
                &format!("`{first}!(\"{{}}\", x)` prints a value"),
                start,
            );
        }
        let mut segments = vec![first];
        while self.check(&TokenKind::ColonColon) {
            if self.peek_at(1).kind == TokenKind::Lt {
                return self.fail_help(
                    129,
                    "explicit type arguments `::<>` are not supported",
                    "annotate the binding: `let v: Vec<i64> = Vec::new();`",
                    self.peek().span,
                );
            }
            self.advance();
            segments.push(self.consume_member_name("a path segment")?);
            if self.check(&TokenKind::Bang) && matches!(self.peek_at(1).kind, TokenKind::LParen | TokenKind::LBracket) {
                return self.fail(392, "a macro is not a path; write the macro name alone, as `println!`", start);
            }
        }
        let span = start.merge(self.previous().span);
        self.lower_path(segments, span)
    }

    fn lower_path(&mut self, segments: Vec<String>, span: Span) -> Result<Expr> {
        let last = segments.last().unwrap().clone();
        if segments.len() == 1 {
            return self.single_name(last, span);
        }
        if segments.len() == 2 && segments[0] == "ExitCode" {
            match segments[1].as_str() {
                "SUCCESS" => return Ok(expr(ExprKind::Int(0), span)),
                "FAILURE" => return Ok(expr(ExprKind::Int(1), span)),
                "from" if self.check(&TokenKind::LParen) => {
                    self.advance();
                    let mut args = self.call_args(TokenKind::RParen)?;
                    if args.len() != 1 {
                        return self.fail(301, "`ExitCode::from` takes one `u8`", span);
                    }
                    let span = span.merge(self.previous().span);
                    return Ok(expr(
                        ExprKind::Cast {
                            expr: Box::new(args.pop().unwrap()),
                            target: TypeAnnotation::new("i64".to_string(), span),
                        },
                        span,
                    ));
                }
                _ => {}
            }
        }
        let modules = segments
            .iter()
            .take_while(|s| s.starts_with(|c: char| c.is_lowercase() || c == '_') && !is_primitive(s))
            .count();
        if modules == segments.len() {
            let mut e = expr(ExprKind::Identifier(segments[0].clone()), span);
            for s in &segments[1..] {
                e = expr(
                    ExprKind::Member {
                        object: Box::new(e),
                        member: s.clone(),
                    },
                    span,
                );
            }
            return Ok(e);
        }
        let rest = &segments[modules..];
        if rest.len() == 1 {
            let qualified = format!("{}.{}", segments[..modules].join("."), rest[0]);
            if self.check(&TokenKind::LBrace) && !self.no_struct && rest[0].starts_with(char::is_uppercase) {
                return self.struct_literal(qualified, span);
            }
            let mut e = expr(ExprKind::Identifier(segments[0].clone()), span);
            for s in &segments[1..] {
                e = expr(
                    ExprKind::Member {
                        object: Box::new(e),
                        member: s.clone(),
                    },
                    span,
                );
            }
            return Ok(e);
        }
        if rest.len() > 2 {
            return self.fail(101, &format!("`{}` names a path that is too deep", segments.join("::")), span);
        }
        let mut enum_name = segments[..modules].to_vec();
        enum_name.push(canonical_type_name(&rest[0]).to_string());
        let enum_name = enum_name.join(".");
        let variant = rest[1].clone();
        if self.check(&TokenKind::LParen) {
            self.advance();
            let args = self.call_args(TokenKind::RParen)?;
            return Ok(expr(
                ExprKind::EnumVariant {
                    enum_name,
                    variant,
                    args,
                    unqualified: false,
                },
                span.merge(self.previous().span),
            ));
        }
        Ok(expr(
            ExprKind::EnumVariant {
                enum_name,
                variant,
                args: Vec::new(),
                unqualified: false,
            },
            span,
        ))
    }

    fn single_name(&mut self, name: String, span: Span) -> Result<Expr> {
        if self.check(&TokenKind::LBrace) && !self.no_struct && name.starts_with(char::is_uppercase) {
            return self.struct_literal(name, span);
        }
        Ok(match aelys_syntax::prelude_variant_enum(&name) {
            Some(enum_name) => expr(
                ExprKind::EnumVariant {
                    enum_name: enum_name.to_string(),
                    variant: name,
                    args: Vec::new(),
                    unqualified: true,
                },
                span,
            ),
            None => expr(ExprKind::Identifier(name), span),
        })
    }

    pub(super) fn struct_literal(&mut self, name: String, start: Span) -> Result<Expr> {
        self.consume(&TokenKind::LBrace, "`{`")?;
        let saved = std::mem::replace(&mut self.no_struct, false);
        let result = self.struct_literal_fields(name, start);
        self.no_struct = saved;
        result
    }

    fn struct_literal_fields(&mut self, name: String, start: Span) -> Result<Expr> {
        let mut fields = Vec::new();
        let mut base = None;
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            if self.match_token(&TokenKind::DotDot) {
                base = Some(self.expression()?);
                break;
            }
            let field_span = self.peek().span;
            let field_name = match self.peek().kind {
                TokenKind::Int(n) => {
                    self.advance();
                    n.to_string()
                }
                _ => self.consume_member_name("a field name")?,
            };
            let value = if self.match_token(&TokenKind::Colon) {
                self.expression()?
            } else {
                expr(ExprKind::Identifier(field_name.clone()), field_span)
            };
            fields.push(StructFieldInit {
                name: field_name,
                value: Box::new(value),
                span: field_span.merge(self.previous().span),
            });
            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }
        self.consume(&TokenKind::RBrace, "`}`")?;
        let span = start.merge(self.previous().span);
        Ok(expr(
            ExprKind::StructLiteral {
                name,
                fields,
                base: base.map(Box::new),
            },
            span,
        ))
    }

    fn closure(&mut self) -> Result<Expr> {
        let start = self.peek().span;
        self.match_token(&TokenKind::Move);
        let mut params = Vec::new();
        if !self.match_token(&TokenKind::OrOr) {
            self.consume(&TokenKind::Pipe, "`|` to open the closure parameters")?;
            while !self.check(&TokenKind::Pipe) {
                let span = self.peek().span;
                if self.check(&TokenKind::LParen) || self.check(&TokenKind::Ampersand) {
                    return self.fail(137, "a pattern in a closure parameter is not supported yet; bind a name", span);
                }
                let mutable = self.match_token(&TokenKind::Mut);
                let name = self.consume_value_name("a closure parameter")?;
                let type_annotation = if self.match_token(&TokenKind::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                params.push(Parameter::new(name, mutable, type_annotation, span.merge(self.previous().span)));
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&TokenKind::Pipe, "`|` to close the closure parameters")?;
        }
        let return_type = if self.match_token(&TokenKind::Arrow) {
            let ty = self.parse_type()?;
            if !self.check(&TokenKind::LBrace) {
                return self.unexpected("`{`: a closure with a return type has a block body");
            }
            Some(ty)
        } else {
            None
        };
        let body = if self.match_token(&TokenKind::LBrace) {
            self.closure_body()?
        } else {
            let saved = std::mem::take(&mut self.loops);
            let value = self.expr_bp(0);
            self.loops = saved;
            let value = value?;
            let span = value.span;
            vec![Stmt::new(StmtKind::Expression(value), span)]
        };
        Ok(expr(
            ExprKind::Lambda {
                params,
                return_type,
                body,
            },
            start.merge(self.previous().span),
        ))
    }
}
