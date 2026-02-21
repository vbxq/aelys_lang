pub mod constraint;
pub mod env;
pub mod infer;
pub mod typed_ast;
pub mod types;
pub mod unify;

pub use constraint::{Constraint, ConstraintReason, TypeError};
pub use env::TypeEnv;
pub use infer::{TypeInference, entry::InferenceResult};
pub use typed_ast::{
    TypedExpr, TypedExprKind, TypedFmtStringPart, TypedFunction, TypedParam, TypedProgram,
    TypedStmt, TypedStmtKind,
};
pub use types::{
    InferType, ResolvedType, StructDef, StructField, TypeTable, TypeVarGen, TypeVarId,
};
pub use unify::{Substitution, UnifyError};
