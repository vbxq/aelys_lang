use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorSuggestion};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, Pattern, Span, Stmt, StmtKind, StructFieldInit};

impl TypeInference {
    // balance retain/release, but never in a generic slot, where the type erases and the
    pub(crate) fn reject_rc_out_of_carrier_surface(
        &mut self,
        ty: &InferType,
        generic_container: Option<&'static str>,
        span: Span,
        what: &str,
    ) {
        if let Some(container) = generic_container {
            if ty.is_rc() || ty.contains_rc() || self.type_table.contains_rc_nominal(ty) {
                self.errors.push(TypeError::rc_out_of_surface(
                    format!(
                        "{what} stores a value of type `{ty}` carrying an `Rc<T>` straight \
                     into a generic {container}, which is not supported yet"
                    ),
                    span,
                ));
            }
            return;
        }
        let transparent_aggregate_of_rc = matches!(
            ty,
            InferType::Array(..) | InferType::Vec(_) | InferType::Tuple(_)
        ) && ty.contains_rc();
        if transparent_aggregate_of_rc {
            self.errors.push(TypeError::rc_out_of_surface(
                format!(
                    "{what} is initialized with a value of type `{ty}` (a transparent \
                 aggregate embedding an `Rc<T>`); storing an Rc inside an \
                 array/vec/tuple is not supported yet"
                ),
                span,
            ));
        }
    }
    pub(super) fn infer_member_expr(
        &mut self,
        object: &Expr,
        member: &str,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        if let ExprKind::Identifier(namespace) = &object.kind {
            if self.is_module_namespace(namespace) {
                let (qualified, ty) = self.resolve_module_member(namespace, member, _span);
                if let Some(found) = self.const_literal(&qualified, _span) {
                    return found;
                }
                self.note_generic_value(&qualified, _span);
                return (TypedExprKind::Identifier(qualified), ty);
            }
        }

        let typed_object = self.infer_expr(object);
        let mut base = &typed_object.ty;
        while let InferType::Ref { referent, .. } = base {
            base = referent;
        }
        let builtin = matches!(
            base,
            InferType::String | InferType::Slice { .. } | InferType::Vec(_) | InferType::Array(..)
        );
        if builtin && (member == "len" || (member == "bytes" && matches!(base, InferType::String))) {
            let receiver = receiver_text(object).unwrap_or_else(|| "v".to_string());
            let (message, fix) = if member == "len" {
                (format!("`{receiver}.len` is a method; write `{receiver}.len()`"), ".len()")
            } else {
                (format!("`{receiver}.bytes` is written `{receiver}.as_bytes()`"), ".as_bytes()")
            };
            let mut err = TypeError::coded("E0121", message, "add the parentheses", _span);
            err.help = Some(format!("call it: `{receiver}{fix}`"));
            self.errors.push(err);
        }
        self.member_of_typed(typed_object, member, _span)
    }

    pub(super) fn deref_struct_ref(typed_object: TypedExpr) -> TypedExpr {
        match &typed_object.ty {
            InferType::Ref { referent, .. }
                if matches!(referent.as_ref(), InferType::Struct(..)) =>
            {
                let ty = (**referent).clone();
                let span = typed_object.span;
                TypedExpr::new(TypedExprKind::Deref(Box::new(typed_object)), ty, span)
            }
            _ => typed_object,
        }
    }

    pub(super) fn member_of_typed(
        &mut self,
        typed_object: TypedExpr,
        member: &str,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let struct_like = match &typed_object.ty {
            InferType::Struct(..) | InferType::Rc(_) => true,
            InferType::Ref { referent, .. } => matches!(referent.as_ref(), InferType::Struct(..)),
            _ => false,
        };
        if member.starts_with(|c: char| c.is_ascii_digit()) && !struct_like {
            let ty = self.tuple_element_type(&typed_object.ty, member, span);
            return (
                TypedExprKind::Member {
                    object: Box::new(typed_object),
                    member: member.to_string(),
                },
                ty,
            );
        }
        let typed_object = Self::deref_struct_ref(typed_object);
        let ty = if Self::projection_waits(&typed_object.ty) {
            let kind = crate::infer::projection::ProjKind::Field(member.to_string());
            self.defer_read(&typed_object.ty, kind, span)
        } else {
            self.member_result_type(&typed_object.ty, member)
        };

        (
            TypedExprKind::Member {
                object: Box::new(typed_object),
                member: member.to_string(),
            },
            ty,
        )
    }

    pub(crate) fn tuple_element_type(&mut self, ty: &InferType, member: &str, span: Span) -> InferType {
        let index: usize = member.parse().unwrap_or(usize::MAX);
        match ty {
            InferType::Tuple(elems) => match elems.get(index) {
                Some(t) => t.clone(),
                None => {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "the tuple `{ty}` has {} elements, so it has no element {member}",
                            elems.len()
                        ),
                        span,
                    ));
                    InferType::Dynamic
                }
            },
            InferType::Dynamic => InferType::Dynamic,
            InferType::Var(_) => self.defer_read(
                ty,
                crate::infer::projection::ProjKind::Field(member.to_string()),
                span,
            ),
            other => {
                self.errors.push(TypeError::member_access(
                    format!("`{other}` has no element {member}; only a tuple has numbered elements"),
                    span,
                ));
                InferType::Dynamic
            }
        }
    }

    pub(super) fn member_result_type(&mut self, object_ty: &InferType, member: &str) -> InferType {
        match object_ty {
            InferType::String | InferType::StrRef => match member {
                "len" => InferType::I64,
                "bytes" => InferType::Slice {
                    elem: Box::new(InferType::U8),
                    mutable: false,
                },
                _ => InferType::Dynamic,
            },
            InferType::Slice { .. } | InferType::Vec(_) | InferType::Array(..) => {
                if member == "len" {
                    InferType::I64
                } else {
                    InferType::Dynamic
                }
            }
            InferType::Struct(name, args) => self
                .type_table
                .field_type(name, args, member)
                .unwrap_or(InferType::Dynamic),
            InferType::Rc(inner) => {
                if let InferType::Struct(name, args) = inner.as_ref() {
                    self.type_table
                        .field_type(name, args, member)
                        .unwrap_or(InferType::Dynamic)
                } else {
                    InferType::Dynamic
                }
            }
            InferType::Dynamic => InferType::Dynamic,
            // resolves it; finalization widens it to dynamic anyway if it never does
            InferType::Var(_) => self.type_gen.fresh(),
            _other => InferType::Dynamic,
        }
    }

    pub(crate) fn reject_rc_field_write(
        &mut self,
        field: &str,
        field_ty: &InferType,
        object: &TypedExpr,
        value: &TypedExpr,
        span: Span,
    ) {
        let releases_old =
            object.ty.is_rc() || (!self.in_nogc_fn && crate::place_spine::owns_field_place(object));
        let field_is_rc = field_ty.is_rc();
        let field_is_nominal_carrier = !field_is_rc
            && self.type_table.contains_rc_nominal(field_ty)
            && !(releases_old && !self.type_table.rc_outside_variant_tags(field_ty));
        if field_is_nominal_carrier || (field_is_rc && !releases_old) {
            self.errors.push(TypeError::rc_out_of_surface(
                format!(
                    "in-place reassignment of field `{field}` (type `{field_ty}`, \
                 which carries an `Rc<T>`) is not supported here; \
                 reassigning would leak the previously-held reference"
                ),
                span,
            ));
        }
        if field_is_rc && releases_old && !Self::is_direct_rc_producer(&value.kind) {
            self.errors
                .push(TypeError::rc_field_assign_indirect(value.span));
        }
    }

    pub(super) fn infer_field_assign_expr(
        &mut self,
        object: &Expr,
        field: &str,
        value: &Expr,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);
        let object_through_ref = matches!(&typed_object.ty, InferType::Ref { .. });
        let typed_object = Self::deref_struct_ref(typed_object);
        let mut typed_value = self.infer_expr(value);

        let object_through_rc_handle = matches!(&typed_object.ty, InferType::Rc(_));
        if let ExprKind::Identifier(ref name) = object.kind {
            if !object_through_rc_handle && !object_through_ref && !self.env.is_mutable(name) {
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
                let err = TypeError::assign_to_immutable(
                    name.to_string(),
                    span,
                    binding_span,
                    suggestion,
                );
                if matches!(typed_object.ty, InferType::Var(_)) {
                    self.defer_mutability(&typed_object.ty, err);
                } else {
                    self.errors.push(err);
                }
            }
        }

        let object_struct: Option<(String, Vec<InferType>)> = match &typed_object.ty {
            InferType::Struct(name, args) => Some((name.clone(), args.clone())),
            InferType::Rc(inner) => match inner.as_ref() {
                InferType::Struct(name, args) => Some((name.clone(), args.clone())),
                _ => None,
            },
            _ => None,
        };
        if let Some((struct_name, _)) = object_struct.clone() {
            self.reject_private_field(&struct_name, field, span);
        }
        if field.starts_with(|c: char| c.is_ascii_digit())
            && object_struct.is_none()
            && !Self::projection_waits(&typed_object.ty)
        {
            let field_ty = self.tuple_element_type(&typed_object.ty, field, span);
            if field_ty != InferType::Dynamic {
                if field_ty.is_rc() || field_ty.contains_rc() {
                    self.errors.push(TypeError::rc_out_of_surface(
                        "an `Rc<T>` inside a tuple is not supported; put it in a struct field",
                        span,
                    ));
                }
                self.try_narrow_literal(&mut typed_value, &field_ty);
                self.constraints.push(Constraint::flows(
                    typed_value.ty.clone(),
                    field_ty,
                    span,
                    ConstraintReason::Assignment {
                        var_name: field.to_string(),
                    },
                ));
            }
        } else if let Some((ref struct_name, ref struct_args)) = object_struct {
            if self.type_table.has_struct(struct_name) {
                if let Some(field_ty) = self.type_table.field_type(struct_name, struct_args, field)
                {
                    self.reject_rc_field_write(field, &field_ty, &typed_object, &typed_value, span);
                    self.coerce_dyn(&mut typed_value, &field_ty);
                    self.try_narrow_literal(&mut typed_value, &field_ty);
                    if typed_value.ty != field_ty && typed_value.ty.can_implicit_widen_to(&field_ty)
                    {
                        let vspan = typed_value.span;
                        let original = std::mem::replace(
                            &mut typed_value,
                            TypedExpr {
                                kind: TypedExprKind::Null,
                                ty: InferType::Null,
                                span: vspan,
                            },
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
                    self.constraints.push(Constraint::flows(
                        typed_value.ty.clone(),
                        field_ty,
                        span,
                        ConstraintReason::Assignment {
                            var_name: format!("{}", field),
                        },
                    ));
                } else {
                    self.errors.push(TypeError::member_access(
                        format!("struct '{}' has no field '{}'", struct_name, field),
                        span,
                    ));
                }
            }
        } else if Self::projection_waits(&typed_object.ty) {
            self.defer_write(
                &typed_object.ty,
                crate::infer::projection::ProjKind::FieldWrite(field.to_string()),
                &typed_value.ty,
                span,
            );
        } else if let InferType::Rc(inner) = &typed_object.ty {
            self.errors.push(TypeError::member_access(
                format!("field assignment through `Rc<{inner}>` requires a struct payload"),
                span,
            ));
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

    pub(super) fn struct_constructor(&mut self, path: &Expr, args: Option<&[Expr]>, span: Span) -> Option<Expr> {
        let (written, qualified) = self.named_struct(path)?;
        let declared = &self.type_table.get_struct(&qualified)?.fields;
        let tuple = declared.first().is_none_or(|f| f.name == "0");
        let arity = declared.len();
        let args = match args {
            None if arity == 0 => &[][..],
            Some(args) if tuple => args,
            _ => return None,
        };
        if args.len() != arity {
            self.errors.push(TypeError::coded(
                "E0301",
                format!("`{written}` has {arity} field(s), but {} value(s) are given", args.len()),
                "wrong number of fields",
                span,
            ));
        }
        let fields = args
            .iter()
            .enumerate()
            .map(|(i, value)| StructFieldInit {
                name: i.to_string(),
                value: Box::new(value.clone()),
                span: value.span,
            })
            .collect();
        Some(Expr::new(
            ExprKind::StructLiteral {
                name: written,
                fields,
                base: None,
            },
            span,
        ))
    }

    pub(crate) fn resolve_pattern_path(&mut self, pattern: &Pattern) -> Option<Pattern> {
        let Pattern::Variant {
            enum_name,
            variant,
            fields,
            span,
            ..
        } = pattern
        else {
            return None;
        };
        if enum_name.is_empty()
            && let Some((enum_name, variant)) = self.imported_variant(variant)
        {
            return Some(Pattern::Variant {
                enum_name,
                variant,
                fields: fields.clone(),
                span: *span,
                unqualified: false,
            });
        }
        let name = Expr::new(ExprKind::Identifier(variant.clone()), *span);
        let path = if enum_name.is_empty() {
            name
        } else {
            Expr::new(
                ExprKind::Member {
                    object: Box::new(Expr::new(ExprKind::Identifier(enum_name.clone()), *span)),
                    member: variant.clone(),
                },
                *span,
            )
        };
        let (written, qualified) = self.named_struct(&path)?;
        let arity = self.type_table.get_struct(&qualified)?.fields.len();
        if fields.len() != arity {
            self.errors.push(TypeError::coded(
                "E0301",
                format!("`{written}` has {arity} field(s), but the pattern has {}", fields.len()),
                "wrong number of fields",
                *span,
            ));
            return Some(Pattern::Wildcard(*span));
        }
        Some(Pattern::Struct {
            name: written,
            fields: fields
                .iter()
                .enumerate()
                .map(|(i, p)| (i.to_string(), p.clone(), p.span()))
                .collect(),
            rest: false,
            span: *span,
        })
    }

    pub(super) fn struct_update(&mut self, name: &str, fields: &[StructFieldInit], base: &Expr, span: Span) -> Expr {
        let qualified = self.air_type_name(name, span);
        let declared: Vec<String> = self
            .type_table
            .get_struct(&qualified)
            .map(|d| d.fields.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default();
        let mut stmts = Vec::new();
        let mut bind = |this: &mut Self, value: Expr| {
            let n = this.try_counter;
            this.try_counter += 1;
            let temp = format!("__u${n}");
            let at = value.span;
            stmts.push(Stmt::new(
                StmtKind::Let {
                    name: temp.clone(),
                    mutable: false,
                    type_annotation: None,
                    initializer: value,
                    is_pub: false,
                },
                at,
            ));
            Expr::new(ExprKind::Identifier(temp), at)
        };
        let mut fields: Vec<StructFieldInit> = fields
            .iter()
            .map(|f| StructFieldInit {
                name: f.name.clone(),
                value: Box::new(bind(self, (*f.value).clone())),
                span: f.span,
            })
            .collect();
        let base = bind(self, base.clone());
        for field in declared {
            if fields.iter().any(|f| f.name == field) {
                continue;
            }
            fields.push(StructFieldInit {
                value: Box::new(Expr::new(
                    ExprKind::Member {
                        object: Box::new(base.clone()),
                        member: field.clone(),
                    },
                    span,
                )),
                name: field,
                span,
            });
        }
        let literal = Expr::new(
            ExprKind::StructLiteral {
                name: name.to_string(),
                fields,
                base: None,
            },
            span,
        );
        Expr::new(
            ExprKind::Block {
                stmts,
                tail: Box::new(literal),
            },
            span,
        )
    }

    pub(super) fn infer_struct_literal(
        &mut self,
        name: &str,
        fields: &[StructFieldInit],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        if name == "Self" {
            self.errors.push(TypeError::self_outside_impl(span));
            for f in fields {
                self.infer_expr(&f.value);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }
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

        let resolved = self.air_type_name(name, span);
        let name = resolved.as_str();
        let struct_exists = self.type_table.has_struct(name);
        if !struct_exists && self.alias_names.contains(name) {
            self.errors.push(TypeError::member_access(
                format!("`{name}` is a type alias; a struct literal names the struct itself"),
                span,
            ));
        } else if !struct_exists {
            self.errors.push(TypeError::member_access(
                format!("unknown struct '{}'", name),
                span,
            ));
        }

        for f in fields {
            let (owner, field) = (name.to_string(), f.name.clone());
            self.reject_private_field(&owner, &field, f.span);
        }

        if let Some(def) = self.type_table.get_struct(name) {
            let def_field_names: Vec<String> = def.fields.iter().map(|f| f.name.clone()).collect();

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

        let hint = self.struct_literal_hint.take();
        let type_params: Vec<String> = self
            .type_table
            .get_struct(name)
            .map(|d| d.type_params.clone())
            .unwrap_or_default();
        let mut inst_args: Vec<InferType> = match hint {
            Some(InferType::Struct(hname, hargs))
                if hname == name && hargs.len() == type_params.len() =>
            {
                hargs
            }
            _ => type_params.iter().map(|_| self.type_gen.fresh()).collect(),
        };

        let typed_fields: Vec<(String, Box<TypedExpr>)> = fields
            .iter()
            .map(|f| {
                let declared = self
                    .type_table
                    .get_struct(name)
                    .and_then(|def| def.fields.iter().find(|df| df.name == f.name))
                    .map(|fd| fd.ty.clone());
                let field_ty = declared
                    .as_ref()
                    .map(|ty| ty.subst_params(&type_params, &inst_args));

                if let Some(InferType::Struct(..)) = &field_ty
                    && matches!(f.value.kind, ExprKind::StructLiteral { .. })
                {
                    self.struct_literal_hint = field_ty.clone();
                }
                let mut typed_value = self.infer_expr(&f.value);
                self.struct_literal_hint = None;

                let param_slot = declared
                    .as_ref()
                    .is_some_and(|ty| Self::slot_mentions_param(ty, &type_params));
                self.check_carrier(crate::infer::projection::CarrierCheck::Surface {
                    ty: typed_value.ty.clone(),
                    generic: param_slot.then_some("struct"),
                    rc_slot: false,
                    span: f.span,
                    what: format!("field `{}` of struct literal `{}`", f.name, name),
                });

                if let Some(field_ty) = field_ty {
                    self.coerce_dyn(&mut typed_value, &field_ty);
                    if !field_ty.has_vars() {
                        self.try_narrow_literal(&mut typed_value, &field_ty);
                    }

                    if typed_value.ty != field_ty
                        && !field_ty.has_vars()
                        && typed_value.ty.can_implicit_widen_to(&field_ty)
                    {
                        let vspan = typed_value.span;
                        let original = std::mem::replace(
                            &mut typed_value,
                            TypedExpr {
                                kind: TypedExprKind::Null,
                                ty: InferType::Null,
                                span: vspan,
                            },
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

                    if let Some(InferType::Struct(p, pargs)) = &declared
                        && pargs.is_empty()
                        && let Some(i) = type_params.iter().position(|tp| tp == p)
                        && field_ty == inst_args[i]
                        && matches!(field_ty, InferType::Var(_))
                        && !matches!(typed_value.ty, InferType::Var(_))
                    {
                        inst_args[i] = typed_value.ty.clone();
                    }
                    self.constraints.push(Constraint::flows(
                        typed_value.ty.clone(),
                        field_ty,
                        typed_value.span,
                        ConstraintReason::StructField {
                            struct_name: name.to_string(),
                            field: f.name.clone(),
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
                InferType::Struct(name.to_string(), inst_args)
            } else {
                InferType::Dynamic
            },
        )
    }

    pub(crate) fn slot_mentions_param(ty: &InferType, params: &[String]) -> bool {
        let go = |t: &InferType| Self::slot_mentions_param(t, params);
        match ty {
            InferType::Rc(_) => false,
            InferType::Struct(n, args) if args.is_empty() => params.iter().any(|p| p == n),
            InferType::Struct(_, args) | InferType::Enum(_, args) | InferType::Tuple(args) => {
                args.iter().any(go)
            }
            InferType::Array(inner, _) | InferType::Vec(inner) => go(inner),
            InferType::Ref { referent, .. } => go(referent),
            InferType::Slice { elem, .. } => go(elem),
            InferType::Function {
                params: ps, ret, ..
            } => ps.iter().any(go) || go(ret),
            _ => false,
        }
    }
}

fn receiver_text(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Identifier(name) => Some(name.clone()),
        ExprKind::Member { object, member } => Some(format!("{}.{member}", receiver_text(object)?)),
        _ => None,
    }
}
