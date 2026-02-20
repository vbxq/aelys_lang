use aelys_sema::constraint::{Constraint, ConstraintReason};
use aelys_sema::types::{InferType, TypeVarId};
use aelys_syntax::Span;

#[test]
fn test_constraint_creation() {
    let c = Constraint::equal(
        InferType::I64,
        InferType::F64,
        Span::dummy(),
        ConstraintReason::BinaryOp {
            op: "+".to_string(),
        },
    );
    assert!(matches!(c, Constraint::Equal { .. }));
}

#[test]
fn test_one_of_constraint() {
    let c = Constraint::one_of(
        InferType::Var(TypeVarId(0)),
        vec![InferType::I64, InferType::F64, InferType::String],
        Span::dummy(),
        ConstraintReason::BinaryOp {
            op: "+".to_string(),
        },
    );
    assert!(matches!(c, Constraint::OneOf { .. }));
}

#[test]
fn test_type_error_display() {
    let err = aelys_sema::constraint::TypeError::mismatch(
        InferType::I64,
        InferType::F64,
        Span::dummy(),
        ConstraintReason::BinaryOp {
            op: "+".to_string(),
        },
    );
    let msg = format!("{}", err);
    assert!(msg.contains("type mismatch"));
    assert!(msg.contains("i64"));
    assert!(msg.contains("f64"));
}
