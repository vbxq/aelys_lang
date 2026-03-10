use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::types::{InferType, StructDef, StructField};
use aelys_common::{Warning, WarningKind};
use aelys_syntax::{Stmt, StmtKind};
use std::collections::HashSet;

impl TypeInference {
    /// Register all struct names (without fields) so that forward references
    /// between structs and enums are valid. Must be called before collect_enums
    /// so that enum variant fields can reference struct types.
    pub(super) fn register_struct_names(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let StmtKind::StructDecl {
                name, type_params, ..
            } = &stmt.kind
            {
                if self.type_table.has_struct(name) {
                    self.warnings.push(Warning::new(
                        WarningKind::UnknownType {
                            name: format!("duplicate struct '{}'", name),
                        },
                        stmt.span,
                    ));
                    continue;
                }

                self.type_table.register_struct(StructDef {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    fields: Vec::new(),
                });
            }
        }
    }

    /// Validate field type annotations and populate struct fields.
    /// Must be called after collect_enums so that struct fields can reference
    /// enum types.
    pub(super) fn resolve_struct_fields(&mut self, stmts: &[Stmt]) {
        let mut processed = HashSet::new();
        for stmt in stmts {
            if let StmtKind::StructDecl {
                name,
                type_params,
                fields,
                ..
            } = &stmt.kind
            {
                // skip duplicates (already warned in pass 1)
                if !processed.insert(name.clone()) {
                    continue;
                }

                // check for duplicate field names
                let mut seen_fields = HashSet::new();
                for f in fields {
                    if !seen_fields.insert(&f.name) {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: InferType::Dynamic,
                                found: InferType::Dynamic,
                            },
                            span: f.span,
                            reason: ConstraintReason::Other(format!(
                                "duplicate field '{}' in struct '{}'",
                                f.name, name
                            )),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                    }
                }

                // set type params in scope so generic struct fields like `T` are recognized
                let saved_type_params =
                    std::mem::replace(&mut self.type_params_in_scope, type_params.clone());

                let struct_fields: Vec<StructField> = fields
                    .iter()
                    .map(|f| {
                        let ty = self.type_from_annotation(&f.type_annotation);
                        StructField {
                            name: f.name.clone(),
                            ty,
                        }
                    })
                    .collect();

                self.type_params_in_scope = saved_type_params;

                self.type_table.register_struct(StructDef {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    fields: struct_fields,
                });
            }
        }
    }
}
