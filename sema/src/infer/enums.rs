use super::TypeInference;
use crate::types::{EnumDef, EnumVariant};
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
                    continue;
                }

                // set type params in scope so generic enum fields like `t` are recognized
                let saved_type_params =
                    std::mem::replace(&mut self.type_params_in_scope, type_params.clone());

                let enum_variants: Vec<EnumVariant> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let data = v
                            .fields
                            .iter()
                            .map(|ann| {
                                // an enum may carry an rc payload directly, but not one
                                let ty = self.type_from_annotation(ann);
                                if Self::is_transparent_aggregate_of_rc(&ty) {
                                    self.errors.push(
                                        crate::constraint::TypeError::rc_out_of_surface(format!(
                                            "variant `{}` of enum `{}` has a payload of type `{}`, \
                                             a transparent aggregate embedding an `Rc<T>`; an Rc \
                                             inside an array/vec/tuple payload is not supported yet",
                                            v.name, name, ty
                                        ), ann.span),
                                    );
                                }
                                ty
                            })
                            .collect();
                        EnumVariant {
                            name: v.name.clone(),
                            tag: i as u32,
                            data,
                        }
                    })
                    .collect();

                self.type_params_in_scope = saved_type_params;

                self.type_table.register_enum(EnumDef {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    variants: enum_variants,
                });
            }
        }
    }
}
