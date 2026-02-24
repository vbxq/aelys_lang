use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span, StructFieldInit};

impl TypeInference {
    pub(super) fn infer_member_expr(
        &mut self,
        object: &Expr,
        member: &str,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);

        let ty = match &typed_object.ty {
            // `Str` field access is a byte-length view over UTF-8 payload
            // future char/other views should be separate APIs
            InferType::String => {
                if member == "len" {
                    InferType::I64
                } else {
                    self.errors.push(TypeError::member_access(
                        format!("unknown field '{}' on Str; supported: 'len'", member),
                        span,
                    ));
                    InferType::Dynamic
                }
            }
            InferType::Struct(name) => {
                if let Some(def) = self.type_table.get_struct(name) {
                    def.fields
                        .iter()
                        .find(|f| f.name == member)
                        .map(|f| f.ty.clone())
                        .unwrap_or_else(|| {
                            self.errors.push(TypeError::member_access(
                                format!("unknown field '{}' on struct '{}'", member, name),
                                span,
                            ));
                            InferType::Dynamic
                        })
                } else {
                    InferType::Dynamic
                }
            }
            InferType::Dynamic | InferType::Var(_) => InferType::Dynamic,
            other => {
                self.errors.push(TypeError::member_access(
                    format!("field access on non-struct type {}", other),
                    span,
                ));
                InferType::Dynamic
            }
        };

        (
            TypedExprKind::Member {
                object: Box::new(typed_object),
                member: member.to_string(),
            },
            ty,
        )
    }

    pub(super) fn infer_struct_literal(
        &mut self,
        name: &str,
        fields: &[StructFieldInit],
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_fields: Vec<(String, Box<TypedExpr>)> = fields
            .iter()
            .map(|f| {
                let typed_value = self.infer_expr(&f.value);

                if let Some(def) = self.type_table.get_struct(name)
                    && let Some(field_def) = def.fields.iter().find(|df| df.name == f.name)
                {
                    self.constraints.push(Constraint::equal(
                        typed_value.ty.clone(),
                        field_def.ty.clone(),
                        f.span,
                        ConstraintReason::TypeAnnotation {
                            var_name: format!("{}.{}", name, f.name),
                        },
                    ));
                }

                (f.name.clone(), Box::new(typed_value))
            })
            .collect();

        (
            TypedExprKind::StructLiteral {
                name: name.to_string(),
                fields: typed_fields,
            },
            InferType::Struct(name.to_string()),
        )
    }
}
