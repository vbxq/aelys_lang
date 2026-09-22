use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedStmt, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::Span;
use std::collections::HashSet;

impl TypeInference {
    fn field_is_exported(&self, struct_name: &str, field: &str) -> bool {
        self.type_table
            .get_struct(struct_name)
            .and_then(|def| def.fields.iter().find(|f| f.name == field))
            .is_some_and(|f| f.is_pub)
    }

    // a public signature naming a private type would hand an importer a value it can never name,
    pub(super) fn reject_private_types_in_public_api(&mut self, stmts: &[TypedStmt]) {
        if !self.module_is_importable {
            return;
        }
        let mut private: HashSet<String> = HashSet::new();
        for stmt in stmts {
            match &stmt.kind {
                TypedStmtKind::StructDecl { name, is_pub, .. }
                | TypedStmtKind::EnumDecl { name, is_pub, .. }
                    if !is_pub =>
                {
                    private.insert(name.clone());
                }
                _ => {}
            }
        }
        if private.is_empty() {
            return;
        }

        let mut found: Vec<(String, String, Span)> = Vec::new();
        for stmt in stmts {
            match &stmt.kind {
                TypedStmtKind::Function(func) if func.is_pub => {
                    for param in &func.params {
                        collect(&param.ty, &private, &func.name, func.span, &mut found);
                    }
                    collect(
                        &func.return_type,
                        &private,
                        &func.name,
                        func.span,
                        &mut found,
                    );
                }
                TypedStmtKind::Let {
                    name,
                    var_type,
                    is_pub: true,
                    ..
                } => collect(var_type, &private, name, stmt.span, &mut found),
                TypedStmtKind::StructDecl {
                    name,
                    fields,
                    is_pub: true,
                    ..
                } => {
                    for (field, ty) in fields {
                        if !self.field_is_exported(name, field) {
                            continue;
                        }
                        let owner = format!("{}.{}", name, field);
                        collect(ty, &private, &owner, stmt.span, &mut found);
                    }
                }
                TypedStmtKind::EnumDecl {
                    name,
                    variants,
                    is_pub: true,
                    ..
                } => {
                    for (variant, _, payload) in variants {
                        let owner = format!("{}::{}", name, variant);
                        for ty in payload {
                            collect(ty, &private, &owner, stmt.span, &mut found);
                        }
                    }
                }
                _ => {}
            }
        }

        for (item, ty, span) in found {
            self.errors.push(TypeError {
                kind: TypeErrorKind::PrivateTypeInPublicApi { item, ty },
                span,
                reason: ConstraintReason::Other(String::new()),
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
        }
    }
}

fn collect(
    ty: &InferType,
    private: &HashSet<String>,
    item: &str,
    span: Span,
    found: &mut Vec<(String, String, Span)>,
) {
    match ty {
        InferType::Struct(name) | InferType::Enum(name, _) if private.contains(name) => {
            found.push((item.to_string(), name.clone(), span));
        }
        _ => {}
    }
    match ty {
        InferType::Enum(_, args) | InferType::Tuple(args) => {
            for arg in args {
                collect(arg, private, item, span, found);
            }
        }
        InferType::Function { params, ret, .. } => {
            for param in params {
                collect(param, private, item, span, found);
            }
            collect(ret, private, item, span, found);
        }
        InferType::Array(inner, _)
        | InferType::Vec(inner)
        | InferType::Rc(inner)
        | InferType::Ref {
            referent: inner, ..
        }
        | InferType::Slice { elem: inner, .. } => collect(inner, private, item, span, found),
        _ => {}
    }
}
