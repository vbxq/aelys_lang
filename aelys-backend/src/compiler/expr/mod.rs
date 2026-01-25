mod binary;
mod control;
mod identifier;
mod identifier_helpers;
mod literal;
mod logic;
mod typed;
mod unary;

use super::Compiler;
use aelys_common::Result;
use aelys_syntax::ast::{Expr, ExprKind};

impl Compiler {
    pub fn compile_expr(&mut self, expr: &Expr, dest: u8) -> Result<()> {
        match &expr.kind {
            ExprKind::Int(n) => self.compile_literal_int(*n, dest, expr.span),
            ExprKind::Float(f) => self.compile_literal_float(*f, dest, expr.span),
            ExprKind::String(s) => self.compile_literal_string(s, dest, expr.span),
            ExprKind::Bool(b) => self.compile_literal_bool(*b, dest, expr.span),
            ExprKind::Null => self.compile_literal_null(dest, expr.span),
            ExprKind::Identifier(name) => self.compile_identifier(name, dest, expr.span),
            ExprKind::Binary { left, op, right } => {
                self.compile_binary(left, *op, right, dest, expr.span)
            }
            ExprKind::Unary { op, operand } => self.compile_unary(*op, operand, dest, expr.span),
            ExprKind::And { left, right } => self.compile_and(left, right, dest, expr.span),
            ExprKind::Or { left, right } => self.compile_or(left, right, dest, expr.span),
            ExprKind::Call { callee, args } => self.compile_call(callee, args, dest, expr.span),
            ExprKind::Assign { name, value } => self.compile_assign(name, value, dest, expr.span),
            ExprKind::Grouping(inner) => self.compile_expr(inner, dest),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.compile_if_expr(condition, then_branch, else_branch, dest),
            ExprKind::Lambda {
                params,
                return_type: _,
                body,
            } => self.compile_lambda(params, body, dest, expr.span),
            ExprKind::Member { object, member } => {
                self.compile_member_access(object, member, dest, expr.span)
            }
        }
    }
}
