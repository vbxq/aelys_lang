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
mod validate;

use crate::constraint::{Constraint, TypeError};
use crate::env::TypeEnv;
use crate::types::{InferType, TypeTable, TypeVarGen};
use aelys_common::Warning;
use std::collections::HashMap;

pub(crate) use stmt::let_stmt::LiteralInit;

const MAX_INFERENCE_DEPTH: usize = 200;

const KNOWN_TYPE_NAMES: &[&str] = &[
    "int", "i8", "i16", "i32", "i64", "int8", "int16", "int32", "int64", "u8", "u16", "u32", "u64",
    "uint8", "uint16", "uint32", "uint64", "float", "f32", "f64", "float32", "float64", "bool",
    "string", "str", "null", "void", "array", "vec",
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
    type_params_in_scope: Vec<String>,
    // tracks variables initialized with numeric literal values (without type annotation).
    //
    // used by `try_narrow_literal` to narrow Identifier expressions whose original value is a known literal
    literal_init_vars: HashMap<String, LiteralInit>,
}
