use aelys_syntax::{CatchHandler, Expr, ExprKind, FmtStringPart, Stmt, StmtKind, TypeAnnotation};

pub(super) fn stmts(list: &mut [Stmt], f: &mut dyn FnMut(&mut TypeAnnotation)) {
    for s in list {
        stmt(s, f);
    }
}

fn stmt(s: &mut Stmt, f: &mut dyn FnMut(&mut TypeAnnotation)) {
    match &mut s.kind {
        StmtKind::Expression(e) | StmtKind::Discard(e) => expr(e, f),
        StmtKind::Let {
            type_annotation,
            initializer,
            ..
        } => {
            if let Some(ann) = type_annotation {
                f(ann);
            }
            expr(initializer, f);
        }
        StmtKind::LetPattern {
            type_annotation,
            initializer,
            ..
        } => {
            if let Some(ann) = type_annotation {
                f(ann);
            }
            expr(initializer, f);
        }
        StmtKind::Block(list) => stmts(list, f),
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr(condition, f);
            stmt(then_branch, f);
            if let Some(e) = else_branch {
                stmt(e, f);
            }
        }
        StmtKind::While { condition, body } => {
            expr(condition, f);
            stmt(body, f);
        }
        StmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            expr(start, f);
            expr(end, f);
            if let Some(e) = step.as_mut() {
                expr(e, f);
            }
            stmt(body, f);
        }
        StmtKind::ForEach { iterable, body, .. } => {
            expr(iterable, f);
            stmt(body, f);
        }
        StmtKind::Return(e) => {
            if let Some(e) = e {
                expr(e, f);
            }
        }
        StmtKind::Function(func) => {
            for p in &mut func.params {
                if let Some(ann) = &mut p.type_annotation {
                    f(ann);
                }
            }
            if let Some(ann) = &mut func.return_type {
                f(ann);
            }
            stmts(&mut func.body, f);
        }
        StmtKind::StructDecl { fields, .. } => {
            for field in fields {
                f(&mut field.type_annotation);
            }
        }
        StmtKind::EnumDecl { variants, .. } => {
            for v in variants {
                for ann in &mut v.fields {
                    f(ann);
                }
            }
        }
        StmtKind::Const {
            type_annotation,
            initializer,
            ..
        } => {
            f(type_annotation);
            expr(initializer, f);
        }
        StmtKind::TypeAlias { target, .. } => f(target),
        StmtKind::Break | StmtKind::Continue | StmtKind::Needs(_) => {}
    }
}

fn expr(e: &mut Expr, f: &mut dyn FnMut(&mut TypeAnnotation)) {
    match &mut e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::String(_)
        | ExprKind::Char(_)
        | ExprKind::Bool(_)
        | ExprKind::Null
        | ExprKind::Identifier(_) => {}
        ExprKind::FmtString(parts) => {
            for p in parts {
                if let FmtStringPart::Expr(inner) = p {
                    expr(inner, f);
                }
            }
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::And { left, right }
        | ExprKind::Or { left, right } => {
            expr(left, f);
            expr(right, f);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Reference { operand, .. } => expr(operand, f),
        ExprKind::Call { callee, args } => {
            expr(callee, f);
            for a in args {
                expr(a, f);
            }
        }
        ExprKind::Assign { value, .. } => expr(value, f),
        ExprKind::Grouping(inner)
        | ExprKind::Deref(inner)
        | ExprKind::Try(inner)
        | ExprKind::Unsafe(inner) => expr(inner, f),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr(condition, f);
            expr(then_branch, f);
            expr(else_branch, f);
        }
        ExprKind::Lambda {
            params,
            return_type,
            body,
        } => {
            for p in params {
                if let Some(ann) = &mut p.type_annotation {
                    f(ann);
                }
            }
            if let Some(ann) = return_type {
                f(ann);
            }
            stmts(body, f);
        }
        ExprKind::Member { object, .. } => expr(object, f),
        ExprKind::ArrayLiteral { elements } | ExprKind::Tuple(elements) => {
            for x in elements {
                expr(x, f);
            }
        }
        ExprKind::ArraySized { size, fill_value } => {
            expr(size, f);
            if let Some(v) = fill_value {
                expr(v, f);
            }
        }
        ExprKind::VecLiteral {
            element_type,
            elements,
        } => {
            if let Some(ann) = element_type {
                f(ann);
            }
            for x in elements {
                expr(x, f);
            }
        }
        ExprKind::Index { object, index } => {
            expr(object, f);
            expr(index, f);
        }
        ExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            expr(object, f);
            expr(index, f);
            expr(value, f);
        }
        ExprKind::FieldAssign { object, value, .. } => {
            expr(object, f);
            expr(value, f);
        }
        ExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                expr(s, f);
            }
            if let Some(e) = end {
                expr(e, f);
            }
        }
        ExprKind::Slice { object, range } => {
            expr(object, f);
            expr(range, f);
        }
        ExprKind::DerefAssign { target, value } => {
            expr(target, f);
            expr(value, f);
        }
        ExprKind::StructLiteral { fields, .. } => {
            for field in fields {
                expr(&mut field.value, f);
            }
        }
        ExprKind::EnumVariant { args, .. } => {
            for a in args {
                expr(a, f);
            }
        }
        ExprKind::Cast { expr: inner, target } => {
            expr(inner, f);
            f(target);
        }
        ExprKind::Match {
            scrutinee, arms, ..
        } => {
            expr(scrutinee, f);
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    expr(guard, f);
                }
                expr(&mut arm.body, f);
            }
        }
        ExprKind::Block { stmts: list, tail } => {
            stmts(list, f);
            expr(tail, f);
        }
        ExprKind::Catch { scrutinee, handler } => {
            expr(scrutinee, f);
            match handler {
                CatchHandler::Binding { body, .. } => expr(body, f),
                CatchHandler::Arms(arms) => {
                    for arm in arms {
                        if let Some(guard) = &mut arm.guard {
                            expr(guard, f);
                        }
                        expr(&mut arm.body, f);
                    }
                }
            }
        }
    }
}
