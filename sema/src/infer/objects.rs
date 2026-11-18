use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError};
use crate::infer::traits::trait_shown;
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Receiver, Span, TypeAnnotation};

pub const DYN_CALL: &str = "__dyn$";

pub fn dyn_call_name(q: &str, method: &str) -> String {
    format!("{DYN_CALL}{q}${method}")
}

pub fn dyn_target(ty: &InferType) -> Option<&str> {
    match ty {
        InferType::Rc(inner) | InferType::Ref { referent: inner, .. } => match inner.as_ref() {
            InferType::Dyn(q) => Some(q),
            _ => None,
        },
        _ => None,
    }
}

fn generic_trait_object(q: &str, span: Span) -> TypeError {
    TypeError::coded(
        "E0767",
        format!(
            "`dyn {}` names a generic trait, and trait objects of a generic trait are not supported",
            trait_shown(q)
        ),
        "generic trait object",
        span,
    )
}

fn mentions_self(ty: &InferType) -> bool {
    match ty {
        InferType::Struct(n, a) => (n == "Self" && a.is_empty()) || a.iter().any(mentions_self),
        InferType::Enum(_, a) | InferType::Tuple(a) => a.iter().any(mentions_self),
        InferType::Function { params, ret, .. } => {
            params.iter().any(mentions_self) || mentions_self(ret)
        }
        InferType::Array(i, _) | InferType::Vec(i) | InferType::Rc(i) => mentions_self(i),
        InferType::Ref { referent, .. } => mentions_self(referent),
        InferType::Slice { elem, .. } => mentions_self(elem),
        _ => false,
    }
}

impl TypeInference {
    pub(super) fn note_trait_names(&mut self, stmts: &[aelys_syntax::Stmt]) {
        let path = self.module_path();
        for stmt in stmts {
            if let aelys_syntax::StmtKind::Trait(decl) = &stmt.kind {
                let q = crate::modules::qualify_type_name(&path, &decl.name);
                self.early_trait_names.insert(decl.name.clone(), q);
            }
        }
    }

    pub(super) fn lower_dyn(&mut self, ann: &TypeAnnotation) -> InferType {
        let Some(q) = self
            .resolve_trait_name(&ann.name)
            .or_else(|| self.early_trait_names.get(&ann.name).cloned())
        else {
            self.errors.push(TypeError::coded(
                "E0106",
                format!("no trait named `{}`; `dyn` takes a trait in scope", ann.name),
                "unknown trait",
                ann.span,
            ));
            return InferType::Dynamic;
        };
        if ann.type_param.is_some() || !ann.type_params.is_empty() {
            self.errors.push(generic_trait_object(&q, ann.span));
            return InferType::Dynamic;
        }
        if let Some(def) = self.trait_defs.get(&q).cloned() {
            if !self.dyn_compatible(&q, ann.span) {
                return InferType::Dynamic;
            }
            for m in &def.methods {
                self.note_stub_use(&m.stub);
            }
        } else {
            self.unchecked_dyns.push((q.clone(), ann.span));
        }
        InferType::Dyn(q)
    }

    pub(super) fn check_unchecked_dyns(&mut self) {
        for (q, span) in std::mem::take(&mut self.unchecked_dyns) {
            self.dyn_compatible(&q, span);
        }
    }

    fn dyn_compatible(&mut self, q: &str, span: Span) -> bool {
        let Some(def) = self.trait_defs.get(q).cloned() else {
            return false;
        };
        let shown = trait_shown(q).to_string();
        if !def.type_params.is_empty() {
            self.errors.push(generic_trait_object(q, span));
            return false;
        }
        for m in &def.methods {
            if m.receiver == Receiver::None {
                self.errors.push(TypeError::coded(
                    "E0762",
                    format!(
                        "the trait `{shown}` cannot be made into an object: associated function \
                         `{}` has no `self` receiver",
                        m.name
                    ),
                    "not dyn-compatible",
                    span,
                ));
                return false;
            }
            let InferType::Function { params, ret, .. } = &m.ty else {
                continue;
            };
            if params.iter().skip(1).any(mentions_self) || mentions_self(ret) {
                self.errors.push(TypeError::coded(
                    "E0761",
                    format!(
                        "the trait `{shown}` cannot be made into an object: method `{}` mentions \
                         `Self` outside its receiver",
                        m.name
                    ),
                    "not dyn-compatible",
                    span,
                ));
                return false;
            }
        }
        true
    }

    pub(super) fn unsized_dyn(&mut self, ann: &TypeAnnotation) -> InferType {
        self.errors.push(TypeError::coded(
            "E0760",
            format!(
                "`dyn {}` has no size known at compile time; put it behind `Rc` or `&`, as \
                 `Rc<dyn {}>` or `&dyn {}`",
                ann.name, ann.name, ann.name
            ),
            "unsized trait object",
            ann.span,
        ));
        InferType::Dynamic
    }

    pub(crate) fn coerce_dyn(&mut self, expr: &mut TypedExpr, expected: &InferType) {
        let Some(q) = dyn_target(expected).map(str::to_string) else {
            return;
        };
        let source = match (expected, self.probe_type(&expr.ty)) {
            (InferType::Rc(_), InferType::Rc(inner)) => *inner,
            (InferType::Ref { mutable, .. }, InferType::Ref { referent, mutable: from_mut })
                if from_mut || !mutable =>
            {
                *referent
            }
            _ => return,
        };
        if matches!(source, InferType::Dyn(_) | InferType::Var(_) | InferType::Dynamic) {
            return;
        }
        if !self.satisfies_trait(&q, &source) {
            self.errors.push(TypeError::coded(
                "E0763",
                format!(
                    "`{source}` does not implement `{}`, so it cannot become a `dyn {}`",
                    trait_shown(&q),
                    trait_shown(&q)
                ),
                "trait not implemented",
                expr.span,
            ));
        }
        let span = expr.span;
        let inner = std::mem::replace(expr, TypedExpr::new(TypedExprKind::Null, InferType::Null, span));
        *expr = TypedExpr::new(
            TypedExprKind::Cast {
                expr: Box::new(inner),
                target: expected.clone(),
            },
            expected.clone(),
            span,
        );
    }

    pub(super) fn dyn_method_call(
        &mut self,
        recv: TypedExpr,
        q: &str,
        member: &str,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let shown = trait_shown(q).to_string();
        let sig = self
            .trait_defs
            .get(q)
            .and_then(|d| d.methods.iter().find(|m| m.name == member).cloned());
        let refusal = match &sig {
            None => Some(TypeError::coded(
                "E0765",
                format!("no method `{member}` in trait `{shown}`; a `dyn {shown}` only has the methods of its trait"),
                "not a trait method",
                span,
            )),
            Some(sig) if sig.receiver == Receiver::Value => Some(TypeError::coded(
                "E0766",
                format!("`{member}` takes `self` by value, so it cannot be called through a `dyn {shown}`"),
                "by-value receiver",
                span,
            )),
            Some(sig)
                if sig.receiver == Receiver::RefMut
                    && matches!(recv.ty, InferType::Ref { mutable: false, .. }) =>
            {
                Some(TypeError::coded(
                    "E0422",
                    format!(
                        "`{member}` takes `&mut self`, and its receiver is a shared reference \
                         `{}`; mutation through a shared reference",
                        recv.ty
                    ),
                    "shared reference",
                    recv.span,
                ))
            }
            Some(_) => None,
        };
        let sig = match (sig, refusal) {
            (Some(sig), None) => sig,
            (_, refusal) => {
                self.errors.extend(refusal);
                for a in args {
                    self.infer_expr(a);
                }
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };
        let InferType::Function { params, ret, .. } = sig.ty.clone() else {
            return (TypedExprKind::Null, InferType::Dynamic);
        };
        if params.len() != args.len() + 1 {
            self.errors.push(TypeError::coded(
                "E0302",
                format!(
                    "`{member}` takes {} argument(s) but {} were supplied",
                    params.len() - 1,
                    args.len()
                ),
                "wrong number of arguments",
                span,
            ));
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        let recv_ty = InferType::Ref {
            referent: Box::new(InferType::Dyn(q.to_string())),
            mutable: sig.receiver == Receiver::RefMut,
        };
        let recv = match &recv.ty {
            InferType::Ref { .. } => recv,
            _ => {
                let rspan = recv.span;
                TypedExpr::new(
                    TypedExprKind::Cast {
                        expr: Box::new(recv),
                        target: recv_ty.clone(),
                    },
                    recv_ty.clone(),
                    rspan,
                )
            }
        };
        let mut call_args = vec![recv];
        for (arg, param_ty) in args.iter().zip(params.iter().skip(1)) {
            let mut typed = self.infer_expr(arg);
            self.coerce_dyn(&mut typed, param_ty);
            self.try_narrow_literal(&mut typed, param_ty);
            self.constraints.push(Constraint::flows(
                typed.ty.clone(),
                param_ty.clone(),
                typed.span,
                ConstraintReason::Other(format!("argument of `{shown}::{member}`")),
            ));
            call_args.push(typed);
        }
        let mut callee_params = vec![recv_ty];
        callee_params.extend(params.into_iter().skip(1));
        let callee = TypedExpr::new(
            TypedExprKind::Identifier(dyn_call_name(q, member)),
            InferType::Function {
                params: callee_params,
                ret: ret.clone(),
                nogc: sig.nogc,
            },
            span,
        );
        (
            TypedExprKind::Call {
                callee: Box::new(callee),
                args: call_args,
            },
            *ret,
        )
    }

    pub(crate) fn display_through_dyn(&mut self, mut expr: TypedExpr) -> TypedExpr {
        let seen = self.probe_type(&expr.ty);
        if dyn_target(&seen) != Some("Display") {
            return expr;
        }
        let span = expr.span;
        expr.ty = seen;
        let (kind, ty) = self.dyn_method_call(expr, "Display", "to_string", &[], span);
        TypedExpr::new(kind, ty, span)
    }
}
