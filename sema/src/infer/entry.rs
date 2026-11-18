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
            type_param_bounds: HashMap::new(),
            instantiated_param_names: HashMap::new(),
            literal_init_vars: HashMap::new(),
            struct_literal_hint: None,
            literal_globals: HashMap::new(),
            keep_binding_use: false,
            try_counter: 0,
            unsafe_depth: 0,
            refused_matches: std::collections::HashSet::new(),
            nogc_fn_params: HashSet::new(),
            bound_generic_sigs: HashMap::new(),
            foreign_sigs: HashSet::new(),
            foreign_shadowed_spans: HashSet::new(),
            unqualified_variant_spans: HashSet::new(),
            generic_fn_sigs: HashSet::new(),
            generic_value_spans: HashSet::new(),
            module_globals: HashSet::new(),
            shadowed_globals: HashSet::new(),
            lambda_depth: 0,
            in_nogc_fn: false,
            lambda_captures: HashSet::new(),
            module_imports: crate::modules::ModuleImports::default(),
            import_aliases: HashMap::new(),
            imported_globals: HashSet::new(),
            imported_types: HashSet::new(),
            module_is_importable: false,
            deferred: Vec::new(),
            deferred_sites: HashSet::new(),
            reported_sites: HashSet::new(),
            deferred_results: HashSet::new(),
            deferred_carriers: Vec::new(),
            deferred_mutability: Vec::new(),
            consts: HashMap::new(),
            alias_names: HashSet::new(),
            failed_consts: HashSet::new(),
            alias_checks: Vec::new(),
            module_decls: Default::default(),
            methods: HashMap::new(),
            trait_methods: HashMap::new(),
            recv_counter: 0,
            pretyped: HashMap::new(),
            method_tparams: HashMap::new(),
            probe_subst: crate::unify::Substitution::new(),
            probe_upto: 0,
            trait_names: HashMap::new(),
            trait_defs: HashMap::new(),
            impl_defs: Vec::new(),
            local_impls: Vec::new(),
            local_traits: Vec::new(),
            used_builtin_stubs: HashSet::new(),
        }
    }
}

impl TypeInference {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn type_from_annotation(&mut self, ann: &TypeAnnotation) -> InferType {
        // default position: a `nogc fn(...)` type is out of position here and gets rejected
        self.lower_known_type(ann, false)
    }

    pub(crate) fn type_from_param_annotation(&mut self, ann: &TypeAnnotation) -> InferType {
        self.lower_known_type(ann, true)
    }

    fn lower_known_type(&mut self, ann: &TypeAnnotation, nogc_ok: bool) -> InferType {
        let before = self.errors.len();
        let ty = self.lower_type(ann, nogc_ok);
        if self.errors[before..].iter().any(|e| !e.is_type_arg_count()) {
            InferType::Dynamic
        } else {
            ty
        }
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

        if let Some(elems) = &ann.tuple_elems {
            let ty = InferType::Tuple(elems.iter().map(|e| self.lower_type(e, false)).collect());
            self.reject_tuple_carriers(&ty, ann.span);
            return ty;
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
            self.check_type_arg_count("type", &ann.name, 1, ann);
            let inner = ann
                .type_param
                .as_ref()
                .map(|p| self.type_from_annotation(p))
                .unwrap_or(InferType::Dynamic);
            return InferType::Vec(Box::new(inner));
        }
        if ann.name == "Rc" {
            self.check_type_arg_count("type", &ann.name, 1, ann);
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
            None if ann.name.starts_with(crate::modules::TYPE_HEAD)
                && (self.type_table.has_struct(&ann.name) || self.type_table.has_enum(&ann.name)) =>
            {
                Some(ann.name.clone())
            }
            None if ann.name.contains('.') => {
                let before = self.errors.len();
                match self.resolve_module_type_name(&ann.name, ann.span) {
                    Some(q) => Some(q),
                    None => {
                        if self.errors.len() == before {
                            self.errors.push(TypeError {
                                kind: TypeErrorKind::Mismatch {
                                    expected: InferType::Dynamic,
                                    found: InferType::Struct(ann.name.clone(), Vec::new()),
                                },
                                span: ann.span,
                                reason: ConstraintReason::UnknownType {
                                    name: ann.to_string(),
                                },
                                secondary_spans: Vec::new(),
                                help: Some(format!(
                                    "a type of another module is written `<namespace>.{}`, \
                                     with the namespace bound by a `needs`",
                                    crate::modules::source_type_name(&ann.name)
                                )),
                                suggestion: None,
                            });
                        }
                        return InferType::Dynamic;
                    }
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
                let type_args = self.collect_type_args(ann);
                self.check_enum_type_arg_count(&qualified, type_args.len(), ann);
                let type_args = self.fit_enum_type_args(&qualified, type_args);
                return InferType::Enum(qualified, type_args);
            }
            if self.type_table.has_struct(&qualified) {
                return self.struct_annotation(&qualified, ann);
            }
            return InferType::Struct(qualified, Vec::new());
        }

        let ty = InferType::from_annotation(ann);
        if let InferType::Struct(ref name, _) = ty {
            if self.type_table.has_enum(name) {
                let type_args = self.collect_type_args(ann);
                self.check_enum_type_arg_count(name, type_args.len(), ann);
                let type_args = self.fit_enum_type_args(name, type_args);
                return InferType::Enum(name.clone(), type_args);
            }
            if self.type_table.has_struct(name) {
                return self.struct_annotation(&name.clone(), ann);
            }
        }
        ty
    }

    fn fit_enum_type_args(&self, name: &str, mut args: Vec<InferType>) -> Vec<InferType> {
        if let Some(expected) = self.type_table.get_enum(name).map(|d| d.type_params.len()) {
            args.resize(expected, InferType::Dynamic);
        }
        args
    }

    fn struct_annotation(&mut self, name: &str, ann: &TypeAnnotation) -> InferType {
        let found = Self::annotated_type_arg_count(ann);
        if found == 0 && self.type_params_in_scope.iter().any(|p| p == name) {
            return InferType::Struct(name.to_string(), Vec::new());
        }
        let expected = self
            .type_table
            .get_struct(name)
            .map_or(0, |d| d.type_params.len());
        self.check_struct_type_arg_count(name, ann);
        if expected == 0 {
            return InferType::Struct(name.to_string(), Vec::new());
        }
        if expected != found {
            return InferType::Dynamic;
        }
        let args = self.collect_type_args(ann);
        InferType::Struct(name.to_string(), args)
    }

    fn annotated_type_arg_count(ann: &TypeAnnotation) -> usize {
        if !ann.type_params.is_empty() {
            ann.type_params.len()
        } else {
            usize::from(ann.type_param.is_some())
        }
    }

    fn check_type_arg_count(
        &mut self,
        what: &str,
        name: &str,
        expected: usize,
        ann: &TypeAnnotation,
    ) {
        let found = Self::annotated_type_arg_count(ann);
        if expected != found {
            self.errors.push(TypeError::type_arg_count(
                what,
                crate::modules::source_type_name(name),
                expected,
                found,
                ann.span,
            ));
        }
    }

    fn check_struct_type_arg_count(&mut self, name: &str, ann: &TypeAnnotation) {
        let expected = self
            .type_table
            .get_struct(name)
            .map_or(0, |d| d.type_params.len());
        self.check_type_arg_count("struct", name, expected, ann);
    }

    fn check_enum_type_arg_count(&mut self, name: &str, found: usize, ann: &TypeAnnotation) {
        let Some(expected) = self.type_table.get_enum(name).map(|d| d.type_params.len()) else {
            return;
        };
        if expected != found {
            self.errors.push(TypeError::type_arg_count(
                "enum",
                crate::modules::source_type_name(name),
                expected,
                found,
                ann.span,
            ));
        }
    }

    fn collect_type_args(&mut self, ann: &TypeAnnotation) -> Vec<InferType> {
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

    pub(crate) fn check_type_annotation(&mut self, ann: &TypeAnnotation) {
        if self.type_params_in_scope.iter().any(|tp| tp == &ann.name) {
            return;
        }

        if ann.name.contains('.') || self.import_aliases.contains_key(&ann.name) {
            return;
        }

        if let Some(elems) = &ann.tuple_elems {
            for e in elems {
                self.check_type_annotation(e);
            }
            return;
        }

        if ann.is_slice {
            if let Some(ref elem) = ann.type_param {
                self.check_type_annotation(elem);
            }
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
                    found: InferType::Struct(ann.name.clone(), Vec::new()),
                },
                span: ann.span,
                reason: ConstraintReason::UnknownType {
                    name: ann.to_string(),
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
                "[result] `Never` may only appear as the error type of a `Result`".to_string(),
                ann.span,
            ));
            return;
        }

        if ann.name == "Self" {
            self.errors.push(TypeError::self_outside_impl(ann.span));
            return;
        }

        if self.is_trait_name(&ann.name) {
            self.errors.push(TypeError::coded(
                "E0750",
                format!(
                    "the trait `{}` is used as a type, and trait objects are not supported; take \
                     a type parameter `<T: {}>` instead",
                    ann.name, ann.name
                ),
                "trait used as a type",
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
                    found: InferType::Struct(ann.name.clone(), Vec::new()),
                },
                span: ann.span,
                reason: ConstraintReason::UnknownType {
                    name: ann.to_string(),
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
                name: ann.to_string(),
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
        mut stmts: Vec<Stmt>,
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

        inf.prepare_decls(&mut stmts);
        inf.reject_reserved_type_names(&stmts);
        inf.register_struct_names(&stmts);
        inf.collect_enums(&stmts);
        inf.resolve_struct_fields(&stmts);
        inf.check_alias_bodies();
        let stmts = inf.collect_traits(stmts);
        let stmts = inf.collect_impls(stmts);
        // a nominal holding a vec by value would leak its buffer, since carrier-vec
        inf.reject_nominal_vec_carriers(&stmts);
        inf.reject_multi_variant_rc_enums(&stmts);
        inf.reject_polymorphic_recursion(&stmts);
        inf.collect_signatures(&stmts, "");
        inf.fill_trait_sigs();
        inf.check_ord_has_eq();

        let mut typed_stmts = inf.infer_stmts(&stmts);
        typed_stmts.extend(inf.infer_used_builtin_stubs());

        let subst = inf.solve_constraints();
        inf.replay_carrier_checks(&subst);
        inf.replay_mutability_checks(&subst);

        let resolved_stmts = inf.apply_substitution_stmts(&typed_stmts, &subst);

        let declared_type_params = collect_declared_type_params(&stmts);

        inf.validate_resolved_stmts(&resolved_stmts, &declared_type_params);
        inf.check_vec_producing_forms(&resolved_stmts);
        inf.reject_private_types_in_public_api(&resolved_stmts);
        inf.drop_generic_value_cascades();

        let is_type_param = |inf: &TypeInference, ty: &InferType| -> bool {
            matches!(ty, InferType::Struct(name, args) if args.is_empty() && declared_type_params.contains(name.as_str()) && !inf.type_table.has_struct(name))
        };
        let defers = |kind: &TypeErrorKind| {
            matches!(
                kind,
                TypeErrorKind::NestedFnShadowsOuter { .. }
                    | TypeErrorKind::ConstIndexOutOfBounds { .. }
            )
        };
        let reported = |inf: &TypeInference, err: &TypeError| match &err.kind {
            TypeErrorKind::Mismatch { expected, found } => {
                !(matches!(
                    &err.reason,
                    ConstraintReason::TypeAnnotation { .. } | ConstraintReason::StructField { .. }
                ) && (is_type_param(inf, expected) || is_type_param(inf, found)))
            }
            _ => true,
        };

        if !inf.errors.iter().any(|err| reported(&inf, err)) {
            inf.reject_unresolved_types(&resolved_stmts);
        }

        let final_stmts = inf.finalize_stmts(resolved_stmts);

        let all_errors: Vec<_> = inf
            .errors
            .iter()
            .filter(|err| defers(&err.kind) || reported(&inf, err))
            .cloned()
            .collect();

        if all_errors.iter().any(|err| !defers(&err.kind)) {
            return Err(all_errors);
        }
        let deferred_errors = all_errors;

        let type_table = inf.type_table;

        Ok(InferenceResult {
            program: TypedProgram {
                stmts: final_stmts,
                source,
                type_table: type_table.clone(),
                literal_globals: inf.literal_globals,
                decls: inf.module_decls,
                traits: inf
                    .local_traits
                    .iter()
                    .filter_map(|q| inf.trait_defs.get(q).cloned())
                    .collect(),
                impls: inf
                    .local_impls
                    .iter()
                    .map(|&i| inf.impl_defs[i].clone())
                    .collect(),
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
