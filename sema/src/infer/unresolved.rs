use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{
    TypedExpr, TypedExprKind, TypedFmtStringPart, TypedFunction, TypedPattern, TypedStmt,
    TypedStmtKind,
};
use crate::types::{InferType, TypeVarId};
use aelys_syntax::Span;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Param,
    Binding,
    Expression,
}

#[derive(Clone)]
struct Site {
    rank: Rank,
    span: Span,
    name: Option<String>,
    ty: InferType,
}

impl Site {
    fn key(&self) -> (Rank, usize, usize) {
        (self.rank, self.span.start, self.span.end)
    }
}

fn collect_vars(ty: &InferType, out: &mut Vec<TypeVarId>) {
    match ty {
        InferType::Var(id) => out.push(*id),
        InferType::Function { params, ret, .. } => {
            params.iter().for_each(|p| collect_vars(p, out));
            collect_vars(ret, out);
        }
        InferType::Array(inner, _) | InferType::Vec(inner) | InferType::Rc(inner) => {
            collect_vars(inner, out)
        }
        InferType::Ref { referent, .. } => collect_vars(referent, out),
        InferType::Slice { elem, .. } => collect_vars(elem, out),
        InferType::Tuple(elems) => elems.iter().for_each(|e| collect_vars(e, out)),
        InferType::Enum(_, args) => args.iter().for_each(|a| collect_vars(a, out)),
        _ => {}
    }
}

pub(crate) fn spelled(ty: &InferType) -> String {
    match ty {
        InferType::Var(_) => "T".to_string(),
        InferType::Rc(inner) => format!("Rc<{}>", spelled(inner)),
        InferType::Vec(inner) => format!("Vec<{}>", spelled(inner)),
        InferType::Array(inner, Some(n)) => format!("[{}; {n}]", spelled(inner)),
        InferType::Array(inner, None) => format!("[{}; N]", spelled(inner)),
        InferType::Ref { referent, mutable } => {
            format!(
                "&{}{}",
                if *mutable { "mut " } else { "" },
                spelled(referent)
            )
        }
        InferType::Slice { elem, mutable } => {
            format!("&{}[{}]", if *mutable { "mut " } else { "" }, spelled(elem))
        }
        InferType::Function { params, ret, .. } => {
            let params: Vec<String> = params.iter().map(spelled).collect();
            match ret.as_ref() {
                InferType::Null => format!("fn({})", params.join(", ")),
                ret => format!("fn({}) -> {}", params.join(", "), spelled(ret)),
            }
        }
        InferType::Enum(name, args) if !args.is_empty() => {
            let args: Vec<String> = args.iter().map(spelled).collect();
            format!(
                "{}<{}>",
                crate::modules::source_type_name(name),
                args.join(", ")
            )
        }
        InferType::Struct(name) => crate::modules::source_type_name(name).to_string(),
        InferType::Enum(name, _) => crate::modules::source_type_name(name).to_string(),
        other => other.to_string(),
    }
}

impl TypeInference {
    pub(super) fn reject_unresolved_types(&mut self, stmts: &[TypedStmt]) {
        let mut sites = Vec::new();
        for stmt in stmts {
            sites_in_stmt(stmt, &mut sites);
        }
        let mut best: HashMap<TypeVarId, usize> = HashMap::new();
        for (i, site) in sites.iter().enumerate() {
            let mut vars = Vec::new();
            collect_vars(&site.ty, &mut vars);
            for id in vars {
                let keep = best.get(&id).is_some_and(|&j| sites[j].key() <= site.key());
                if !keep {
                    best.insert(id, i);
                }
            }
        }
        let mut chosen: Vec<usize> = best.into_values().collect();
        chosen.sort_by_key(|&i| (sites[i].span.start, sites[i].span.end, sites[i].rank));
        chosen.dedup_by_key(|i| (sites[*i].span.start, sites[*i].span.end));
        for i in chosen {
            self.errors.push(unresolved_type(&sites[i]));
        }
        if let Some(site) = sites
            .iter()
            .filter(|s| s.rank == Rank::Expression && holds_rc_of_unit(&s.ty))
            .min_by_key(|s| (s.span.start, std::cmp::Reverse(s.span.end)))
        {
            self.errors.push(TypeError::rc_of_unit(site.span));
        }
    }
}

fn holds_rc_of_unit(ty: &InferType) -> bool {
    match ty {
        InferType::Rc(inner) => {
            matches!(inner.as_ref(), InferType::Null) || holds_rc_of_unit(inner)
        }
        InferType::Function { params, ret, .. } => {
            params.iter().any(holds_rc_of_unit) || holds_rc_of_unit(ret)
        }
        InferType::Array(inner, _) | InferType::Vec(inner) => holds_rc_of_unit(inner),
        InferType::Ref { referent, .. } => holds_rc_of_unit(referent),
        InferType::Slice { elem, .. } => holds_rc_of_unit(elem),
        InferType::Tuple(elems) => elems.iter().any(holds_rc_of_unit),
        InferType::Enum(_, args) => args.iter().any(holds_rc_of_unit),
        _ => false,
    }
}

fn unresolved_type(site: &Site) -> TypeError {
    let shown = spelled(&site.ty);
    let (message, help) = match (site.rank, &site.name) {
        (Rank::Param, Some(name)) => (
            format!(
                "type annotations needed: the type of the parameter `{name}` is never inferred"
            ),
            format!("annotate the parameter: `{name}: {shown}`, with `T` a concrete type"),
        ),
        (_, Some(name)) => (
            format!("type annotations needed: the type of `{name}` is never inferred"),
            format!("annotate the binding: `let {name}: {shown} = ..`, with `T` a concrete type"),
        ),
        (_, None) => (
            "type annotations needed: the type of this expression is never inferred".to_string(),
            format!("bind it with an annotation: `let x: {shown} = ..`, with `T` a concrete type"),
        ),
    };
    TypeError {
        kind: TypeErrorKind::MemberAccess { message },
        span: site.span,
        reason: ConstraintReason::Other("type annotations needed".to_string()),
        secondary_spans: Vec::new(),
        help: Some(help),
        suggestion: None,
    }
}

fn push(sites: &mut Vec<Site>, rank: Rank, span: Span, name: Option<&str>, ty: &InferType) {
    if ty.has_vars() || holds_rc_of_unit(ty) {
        sites.push(Site {
            rank,
            span,
            name: name.map(str::to_string),
            ty: ty.clone(),
        });
    }
}

fn sites_in_function(func: &TypedFunction, sites: &mut Vec<Site>) {
    if func.foreign.is_some() {
        return;
    }
    for param in &func.params {
        push(sites, Rank::Param, param.span, Some(&param.name), &param.ty);
    }
    for stmt in &func.body {
        sites_in_stmt(stmt, sites);
    }
}

fn sites_in_stmt(stmt: &TypedStmt, sites: &mut Vec<Site>) {
    match &stmt.kind {
        TypedStmtKind::Expression(expr) => sites_in_expr(expr, sites),
        TypedStmtKind::Let {
            name,
            initializer,
            var_type,
            ..
        } => {
            if !diverges(initializer) {
                push(sites, Rank::Binding, stmt.span, Some(name), var_type);
            }
            sites_in_expr(initializer, sites);
        }
        TypedStmtKind::Block(stmts) => stmts.iter().for_each(|s| sites_in_stmt(s, sites)),
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            sites_in_expr(condition, sites);
            sites_in_stmt(then_branch, sites);
            if let Some(e) = else_branch {
                sites_in_stmt(e, sites);
            }
        }
        TypedStmtKind::While { condition, body } => {
            sites_in_expr(condition, sites);
            sites_in_stmt(body, sites);
        }
        TypedStmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            sites_in_expr(start, sites);
            sites_in_expr(end, sites);
            if let Some(step) = step.as_ref() {
                sites_in_expr(step, sites);
            }
            sites_in_stmt(body, sites);
        }
        TypedStmtKind::ForEach {
            iterator,
            iterable,
            elem_type,
            body,
        } => {
            sites_in_expr(iterable, sites);
            push(sites, Rank::Binding, stmt.span, Some(iterator), elem_type);
            sites_in_stmt(body, sites);
        }
        TypedStmtKind::Return(Some(expr)) => sites_in_expr(expr, sites),
        TypedStmtKind::Function(func) => sites_in_function(func, sites),
        TypedStmtKind::Return(None)
        | TypedStmtKind::Break
        | TypedStmtKind::Continue
        | TypedStmtKind::Needs(_)
        | TypedStmtKind::StructDecl { .. }
        | TypedStmtKind::EnumDecl { .. } => {}
    }
}

pub(crate) fn diverges(expr: &TypedExpr) -> bool {
    match &expr.kind {
        _ if matches!(expr.ty, InferType::Never) => true,
        TypedExprKind::Grouping(inner) => diverges(inner),
        TypedExprKind::Match { arms, .. } => {
            !arms.is_empty() && arms.iter().all(|arm| diverges(&arm.body))
        }
        TypedExprKind::If {
            then_branch,
            else_branch,
            ..
        } => diverges(then_branch) && diverges(else_branch),
        TypedExprKind::Block { stmts, tail } => stmts.iter().any(stmt_diverges) || diverges(tail),
        _ => false,
    }
}

fn stmt_diverges(stmt: &TypedStmt) -> bool {
    match &stmt.kind {
        TypedStmtKind::Return(_) | TypedStmtKind::Break | TypedStmtKind::Continue => true,
        TypedStmtKind::Expression(expr) => diverges(expr),
        TypedStmtKind::Block(stmts) => stmts.iter().any(stmt_diverges),
        _ => false,
    }
}

fn sites_in_expr(expr: &TypedExpr, sites: &mut Vec<Site>) {
    if !diverges(expr) {
        push(sites, Rank::Expression, expr.span, None, &expr.ty);
    }
    match &expr.kind {
        TypedExprKind::FmtString(parts) => {
            for part in parts {
                if let TypedFmtStringPart::Expr(e) = part {
                    sites_in_expr(e, sites);
                }
            }
        }
        TypedExprKind::Binary { left, right, .. }
        | TypedExprKind::And { left, right }
        | TypedExprKind::Or { left, right } => {
            sites_in_expr(left, sites);
            sites_in_expr(right, sites);
        }
        TypedExprKind::Unary { operand, .. }
        | TypedExprKind::Grouping(operand)
        | TypedExprKind::Lambda(operand)
        | TypedExprKind::Deref(operand)
        | TypedExprKind::Reference { operand, .. } => sites_in_expr(operand, sites),
        TypedExprKind::Call { callee, args } => {
            sites_in_expr(callee, sites);
            args.iter().for_each(|a| sites_in_expr(a, sites));
        }
        TypedExprKind::Assign { value, .. } => sites_in_expr(value, sites),
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            sites_in_expr(condition, sites);
            sites_in_expr(then_branch, sites);
            sites_in_expr(else_branch, sites);
        }
        TypedExprKind::LambdaInner { params, body, .. } => {
            for param in params {
                push(sites, Rank::Param, param.span, Some(&param.name), &param.ty);
            }
            body.iter().for_each(|s| sites_in_stmt(s, sites));
        }
        TypedExprKind::Member { object, .. } => sites_in_expr(object, sites),
        TypedExprKind::ArrayLiteral { elements } | TypedExprKind::VecLiteral { elements, .. } => {
            elements.iter().for_each(|e| sites_in_expr(e, sites))
        }
        TypedExprKind::ArraySized { size, fill_value } => {
            sites_in_expr(size, sites);
            if let Some(fv) = fill_value {
                sites_in_expr(fv, sites);
            }
        }
        TypedExprKind::Index { object, index } => {
            sites_in_expr(object, sites);
            sites_in_expr(index, sites);
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            sites_in_expr(object, sites);
            sites_in_expr(index, sites);
            sites_in_expr(value, sites);
        }
        TypedExprKind::FieldAssign { object, value, .. } => {
            sites_in_expr(object, sites);
            sites_in_expr(value, sites);
        }
        TypedExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                sites_in_expr(s, sites);
            }
            if let Some(e) = end {
                sites_in_expr(e, sites);
            }
        }
        TypedExprKind::Slice { object, range } => {
            sites_in_expr(object, sites);
            sites_in_expr(range, sites);
        }
        TypedExprKind::DerefAssign { target, value } => {
            sites_in_expr(target, sites);
            sites_in_expr(value, sites);
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().for_each(|(_, v)| sites_in_expr(v, sites))
        }
        TypedExprKind::Cast { expr: inner, .. } => sites_in_expr(inner, sites),
        TypedExprKind::EnumVariant { args, .. } => {
            args.iter().for_each(|a| sites_in_expr(a, sites))
        }
        TypedExprKind::Block { stmts, tail } => {
            stmts.iter().for_each(|s| sites_in_stmt(s, sites));
            sites_in_expr(tail, sites);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            sites_in_expr(scrutinee, sites);
            for arm in arms {
                if let TypedPattern::Variant { bindings, span, .. } = &arm.pattern {
                    for (name, ty) in bindings.iter().filter(|(name, _)| name != "_") {
                        push(sites, Rank::Binding, *span, Some(name), ty);
                    }
                }
                sites_in_expr(&arm.body, sites);
            }
        }
        TypedExprKind::ResultAssert { scrutinee, .. } => sites_in_expr(scrutinee, sites),
        TypedExprKind::Identifier(_)
        | TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Char(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null => {}
    }
}
