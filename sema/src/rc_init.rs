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
}

pub const RUNTIME_CARRIER_EXEMPT: &[&str] = &["Rc", "Vec", "string", "char"];

pub fn rc_init_provenance(expr: &TypedExpr) -> RcInit {
    match &expr.kind {
        TypedExprKind::Grouping(inner) => rc_init_provenance(inner),
        TypedExprKind::EnumVariant {
            enum_name, variant, ..
        } => {
            let fresh = if enum_name == "Rc" {
                variant == "new" || variant == "null"
            } else {
                !RUNTIME_CARRIER_EXEMPT.contains(&enum_name.as_str())
            };
            if fresh {
                RcInit::Fresh
            } else {
                RcInit::Borrowed
            }
        }
        TypedExprKind::StructLiteral { .. }
        | TypedExprKind::Call { .. }
        | TypedExprKind::Block { .. }
        | TypedExprKind::If { .. }
        | TypedExprKind::Match { .. } => RcInit::Fresh,
        _ => RcInit::Borrowed,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Literal,
    Reference,
    Blocked(RcInitBlocker),
}

fn carrier_surface(expr: &TypedExpr) -> Surface {
    match &expr.kind {
        TypedExprKind::Grouping(inner) => carrier_surface(inner),
        TypedExprKind::StructLiteral { .. } | TypedExprKind::EnumVariant { .. } => Surface::Literal,
        TypedExprKind::Identifier(_) | TypedExprKind::Member { .. } => Surface::Reference,
        TypedExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            let taken = carrier_surface(then_branch);
            if taken == carrier_surface(else_branch) {
                taken
            } else {
                Surface::Blocked(RcInitBlocker::Compound)
            }
        }
        TypedExprKind::Call { .. } => Surface::Blocked(RcInitBlocker::Call),
        _ => Surface::Blocked(RcInitBlocker::Compound),
    }
}

pub fn rc_carrier_init_blocker(expr: &TypedExpr) -> Option<RcInitBlocker> {
    match carrier_surface(expr) {
        Surface::Blocked(blocker) => Some(blocker),
        Surface::Literal | Surface::Reference => None,
    }
}

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
