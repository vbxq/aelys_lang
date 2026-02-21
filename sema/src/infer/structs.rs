use super::TypeInference;
use crate::types::{InferType, StructDef, StructField};
use aelys_common::{Warning, WarningKind};
use aelys_syntax::{Stmt, StmtKind};

impl TypeInference {
    pub(super) fn collect_structs(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let StmtKind::StructDecl { name, type_params, fields, .. } = &stmt.kind {
                if self.type_table.has_struct(name) {
                    self.warnings.push(Warning::new(
                        WarningKind::UnknownType {
                            name: format!("duplicate struct '{}'", name),
                        },
                        stmt.span,
                    ));
                    continue;
                }

                for type_param in type_params {
                    let fresh_var = self.type_gen.fresh();
                    self.env.define_local(type_param.clone(), fresh_var);
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
                    type_params: type_params.clone(),
                    fields: struct_fields,
                });
            }
        }
    }
}
