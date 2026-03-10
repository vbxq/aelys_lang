use super::TypeInference;
use crate::types::{EnumDef, EnumVariant};
use aelys_common::{Warning, WarningKind};
use aelys_syntax::{Stmt, StmtKind};

impl TypeInference {
    pub(super) fn collect_enums(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let StmtKind::EnumDecl {
                name,
                type_params,
                variants,
                ..
            } = &stmt.kind
            {
                if self.type_table.has_enum(name) {
                    self.warnings.push(Warning::new(
                        WarningKind::UnknownType {
                            name: format!("duplicate enum '{}'", name),
                        },
                        stmt.span,
                    ));
                    continue;
                }

                let enum_variants: Vec<EnumVariant> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let data = v
                            .fields
                            .iter()
                            .map(|ann| self.type_from_annotation(ann))
                            .collect();
                        EnumVariant {
                            name: v.name.clone(),
                            tag: i as u32,
                            data,
                        }
                    })
                    .collect();

                self.type_table.register_enum(EnumDef {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    variants: enum_variants,
                });
            }
        }
    }
}
