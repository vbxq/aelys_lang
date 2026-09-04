use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::types::{InferType, StructDef, StructField};
use aelys_common::{Warning, WarningKind};
use aelys_syntax::{Stmt, StmtKind};
use std::collections::HashSet;

impl TypeInference {
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

    pub(super) fn reject_reserved_type_names(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            let name = match &stmt.kind {
                StmtKind::StructDecl { name, .. } | StmtKind::EnumDecl { name, .. } => name,
                _ => continue,
            };
            if name == "Rc" || name == "Vec" {
                self.errors
                    .push(TypeError::reserved_type_name(name.clone(), stmt.span));
            }
        }
    }

    pub(crate) fn is_transparent_aggregate_of_rc(ty: &InferType) -> bool {
        matches!(
            ty,
            InferType::Array(..) | InferType::Vec(_) | InferType::Tuple(_)
        ) && ty.contains_rc()
    }

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
                if !processed.insert(name.clone()) {
                    continue;
                }

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

                let saved_type_params =
                    std::mem::replace(&mut self.type_params_in_scope, type_params.clone());

                let struct_fields: Vec<StructField> = fields
                    .iter()
                    .map(|f| {
                        let ty = self.type_from_annotation(&f.type_annotation);
                        if Self::is_transparent_aggregate_of_rc(&ty) {
                            self.errors.push(TypeError::rc_out_of_surface(
                                format!(
                                    "field `{}` of struct `{}` has type `{}`, a transparent \
                                 aggregate embedding an `Rc<T>`; an Rc inside an \
                                 array/vec/tuple field is not supported",
                                    f.name, name, ty
                                ),
                                f.span,
                            ));
                        }
                        StructField {
                            name: f.name.clone(),
                            ty,
                            is_pub: f.is_pub,
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

    pub(super) fn reject_nominal_vec_carriers(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            match &stmt.kind {
                StmtKind::StructDecl {
                    name,
                    type_params,
                    fields,
                    ..
                } => {
                    if !type_params.is_empty() {
                        continue;
                    }
                    let Some(def) = self.type_table.get_struct(name) else {
                        continue;
                    };
                    let mut offenders = Vec::new();
                    for (field_def, field_ast) in def.fields.iter().zip(fields.iter()) {
                        if self.type_table.contains_vec_by_value(&field_def.ty) {
                            offenders.push((
                                field_ast.name.clone(),
                                field_ast.span,
                                field_def.ty.to_string(),
                            ));
                        }
                    }
                    for (fname, fspan, fty) in offenders {
                        self.errors.push(crate::constraint::TypeError::rc_out_of_surface(
                            format!(
                                "field `{}` of struct `{}` has type `{}`, which holds a `Vec<T>` \
                                 by value; a Vec inside a struct/enum is not supported yet \
                                 (its buffer would leak , the carrier-Vec retain/release is not \
                                 implemented)",
                                fname, name, fty
                            ),
                            fspan,
                        ));
                    }
                }
                StmtKind::EnumDecl {
                    name,
                    type_params,
                    variants,
                    ..
                } => {
                    if !type_params.is_empty() {
                        continue;
                    }
                    let Some(def) = self.type_table.get_enum(name) else {
                        continue;
                    };
                    let mut offenders = Vec::new();
                    for (variant_def, variant_ast) in def.variants.iter().zip(variants.iter()) {
                        for (payload_ty, payload_ann) in
                            variant_def.data.iter().zip(variant_ast.fields.iter())
                        {
                            if self.type_table.contains_vec_by_value(payload_ty) {
                                offenders.push((
                                    variant_ast.name.clone(),
                                    payload_ann.span,
                                    payload_ty.to_string(),
                                ));
                            }
                        }
                    }
                    for (vname, vspan, vty) in offenders {
                        self.errors.push(crate::constraint::TypeError::rc_out_of_surface(
                            format!(
                                "variant `{}` of enum `{}` has a payload of type `{}`, which holds \
                                 a `Vec<T>` by value; a Vec inside a struct/enum is not supported \
                                 yet (its buffer would leak, the carrier-Vec retain/release \
                                 is not implemented)",
                                vname, name, vty
                            ),
                            vspan,
                        ));
                    }
                }
                _ => {}
            }
        }
    }
}
