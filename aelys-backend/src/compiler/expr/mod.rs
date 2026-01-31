mod array;
mod binary;
mod control;
mod fmt_string;
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
            ExprKind::FmtString(parts) => self.compile_fmt_string(parts, &[], dest, expr.span),
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
            ExprKind::ArrayLiteral { elements, .. } => {
                self.compile_array_literal(elements, dest, expr.span)
            }
            ExprKind::ArraySized { element_type, size } => {
                self.compile_array_sized(element_type, size, dest, expr.span)
            }
            ExprKind::VecLiteral { elements, .. } => {
                self.compile_vec_literal(elements, dest, expr.span)
            }
            ExprKind::Index { object, index } => {
                self.compile_index_access(object, index, dest, expr.span)
            }
            ExprKind::IndexAssign { object, index, value } => {
                self.compile_index_assign(object, index, value, dest, expr.span)
            }
            ExprKind::Range { start, end, inclusive } => {
                self.compile_range(start, end, *inclusive, dest, expr.span)
            }
            ExprKind::Slice { object, range } => {
                self.compile_slice(object, range, dest, expr.span)
            }
        }
    }
}
