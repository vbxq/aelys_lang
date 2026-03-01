use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::types::{InferType, StructDef, StructField};
use aelys_common::{Warning, WarningKind};
use aelys_syntax::{Stmt, StmtKind};
use std::collections::HashSet;

impl TypeInference {
    pub(super) fn collect_structs(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let StmtKind::StructDecl {
                name,
                type_params,
                fields,
                ..
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
                        });
                    }
                }

                // important note: type params for generic structs are handled via InferType::from_annotation which maps uppercase names to
                // Struct("T"), so we *do not* define them in the env here, it would pollute the global scope and leak between structs
                let struct_fields: Vec<StructField> = fields
                    .iter()
                    .map(|f| {
                        let ty = InferType::from_annotation(&f.type_annotation);
                        StructField {
                            name: f.name.clone(),
                            ty,
                        }
                    })
                    .collect();

                self.type_table.register_struct(StructDef {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    fields: struct_fields,
                });
            }
        }
    }
}
