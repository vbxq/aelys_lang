use aelys_sema::{InferType, TypedExpr, TypedExprKind, TypedStmt, TypedStmtKind};
use aelys_syntax::BinaryOp;

pub fn may_trap_or_allocate(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Char(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null
        | TypedExprKind::Identifier(_) => false,
        TypedExprKind::Call { .. }
        | TypedExprKind::Assign { .. }
        | TypedExprKind::IndexAssign { .. }
        | TypedExprKind::FieldAssign { .. }
        | TypedExprKind::DerefAssign { .. }
        | TypedExprKind::ResultAssert { .. }
        | TypedExprKind::Index { .. }
        | TypedExprKind::Slice { .. }
        | TypedExprKind::VecLiteral { .. }
        | TypedExprKind::FmtString(_)
        | TypedExprKind::Lambda(_)
        | TypedExprKind::LambdaInner { .. } => true,
        TypedExprKind::Binary { left, op, right } => {
            binary_may_trap_or_allocate(left, *op, right, &expr.ty)
                || may_trap_or_allocate(left)
                || may_trap_or_allocate(right)
        }
        TypedExprKind::Unary { operand, .. }
        | TypedExprKind::Grouping(operand)
        | TypedExprKind::Reference { operand, .. } => may_trap_or_allocate(operand),
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            may_trap_or_allocate(left) || may_trap_or_allocate(right)
        }
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            may_trap_or_allocate(condition)
                || may_trap_or_allocate(then_branch)
                || may_trap_or_allocate(else_branch)
        }
        TypedExprKind::Member { object, .. } => {
            may_be_null(&object.ty) || may_trap_or_allocate(object)
        }
        TypedExprKind::Deref(operand) => may_be_null(&operand.ty) || may_trap_or_allocate(operand),
        TypedExprKind::ArrayLiteral { elements } => elements.iter().any(may_trap_or_allocate),
        TypedExprKind::ArraySized { size, fill_value } => {
            may_trap_or_allocate(size) || fill_value.as_deref().is_some_and(may_trap_or_allocate)
        }
        TypedExprKind::Range { start, end, .. } => {
            start.as_deref().is_some_and(may_trap_or_allocate)
                || end.as_deref().is_some_and(may_trap_or_allocate)
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, value)| may_trap_or_allocate(value))
        }
        TypedExprKind::EnumVariant {
            enum_name,
            variant,
            args,
            ..
        } => {
            builtin_may_trap_or_allocate(enum_name, variant)
                || args.iter().any(may_trap_or_allocate)
        }
        TypedExprKind::Cast { expr: inner, .. } => may_trap_or_allocate(inner),
        TypedExprKind::Match { scrutinee, arms } => {
            may_trap_or_allocate(scrutinee)
                || arms.iter().any(|arm| may_trap_or_allocate(&arm.body))
        }
        TypedExprKind::Block { stmts, tail } => {
            stmts.iter().any(stmt_may_trap_or_allocate) || may_trap_or_allocate(tail)
        }
    }
}

fn stmt_may_trap_or_allocate(stmt: &TypedStmt) -> bool {
    match &stmt.kind {
        TypedStmtKind::Expression(expr) => may_trap_or_allocate(expr),
        TypedStmtKind::Let { initializer, .. } => may_trap_or_allocate(initializer),
        _ => true,
    }
}

fn binary_may_trap_or_allocate(
    left: &TypedExpr,
    op: BinaryOp,
    right: &TypedExpr,
    ty: &InferType,
) -> bool {
    match op {
        BinaryOp::Div | BinaryOp::Mod => {
            if left.ty.is_float() || right.ty.is_float() {
                return false;
            }
            !matches!(right.kind, TypedExprKind::Int(d) if d != 0 && d != -1)
        }
        BinaryOp::Add => {
            matches!(ty, InferType::String)
                || matches!(left.ty, InferType::String)
                || matches!(right.ty, InferType::String)
        }
        _ => false,
    }
}

// these heads are built-in operations spelled as enum variants, not constructors
fn builtin_may_trap_or_allocate(enum_name: &str, variant: &str) -> bool {
    match enum_name {
        "Rc" => !matches!(variant, "null" | "is_null"),
        "Vec" | "string" | "char" => true,
        _ => false,
    }
}

fn may_be_null(ty: &InferType) -> bool {
    !matches!(
        ty,
        InferType::Ref { .. }
            | InferType::Struct(_)
            | InferType::Enum(_, _)
            | InferType::Tuple(_)
            | InferType::Array(_, _)
            | InferType::Vec(_)
            | InferType::Slice { .. }
            | InferType::String
    )
}
