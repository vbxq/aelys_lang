mod captures;
mod constraints;
pub mod entry;
mod enums;
mod expr;
mod finalize;
mod functions;
mod imports;
mod lambda;
mod nogc_bound;
mod returns;
mod signatures;
mod stmt;
mod structs;
mod substitute;
mod validate;
mod vec_form;
mod visibility;

use crate::constraint::{Constraint, TypeError};
use crate::env::TypeEnv;
use crate::types::{InferType, TypeTable, TypeVarGen};
use aelys_common::Warning;
use std::collections::{HashMap, HashSet};

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
    literal_init_vars: HashMap<String, LiteralInit>,
    try_counter: usize,
    unsafe_depth: usize,
    catch_match_pending: bool,
    nogc_fn_params: HashSet<String>,
    nogc_generic_sigs: HashMap<String, Vec<nogc_bound::NogcGenericSig>>,
    pub(crate) module_globals: HashSet<String>,
    pub(crate) shadowed_globals: HashSet<String>,
    pub(crate) lambda_depth: usize,
    pub(crate) lambda_captures: HashSet<String>,
    pub(crate) module_imports: crate::modules::ModuleImports,
    pub(crate) import_aliases: HashMap<String, String>,
    pub(crate) imported_globals: HashSet<String>,
    pub(crate) imported_types: HashSet<String>,
    pub(crate) module_is_importable: bool,
}
