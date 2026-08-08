pub mod constraint;
pub mod env;
pub mod infer;
pub mod place_spine;
pub mod typed_ast;
pub mod types;
pub mod unify;

pub use constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind, TypeErrorSuggestion};
pub use env::TypeEnv;
pub use infer::{TypeInference, entry::InferenceResult};
pub use place_spine::{denotes_a_place, deref_is_shared, spine_is_shared, target_ptr_is_shared};
pub use typed_ast::{
    ResultAssertOnErr, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedFunction, TypedMatchArm,
    TypedParam, TypedPattern, TypedProgram, TypedStmt, TypedStmtKind,
};
pub use types::{
    InferType, ResolvedType, StructDef, StructField, TypeTable, TypeVarGen, TypeVarId,
};
pub use unify::{Substitution, UnifyError};
