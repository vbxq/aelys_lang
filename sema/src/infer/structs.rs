use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::types::{InferType, StructDef, StructField};
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

    pub(super) fn reject_polymorphic_recursion(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            let StmtKind::StructDecl {
                name, type_params, ..
            } = &stmt.kind
            else {
                continue;
            };
            if type_params.is_empty() {
                continue;
            }
            let Some(def) = self.type_table.get_struct(name) else {
                continue;
            };
            let own: Vec<InferType> = type_params
                .iter()
                .map(|p| InferType::Struct(p.clone(), Vec::new()))
                .collect();
            let culprit = def
                .fields
                .iter()
                .find(|f| Self::names_with_other_args(&f.ty, name, &own))
                .map(|f| f.name.clone());
            if let Some(field) = culprit {
                self.errors.push(TypeError::not_supported_yet(
                    format!(
                        "struct `{name}<{}>` names itself with different type arguments in \
                         field `{field}`; polymorphic recursion is not supported",
                        type_params.join(", ")
                    ),
                    stmt.span,
                ));
            }
        }
    }

    fn names_with_other_args(ty: &InferType, name: &str, own: &[InferType]) -> bool {
        let go = |t: &InferType| Self::names_with_other_args(t, name, own);
        match ty {
            InferType::Struct(n, args) if n == name => args != own || args.iter().any(go),
            InferType::Struct(_, args) | InferType::Enum(_, args) | InferType::Tuple(args) => {
                args.iter().any(go)
            }
            InferType::Array(inner, _) | InferType::Vec(inner) | InferType::Rc(inner) => go(inner),
            InferType::Ref { referent, .. } => go(referent),
            InferType::Slice { elem, .. } => go(elem),
            InferType::Function { params, ret, .. } => params.iter().any(go) || go(ret),
            _ => false,
        }
    }

    pub(super) fn reject_multi_variant_rc_enums(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            let StmtKind::EnumDecl { name, variants, .. } = &stmt.kind else {
                continue;
            };
            let Some(def) = self.type_table.get_enum(name) else {
                continue;
            };
            if def.variants.len() < 2 {
                continue;
            }
            let offender = def
                .variants
                .iter()
                .zip(variants.iter())
                .flat_map(|(v, ast)| v.data.iter().zip(ast.fields.iter()).map(move |p| (v, p)))
                .find(|(_, (ty, _))| {
                    !ty.is_rc() && (ty.contains_rc() || self.type_table.contains_rc_nominal(ty))
                })
                .map(|(v, (ty, ann))| (v.name.clone(), ty.to_string(), ann.span));
            if let Some((variant, ty, span)) = offender {
                self.errors.push(TypeError::rc_carrier_out_of_surface(
                    format!(
                        "variant `{name}::{variant}` has a payload of type `{ty}`, which holds an \
                         `Rc` inside it: an enum with more than one variant carries an Rc only as \
                         a direct `Rc<T>` payload yet"
                    ),
                    span,
                ));
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
                    if type_params.is_empty() {
                        continue;
                    }
                    let Some(def) = self.type_table.get_struct(name) else {
                        continue;
                    };
                    let mut offenders = Vec::new();
                    for (field_def, field_ast) in def.fields.iter().zip(fields.iter()) {
                        if self.type_table.contains_vec_by_value(&field_def.ty) {
                            offenders.push((field_ast.name.clone(), field_ast.span));
                        }
                    }
                    for (fname, fspan) in offenders {
                        self.errors.push(crate::constraint::TypeError::vec_out_of_surface(
                            format!(
                                "field `{fname}` of the generic struct `{name}` holds a `Vec`: \
                                 a Vec field in a generic struct is not supported yet"
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
                        self.errors.push(crate::constraint::TypeError::not_supported_yet(
                            format!(
                                "variant `{vname}` of enum `{name}` has a payload of type `{vty}`: \
                                 a Vec inside an enum is not supported yet; keep the Vec in its \
                                 own binding and pass it with `&mut`"
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
