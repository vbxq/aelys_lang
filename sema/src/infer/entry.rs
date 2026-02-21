use super::{KNOWN_TYPE_NAMES, TypeInference};
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::TypedProgram;
use crate::types::{InferType, TypeTable};
use aelys_common::Warning;
use aelys_syntax::{Source, Stmt, TypeAnnotation};
use std::collections::HashSet;
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
        }
    }
}

impl TypeInference {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn type_from_annotation(&mut self, ann: &TypeAnnotation) -> InferType {
        self.check_type_annotation(ann);
        InferType::from_annotation(ann)
    }

    fn check_type_annotation(&mut self, ann: &TypeAnnotation) {
        let name_lower = ann.name.to_lowercase();

        if KNOWN_TYPE_NAMES.contains(&name_lower.as_str()) {
            if let Some(ref param) = ann.type_param {
                self.check_type_annotation(param);
            }
            return;
        }

        if ann.name.chars().next().is_some_and(|c| c.is_uppercase()) {
            if self.type_table.has_struct(&ann.name) || self.env.contains(&ann.name) {
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
        });
    }

    pub fn infer_program(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
    ) -> Result<TypedProgram, Vec<crate::constraint::TypeError>> {
        let result =
            Self::infer_program_full(stmts, source, Default::default(), Default::default())?;
        Ok(result.program)
    }

    pub fn infer_program_with_imports(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
    ) -> Result<TypedProgram, Vec<crate::constraint::TypeError>> {
        let result = Self::infer_program_full(stmts, source, module_aliases, known_globals)?;
        Ok(result.program)
    }

    pub fn infer_program_full(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
    ) -> Result<InferenceResult, Vec<crate::constraint::TypeError>> {
        let mut inf = TypeInference::new();

        for alias in &module_aliases {
            inf.env
                .define_function_owned(alias.clone(), InferType::Dynamic);
        }

        for global in &known_globals {
            inf.env
                .define_function_owned(global.clone(), InferType::Dynamic);
        }

        inf.collect_structs(&stmts);
        inf.collect_signatures(&stmts, "");

        let typed_stmts = inf.infer_stmts(&stmts);

        let subst = inf.solve_constraints();

        let resolved_stmts = inf.apply_substitution_stmts(&typed_stmts, &subst);

        let final_stmts = inf.finalize_stmts(resolved_stmts);

        let (fatal_errors, type_warnings): (Vec<_>, Vec<_>) =
            inf.errors.iter().cloned().partition(|err| {
                matches!(
                    err.reason,
                    ConstraintReason::BitwiseOp { .. }
                        | ConstraintReason::TypeAnnotation { .. }
                        | ConstraintReason::InvalidCast
                        | ConstraintReason::UnknownType { .. }
                        | ConstraintReason::IntLiteralOverflow { .. }
                )
            });

        if !fatal_errors.is_empty() {
            return Err(fatal_errors);
        }

        if !type_warnings.is_empty() && std::env::var("AELYS_TYPE_WARNINGS").is_ok() {
            for err in &type_warnings {
                eprintln!("Type warning: {}", err);
            }
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
