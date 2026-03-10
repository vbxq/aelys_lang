use super::{KNOWN_TYPE_NAMES, TypeInference};
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::TypedProgram;
use crate::types::{InferType, TypeTable};
use aelys_common::Warning;
use aelys_syntax::{Source, Stmt, StmtKind, TypeAnnotation};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct InferenceResult {
    pub program: TypedProgram,
    pub warnings: Vec<Warning>,
    pub type_table: TypeTable,
}

impl Default for TypeInference {
    fn default() -> Self {
        Self {
            type_gen: crate::types::TypeVarGen::new(),
            constraints: Vec::new(),
            env: crate::env::TypeEnv::new(),
            errors: Vec::new(),
            return_type_stack: Vec::new(),
            depth: 0,
            warnings: Vec::new(),
            type_table: TypeTable::new(),
            type_params_in_scope: Vec::new(),
            literal_init_vars: HashMap::new(),
        }
    }
}

impl TypeInference {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn type_from_annotation(&mut self, ann: &TypeAnnotation) -> InferType {
        self.check_type_annotation(ann);

        // Handle function types specially: use enum-aware type_from_annotation
        // recursively for params and return type, instead of raw from_annotation
        // which doesn't know about enums and would produce Struct("Option")
        // instead of Enum("Option", [...]) for types like fn(i64) -> Option<i64>.
        if ann.is_function_type() {
            let params = ann
                .fn_params
                .as_ref()
                .map(|ps| ps.iter().map(|p| self.type_from_annotation(p)).collect())
                .unwrap_or_default();
            let ret = ann
                .fn_ret
                .as_ref()
                .map(|r| self.type_from_annotation(r))
                .unwrap_or(InferType::Null);
            return InferType::Function {
                params,
                ret: Box::new(ret),
            };
        }

        // Handle array/vec annotations specially: use enum-aware type_from_annotation
        // for the inner type. raw from_annotation uses Self::from_annotation which
        // doesn't know about enums, so [Color; 3] would produce Array(Struct("Color"))
        // instead of Array(Enum("Color", [])).
        let name_lower = ann.name.to_lowercase();
        if name_lower == "array" {
            let inner = ann
                .type_param
                .as_ref()
                .map(|p| self.type_from_annotation(p))
                .unwrap_or(InferType::Dynamic);
            return InferType::Array(Box::new(inner), ann.array_size);
        }
        if name_lower == "vec" {
            let inner = ann
                .type_param
                .as_ref()
                .map(|p| self.type_from_annotation(p))
                .unwrap_or(InferType::Dynamic);
            return InferType::Vec(Box::new(inner));
        }

        let ty = InferType::from_annotation(ann);
        // from_annotation maps all uppercase names to Struct(name); remap to Enum if applicable
        if let InferType::Struct(ref name) = ty {
            if self.type_table.has_enum(name) {
                let type_args = self.collect_enum_type_args(ann);
                return InferType::Enum(name.clone(), type_args);
            }
        }
        ty
    }

    /// Extract resolved type arguments from a type annotation for generic enums.
    /// For `Option<i64>`, returns `[InferType::I64]`.
    /// For `Result<i64, string>`, returns `[InferType::I64, InferType::String]`.
    /// For non-generic `Color`, returns `[]`.
    fn collect_enum_type_args(&mut self, ann: &TypeAnnotation) -> Vec<InferType> {
        // type_params takes precedence (multi-param case like Result<T, E>)
        if !ann.type_params.is_empty() {
            return ann
                .type_params
                .iter()
                .map(|p| self.type_from_annotation(p))
                .collect();
        }
        // single type_param case (Option<T>)
        if let Some(ref param) = ann.type_param {
            return vec![self.type_from_annotation(param)];
        }
        Vec::new()
    }

    fn check_type_annotation(&mut self, ann: &TypeAnnotation) {
        if self.type_params_in_scope.iter().any(|tp| tp == &ann.name) {
            return;
        }

        // function type annotations: fn(T1, T2) -> R
        if ann.is_function_type() {
            if let Some(ref params) = ann.fn_params {
                for p in params {
                    self.check_type_annotation(p);
                }
            }
            if let Some(ref ret) = ann.fn_ret {
                self.check_type_annotation(ret);
            }
            return;
        }

        let name_lower = ann.name.to_lowercase();

        if KNOWN_TYPE_NAMES.contains(&name_lower.as_str()) {
            if let Some(ref param) = ann.type_param {
                self.check_type_annotation(param);
            }
            return;
        }

        if ann.name.chars().next().is_some_and(|c| c.is_uppercase()) {
            if self.type_table.has_struct(&ann.name)
                || self.type_table.has_enum(&ann.name)
                || self.env.contains(&ann.name)
            {
                return;
            }
            self.errors.push(TypeError {
                kind: TypeErrorKind::Mismatch {
                    expected: InferType::Dynamic,
                    found: InferType::Struct(ann.name.clone()),
                },
                span: ann.span,
                reason: ConstraintReason::UnknownType {
                    name: ann.name.clone(),
                },
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
            return;
        }

        self.errors.push(TypeError {
            kind: TypeErrorKind::Mismatch {
                expected: InferType::Dynamic,
                found: InferType::Dynamic,
            },
            span: ann.span,
            reason: ConstraintReason::UnknownType {
                name: ann.name.clone(),
            },
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        });
    }

    pub fn infer_program(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
    ) -> Result<TypedProgram, Vec<TypeError>> {
        let result =
            Self::infer_program_full(stmts, source, Default::default(), Default::default())?;
        Ok(result.program)
    }

    pub fn infer_program_with_imports(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
    ) -> Result<TypedProgram, Vec<TypeError>> {
        let result = Self::infer_program_full(stmts, source, module_aliases, known_globals)?;
        Ok(result.program)
    }

    pub fn infer_program_full(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
    ) -> Result<InferenceResult, Vec<TypeError>> {
        let mut inf = TypeInference::new();

        for alias in &module_aliases {
            inf.env
                .define_function_owned(alias.clone(), InferType::Dynamic);
        }

        for global in &known_globals {
            let ty = match global.as_str() {
                "print" | "println" => InferType::Function {
                    params: vec![InferType::Dynamic],
                    ret: Box::new(InferType::Null),
                },
                _ => InferType::Dynamic,
            };
            inf.env.define_function_owned(global.clone(), ty);
        }

        inf.register_struct_names(&stmts);
        inf.collect_enums(&stmts);
        inf.resolve_struct_fields(&stmts);
        inf.collect_signatures(&stmts, "");

        let typed_stmts = inf.infer_stmts(&stmts);

        let subst = inf.solve_constraints();

        let resolved_stmts = inf.apply_substitution_stmts(&typed_stmts, &subst);

        // collect all declared type parameter names from the program (functions and struct declarations)
        //
        // a name is only treated as a type parameter if it appears in declared_type_params and is not
        // also a real struct in the type table, this prevents false-positive filtering when a struct
        // shares a name with a type parameter from an unrelated generic function.
        let declared_type_params = collect_declared_type_params(&stmts);

        inf.validate_resolved_stmts(&resolved_stmts, &declared_type_params);

        let final_stmts = inf.finalize_stmts(resolved_stmts);

        let is_type_param = |ty: &InferType| -> bool {
            matches!(ty, InferType::Struct(name) if declared_type_params.contains(name.as_str()) && !inf.type_table.has_struct(name))
        };
        let fatal_errors: Vec<_> = inf
            .errors
            .iter()
            .filter(|err| match &err.kind {
                TypeErrorKind::Mismatch { expected, found } => {
                    let generic_annotation_mismatch =
                        matches!(&err.reason, ConstraintReason::TypeAnnotation { .. })
                            && (is_type_param(expected) || is_type_param(found));
                    !generic_annotation_mismatch
                }
                _ => true,
            })
            .cloned()
            .collect();

        if !fatal_errors.is_empty() {
            return Err(fatal_errors);
        }

        let type_table = inf.type_table;

        Ok(InferenceResult {
            program: TypedProgram {
                stmts: final_stmts,
                source,
                type_table: type_table.clone(),
            },
            warnings: inf.warnings,
            type_table,
        })
    }
}

/// Walk the AST and collect every type parameter name declared by functions
/// and struct definitions.
///
/// Used for the positive `is_type_param` test in the fatal error filter.
fn collect_declared_type_params(stmts: &[Stmt]) -> HashSet<String> {
    let mut params = HashSet::new();
    collect_type_params_recursive(stmts, &mut params);
    params
}

fn collect_type_params_recursive(stmts: &[Stmt], params: &mut HashSet<String>) {
    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Function(func) => {
                for tp in &func.type_params {
                    params.insert(tp.clone());
                }
                collect_type_params_recursive(&func.body, params);
            }
            StmtKind::StructDecl { type_params, .. } | StmtKind::EnumDecl { type_params, .. } => {
                for tp in type_params {
                    params.insert(tp.clone());
                }
            }
            StmtKind::Block(inner) => {
                collect_type_params_recursive(inner, params);
            }
            StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_type_params_recursive(std::slice::from_ref(then_branch), params);
                if let Some(eb) = else_branch {
                    collect_type_params_recursive(std::slice::from_ref(eb), params);
                }
            }
            StmtKind::While { body, .. } => {
                collect_type_params_recursive(std::slice::from_ref(body), params);
            }
            StmtKind::For { body, .. } => {
                collect_type_params_recursive(std::slice::from_ref(body), params);
            }
            StmtKind::ForEach { body, .. } => {
                collect_type_params_recursive(std::slice::from_ref(body), params);
            }
            _ => {}
        }
    }
}
