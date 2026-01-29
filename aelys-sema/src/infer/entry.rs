use super::TypeInference;
use crate::constraint::ConstraintReason;
use crate::typed_ast::TypedProgram;
use crate::types::InferType;
use aelys_syntax::Source;
use aelys_syntax::Stmt;
use std::collections::HashSet;
use std::sync::Arc;

impl TypeInference {
    /// Create a new inference engine
    pub fn new() -> Self {
        Self {
            type_gen: crate::types::TypeVarGen::new(),
            constraints: Vec::new(),
            env: crate::env::TypeEnv::new(),
            errors: Vec::new(),
            return_type_stack: Vec::new(),
            depth: 0,
        }
    }

    /// Main entry point: infer types for a program
    pub fn infer_program(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
    ) -> Result<TypedProgram, Vec<crate::constraint::TypeError>> {
        Self::infer_program_with_imports(stmts, source, Default::default(), Default::default())
    }

    /// Infer types for a program with module imports pre-registered
    pub fn infer_program_with_imports(
        stmts: Vec<Stmt>,
        source: Arc<Source>,
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
    ) -> Result<TypedProgram, Vec<crate::constraint::TypeError>> {
        let mut inf = TypeInference::new();

        for alias in &module_aliases {
            inf.env.define_local(alias.clone(), InferType::Dynamic);
        }

        for global in &known_globals {
            inf.env.define_local(global.clone(), InferType::Dynamic);
        }

        inf.collect_signatures(&stmts, "");

        let typed_stmts = inf.infer_stmts(&stmts);

        let subst = inf.solve_constraints();

        let resolved_stmts = inf.apply_substitution_stmts(&typed_stmts, &subst);

        let final_stmts = inf.finalize_stmts(resolved_stmts);

        let (fatal_errors, warnings): (Vec<_>, Vec<_>) = inf
            .errors
            .iter()
            .cloned()
            .partition(|err| matches!(
                err.reason,
                ConstraintReason::BitwiseOp { .. } | ConstraintReason::TypeAnnotation { .. }
            ));

        if !fatal_errors.is_empty() {
            return Err(fatal_errors);
        }

        if !warnings.is_empty() && std::env::var("AELYS_TYPE_WARNINGS").is_ok() {
            for err in &warnings {
                eprintln!("Type warning: {}", err);
            }
        }

        Ok(TypedProgram {
            stmts: final_stmts,
            source,
        })
    }
}
