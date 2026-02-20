use super::TypeInference;
use crate::types::{InferType, StructDef, StructField};
use aelys_common::{Warning, WarningKind};
use aelys_syntax::{Stmt, StmtKind};

impl TypeInference {
    pub(super) fn collect_structs(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let StmtKind::StructDecl { name, fields, .. } = &stmt.kind {
                if self.type_table.has_struct(name) {
                    self.warnings.push(Warning::new(
                        WarningKind::UnknownType {
                            name: format!("duplicate struct '{}'", name),
                        },
                        stmt.span,
                    ));
                    continue;
                }

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
                    fields: struct_fields,
                });
            }
        }
    }
}
