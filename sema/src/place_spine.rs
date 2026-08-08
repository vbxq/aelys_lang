use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;

/// dereferencing this type reaches storage borrowed immutably.
pub fn deref_is_shared(ty: &InferType) -> bool {
    matches!(ty, InferType::Ref { mutable: false, .. })
}

/// a `.` or `[]` whose object already holds a pointer auto-derefs it: the object's value is
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

pub fn denotes_a_place(e: &TypedExpr) -> bool {
    match &e.kind {
        TypedExprKind::Grouping(inner) => denotes_a_place(inner),
        TypedExprKind::Identifier(_) => true,
        TypedExprKind::Deref(_) => true,
        _ if is_rc_get(e) => true,
        TypedExprKind::Member { object, .. } | TypedExprKind::Index { object, .. } => {
            projects_through_pointer(&object.ty) || denotes_a_place(object)
        }
        _ => false,
    }
}

pub fn spine_is_shared(e: &TypedExpr) -> bool {
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

/// `derefassign` is the pointer itself, not the place, so it is one level up.
pub fn target_ptr_is_shared(target: &TypedExpr) -> bool {
    let mut t = target;
    while let TypedExprKind::Grouping(inner) = &t.kind {
        t = inner;
    }
    deref_is_shared(&t.ty)
}
