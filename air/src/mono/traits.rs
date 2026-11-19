use super::{MonoContext, MonoError};
use super::substitute::operand_type_from;
use crate::*;
use std::collections::HashMap;

pub(super) const STUB: &str = "__trait$";
const BUILTIN: &[&str] = &["eq", "ord", "Display"];

pub(super) enum StubTarget {
    Symbol(String),
    Instance(String, Vec<AirType>),
    Default,
    Missing(String),
}

pub(super) fn parse_stub(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(STUB)?;
    let cut = rest.rfind('$')?;
    Some((&rest[..cut], &rest[cut + 1..]))
}

fn shown_trait(q: &str) -> &str {
    let bare = q.strip_prefix("__q.").unwrap_or(q);
    bare.rsplit('.').next().unwrap_or(bare)
}

pub(crate) fn shown_type(ty: &AirType) -> String {
    match ty {
        AirType::Struct(r) | AirType::Enum(r) => {
            let bare = r.name.strip_prefix("__q.").unwrap_or(&r.name);
            let base = bare.rsplit('.').next().unwrap_or(bare).to_string();
            if r.args.is_empty() {
                base
            } else {
                let args: Vec<String> = r.args.iter().map(shown_type).collect();
                format!("{base}<{}>", args.join(", "))
            }
        }
        AirType::Str => "string".to_string(),
        AirType::Ptr(inner) => format!("&{}", shown_type(inner)),
        other => super::substitute::type_to_string(other),
    }
}

fn match_air(
    pat: &AirType,
    params: &[TypeParamId],
    ty: &AirType,
    out: &mut HashMap<u32, AirType>,
) -> bool {
    if let AirType::Param(id) = pat
        && params.contains(id)
    {
        if let Some(prev) = out.get(&id.0) {
            return prev == ty;
        }
        out.insert(id.0, ty.clone());
        return true;
    }
    match (pat, ty) {
        (AirType::Struct(a), AirType::Struct(b)) | (AirType::Enum(a), AirType::Enum(b)) => {
            a.name == b.name
                && a.args.len() == b.args.len()
                && a.args
                    .iter()
                    .zip(b.args.iter())
                    .all(|(p, t)| match_air(p, params, t, out))
        }
        (AirType::Array(p, n), AirType::Array(t, m)) => n == m && match_air(p, params, t, out),
        (AirType::Vec(p), AirType::Vec(t))
        | (AirType::Rc(p), AirType::Rc(t))
        | (AirType::Ptr(p), AirType::Ptr(t))
        | (AirType::Slice(p), AirType::Slice(t)) => match_air(p, params, t, out),
        _ => pat == ty,
    }
}

impl MonoContext {
    pub(super) fn resolve_impl_args(
        &self,
        q: &str,
        ty: &AirType,
        targs: &[AirType],
    ) -> Option<(usize, Vec<AirType>)> {
        let mut found = None;
        for (i, imp) in self.impls.iter().enumerate() {
            if imp.trait_q != q {
                continue;
            }
            let mut map = HashMap::new();
            if !match_air(&imp.self_ty, &imp.type_params, ty, &mut map) {
                continue;
            }
            if !targs.is_empty()
                && (targs.len() != imp.trait_args.len()
                    || !imp
                        .trait_args
                        .iter()
                        .zip(targs.iter())
                        .all(|(p, t)| match_air(p, &imp.type_params, t, &mut map)))
            {
                continue;
            }
            let Some(args) = imp
                .type_params
                .iter()
                .map(|p| map.get(&p.0).cloned())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            if found.is_some() {
                return None;
            }
            found = Some((i, args));
        }
        found
    }

    pub(super) fn method_target(&self, q: &str, method: &str, ty: &AirType) -> Option<StubTarget> {
        self.method_target_args(q, method, ty, &[])
    }

    fn method_target_args(
        &self,
        q: &str,
        method: &str,
        ty: &AirType,
        targs: &[AirType],
    ) -> Option<StubTarget> {
        let (i, args) = self.resolve_impl_args(q, ty, targs)?;
        let imp = &self.impls[i];
        let sym = imp.methods.iter().find(|(m, _)| m == method)?.1.clone();
        Some(if args.is_empty() {
            StubTarget::Symbol(sym)
        } else {
            StubTarget::Instance(sym, args)
        })
    }

    pub(super) fn stub_target(&self, q: &str, method: &str, args: &[AirType]) -> StubTarget {
        let Some(self_ty) = args.first() else {
            return StubTarget::Missing(format!(
                "`{}::{method}` has no `Self` type at this call",
                shown_trait(q)
            ));
        };
        let targs = &args[1..];
        if self.resolve_impl_args(q, self_ty, targs).is_some() {
            return self
                .method_target_args(q, method, self_ty, targs)
                .unwrap_or(StubTarget::Default);
        }
        let structural = BUILTIN.contains(&q)
            && !matches!(
                self_ty,
                AirType::Struct(_)
                    | AirType::Array(..)
                    | AirType::Vec(_)
                    | AirType::Slice(_)
                    | AirType::Rc(_)
                    | AirType::Ptr(_)
                    | AirType::FnPtr { .. }
            );
        if structural {
            return StubTarget::Default;
        }
        StubTarget::Missing(format!(
            "`{}` does not implement `{}`, which this instance calls `{method}` through",
            shown_type(self_ty),
            shown_trait(q)
        ))
    }

    fn call_for(&self, q: &str, method: &str, ty: &AirType) -> Option<Callee> {
        match self.method_target(q, method, ty)? {
            StubTarget::Symbol(s) | StubTarget::Instance(s, _) => Some(Callee::Named(s)),
            _ => None,
        }
    }

    pub(super) fn rewrite_trait_ops(&self, func: &mut AirFunction) {
        let mut next = func
            .params
            .iter()
            .map(|p| p.id.0)
            .chain(func.locals.iter().map(|l| l.id.0))
            .max()
            .map_or(0, |m| m + 1);
        let mut new_locals: Vec<AirLocal> = Vec::new();
        let mut fresh = |ty: AirType, new_locals: &mut Vec<AirLocal>| {
            let id = LocalId(next);
            next += 1;
            new_locals.push(AirLocal {
                id,
                ty,
                name: None,
                is_mut: false,
                span: None,
            });
            id
        };
        let params = func.params.clone();
        let locals = func.locals.clone();
        let ty_of = |op: &Operand| operand_type_from(op, &params, &locals);
        for block in &mut func.blocks {
            let mut out: Vec<AirStmt> = Vec::with_capacity(block.stmts.len());
            for stmt in std::mem::take(&mut block.stmts) {
                let span = stmt.span;
                let push = |out: &mut Vec<AirStmt>, kind: AirStmtKind| {
                    out.push(AirStmt { kind, span });
                };
                match &stmt.kind {
                    AirStmtKind::Assign {
                        place,
                        rvalue: Rvalue::BinaryOp(op, a, b),
                    } if matches!(
                        op,
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                    ) =>
                    {
                        let ty = ty_of(a);
                        let (q, m) = if matches!(op, BinOp::Eq | BinOp::Ne) {
                            ("eq", "eq")
                        } else {
                            ("ord", "cmp")
                        };
                        let callee = match (&ty, a, b) {
                            (
                                AirType::Struct(_) | AirType::Enum(_),
                                Operand::Copy(_) | Operand::Move(_),
                                Operand::Copy(_) | Operand::Move(_),
                            ) => self.call_for(q, m, &ty),
                            _ => None,
                        };
                        let Some(callee) = callee else {
                            out.push(stmt);
                            continue;
                        };
                        let la = match a {
                            Operand::Copy(id) | Operand::Move(id) => *id,
                            Operand::Const(_) => unreachable!(),
                        };
                        let lb = match b {
                            Operand::Copy(id) | Operand::Move(id) => *id,
                            Operand::Const(_) => unreachable!(),
                        };
                        let pa = fresh(AirType::Ptr(Box::new(ty.clone())), &mut new_locals);
                        let pb = fresh(AirType::Ptr(Box::new(ty.clone())), &mut new_locals);
                        push(
                            &mut out,
                            AirStmtKind::Assign {
                                place: Place::Local(pa),
                                rvalue: Rvalue::AddressOf(Place::Local(la)),
                            },
                        );
                        push(
                            &mut out,
                            AirStmtKind::Assign {
                                place: Place::Local(pb),
                                rvalue: Rvalue::AddressOf(Place::Local(lb)),
                            },
                        );
                        let args = vec![Operand::Copy(pa), Operand::Copy(pb)];
                        let place = place.clone();
                        match op {
                            BinOp::Eq => push(
                                &mut out,
                                AirStmtKind::Assign {
                                    place,
                                    rvalue: Rvalue::Call { func: callee, args },
                                },
                            ),
                            BinOp::Ne => {
                                let t = fresh(AirType::Bool, &mut new_locals);
                                push(
                                    &mut out,
                                    AirStmtKind::Assign {
                                        place: Place::Local(t),
                                        rvalue: Rvalue::Call { func: callee, args },
                                    },
                                );
                                push(
                                    &mut out,
                                    AirStmtKind::Assign {
                                        place,
                                        rvalue: Rvalue::UnaryOp(UnOp::Not, Operand::Copy(t)),
                                    },
                                );
                            }
                            cmp => {
                                let c = fresh(AirType::I64, &mut new_locals);
                                push(
                                    &mut out,
                                    AirStmtKind::Assign {
                                        place: Place::Local(c),
                                        rvalue: Rvalue::Call { func: callee, args },
                                    },
                                );
                                push(
                                    &mut out,
                                    AirStmtKind::Assign {
                                        place,
                                        rvalue: Rvalue::BinaryOp(
                                            cmp.clone(),
                                            Operand::Copy(c),
                                            Operand::Const(AirConst::Int(0, AirIntSize::I64)),
                                        ),
                                    },
                                );
                            }
                        }
                    }
                    AirStmtKind::CallVoid {
                        func: Callee::Named(name),
                        args,
                    } if (name == "print" || name == "println") && args.len() == 1 => {
                        let ty = ty_of(&args[0]);
                        let (Some(callee), Operand::Copy(x) | Operand::Move(x)) =
                            (self.display_callee(&ty), &args[0])
                        else {
                            out.push(stmt);
                            continue;
                        };
                        let x = *x;
                        let name = name.clone();
                        let p = fresh(AirType::Ptr(Box::new(ty.clone())), &mut new_locals);
                        let s = fresh(AirType::Str, &mut new_locals);
                        let ps = fresh(AirType::Ptr(Box::new(AirType::Str)), &mut new_locals);
                        push(
                            &mut out,
                            AirStmtKind::Assign {
                                place: Place::Local(p),
                                rvalue: Rvalue::AddressOf(Place::Local(x)),
                            },
                        );
                        push(
                            &mut out,
                            AirStmtKind::Assign {
                                place: Place::Local(s),
                                rvalue: Rvalue::Call {
                                    func: callee,
                                    args: vec![Operand::Copy(p)],
                                },
                            },
                        );
                        push(
                            &mut out,
                            AirStmtKind::CallVoid {
                                func: Callee::Named(name),
                                args: vec![Operand::Copy(s)],
                            },
                        );
                        push(
                            &mut out,
                            AirStmtKind::Assign {
                                place: Place::Local(ps),
                                rvalue: Rvalue::AddressOf(Place::Local(s)),
                            },
                        );
                        push(
                            &mut out,
                            AirStmtKind::CallVoid {
                                func: Callee::Named(crate::counts::STR_RELEASE.to_string()),
                                args: vec![Operand::Copy(ps)],
                            },
                        );
                    }
                    AirStmtKind::Assign {
                        place,
                        rvalue:
                            Rvalue::Call {
                                func: Callee::Named(name),
                                args,
                            },
                    } if name == "__aelys_to_string" && args.len() == 1 => {
                        let ty = ty_of(&args[0]);
                        let (Some(callee), Operand::Copy(x) | Operand::Move(x)) =
                            (self.display_callee(&ty), &args[0])
                        else {
                            out.push(stmt);
                            continue;
                        };
                        let x = *x;
                        let place = place.clone();
                        let p = fresh(AirType::Ptr(Box::new(ty.clone())), &mut new_locals);
                        push(
                            &mut out,
                            AirStmtKind::Assign {
                                place: Place::Local(p),
                                rvalue: Rvalue::AddressOf(Place::Local(x)),
                            },
                        );
                        push(
                            &mut out,
                            AirStmtKind::Assign {
                                place,
                                rvalue: Rvalue::Call {
                                    func: callee,
                                    args: vec![Operand::Copy(p)],
                                },
                            },
                        );
                    }
                    _ => out.push(stmt),
                }
            }
            block.stmts = out;
        }
        func.locals.extend(new_locals);
    }

    fn display_callee(&self, ty: &AirType) -> Option<Callee> {
        if !matches!(ty, AirType::Struct(_) | AirType::Enum(_)) {
            return None;
        }
        self.call_for("Display", "to_string", ty)
    }
}

impl MonoContext {
    pub(super) fn collect_dyn_pair(&mut self, from: &AirType, to: &AirType, program: &AirProgram) {
        let Some(q) = crate::objects::dyn_trait_behind(to) else {
            return;
        };
        let (AirType::Ptr(self_ty) | AirType::Rc(self_ty)) = from else {
            return;
        };
        if crate::objects::dyn_trait(self_ty).is_some()
            || self.dyn_pairs.iter().any(|(t, s)| t == q && *s == **self_ty)
        {
            return;
        }
        self.dyn_pairs.push((q.to_string(), (**self_ty).clone()));
        for m in program.dyn_tables.slots_of(q).unwrap_or_default() {
            let stub = format!("{STUB}{q}${m}");
            let type_args = vec![(**self_ty).clone()];
            if !self.generic_functions.contains_key(&stub) {
                self.errors.push(MonoError::mono(
                    aelys_common::Fault::Compiler,
                    format!("the vtable of `{}` for `{}` has no stub for `{m}`", shown_trait(q), shown_type(self_ty)),
                ));
                continue;
            }
            if !self.instantiated.contains_key(&(stub.clone(), self.type_args_key(&type_args))) {
                self.requests.push(super::MonoRequest {
                    function_name: stub,
                    type_args,
                });
            }
        }
    }

    pub(super) fn build_vtables(&self, program: &mut AirProgram) -> Result<(), Vec<MonoError>> {
        let mut vtables = Vec::with_capacity(self.dyn_pairs.len());
        for (q, self_ty) in &self.dyn_pairs {
            let key = self.type_args_key(std::slice::from_ref(self_ty));
            let fns = program
                .dyn_tables
                .slots_of(q)
                .unwrap_or_default()
                .iter()
                .map(|m| {
                    self.instantiated
                        .get(&(format!("{STUB}{q}${m}"), key.clone()))
                        .cloned()
                })
                .collect::<Option<Vec<String>>>();
            let Some(fns) = fns else {
                return Err(vec![MonoError::mono(
                    aelys_common::Fault::Compiler,
                    format!(
                        "the vtable of `{}` for `{}` has a method no instance answers",
                        shown_trait(q),
                        shown_type(self_ty)
                    ),
                )]);
            };
            vtables.push(crate::objects::AirVtable {
                trait_q: q.clone(),
                self_ty: self_ty.clone(),
                fns,
            });
        }
        program.dyn_tables.vtables = vtables;
        Ok(())
    }
}
