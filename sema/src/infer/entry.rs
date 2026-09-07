use super::{KNOWN_TYPE_NAMES, TypeInference};
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::modules::ModuleImports;
use crate::typed_ast::TypedProgram;
use crate::types::{InferType, TypeTable};
use aelys_common::Warning;
use aelys_syntax::{Source, Stmt, StmtKind, TypeAnnotation};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct InferenceResult {
    pub program: TypedProgram,
    pub warnings: Vec<Warning>,
    pub deferred_errors: Vec<TypeError>,
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
            try_counter: 0,
            unsafe_depth: 0,
            catch_match_pending: false,
            nogc_fn_params: HashSet::new(),
            nogc_generic_sigs: HashMap::new(),
            foreign_sigs: HashSet::new(),
            foreign_shadowed_spans: HashSet::new(),
            module_globals: HashSet::new(),
            shadowed_globals: HashSet::new(),
            lambda_depth: 0,
            lambda_captures: HashSet::new(),
            module_imports: crate::modules::ModuleImports::default(),
            import_aliases: HashMap::new(),
            imported_globals: HashSet::new(),
            imported_types: HashSet::new(),
            module_is_importable: false,
        }
    }
}

impl TypeInference {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn type_from_annotation(&mut self, ann: &TypeAnnotation) -> InferType {
        // default position: a `nogc fn(...)` type is out of position here and gets rejected
        self.lower_type(ann, false)
    }

    pub(crate) fn type_from_param_annotation(&mut self, ann: &TypeAnnotation) -> InferType {
        self.lower_type(ann, true)
    }

    // recursion passes false, so a `nogc fn` anywhere but a bare parameter type is rejected
    fn lower_type(&mut self, ann: &TypeAnnotation, nogc_ok: bool) -> InferType {
        if let Some(kind) = ann.reference {
            let mutable = matches!(kind, aelys_syntax::RefKind::Mut);
            if ann.is_slice {
                let elem = ann
                    .type_param
                    .as_ref()
                    .map(|p| self.lower_type(p, false))
                    .unwrap_or(InferType::Dynamic);
                return InferType::Slice {
                    elem: Box::new(elem),
                    mutable,
                };
            }
            let mut base = ann.clone();
            base.reference = None;
            let referent = self.lower_type(&base, false);
            return InferType::Ref {
                referent: Box::new(referent),
                mutable,
            };
        }

        self.check_type_annotation(ann);

        if ann.is_function_type() {
            if ann.nogc && !nogc_ok {
                self.errors.push(TypeError::nogc_out_of_position(ann.span));
            }
            let params = ann
                .fn_params
                .as_ref()
                .map(|ps| ps.iter().map(|p| self.lower_type(p, false)).collect())
                .unwrap_or_default();
            let ret = ann
                .fn_ret
                .as_ref()
                .map(|r| self.lower_type(r, false))
                .unwrap_or(InferType::Null);
            return InferType::Function {
                params,
                ret: Box::new(ret),
                nogc: ann.nogc,
            };
        }

        let name_lower = ann.name.to_lowercase();
        if name_lower == "array" && ann.array_size.is_some() {
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
        if ann.name == "Rc" {
            let inner = ann
                .type_param
                .as_ref()
                .map(|p| self.type_from_annotation(p))
                .unwrap_or(InferType::Dynamic);
            return InferType::Rc(Box::new(inner));
        }

        // map never so into_ok can read it, a plain never annotation is still rejected by check_type_annotation
        if ann.name == "Never" {
            return InferType::Never;
        }

        // resolved once, so the `result` special case and the generic path name the same enum
        let qualified = match self.imported_type_name(&ann.name) {
            Some(q) => Some(q),
            None if ann.name.contains('.') => {
                match self.resolve_module_type_name(&ann.name, ann.span) {
                    Some(q) => Some(q),
                    None => return InferType::Dynamic,
                }
            }
            None => None,
        };

        let result_carrier = match &qualified {
            Some(q) => (crate::modules::source_type_name(q) == "Result").then(|| q.clone()),
            None => (ann.name == "Result" && self.type_table.has_enum("Result"))
                .then(|| ann.name.clone()),
        };
        if let Some(carrier) = result_carrier
            && self.type_table.has_enum(&carrier)
            && ann.type_params.len() == 2
        {
            let t = self.type_from_annotation(&ann.type_params[0]);
            let e = if ann.type_params[1].name == "Never" && !ann.type_params[1].is_function_type()
            {
                InferType::Never
            } else {
                self.type_from_annotation(&ann.type_params[1])
            };
            return InferType::Enum(carrier, vec![t, e]);
        }

        if let Some(qualified) = qualified {
            if self.type_table.has_enum(&qualified) {
                let type_args = self.collect_enum_type_args(ann);
                return InferType::Enum(qualified, type_args);
            }
            return InferType::Struct(qualified);
        }

        let ty = InferType::from_annotation(ann);
        if let InferType::Struct(ref name) = ty {
            if self.type_table.has_enum(name) {
                let type_args = self.collect_enum_type_args(ann);
                return InferType::Enum(name.clone(), type_args);
            }
        }
        ty
    }

    fn collect_enum_type_args(&mut self, ann: &TypeAnnotation) -> Vec<InferType> {
        if !ann.type_params.is_empty() {
            return ann
                .type_params
                .iter()
                .map(|p| self.type_from_annotation(p))
                .collect();
        }
        if let Some(ref param) = ann.type_param {
            return vec![self.type_from_annotation(param)];
        }
        Vec::new()
    }

    fn check_type_annotation(&mut self, ann: &TypeAnnotation) {
        if self.type_params_in_scope.iter().any(|tp| tp == &ann.name) {
            return;
        }

        if ann.name.contains('.') || self.import_aliases.contains_key(&ann.name) {
            return;
        }

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

        if name_lower == "array" && ann.array_size.is_none() {
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
                help: Some("use [T; N] syntax instead of Array<T>".to_string()),
                suggestion: None,
            });
            return;
        }

        if KNOWN_TYPE_NAMES.contains(&name_lower.as_str()) {
            if let Some(ref param) = ann.type_param {
                self.check_type_annotation(param);
            }
            return;
        }

        if ann.name == "Rc" {
            if let Some(ref param) = ann.type_param {
                self.check_type_annotation(param);
            }
            return;
        }

        if ann.name == "Never" {
            self.errors.push(TypeError::member_access(
                "[eh-stage3] `Never` may only appear as the error type of a `Result`".to_string(),
                ann.span,
            ));
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
            Self::infer_program_full(stmts, source, ModuleImports::default(), Default::default())?;
        if !result.deferred_errors.is_empty() {
            return Err(result.deferred_errors);
        }
        Ok(result.program)
    }

    pub fn infer_program_with_imports(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
        imports: ModuleImports,
        known_globals: HashSet<String>,
    ) -> Result<TypedProgram, Vec<TypeError>> {
        let result = Self::infer_program_full(stmts, source, imports, known_globals)?;
        if !result.deferred_errors.is_empty() {
            return Err(result.deferred_errors);
        }
        Ok(result.program)
    }

    pub fn infer_program_full(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
        imports: ModuleImports,
        known_globals: HashSet<String>,
    ) -> Result<InferenceResult, Vec<TypeError>> {
        let mut inf = TypeInference::new();
        inf.module_is_importable = imports.is_importable;
        inf.install_imports(imports);

        for global in &known_globals {
            let ty = match global.as_str() {
                "print" | "println" => InferType::Function {
                    params: vec![InferType::Dynamic],
                    ret: Box::new(InferType::Null),
                    nogc: false,
                },
                "__aelys_collect" => InferType::Function {
                    params: vec![],
                    ret: Box::new(InferType::Null),
                    nogc: false,
                },
                _ => InferType::Dynamic,
            };
            inf.env.define_function_owned(global.clone(), ty);
        }

        inf.reject_reserved_type_names(&stmts);
        inf.register_struct_names(&stmts);
        inf.collect_enums(&stmts);
        inf.resolve_struct_fields(&stmts);
        // a nominal holding a vec by value would leak its buffer, since carrier-vec
        inf.reject_nominal_vec_carriers(&stmts);
        inf.collect_signatures(&stmts, "");

        let typed_stmts = inf.infer_stmts(&stmts);

        let subst = inf.solve_constraints();

        let resolved_stmts = inf.apply_substitution_stmts(&typed_stmts, &subst);

        let declared_type_params = collect_declared_type_params(&stmts);

        inf.validate_resolved_stmts(&resolved_stmts, &declared_type_params);
        inf.check_vec_producing_forms(&resolved_stmts);
        inf.reject_private_types_in_public_api(&resolved_stmts);

        let final_stmts = inf.finalize_stmts(resolved_stmts);

        let is_type_param = |ty: &InferType| -> bool {
            matches!(ty, InferType::Struct(name) if declared_type_params.contains(name.as_str()) && !inf.type_table.has_struct(name))
        };
        let all_errors: Vec<_> = inf
            .errors
            .iter()
            .filter_map(|err| {
                if matches!(&err.kind, TypeErrorKind::NestedFnShadowsOuter { .. }) {
                    return Some(err.clone());
                }
                let fatal = match &err.kind {
                    TypeErrorKind::Mismatch { expected, found } => {
                        let generic_annotation_mismatch =
                            matches!(&err.reason, ConstraintReason::TypeAnnotation { .. })
                                && (is_type_param(expected) || is_type_param(found));
                        !generic_annotation_mismatch
                    }
                    _ => true,
                };
                fatal.then(|| err.clone())
            })
            .collect();

        if all_errors
            .iter()
            .any(|err| !matches!(&err.kind, TypeErrorKind::NestedFnShadowsOuter { .. }))
        {
            return Err(all_errors);
        }
        let deferred_errors = all_errors;

        let type_table = inf.type_table;

        Ok(InferenceResult {
            program: TypedProgram {
                stmts: final_stmts,
                source,
                type_table: type_table.clone(),
            },
            warnings: inf.warnings,
            deferred_errors,
            type_table,
        })
    }
}

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
