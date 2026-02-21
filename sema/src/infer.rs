mod captures;
mod constraints;
pub mod entry;
mod expr;
mod finalize;
mod functions;
mod lambda;
mod returns;
mod signatures;
mod stmt;
mod structs;
mod substitute;

use crate::constraint::{Constraint, TypeError};
use crate::env::TypeEnv;
use crate::types::{InferType, TypeTable, TypeVarGen};
use aelys_common::Warning;

const MAX_INFERENCE_DEPTH: usize = 200;

const KNOWN_TYPE_NAMES: &[&str] = &[
    "int", "i8", "i16", "i32", "i64", "int8", "int16", "int32", "int64", "u8", "u16", "u32",
    "u64", "uint8", "uint16", "uint32", "uint64", "float", "f32", "f64", "float32", "float64",
    "bool", "string", "null", "void", "array", "vec",
];

pub struct TypeInference {
    type_gen: TypeVarGen,
    constraints: Vec<Constraint>,
    env: TypeEnv,
    errors: Vec<TypeError>,
    return_type_stack: Vec<InferType>,
    depth: usize,
    warnings: Vec<Warning>,
    pub(crate) type_table: TypeTable,
}
