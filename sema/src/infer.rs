mod bounds;
mod captures;
mod constraints;
mod decls;
pub mod entry;
mod enums;
mod expr;
mod finalize;
mod functions;
mod impls;
mod imports;
mod lambda;
pub mod objects;
pub(crate) mod projection;
mod returns;
mod signatures;
mod stmt;
mod structs;
mod substitute;
pub(crate) mod traits;
pub(crate) mod unresolved;
mod usefulness;
mod validate;
mod vec_form;
mod visibility;

use crate::constraint::{Constraint, TypeError};
use crate::env::TypeEnv;
use crate::types::{InferType, TypeTable, TypeVarGen};
use aelys_common::Warning;
use std::collections::{HashMap, HashSet};

pub(crate) use crate::typed_ast::LiteralInit;
pub use impls::{builtin_key_of_prefix, is_builtin_key};

const MAX_INFERENCE_DEPTH: usize = 200;

const KNOWN_TYPE_NAMES: &[&str] = &[
    "int", "i8", "i16", "i32", "i64", "int8", "int16", "int32", "int64", "u8", "u16", "u32", "u64",
    "uint8", "uint16", "uint32", "uint64", "float", "f32", "f64", "float32", "float64", "bool",
    "char", "string", "str", "null", "void", "array", "vec", "usize", "isize",
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
    type_param_bounds: HashMap<String, aelys_syntax::TypeBounds>,
    instantiated_param_names: HashMap<crate::types::TypeVarId, String>,
    literal_init_vars: HashMap<crate::env::BindingId, LiteralInit>,
    struct_literal_hint: Option<InferType>,
    dyn_expect: Option<InferType>,
    pub(crate) literal_globals: HashMap<String, LiteralInit>,
    keep_binding_use: bool,
    try_counter: usize,
    loop_breaks: Vec<Option<Vec<InferType>>>,
    unsafe_depth: usize,
    refused_matches: std::collections::HashSet<(usize, usize)>,
    nogc_fn_params: HashSet<String>,
    bound_generic_sigs: HashMap<String, Vec<bounds::BoundGenericSig>>,
    foreign_sigs: HashSet<String>,
    foreign_shadowed_spans: HashSet<(usize, usize)>,
    unqualified_variant_spans: HashSet<(usize, usize)>,
    generic_fn_sigs: HashSet<String>,
    generic_value_spans: HashSet<(usize, usize, String)>,
    pub(crate) module_globals: HashSet<String>,
    pub(crate) shadowed_globals: HashSet<String>,
    pub(crate) lambda_depth: usize,
    pub(crate) in_nogc_fn: bool,
    pub(crate) lambda_captures: HashSet<String>,
    pub(crate) module_imports: crate::modules::ModuleImports,
    pub(crate) import_aliases: HashMap<String, String>,
    pub(crate) imported_globals: HashSet<String>,
    pub(crate) imported_types: HashSet<String>,
    pub(crate) module_is_importable: bool,
    deferred: Vec<projection::DeferredProjection>,
    deferred_sites: HashSet<projection::SiteKey>,
    reported_sites: HashSet<projection::SiteKey>,
    deferred_results: HashSet<crate::types::TypeVarId>,
    deferred_carriers: Vec<projection::CarrierCheck>,
    discarded_values: Vec<(aelys_syntax::Span, crate::types::InferType)>,
    deferred_mutability: Vec<(InferType, TypeError)>,
    consts: HashMap<String, crate::typed_ast::ConstLit>,
    alias_names: HashSet<String>,
    failed_consts: HashSet<String>,
    alias_checks: Vec<(Vec<String>, aelys_syntax::TypeAnnotation)>,
    module_decls: crate::typed_ast::ModuleDecls,
    pub(crate) methods: HashMap<String, Vec<crate::modules::MethodEntry>>,
    pub(crate) trait_methods: HashMap<String, Vec<crate::modules::MethodEntry>>,
    pub(crate) builtin_methods: HashMap<String, Vec<crate::modules::MethodEntry>>,
    recv_counter: usize,
    pretyped: HashMap<String, crate::typed_ast::TypedExpr>,
    method_tparams: HashMap<String, Vec<String>>,
    probe_subst: crate::unify::Substitution,
    probe_upto: usize,
    pub(crate) trait_names: HashMap<String, String>,
    pub(crate) trait_defs: HashMap<String, crate::modules::TraitDef>,
    pub(crate) impl_defs: Vec<crate::modules::ImplDef>,
    pub(crate) local_impls: Vec<usize>,
    pub(crate) local_traits: Vec<String>,
    pub(crate) used_builtin_stubs: HashSet<String>,
    pub(crate) pending_trait_args: Option<Vec<InferType>>,
    declarations_only: bool,
    pub(crate) early_trait_names: HashMap<String, String>,
    pub(crate) unchecked_dyns: Vec<(String, aelys_syntax::Span)>,
}
