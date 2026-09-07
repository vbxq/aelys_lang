use crate::typed_ast::{TypedExpr, TypedExprKind};
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
            InferType::String | InferType::Slice { .. } | InferType::Vec(_) | InferType::Array(..)
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

pub fn denotes_a_place(e: &TypedExpr) -> bool {
    match &e.kind {
        TypedExprKind::Grouping(inner) => denotes_a_place(inner),
        TypedExprKind::Identifier(_) => true,
        TypedExprKind::Deref(_) => true,
        _ if is_rc_get(e) => true,
        TypedExprKind::Member { object, member } => {
            !is_computed_len(&object.ty, member)
                && (projects_through_pointer(&object.ty) || denotes_a_place(object))
        }
        TypedExprKind::Index { object, .. } => {
            projects_through_pointer(&object.ty) || denotes_a_place(object)
        }
        _ => false,
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

