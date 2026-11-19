use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart, TypedStmt, TypedStmtKind};
use crate::types::InferType;

/// dereferencing this type reaches storage borrowed immutably.
pub fn deref_is_shared(ty: &InferType) -> bool {
    matches!(ty, InferType::Ref { mutable: false, .. })
}

pub fn projects_through_pointer(ty: &InferType) -> bool {
    matches!(
        ty,
        InferType::Rc(_) | InferType::Ref { .. } | InferType::Null
    )
}

fn is_rc_get(e: &TypedExpr) -> bool {
    matches!(
        &e.kind,
        TypedExprKind::EnumVariant { enum_name, variant, .. }
            if enum_name == "Rc" && variant == "get"
    )
}

fn rc_get_receiver(e: &TypedExpr) -> Option<&TypedExpr> {
    match &e.kind {
        TypedExprKind::EnumVariant {
            enum_name,
            variant,
            args,
            ..
        } if enum_name == "Rc" && variant == "get" => args.first(),
        _ => None,
    }
}

pub fn spine_root_name(e: &TypedExpr) -> Option<&str> {
    match &e.kind {
        TypedExprKind::Identifier(name) => Some(name),
        TypedExprKind::Grouping(inner) | TypedExprKind::Deref(inner) => spine_root_name(inner),
        TypedExprKind::Member { object, .. }
        | TypedExprKind::Index { object, .. }
        | TypedExprKind::Slice { object, .. } => spine_root_name(object),
        _ => None,
    }
}

pub fn is_computed_len(object_ty: &InferType, member: &str) -> bool {
    member == "len"
        && matches!(
            object_ty,
            InferType::String
                | InferType::StrRef
                | InferType::Slice { .. }
                | InferType::Vec(_)
                | InferType::Array(..)
        )
}

pub fn computed_len_receiver(e: &TypedExpr) -> Option<&InferType> {
    match &e.kind {
        TypedExprKind::Member { object, member } if is_computed_len(&object.ty, member) => {
            Some(&object.ty)
        }
        _ => None,
    }
}

pub fn owns_field_place(e: &TypedExpr) -> bool {
    match &e.kind {
        TypedExprKind::Grouping(inner) => owns_field_place(inner),
        TypedExprKind::Identifier(_) => matches!(e.ty, InferType::Struct(..)),
        TypedExprKind::Deref(inner) => matches!(inner.ty, InferType::Ref { mutable: true, .. }),
        TypedExprKind::Member { object, .. } => {
            matches!(e.ty, InferType::Struct(..))
                && (matches!(object.ty, InferType::Rc(_)) || owns_field_place(object))
        }
        _ => is_rc_get(e),
    }
}

// an rc temporary owns the cell it hands out, so the object's type cannot answer for its storage
pub fn denotes_a_place(e: &TypedExpr) -> bool {
    match &e.kind {
        TypedExprKind::Grouping(inner) => denotes_a_place(inner),
        TypedExprKind::Identifier(_) => true,
        TypedExprKind::Deref(_) => true,
        _ if is_rc_get(e) => rc_get_receiver(e).is_some_and(denotes_a_place),
        // `.bytes` is a view the compiler forms on demand, so it has no address of its own
        TypedExprKind::Member { object, member } => {
            !is_computed_len(&object.ty, member)
                && !(member == "bytes" && object.ty.is_str())
                && denotes_a_place(object)
        }
        TypedExprKind::Index { object, .. } => denotes_a_place(object),
        _ => false,
    }
}

// a string literal is .rodata: its bytes outlive every view of them, which no place rule can see
pub fn bytes_receiver_is_backed(e: &TypedExpr) -> bool {
    match &e.kind {
        TypedExprKind::Grouping(inner) => bytes_receiver_is_backed(inner),
        TypedExprKind::String(_) => true,
        _ => denotes_a_place(e),
    }
}

/// stops at the first view it crosses, so a re-slice reads the view's own mutability, not its base's
pub fn place_is_writable(e: &TypedExpr, name_is_mut: &dyn Fn(&str) -> bool) -> bool {
    match &e.ty {
        InferType::Ref { mutable, .. } | InferType::Slice { mutable, .. } => return *mutable,
        _ => {}
    }
    match &e.kind {
        TypedExprKind::Grouping(inner) => place_is_writable(inner, name_is_mut),
        TypedExprKind::Identifier(name) => name_is_mut(name),
        TypedExprKind::Deref(inner) => !deref_is_shared(&inner.ty),
        TypedExprKind::Member { object, member } if is_computed_len(&object.ty, member) => false,
        TypedExprKind::Member { object, .. } | TypedExprKind::Index { object, .. } => {
            if projects_through_pointer(&object.ty) {
                !deref_is_shared(&object.ty)
            } else {
                place_is_writable(object, name_is_mut)
            }
        }
        _ => false,
    }
}

pub fn spine_is_shared(e: &TypedExpr) -> bool {
    // a slice carries its own mutability, so the walk answers from the view and stops there
    if let InferType::Slice { mutable, .. } = &e.ty {
        return !*mutable;
    }
    match &e.kind {
        TypedExprKind::Grouping(inner) => spine_is_shared(inner),
        TypedExprKind::Deref(inner) => deref_is_shared(&inner.ty),
        // an rc handle is not a shared borrow, so it never sets the flag
        _ if is_rc_get(e) => false,
        TypedExprKind::Member { object, .. } | TypedExprKind::Index { object, .. } => {
            if projects_through_pointer(&object.ty) {
                deref_is_shared(&object.ty)
            } else {
                spine_is_shared(object)
            }
        }
        _ => false,
    }
}

pub fn shared_slice_view(e: &TypedExpr) -> Option<InferType> {
    if let InferType::Slice { mutable: false, .. } = &e.ty {
        return Some(e.ty.clone());
    }
    match &e.kind {
        TypedExprKind::Grouping(inner) => shared_slice_view(inner),
        TypedExprKind::Member { object, .. } | TypedExprKind::Index { object, .. } => {
            if projects_through_pointer(&object.ty) {
                None
            } else {
                shared_slice_view(object)
            }
        }
        _ => None,
    }
}

pub fn target_ptr_is_shared(target: &TypedExpr) -> bool {
    let mut t = target;
    while let TypedExprKind::Grouping(inner) = &t.kind {
        t = inner;
    }
    deref_is_shared(&t.ty)
}

fn writes_root(place: &TypedExpr, name: &str) -> bool {
    spine_root_name(place) == Some(name)
}

pub fn may_write_local(expr: &TypedExpr, name: &str) -> bool {
    let w = |e: &TypedExpr| may_write_local(e, name);
    match &expr.kind {
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Char(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null
        | TypedExprKind::Identifier(_)
        | TypedExprKind::Lambda(_)
        | TypedExprKind::LambdaInner { .. } => false,
        TypedExprKind::Assign { name: n, value } => n == name || w(value),
        TypedExprKind::Reference {
            mutable, operand, ..
        } => (*mutable && writes_root(operand, name)) || w(operand),
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => writes_root(object, name) || w(object) || w(index) || w(value),
        TypedExprKind::FieldAssign { object, value, .. } => {
            writes_root(object, name) || w(object) || w(value)
        }
        TypedExprKind::DerefAssign { target, value } => w(target) || w(value),
        TypedExprKind::Grouping(inner) | TypedExprKind::Deref(inner) => w(inner),
        TypedExprKind::Unary { operand, .. } => w(operand),
        TypedExprKind::Cast { expr, .. } => w(expr),
        TypedExprKind::Member { object, .. } => w(object),
        TypedExprKind::Binary { left, right, .. }
        | TypedExprKind::And { left, right }
        | TypedExprKind::Or { left, right } => w(left) || w(right),
        TypedExprKind::Index { object, index } => w(object) || w(index),
        TypedExprKind::Slice { object, range } => w(object) || w(range),
        TypedExprKind::Range { start, end, .. } => {
            start.as_deref().is_some_and(w) || end.as_deref().is_some_and(w)
        }
        TypedExprKind::FmtString(parts) => parts
            .iter()
            .any(|part| matches!(part, TypedFmtStringPart::Expr(e) if w(e))),
        TypedExprKind::Call { callee, args } => w(callee) || args.iter().any(w),
        TypedExprKind::EnumVariant {
            enum_name,
            variant,
            args,
            ..
        } => {
            let grows_string =
                enum_name == "string" && (variant == "push_str" || variant == "push");
            (grows_string && args.first().is_some_and(|recv| writes_root(recv, name)))
                || args
                    .iter()
                    .any(|a| (enum_name == "Vec" && writes_root(a, name)) || w(a))
        }
        TypedExprKind::ArrayLiteral { elements } | TypedExprKind::VecLiteral { elements, .. } => {
            elements.iter().any(w)
        }
        TypedExprKind::ArraySized { size, fill_value } => {
            w(size) || fill_value.as_deref().is_some_and(w)
        }
        TypedExprKind::StructLiteral { fields, .. } => fields.iter().any(|(_, e)| w(e)),
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => w(condition) || w(then_branch) || w(else_branch),
        TypedExprKind::Match {
            scrutinee, arms, ..
        } => w(scrutinee) || arms.iter().any(|arm| arm.exprs().any(w)),
        TypedExprKind::ResultAssert { scrutinee, .. } => w(scrutinee),
        TypedExprKind::Block { stmts, tail } => {
            stmts.iter().any(|s| stmt_may_write_local(s, name)) || w(tail)
        }
        TypedExprKind::Loop(body) => stmt_may_write_local(body, name),
    }
}

fn stmt_may_write_local(stmt: &TypedStmt, name: &str) -> bool {
    let w = |e: &TypedExpr| may_write_local(e, name);
    let ws = |s: &TypedStmt| stmt_may_write_local(s, name);
    match &stmt.kind {
        TypedStmtKind::Expression(e) => w(e),
        TypedStmtKind::Let { initializer, .. } => w(initializer),
        TypedStmtKind::Block(stmts) => stmts.iter().any(ws),
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => w(condition) || ws(then_branch) || else_branch.as_deref().is_some_and(ws),
        TypedStmtKind::While { condition, body } => w(condition) || ws(body),
        TypedStmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => w(start) || w(end) || step.as_ref().as_ref().is_some_and(w) || ws(body),
        TypedStmtKind::ForEach { iterable, body, .. } => w(iterable) || ws(body),
        TypedStmtKind::Return(value) | TypedStmtKind::Break(value) => value.as_ref().is_some_and(w),
        TypedStmtKind::Continue
        | TypedStmtKind::Function(_)
        | TypedStmtKind::Needs(_)
        | TypedStmtKind::StructDecl { .. }
        | TypedStmtKind::EnumDecl { .. } => false,
    }
}
