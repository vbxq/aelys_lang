mod infer_type;
mod nogc_value;
mod resolved_type;
mod type_table;
mod type_var;

pub use infer_type::InferType;

pub const TUPLE_LITERAL: &str = "$tuple";
pub use resolved_type::ResolvedType;
pub use type_table::{EnumDef, EnumVariant, RcNominalScan, StructDef, StructField, TypeTable};
pub use type_var::{TypeVarGen, TypeVarId};
