use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::types::InferType;
use aelys_syntax::{Function, Stmt, StmtKind};
use std::collections::HashSet;
use std::rc::Rc;

impl TypeInference {
    /// Collect function signatures before inference (pre-pass)
    ///
    /// only registers functions at the current scope level.
    ///
    /// Does not recurse into nested blocks/if/while/for/for-each bodies, because
    /// functions defined inside those constructs belong to inner lexical scopes.
    pub(super) fn collect_signatures(&mut self, stmts: &[Stmt], prefix: &str) {
        for stmt in stmts {
            match &stmt.kind {
                StmtKind::Function(func) => {
                    self.collect_function_signature(func, prefix);
                }
                // nested scopes are collected when those scopes are inferred
                _ => {}
            }
        }
    }

    /// Collect a single function's signature
    fn collect_function_signature(&mut self, func: &Function, prefix: &str) {
        let full_name = if prefix.is_empty() {
            func.name.clone()
        } else {
            format!("{}::{}", prefix, func.name)
        };

        // check for duplicate function definitions at the same scope level
        if self.env.has_function(&full_name) {
            self.errors.push(TypeError {
                kind: TypeErrorKind::Mismatch {
                    expected: InferType::Dynamic,
                    found: InferType::Dynamic,
                },
                span: func.span,
                reason: ConstraintReason::Other(format!(
                    "duplicate function definition '{}'",
                    func.name
                )),
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
        }

        // check for duplicate parameter names
        {
            let mut seen_params = HashSet::new();
            for p in &func.params {
                if !seen_params.insert(&p.name) {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: InferType::Dynamic,
                            found: InferType::Dynamic,
                        },
                        span: p.span,
                        reason: ConstraintReason::Other(format!(
                            "duplicate parameter '{}' in function '{}'",
                            p.name, func.name
                        )),
                        secondary_spans: Vec::new(),
                        help: None,
                        suggestion: None,
                    });
                }
            }
        }

        let saved_type_params =
            std::mem::replace(&mut self.type_params_in_scope, func.type_params.clone());

        let mut param_types = Vec::with_capacity(func.params.len());
        for p in &func.params {
            let ty = match &p.type_annotation {
                Some(ann) => self.type_from_annotation(ann),
                None => self.type_gen.fresh(),
            };
            param_types.push(ty);
        }

        let ret_type = match &func.return_type {
            Some(ann) => self.type_from_annotation(ann),
            None => self.type_gen.fresh(),
        };

        self.type_params_in_scope = saved_type_params;

        let fn_type = Rc::new(InferType::Function {
            params: param_types,
            ret: Box::new(ret_type),
        });

        self.env.define_function(full_name.clone(), fn_type.clone());

        // Also register with unqualified name so nested functions are
        // reachable by local lookup (e.g. `inner(41)` inside `outer`).
        if !prefix.is_empty() {
            self.env.define_function(func.name.clone(), fn_type);
        }
    }
}
