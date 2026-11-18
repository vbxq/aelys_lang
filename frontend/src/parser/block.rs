use super::Parser;
use aelys_common::Result;
use aelys_syntax::{
    BinaryOp, Expr, ExprKind, MatchArm, MatchOrigin, Pattern, Span, Stmt, StmtKind, TokenKind,
    TypeAnnotation,
};

pub(super) struct LoopCtx {
    label: Option<String>,
    is_loop: bool,
    slot: Option<String>,
    ctl: Option<String>,
    passing: Vec<(usize, bool, bool)>,
    plain_break: Option<Span>,
}

pub(super) struct Body {
    stmts: Vec<Stmt>,
    tail: Option<Tail>,
    span: Span,
}

pub(super) enum Tail {
    Expr(Expr),
    Block(Box<BlockLike>),
}

pub(super) enum BlockLike {
    If {
        cond: Expr,
        then: Body,
        els: Option<Box<Else>>,
        span: Span,
    },
    IfLet {
        pattern: Pattern,
        scrutinee: Expr,
        then: Body,
        els: Option<Box<Else>>,
        span: Span,
    },
    Match(Expr),
    Block(Body),
    Unsafe(Body, Span),
    Loop(LoopForm),
}

pub(super) enum Else {
    If(BlockLike),
    Block(Body),
}

pub(super) struct LoopForm {
    pre: Vec<Stmt>,
    stmt: Stmt,
    post: Vec<Stmt>,
    slot: Option<String>,
    span: Span,
}

fn expr(kind: ExprKind, span: Span) -> Expr {
    Expr::new(kind, span)
}

fn ident(name: &str, span: Span) -> Expr {
    expr(ExprKind::Identifier(name.to_string()), span)
}

fn assign(name: &str, value: Expr, span: Span) -> Stmt {
    let e = expr(
        ExprKind::Assign {
            name: name.to_string(),
            value: Box::new(value),
        },
        span,
    );
    Stmt::new(StmtKind::Semi(e), span)
}

fn let_mut(name: &str, init: Expr, span: Span) -> Stmt {
    Stmt::new(
        StmtKind::Let {
            name: name.to_string(),
            mutable: true,
            type_annotation: None,
            initializer: init,
            is_pub: false,
        },
        span,
    )
}

fn err_zero(span: Span) -> Expr {
    expr(
        ExprKind::EnumVariant {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            args: vec![expr(ExprKind::Int(0), span)],
            unqualified: true,
        },
        span,
    )
}

pub(super) fn method_call(object: Expr, method: &str, args: Vec<Expr>, span: Span) -> Expr {
    let callee = expr(
        ExprKind::Member {
            object: Box::new(object),
            member: method.to_string(),
        },
        span,
    );
    expr(
        ExprKind::Call {
            callee: Box::new(callee),
            args,
        },
        span,
    )
}

fn unit_block(stmts: Vec<Stmt>, span: Span) -> Expr {
    expr(
        ExprKind::Block {
            stmts,
            tail: Box::new(expr(ExprKind::Null, span)),
        },
        span,
    )
}

fn compare_ctl(name: &str, op: BinaryOp, value: i64, span: Span) -> Expr {
    expr(
        ExprKind::Binary {
            left: Box::new(ident(name, span)),
            op,
            right: Box::new(expr(ExprKind::Int(value), span)),
        },
        span,
    )
}

fn if_then(cond: Expr, then: Vec<Stmt>, span: Span) -> Stmt {
    Stmt::new(
        StmtKind::If {
            condition: cond,
            then_branch: Box::new(Stmt::new(StmtKind::Block(then), span)),
            else_branch: None,
        },
        span,
    )
}

fn is_jump(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue => true,
        StmtKind::Semi(e) | StmtKind::Expression(e) => expr_is_jump(e),
        StmtKind::Block(stmts) => stmts.last().is_some_and(is_jump),
        _ => false,
    }
}

fn expr_is_jump(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Block { stmts, tail } if matches!(tail.kind, ExprKind::Null) && stmts.last().is_some_and(is_jump))
}

impl Parser {
    pub(super) fn function_body(&mut self, returns: bool) -> Result<Vec<Stmt>> {
        self.consume(&TokenKind::LBrace, "`{`")?;
        let saved = std::mem::take(&mut self.loops);
        self.fn_returns.push(returns);
        let body = self.block_body();
        self.fn_returns.pop();
        self.loops = saved;
        let body = body?;
        self.body_stmts(body, returns)
    }

    pub(super) fn closure_body(&mut self) -> Result<Vec<Stmt>> {
        let saved = std::mem::take(&mut self.loops);
        let saved_struct = std::mem::replace(&mut self.no_struct, false);
        self.fn_returns.push(true);
        let body = self.block_body();
        self.fn_returns.pop();
        self.no_struct = saved_struct;
        self.loops = saved;
        let body = body?;
        self.body_stmts(body, true)
    }

    pub(super) fn block_body(&mut self) -> Result<Body> {
        let open = self.previous().span;
        let saved_struct = std::mem::replace(&mut self.no_struct, false);
        self.block_depth += 1;
        let result = self.block_items(open);
        self.block_depth -= 1;
        self.no_struct = saved_struct;
        result
    }

    fn block_items(&mut self, open: Span) -> Result<Body> {
        let mut stmts = Vec::new();
        loop {
            if self.match_token(&TokenKind::RBrace) {
                return Ok(Body {
                    stmts,
                    tail: None,
                    span: open.merge(self.previous().span),
                });
            }
            if self.is_at_end() {
                return self.unexpected("`}`");
            }
            if self.match_token(&TokenKind::Semicolon) {
                continue;
            }
            match self.peek().kind.clone() {
                TokenKind::Let => {
                    stmts.extend(self.let_statement()?);
                    continue;
                }
                TokenKind::Return | TokenKind::Break | TokenKind::Continue => {
                    stmts.extend(self.jump_statement()?);
                    if !self.check(&TokenKind::RBrace) {
                        self.expect_semi("this statement")?;
                    }
                    continue;
                }
                TokenKind::Identifier(w) if w == "discard" && self.discard_ahead() => {
                    return self.fail_help(
                        118,
                        "`discard` is not a statement; write `let _ = e;`",
                        "`let _ = e;` evaluates `e` and drops its value",
                        self.peek().span,
                    );
                }
                _ => {}
            }
            if self.starts_item() {
                stmts.extend(self.item()?);
                continue;
            }
            if self.starts_block_like() {
                let form = self.block_like()?;
                if self.match_token(&TokenKind::RBrace) {
                    return Ok(Body {
                        stmts,
                        tail: Some(Tail::Block(Box::new(form))),
                        span: open.merge(self.previous().span),
                    });
                }
                if self.match_token(&TokenKind::Semicolon) {
                    let mut parts = form.into_stmts(self, false)?;
                    for part in &mut parts {
                        discard_value(part);
                    }
                    stmts.extend(parts);
                    continue;
                }
                if matches!(self.peek().kind, TokenKind::Dot | TokenKind::Question) {
                    let value = form.into_value(self, true)?;
                    let value = self.postfix_rest(value)?;
                    let value = self.binary_rest(value, 0)?;
                    if self.match_token(&TokenKind::RBrace) {
                        return Ok(Body {
                            stmts,
                            tail: Some(Tail::Expr(value)),
                            span: open.merge(self.previous().span),
                        });
                    }
                    self.expect_semi("this expression")?;
                    stmts.push(self.semi_stmt(value));
                    continue;
                }
                stmts.extend(form.into_stmts(self, false)?);
                continue;
            }
            let value = self.expression()?;
            if self.match_token(&TokenKind::RBrace) {
                return Ok(Body {
                    stmts,
                    tail: Some(Tail::Expr(value)),
                    span: open.merge(self.previous().span),
                });
            }
            self.expect_semi("this expression")?;
            stmts.push(self.semi_stmt(value));
        }
    }

    fn discard_ahead(&self) -> bool {
        let next = &self.peek_at(1).kind;
        !matches!(next, TokenKind::LParen) && super::expr::starts_expression(next, false)
    }

    pub(super) fn semi_stmt(&mut self, value: Expr) -> Stmt {
        let span = value.span;
        if self.panics.contains(&(span.start, span.end)) {
            return self.panic_stmt(value);
        }
        Stmt::new(StmtKind::Semi(value), span)
    }

    // the panic expression `Err(0).expect(msg)` takes its type from the context, which a statement does not give
    fn panic_stmt(&mut self, value: Expr) -> Stmt {
        let span = value.span;
        let ExprKind::Call { args, .. } = value.kind else {
            return Stmt::new(StmtKind::Semi(value), span);
        };
        let slot = self.temp("panic");
        let mut ty = TypeAnnotation::new("Result".to_string(), span);
        ty.type_params = vec![
            TypeAnnotation::new("i64".to_string(), span),
            TypeAnnotation::new("i64".to_string(), span),
        ];
        let binding = Stmt::new(
            StmtKind::Let {
                name: slot.clone(),
                mutable: false,
                type_annotation: Some(ty),
                initializer: err_zero(span),
                is_pub: false,
            },
            span,
        );
        let call = method_call(ident(&slot, span), "expect", args, span);
        Stmt::new(
            StmtKind::Block(vec![binding, Stmt::new(StmtKind::Semi(call), span)]),
            span,
        )
    }

    pub(super) fn starts_block_like(&self) -> bool {
        match self.peek().kind {
            TokenKind::If
            | TokenKind::Match
            | TokenKind::While
            | TokenKind::For
            | TokenKind::Loop
            | TokenKind::LBrace => true,
            TokenKind::Unsafe => self.peek_at(1).kind == TokenKind::LBrace,
            TokenKind::Lifetime(_) => self.peek_at(1).kind == TokenKind::Colon,
            _ => false,
        }
    }

    pub(super) fn block_like(&mut self) -> Result<BlockLike> {
        self.enter_recursion()?;
        let result = self.block_like_inner();
        self.exit_recursion();
        result
    }

    fn block_like_inner(&mut self) -> Result<BlockLike> {
        let start = self.peek().span;
        match self.peek().kind.clone() {
            TokenKind::Lifetime(label) => {
                self.advance();
                self.advance();
                if !matches!(self.peek().kind, TokenKind::Loop | TokenKind::While | TokenKind::For) {
                    return self.fail(123, "a label is only allowed on a loop", start);
                }
                self.loop_like(Some(label))
            }
            TokenKind::If => self.if_like(),
            TokenKind::Match => Ok(BlockLike::Match(self.match_expression()?)),
            TokenKind::While | TokenKind::For | TokenKind::Loop => self.loop_like(None),
            TokenKind::LBrace => {
                self.advance();
                Ok(BlockLike::Block(self.block_body()?))
            }
            TokenKind::Unsafe => {
                self.advance();
                self.consume(&TokenKind::LBrace, "`{`")?;
                let body = self.block_body()?;
                Ok(BlockLike::Unsafe(body, start.merge(self.previous().span)))
            }
            _ => self.unexpected("a block"),
        }
    }

    fn if_like(&mut self) -> Result<BlockLike> {
        let start = self.peek().span;
        self.advance();
        if self.match_token(&TokenKind::Let) {
            let (pattern, scrutinee) = self.let_pattern_head()?;
            self.consume(&TokenKind::LBrace, "`{`")?;
            let then = self.block_body()?;
            let els = self.else_branch()?;
            return Ok(BlockLike::IfLet {
                pattern,
                scrutinee,
                then,
                els,
                span: start.merge(self.previous().span),
            });
        }
        let cond = self.no_struct_expr()?;
        self.consume(&TokenKind::LBrace, "`{` after the condition")?;
        let then = self.block_body()?;
        let els = self.else_branch()?;
        Ok(BlockLike::If {
            cond,
            then,
            els,
            span: start.merge(self.previous().span),
        })
    }

    fn else_branch(&mut self) -> Result<Option<Box<Else>>> {
        if !self.match_token(&TokenKind::Else) {
            return Ok(None);
        }
        if self.check(&TokenKind::If) {
            return Ok(Some(Box::new(Else::If(self.if_like()?))));
        }
        self.consume(&TokenKind::LBrace, "`{` or `if` after `else`")?;
        Ok(Some(Box::new(Else::Block(self.block_body()?))))
    }

    pub(super) fn let_pattern_head(&mut self) -> Result<(Pattern, Expr)> {
        let pattern = self.parse_pattern()?;
        self.consume(&TokenKind::Eq, "`=`")?;
        let scrutinee = self.no_struct_expr()?;
        if matches!(scrutinee.kind, ExprKind::And { .. } | ExprKind::Or { .. }) {
            self.refuse_pattern(
                "`if let` chains are not supported; nest the `if` inside the `if let`",
                scrutinee.span,
            );
        }
        Ok((pattern, scrutinee))
    }

    pub(super) fn match_expression(&mut self) -> Result<Expr> {
        let start = self.peek().span;
        self.advance();
        let scrutinee = self.no_struct_expr()?;
        self.consume(&TokenKind::LBrace, "`{`")?;
        let saved_struct = std::mem::replace(&mut self.no_struct, false);
        let arms = self.match_arms();
        self.no_struct = saved_struct;
        let arms = arms?;
        Ok(expr(
            ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
                origin: MatchOrigin::Match,
            },
            start.merge(self.previous().span),
        ))
    }

    fn match_arms(&mut self) -> Result<Vec<MatchArm>> {
        let mut arms = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let arm_start = self.peek().span;
            let pattern = self.parse_pattern()?;
            let guard = if self.match_token(&TokenKind::If) {
                Some(Box::new(self.expression()?))
            } else {
                None
            };
            self.consume(&TokenKind::FatArrow, "`=>`")?;
            let (body, needs_comma) = if self.starts_block_like() {
                let form = self.block_like()?;
                (form.into_value(self, false)?, false)
            } else if matches!(self.peek().kind, TokenKind::Return | TokenKind::Break | TokenKind::Continue) {
                let span = self.peek().span;
                let stmts = self.jump_statement()?;
                (unit_block(stmts, span.merge(self.previous().span)), true)
            } else {
                (self.expression()?, true)
            };
            arms.push(MatchArm {
                pattern,
                guard,
                body: Box::new(body),
                span: arm_start.merge(self.previous().span),
            });
            if self.match_token(&TokenKind::Comma) || self.check(&TokenKind::RBrace) || !needs_comma {
                continue;
            }
            let found = self.peek().kind.to_string();
            return self.fail_help(
                110,
                &format!("expected `,` after this match arm, found {found}"),
                "add `,` here",
                self.after_previous(),
            );
        }
        self.consume(&TokenKind::RBrace, "`}`")?;
        Ok(arms)
    }

    fn loop_like(&mut self, label: Option<String>) -> Result<BlockLike> {
        let start = self.peek().span;
        let keyword = self.advance().kind.clone();
        let stmt = match keyword {
            TokenKind::Loop => {
                self.push_loop(label, true);
                let body = self.loop_body();
                let body = self.finish_loop_body(body)?;
                let span = start.merge(self.previous().span);
                Stmt::new(
                    StmtKind::While {
                        condition: expr(ExprKind::Bool(true), start),
                        body: Box::new(body),
                    },
                    span,
                )
            }
            TokenKind::While if self.check(&TokenKind::Let) => {
                self.advance();
                let (pattern, scrutinee) = self.let_pattern_head()?;
                self.push_loop(label, false);
                let body = self.loop_body();
                let body = self.finish_loop_body(body)?;
                let span = start.merge(self.previous().span);
                let body_span = body.span;
                let exit = unit_block(vec![Stmt::new(StmtKind::Break, span)], span);
                let matched = expr(
                    ExprKind::Match {
                        scrutinee: Box::new(scrutinee),
                        arms: vec![
                            MatchArm {
                                pattern,
                                guard: None,
                                body: Box::new(unit_block(vec![body], body_span)),
                                span: body_span,
                            },
                            MatchArm {
                                pattern: Pattern::Wildcard(span),
                                guard: None,
                                body: Box::new(exit),
                                span,
                            },
                        ],
                        origin: MatchOrigin::WhileLet,
                    },
                    span,
                );
                Stmt::new(
                    StmtKind::While {
                        condition: expr(ExprKind::Bool(true), start),
                        body: Box::new(Stmt::new(
                            StmtKind::Block(vec![Stmt::new(StmtKind::Expression(matched), span)]),
                            span,
                        )),
                    },
                    span,
                )
            }
            TokenKind::While => {
                let condition = self.no_struct_expr()?;
                self.push_loop(label, false);
                let body = self.loop_body();
                let body = self.finish_loop_body(body)?;
                Stmt::new(
                    StmtKind::While {
                        condition,
                        body: Box::new(body),
                    },
                    start.merge(self.previous().span),
                )
            }
            _ => self.for_loop(start, label)?,
        };
        let ctx = self.loops.pop().expect("pushed by the loop parser");
        if let (Some(_), Some(at)) = (&ctx.slot, ctx.plain_break) {
            return self.fail_help(
                124,
                "`break` without a value inside a `loop` that breaks with a value",
                "every `break` of this loop gives its value: write `break value;`",
                at,
            );
        }
        let span = start.merge(self.previous().span);
        let mut pre = Vec::new();
        if let Some(ctl) = &ctx.ctl {
            pre.push(let_mut(ctl, expr(ExprKind::Int(0), span), span));
        }
        if let Some(slot) = &ctx.slot {
            pre.push(let_mut(slot, err_zero(span), span));
        }
        let mut post = Vec::new();
        let parent = self.loops.len().checked_sub(1);
        for (target, breaks, continues) in ctx.passing {
            let ctl = self.loops[target].ctl.clone().expect("a passing jump names its target's flag");
            if Some(target) == parent {
                if breaks {
                    post.push(if_then(
                        compare_ctl(&ctl, BinaryOp::Eq, 1, span),
                        vec![Stmt::new(StmtKind::Break, span)],
                        span,
                    ));
                }
                if continues {
                    post.push(if_then(
                        compare_ctl(&ctl, BinaryOp::Eq, 2, span),
                        vec![
                            assign(&ctl, expr(ExprKind::Int(0), span), span),
                            Stmt::new(StmtKind::Continue, span),
                        ],
                        span,
                    ));
                }
            } else {
                post.push(if_then(
                    compare_ctl(&ctl, BinaryOp::Ne, 0, span),
                    vec![Stmt::new(StmtKind::Break, span)],
                    span,
                ));
            }
        }
        Ok(BlockLike::Loop(LoopForm {
            pre,
            stmt,
            post,
            slot: ctx.slot,
            span,
        }))
    }

    fn push_loop(&mut self, label: Option<String>, is_loop: bool) {
        self.loops.push(LoopCtx {
            label,
            is_loop,
            slot: None,
            ctl: None,
            passing: Vec::new(),
            plain_break: None,
        });
    }

    fn loop_body(&mut self) -> Result<Body> {
        self.consume(&TokenKind::LBrace, "`{` to open the loop body")?;
        self.block_body()
    }

    fn finish_loop_body(&mut self, body: Result<Body>) -> Result<Stmt> {
        let body = match body {
            Ok(body) => body,
            Err(e) => {
                self.loops.pop();
                return Err(e);
            }
        };
        let span = body.span;
        let stmts = self.body_stmts(body, false)?;
        Ok(Stmt::new(StmtKind::Block(stmts), span))
    }

    fn for_loop(&mut self, start: Span, label: Option<String>) -> Result<Stmt> {
        if self.check(&TokenKind::LParen) {
            return self.fail(
                902,
                "a tuple pattern in a for loop is not supported; bind the element and read its parts",
                self.peek().span,
            );
        }
        let iterator = self.consume_value_name("a loop variable")?;
        self.consume(&TokenKind::In, "`in`")?;
        let iterable = self.no_struct_expr()?;
        if self.check_ident("step") {
            return self.fail_help(
                120,
                "`step` is not part of a range; write `(a..b).step_by(n)`",
                "`for i in (0..10).step_by(3)` visits 0, 3, 6, 9",
                self.peek().span,
            );
        }
        let head = match iterable.kind {
            ExprKind::Range {
                start: Some(lo),
                end: Some(hi),
                inclusive,
            } => Ok((*lo, *hi, inclusive, None)),
            ExprKind::Call { callee, mut args } => match callee.kind {
                ExprKind::Member { object, member } if member == "step_by" && args.len() == 1 => {
                    match Self::unwrap_grouping(*object).kind {
                        ExprKind::Range {
                            start: Some(lo),
                            end: Some(hi),
                            inclusive,
                        } => {
                            let step = args.pop().unwrap();
                            if matches!(step.kind, ExprKind::Int(n) if n <= 0) {
                                return self.fail(120, "`step_by` takes a positive step", step.span);
                            }
                            Ok((*lo, *hi, inclusive, Some(step)))
                        }
                        other => Err(expr(
                            ExprKind::Call {
                                callee: Box::new(expr(
                                    ExprKind::Member {
                                        object: Box::new(expr(other, iterable.span)),
                                        member,
                                    },
                                    iterable.span,
                                )),
                                args,
                            },
                            iterable.span,
                        )),
                    }
                }
                other => Err(expr(
                    ExprKind::Call {
                        callee: Box::new(expr(other, callee.span)),
                        args,
                    },
                    iterable.span,
                )),
            },
            other => Err(expr(other, iterable.span)),
        };
        self.push_loop(label, false);
        let body = self.loop_body();
        let body = self.finish_loop_body(body)?;
        let span = start.merge(self.previous().span);
        Ok(match head {
            Ok((lo, hi, inclusive, step)) => Stmt::new(
                StmtKind::For {
                    iterator,
                    start: lo,
                    end: hi,
                    inclusive,
                    step: Box::new(step),
                    body: Box::new(body),
                },
                span,
            ),
            Err(iterable) => Stmt::new(
                StmtKind::ForEach {
                    iterator,
                    iterable,
                    body: Box::new(body),
                },
                span,
            ),
        })
    }

    pub(super) fn jump_statement(&mut self) -> Result<Vec<Stmt>> {
        let start = self.peek().span;
        let keyword = self.advance().kind.clone();
        match keyword {
            TokenKind::Return => {
                let value = if super::expr::starts_expression(&self.peek().kind, false) {
                    Some(self.expression()?)
                } else {
                    None
                };
                Ok(vec![Stmt::new(StmtKind::Return(value), start.merge(self.previous().span))])
            }
            TokenKind::Break => {
                let label = self.jump_label()?;
                let value = if super::expr::starts_expression(&self.peek().kind, self.no_struct) {
                    Some(self.expression()?)
                } else {
                    None
                };
                self.break_stmts(label, value, start)
            }
            _ => {
                let label = self.jump_label()?;
                self.continue_stmts(label, start)
            }
        }
    }

    fn jump_label(&mut self) -> Result<Option<(String, Span)>> {
        if let TokenKind::Lifetime(name) = self.peek().kind.clone() {
            let span = self.peek().span;
            self.advance();
            return Ok(Some((name, span)));
        }
        Ok(None)
    }

    fn jump_target(&self, label: &Option<(String, Span)>) -> Result<Option<usize>> {
        let Some((name, span)) = label else {
            return Ok(self.loops.len().checked_sub(1));
        };
        match self.loops.iter().rposition(|l| l.label.as_deref() == Some(name.as_str())) {
            Some(i) => Ok(Some(i)),
            None => self.fail(123, &format!("use of undeclared label `'{name}`"), *span),
        }
    }

    fn break_stmts(&mut self, label: Option<(String, Span)>, value: Option<Expr>, start: Span) -> Result<Vec<Stmt>> {
        let span = start.merge(self.previous().span);
        let Some(target) = self.jump_target(&label)? else {
            return Ok(vec![Stmt::new(StmtKind::Break, span)]);
        };
        let mut out = Vec::new();
        if value.is_none() && self.loops[target].plain_break.is_none() {
            self.loops[target].plain_break = Some(span);
        }
        if let Some(value) = value {
            if !self.loops[target].is_loop {
                return self.fail(
                    124,
                    "`break` with a value is only allowed inside `loop`, not `while` or `for`",
                    span,
                );
            }
            let slot = match &self.loops[target].slot {
                Some(s) => s.clone(),
                None => {
                    let s = self.temp("loop");
                    self.loops[target].slot = Some(s.clone());
                    s
                }
            };
            let wrapped = expr(
                ExprKind::EnumVariant {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    args: vec![value],
                    unqualified: true,
                },
                span,
            );
            out.push(assign(&slot, wrapped, span));
        }
        self.route_outer(target, true, span, &mut out);
        out.push(Stmt::new(StmtKind::Break, span));
        Ok(out)
    }

    fn continue_stmts(&mut self, label: Option<(String, Span)>, start: Span) -> Result<Vec<Stmt>> {
        let span = start.merge(self.previous().span);
        let Some(target) = self.jump_target(&label)? else {
            return Ok(vec![Stmt::new(StmtKind::Continue, span)]);
        };
        let mut out = Vec::new();
        if target + 1 == self.loops.len() {
            out.push(Stmt::new(StmtKind::Continue, span));
            return Ok(out);
        }
        self.route_outer(target, false, span, &mut out);
        out.push(Stmt::new(StmtKind::Break, span));
        Ok(out)
    }

    // a labeled jump past inner loops sets the target's flag and breaks; each loop it crosses re-tests the flag
    fn route_outer(&mut self, target: usize, breaks: bool, span: Span, out: &mut Vec<Stmt>) {
        if target + 1 == self.loops.len() {
            return;
        }
        let ctl = match &self.loops[target].ctl {
            Some(c) => c.clone(),
            None => {
                let c = self.temp("label");
                self.loops[target].ctl = Some(c.clone());
                c
            }
        };
        out.push(assign(&ctl, expr(ExprKind::Int(if breaks { 1 } else { 2 }), span), span));
        for depth in target + 1..self.loops.len() {
            let passing = &mut self.loops[depth].passing;
            match passing.iter_mut().find(|p| p.0 == target) {
                Some(entry) => {
                    entry.1 |= breaks;
                    entry.2 |= !breaks;
                }
                None => passing.push((target, breaks, !breaks)),
            }
        }
    }

    pub(super) fn let_statement(&mut self) -> Result<Vec<Stmt>> {
        let start = self.peek().span;
        self.advance();
        let pattern = self.parse_let_pattern()?;
        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        if !self.match_token(&TokenKind::Eq) {
            if self.check(&TokenKind::Semicolon) {
                return self.fail_help(
                    133,
                    "a `let` needs an initializer",
                    "give the binding its first value: `let x = 0;`",
                    start.merge(self.previous().span),
                );
            }
            return self.unexpected("`=`");
        }
        let initializer = self.expression()?;
        if self.match_token(&TokenKind::Else) {
            self.consume(&TokenKind::LBrace, "`{` after `else`")?;
            let otherwise = self.block_body()?;
            self.expect_semi("this `let ... else` statement")?;
            let span = start.merge(self.previous().span);
            return self.let_else(pattern, type_annotation, initializer, otherwise, span);
        }
        self.expect_semi("this `let` statement")?;
        let span = start.merge(self.previous().span);
        let kind = match pattern {
            Pattern::Binding {
                name,
                mutable,
                sub: None,
                ..
            } => StmtKind::Let {
                name,
                mutable,
                type_annotation,
                initializer,
                is_pub: false,
            },
            Pattern::Wildcard(_) => StmtKind::Discard(initializer),
            pattern @ (Pattern::Tuple { .. } | Pattern::Struct { .. }) => StmtKind::LetPattern {
                pattern,
                type_annotation,
                initializer,
            },
            other => {
                return self.fail_help(
                    101,
                    "this pattern can fail to match, so `let` needs an `else`",
                    "write `let PATTERN = value else { return; };`",
                    other.span(),
                );
            }
        };
        Ok(vec![Stmt::new(kind, span)])
    }

    fn let_else(
        &mut self,
        mut pattern: Pattern,
        type_annotation: Option<TypeAnnotation>,
        initializer: Expr,
        otherwise: Body,
        span: Span,
    ) -> Result<Vec<Stmt>> {
        if !body_diverges(&otherwise, &self.panics) {
            return self.fail(
                125,
                "the `else` of a `let ... else` must leave the scope with `return`, `break`, `continue` or `panic!`",
                otherwise.span,
            );
        }
        let mut bindings = Vec::new();
        collect_bindings(&mut pattern, &mut bindings);
        let otherwise = self.body_value(otherwise, false)?;
        let value = match bindings.len() {
            0 => expr(ExprKind::Null, span),
            1 => ident(&bindings[0].0, span),
            _ => expr(
                ExprKind::Tuple(bindings.iter().map(|(n, _)| ident(n, span)).collect()),
                span,
            ),
        };
        let matched = expr(
            ExprKind::Match {
                scrutinee: Box::new(initializer),
                arms: vec![
                    MatchArm {
                        pattern,
                        guard: None,
                        body: Box::new(value),
                        span,
                    },
                    MatchArm {
                        pattern: Pattern::Wildcard(span),
                        guard: None,
                        body: Box::new(otherwise),
                        span,
                    },
                ],
                origin: MatchOrigin::Match,
            },
            span,
        );
        Ok(match bindings.len() {
            0 => vec![Stmt::new(StmtKind::Semi(matched), span)],
            1 => vec![Stmt::new(
                StmtKind::Let {
                    name: bindings[0].0.clone(),
                    mutable: bindings[0].1,
                    type_annotation,
                    initializer: matched,
                    is_pub: false,
                },
                span,
            )],
            _ => {
                let whole = self.temp("else");
                let mut stmts = vec![Stmt::new(
                    StmtKind::Let {
                        name: whole.clone(),
                        mutable: false,
                        type_annotation: None,
                        initializer: matched,
                        is_pub: false,
                    },
                    span,
                )];
                for (i, (name, mutable)) in bindings.into_iter().enumerate() {
                    let part = expr(
                        ExprKind::Member {
                            object: Box::new(ident(&whole, span)),
                            member: i.to_string(),
                        },
                        span,
                    );
                    stmts.push(Stmt::new(
                        StmtKind::Let {
                            name,
                            mutable,
                            type_annotation: None,
                            initializer: part,
                            is_pub: false,
                        },
                        span,
                    ));
                }
                stmts
            }
        })
    }

    pub(super) fn body_stmts(&mut self, body: Body, value_tail: bool) -> Result<Vec<Stmt>> {
        let mut stmts = body.stmts;
        match body.tail {
            None => {}
            Some(Tail::Expr(e)) => {
                let span = e.span;
                if !value_tail && self.panics.contains(&(span.start, span.end)) {
                    stmts.push(self.panic_stmt(e));
                } else {
                    stmts.push(Stmt::new(StmtKind::Expression(e), span));
                }
            }
            Some(Tail::Block(form)) => stmts.extend(form.into_stmts(self, value_tail)?),
        }
        Ok(stmts)
    }

    pub(super) fn body_value(&mut self, body: Body, required: bool) -> Result<Expr> {
        let span = body.span;
        let stmts = body.stmts;
        let tail = match body.tail {
            Some(Tail::Expr(e)) => e,
            Some(Tail::Block(form)) => form.into_value(self, required)?,
            None => {
                if required && !stmts.last().is_some_and(is_jump) {
                    return match stmts.last() {
                        Some(last) if matches!(last.kind, StmtKind::Semi(_)) => self.fail_help(
                            114,
                            "this block ends with `;`, so it has no value",
                            "remove the `;` after its last expression",
                            last.span,
                        ),
                        _ => self.fail(114, "this block has no value; end it with an expression", span),
                    };
                }
                if stmts.is_empty() {
                    return Ok(expr(ExprKind::Null, span));
                }
                return Ok(unit_block(stmts, span));
            }
        };
        if stmts.is_empty() {
            return Ok(tail);
        }
        Ok(expr(
            ExprKind::Block {
                stmts,
                tail: Box::new(tail),
            },
            span,
        ))
    }

    pub(super) fn block_like_value(&mut self) -> Result<Expr> {
        let form = self.block_like()?;
        form.into_value(self, true)
    }
}

impl BlockLike {
    pub(super) fn into_stmts(self, p: &mut Parser, value_tail: bool) -> Result<Vec<Stmt>> {
        Ok(match self {
            BlockLike::If { cond, then, els, span } => {
                let then_span = then.span;
                let then = Stmt::new(StmtKind::Block(p.body_stmts(then, value_tail)?), then_span);
                let els = match els {
                    None => None,
                    Some(e) => Some(Box::new(match *e {
                        Else::Block(b) => {
                            let s = b.span;
                            Stmt::new(StmtKind::Block(p.body_stmts(b, value_tail)?), s)
                        }
                        Else::If(form) => {
                            let mut parts = form.into_stmts(p, value_tail)?;
                            if parts.len() == 1 {
                                parts.pop().unwrap()
                            } else {
                                Stmt::new(StmtKind::Block(parts), span)
                            }
                        }
                    })),
                };
                vec![Stmt::new(
                    StmtKind::If {
                        condition: cond,
                        then_branch: Box::new(then),
                        else_branch: els,
                    },
                    span,
                )]
            }
            BlockLike::IfLet {
                pattern,
                scrutinee,
                then,
                els,
                span,
            } => {
                if value_tail && els.is_some() {
                    let value = BlockLike::IfLet {
                        pattern,
                        scrutinee,
                        then,
                        els,
                        span,
                    }
                    .into_value(p, false)?;
                    return Ok(vec![Stmt::new(StmtKind::Expression(value), span)]);
                }
                let then_span = then.span;
                let then = Stmt::new(StmtKind::Block(p.body_stmts(then, false)?), then_span);
                let els = match els {
                    None => Stmt::new(StmtKind::Block(Vec::new()), span),
                    Some(e) => match *e {
                        Else::Block(b) => {
                            let s = b.span;
                            Stmt::new(StmtKind::Block(p.body_stmts(b, false)?), s)
                        }
                        Else::If(form) => {
                            let parts = form.into_stmts(p, false)?;
                            Stmt::new(StmtKind::Block(parts), span)
                        }
                    },
                };
                let matched = if_let_match(
                    pattern,
                    scrutinee,
                    unit_block(vec![then], then_span),
                    unit_block(vec![els], span),
                    span,
                );
                vec![Stmt::new(StmtKind::Expression(matched), span)]
            }
            BlockLike::Match(e) => {
                let span = e.span;
                vec![Stmt::new(StmtKind::Expression(e), span)]
            }
            BlockLike::Block(b) => {
                let span = b.span;
                vec![Stmt::new(StmtKind::Block(p.body_stmts(b, value_tail)?), span)]
            }
            BlockLike::Unsafe(b, span) => {
                let value = p.body_value(b, value_tail)?;
                vec![Stmt::new(StmtKind::Expression(expr(ExprKind::Unsafe(Box::new(value)), span)), span)]
            }
            BlockLike::Loop(form) => {
                if value_tail && form.slot.is_some() {
                    let span = form.span;
                    let value = BlockLike::Loop(form).into_value(p, true)?;
                    return Ok(vec![Stmt::new(StmtKind::Expression(value), span)]);
                }
                let mut out = form.pre;
                out.push(form.stmt);
                out.extend(form.post);
                out
            }
        })
    }

    pub(super) fn into_value(self, p: &mut Parser, required: bool) -> Result<Expr> {
        match self {
            BlockLike::If {
                cond,
                then,
                els: Some(els),
                span,
            } => {
                let then = p.body_value(then, required)?;
                let els = else_value(*els, p, required)?;
                Ok(expr(
                    ExprKind::If {
                        condition: Box::new(cond),
                        then_branch: Box::new(then),
                        else_branch: Box::new(els),
                    },
                    span,
                ))
            }
            BlockLike::IfLet {
                pattern,
                scrutinee,
                then,
                els: Some(els),
                span,
            } => {
                let then = p.body_value(then, required)?;
                let els = else_value(*els, p, required)?;
                Ok(if_let_match(pattern, scrutinee, then, els, span))
            }
            form @ (BlockLike::If { els: None, .. } | BlockLike::IfLet { els: None, .. }) => {
                let span = match &form {
                    BlockLike::If { span, .. } | BlockLike::IfLet { span, .. } => *span,
                    _ => unreachable!(),
                };
                if required {
                    return p.fail_help(
                        114,
                        "an `if` without `else` has no value",
                        "add an `else` branch",
                        span,
                    );
                }
                let stmts = form.into_stmts(p, false)?;
                Ok(unit_block(stmts, span))
            }
            BlockLike::Match(e) => Ok(e),
            BlockLike::Block(b) => p.body_value(b, required),
            BlockLike::Unsafe(b, span) => {
                let value = p.body_value(b, required)?;
                Ok(expr(ExprKind::Unsafe(Box::new(value)), span))
            }
            BlockLike::Loop(form) => {
                let span = form.span;
                let mut stmts = form.pre;
                stmts.push(form.stmt);
                stmts.extend(form.post);
                match form.slot {
                    Some(slot) => Ok(expr(
                        ExprKind::Block {
                            stmts,
                            tail: Box::new(method_call(ident(&slot, span), "unwrap", Vec::new(), span)),
                        },
                        span,
                    )),
                    None if required => p.fail_help(
                        114,
                        "a loop has no value",
                        "give `loop` a value with `break value;`",
                        span,
                    ),
                    None => Ok(unit_block(stmts, span)),
                }
            }
        }
    }
}

fn else_value(els: Else, p: &mut Parser, required: bool) -> Result<Expr> {
    match els {
        Else::Block(b) => p.body_value(b, required),
        Else::If(form) => form.into_value(p, required),
    }
}

pub(super) fn if_let_match(pattern: Pattern, scrutinee: Expr, then: Expr, els: Expr, span: Span) -> Expr {
    let then_span = then.span;
    let else_span = els.span;
    expr(
        ExprKind::Match {
            scrutinee: Box::new(scrutinee),
            arms: vec![
                MatchArm {
                    pattern,
                    guard: None,
                    body: Box::new(then),
                    span: then_span,
                },
                MatchArm {
                    pattern: Pattern::Wildcard(else_span),
                    guard: None,
                    body: Box::new(els),
                    span: else_span,
                },
            ],
            origin: MatchOrigin::IfLet,
        },
        span,
    )
}

fn discard_value(stmt: &mut Stmt) {
    match &mut stmt.kind {
        StmtKind::Expression(_) => {
            let StmtKind::Expression(e) = std::mem::replace(&mut stmt.kind, StmtKind::Break) else {
                unreachable!()
            };
            stmt.kind = StmtKind::Semi(e);
        }
        StmtKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            discard_value(then_branch);
            if let Some(e) = else_branch {
                discard_value(e);
            }
        }
        StmtKind::Block(stmts) => {
            if let Some(last) = stmts.last_mut() {
                discard_value(last);
            }
        }
        _ => {}
    }
}

fn body_diverges(body: &Body, panics: &std::collections::HashSet<(usize, usize)>) -> bool {
    let is_panic = |e: &Expr| panics.contains(&(e.span.start, e.span.end));
    match &body.tail {
        Some(Tail::Expr(e)) => expr_is_jump(e) || is_panic(e),
        Some(Tail::Block(_)) => false,
        None => body.stmts.last().is_some_and(|s| {
            is_jump(s)
                || matches!(&s.kind, StmtKind::Block(inner) if inner.last().is_some_and(|l| matches!(&l.kind, StmtKind::Semi(c) if is_panic_call(c))))
        }),
    }
}

fn is_panic_call(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::Call { callee, .. } if matches!(&callee.kind, ExprKind::Member { member, .. } if member == "expect"))
}

fn collect_bindings(pattern: &mut Pattern, out: &mut Vec<(String, bool)>) {
    match pattern {
        Pattern::Binding {
            name, mutable, sub, ..
        } => {
            out.push((name.clone(), *mutable));
            *mutable = false;
            if let Some(sub) = sub {
                collect_bindings(sub, out);
            }
        }
        Pattern::Tuple { elems, .. } | Pattern::Variant { fields: elems, .. } => {
            for e in elems {
                collect_bindings(e, out);
            }
        }
        Pattern::Struct { fields, .. } => {
            for (_, p, _) in fields {
                collect_bindings(p, out);
            }
        }
        Pattern::Or(alts, _) => {
            if let Some(first) = alts.first_mut() {
                collect_bindings(first, out);
            }
        }
        _ => {}
    }
}
