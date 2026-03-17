use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorSuggestion};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, Span, StructFieldInit};

impl TypeInference {
    pub(super) fn infer_member_expr(
        &mut self,
        object: &Expr,
        member: &str,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);

        // determine the result type for error recovery; actual error reporting happens post-substitution in validate.rs to avoid duplicate diagnostics
        let ty = match &typed_object.ty {
            InferType::String => {
                if member == "len" {
                    InferType::I64
                } else {
                    InferType::Dynamic
                }
            }
            InferType::Struct(name) => {
                if let Some(def) = self.type_table.get_struct(name) {
                    def.fields
                        .iter()
                        .find(|f| f.name == member)
                        .map(|f| f.ty.clone())
                        .unwrap_or(InferType::Dynamic)
                } else {
                    InferType::Dynamic
                }
            }
            InferType::Dynamic => InferType::Dynamic,
            // when the object is an unresolved type variable, return a fresh type variable instead
            // of Dynamic so that type information can propagate once the Var is resolved by the constraint solver
            //
            // if it is never resolved, finalization converts the fresh Var to Dynamic, it's the same end result, but without premature widening
            InferType::Var(_) => self.type_gen.fresh(),
            _other => InferType::Dynamic,
        };

        (
            TypedExprKind::Member {
                object: Box::new(typed_object),
                member: member.to_string(),
            },
            ty,
        )
    }

    pub(super) fn infer_field_assign_expr(
        &mut self,
        object: &Expr,
        field: &str,
        value: &Expr,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);
        let mut typed_value = self.infer_expr(value);

        // Check mutability: the object variable must be declared `let mut`
        if let ExprKind::Identifier(ref name) = object.kind {
            if !self.env.is_mutable(name) {
                let binding_span = self.env.lookup_binding_span(name);
                let suggestion = binding_span.map(|bs| {
                    let insert_offset = bs.start + 4;
                    let insert_span =
                        Span::new(insert_offset, insert_offset, bs.line, bs.column + 4);
                    TypeErrorSuggestion {
                        message: "make the binding mutable".to_string(),
                        span: insert_span,
                        new_text: "mut ".to_string(),
                    }
                });
                self.errors.push(TypeError::assign_to_immutable(
                    name.to_string(),
                    span,
                    binding_span,
                    suggestion,
                ));
            }
        }

        // Validate the field exists and constrain the value type
        if let InferType::Struct(ref struct_name) = typed_object.ty {
            if let Some(def) = self.type_table.get_struct(struct_name) {
                if let Some(field_def) = def.fields.iter().find(|f| f.name == field) {
                    let field_ty = field_def.ty.clone();
                    self.try_narrow_literal(&mut typed_value, &field_ty);
                    // Implicit numeric widening for field assignment
                    if typed_value.ty != field_ty
                        && typed_value.ty.can_implicit_widen_to(&field_ty)
                    {
                        let vspan = typed_value.span;
                        let original = std::mem::replace(
                            &mut typed_value,
                            TypedExpr { kind: TypedExprKind::Null, ty: InferType::Null, span: vspan },
                        );
                        typed_value = TypedExpr {
                            kind: TypedExprKind::Cast {
                                expr: Box::new(original),
                                target: field_ty.clone(),
                            },
                            ty: field_ty.clone(),
                            span: vspan,
                        };
                    }
                    self.constraints.push(Constraint::equal(
                        typed_value.ty.clone(),
                        field_ty,
                        span,
                        ConstraintReason::Assignment {
                            var_name: format!("{}", field),
                        },
                    ));
                } else {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "struct '{}' has no field '{}'",
                            struct_name, field
                        ),
                        span,
                    ));
                }
            }
        }

        (
            TypedExprKind::FieldAssign {
                object: Box::new(typed_object),
                field: field.to_string(),
                value: Box::new(typed_value),
            },
            InferType::Null,
        )
    }

    pub(super) fn infer_struct_literal(
        &mut self,
        name: &str,
        fields: &[StructFieldInit],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        // check for duplicate fields in the literal
        {
            let mut seen = std::collections::HashSet::new();
            for f in fields {
                if !seen.insert(&f.name) {
                    self.errors.push(TypeError::member_access(
                        format!("duplicate field '{}' in struct literal '{}'", f.name, name),
                        f.span,
                    ));
                }
            }
        }

        let struct_exists = self.type_table.has_struct(name);
        if !struct_exists {
            self.errors.push(TypeError::member_access(
                format!("unknown struct '{}'", name),
                span,
            ));
        }

        // validate struct fields: check for unknown and missing fields
        if let Some(def) = self.type_table.get_struct(name) {
            let def_field_names: Vec<String> = def.fields.iter().map(|f| f.name.clone()).collect();

            // check for unknown fields
            for f in fields {
                if !def_field_names.contains(&f.name) {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "unknown field '{}' on struct '{}'; known fields: {}",
                            f.name,
                            name,
                            def_field_names.join(", ")
                        ),
                        f.span,
                    ));
                }
            }

            // check for missing fields only for non-generic structs because generic structs may have partial  initialization patterns
            if def.type_params.is_empty() {
                let provided: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
                for def_field in &def_field_names {
                    if !provided.contains(&def_field.as_str()) {
                        self.errors.push(TypeError::member_access(
                            format!("missing field '{}' in struct literal '{}'", def_field, name),
                            span,
                        ));
                    }
                }
            }
        }

        let typed_fields: Vec<(String, Box<TypedExpr>)> = fields
            .iter()
            .map(|f| {
                let mut typed_value = self.infer_expr(&f.value);

                if let Some(field_ty) = self
                    .type_table
                    .get_struct(name)
                    .and_then(|def| def.fields.iter().find(|df| df.name == f.name))
                    .map(|fd| fd.ty.clone())
                {
                    self.try_narrow_literal(&mut typed_value, &field_ty);

                    // always push a constraint so the solver validates the narrowing decision. when narrowing succeeded the
                    // constraint is trivially satisfied; when it didn't, the solver will catch the mismatch
                    self.constraints.push(Constraint::equal(
                        typed_value.ty.clone(),
                        field_ty,
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
            if struct_exists {
                InferType::Struct(name.to_string())
            } else {
                InferType::Dynamic
            },
        )
    }
}
