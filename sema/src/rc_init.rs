use crate::typed_ast::{TypedExpr, TypedExprKind};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RcInitBlocker {
    Call,
    Compound,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RcInit {
    Fresh,
    Borrowed,
    Unaccountable(RcInitBlocker),
}

pub const RUNTIME_CARRIER_EXEMPT: &[&str] = &["Rc", "Vec", "string", "char"];

pub fn rc_init_provenance(expr: &TypedExpr) -> RcInit {
    match &expr.kind {
        TypedExprKind::Grouping(inner) => rc_init_provenance(inner),
        TypedExprKind::StructLiteral { .. } | TypedExprKind::EnumVariant { .. } => RcInit::Fresh,
        TypedExprKind::Identifier(_) | TypedExprKind::Member { .. } => RcInit::Borrowed,
        TypedExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            let taken = rc_init_provenance(then_branch);
            if taken == rc_init_provenance(else_branch) {
                taken
            } else {
                RcInit::Unaccountable(RcInitBlocker::Compound)
            }
        }
        TypedExprKind::Call { .. } => RcInit::Unaccountable(RcInitBlocker::Call),
        _ => RcInit::Unaccountable(RcInitBlocker::Compound),
    }
}

// sema refuses and lowering accounts, so both have to read the same words
pub fn rc_init_refusal(blocker: RcInitBlocker, what: &str) -> String {
    match blocker {
        RcInitBlocker::Call => format!(
            "[rc-stage3a] {what} is initialized from a call returning an `Rc<T>`-bearing value; \
             nothing in the signature says whether the callee transfers its count or lends it, so \
             ownership transfer into a carrier field is not supported yet"
        ),
        RcInitBlocker::Compound => format!(
            "[rc-stage3a] {what} is initialized from a conditional/compound expression producing \
             an `Rc<T>`-bearing value; only a direct reference (clone), a fresh literal, or a \
             conditional whose branches are both one or both the other is supported yet"
        ),
    }
}
