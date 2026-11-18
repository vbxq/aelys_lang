use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::modules::{Lookup, ModuleExports};
use crate::typed_ast::{ConstLit, ConstScalar};
use crate::types::InferType;
use aelys_syntax::{BinaryOp, Expr, ExprKind, Span, TypeAnnotation, UnaryOp};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
enum Val {
    Int(i128),
    Float(f64),
    Bool(bool),
    Char(u32),
    Str(String),
}

pub(super) struct ConstSource {
    pub ann: TypeAnnotation,
    pub init: Expr,
}

enum State {
    Busy,
    Done(Option<ConstLit>),
}

pub(super) struct ConstEval<'a> {
    pub sources: &'a HashMap<String, ConstSource>,
    pub imported: &'a HashMap<String, ConstLit>,
    pub namespaces: &'a HashMap<String, Arc<ModuleExports>>,
    pub errors: Vec<TypeError>,
    state: HashMap<String, State>,
    stack: Vec<String>,
}

struct Fail;

type R<T> = Result<T, Fail>;

pub(super) fn scalar_type(ann: &TypeAnnotation) -> Option<InferType> {
    if matches!(ann.reference, Some(aelys_syntax::RefKind::Shared))
        && ann.name == "str"
        && !ann.is_slice
    {
        return Some(InferType::StrRef);
    }
    if ann.reference.is_some()
        || ann.is_function_type()
        || ann.type_param.is_some()
        || !ann.type_params.is_empty()
        || ann.array_size.is_some()
        || ann.array_size_name.is_some()
    {
        return None;
    }
    let ty = InferType::from_name(&ann.name);
    (ty.is_numeric() || matches!(ty, InferType::Bool | InferType::Char | InferType::String))
        .then_some(ty)
}

fn width(ty: &InferType) -> Option<u32> {
    ty.int_bounds().map(|(lo, hi)| 128 - (hi - lo).leading_zeros())
}

fn signed(ty: &InferType) -> bool {
    matches!(
        ty,
        InferType::I8 | InferType::I16 | InferType::I32 | InferType::I64
    )
}

fn wrap(v: i128, ty: &InferType) -> Option<i128> {
    let w = width(ty)?;
    let mask = (1i128 << w) - 1;
    let low = v & mask;
    Some(if signed(ty) && low >= (1i128 << (w - 1)) {
        low - (1i128 << w)
    } else {
        low
    })
}

impl<'a> ConstEval<'a> {
    pub fn new(
        sources: &'a HashMap<String, ConstSource>,
        imported: &'a HashMap<String, ConstLit>,
        namespaces: &'a HashMap<String, Arc<ModuleExports>>,
    ) -> Self {
        Self {
            sources,
            imported,
            namespaces,
            errors: Vec::new(),
            state: HashMap::new(),
            stack: Vec::new(),
        }
    }

    pub fn eval_const(&mut self, name: &str) -> Option<ConstLit> {
        match self.state.get(name) {
            Some(State::Done(lit)) => return lit.clone(),
            Some(State::Busy) => return None,
            None => {}
        }
        let src = self.sources.get(name)?;
        self.state.insert(name.to_string(), State::Busy);
        self.stack.push(name.to_string());
        let lit = self.compute(name, src);
        self.stack.pop();
        self.state.insert(name.to_string(), State::Done(lit.clone()));
        lit
    }

    fn compute(&mut self, name: &str, src: &ConstSource) -> Option<ConstLit> {
        let Some(ty) = scalar_type(&src.ann) else {
            self.errors.push(TypeError::not_supported_yet(
                "a const holds a number, a bool, a char or a string; use a module-level let",
                src.ann.span,
            ));
            return None;
        };
        let (val, found) = self.eval(name, &src.init, Some(&ty)).ok()?;
        if found != ty && !(found.is_str() && ty.is_str()) {
            self.errors.push(mismatch(&ty, &found, src.init.span));
            return None;
        }
        let value = match val {
            Val::Int(v) => ConstScalar::Int(v as i64),
            Val::Float(v) => ConstScalar::Float(v),
            Val::Bool(b) => ConstScalar::Bool(b),
            Val::Char(c) => ConstScalar::Char(c),
            Val::Str(s) => ConstScalar::Str(s),
        };
        Some(ConstLit { ty, value })
    }

    fn fail(&mut self, name: &str, what: &str, span: Span) -> Fail {
        self.errors.push(TypeError::coded(
            "E0370",
            format!("the initializer of `{name}` {what}"),
            "not a constant",
            span,
        ));
        Fail
    }

    fn not_const(&mut self, name: &str, span: Span) -> Fail {
        let mut err = TypeError::coded(
            "E0370",
            format!("the initializer of `{name}` is not a constant expression"),
            "computed at run time",
            span,
        );
        err.help = Some(
            "a const initializer uses literals, other consts, arithmetic, comparisons, \
             `&&`, `||`, `!` and `as`"
                .to_string(),
        );
        self.errors.push(err);
        Fail
    }

    fn check_int(&mut self, name: &str, v: i128, ty: &InferType, span: Span) -> R<Val> {
        let Some((lo, hi)) = ty.int_bounds() else {
            return Err(self.not_const(name, span));
        };
        if v < lo || v > hi {
            return Err(self.fail(name, &format!("overflows `{ty}`"), span));
        }
        Ok(Val::Int(v))
    }

    fn natural(&mut self, owner: &str, e: &Expr) -> R<Option<InferType>> {
        Ok(match &e.kind {
            ExprKind::Bool(_) | ExprKind::And { .. } | ExprKind::Or { .. } => Some(InferType::Bool),
            ExprKind::Char(_) => Some(InferType::Char),
            ExprKind::String(_) => Some(InferType::StrRef),
            ExprKind::Identifier(_) | ExprKind::Member { .. } => {
                Some(self.eval(owner, e, None)?.1)
            }
            ExprKind::Cast { target, .. } => scalar_type(target),
            ExprKind::Grouping(inner) => self.natural(owner, inner)?,
            ExprKind::Unary { operand, .. } => self.natural(owner, operand)?,
            ExprKind::Binary { left, op, right } => match op {
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => Some(InferType::Bool),
                BinaryOp::Shl | BinaryOp::Shr => self.natural(owner, left)?,
                _ => match self.natural(owner, left)? {
                    Some(t) => Some(t),
                    None => self.natural(owner, right)?,
                },
            },
            _ => None,
        })
    }

    fn lookup_name(&mut self, owner: &str, name: &str, span: Span) -> R<(Val, InferType)> {
        if self.sources.contains_key(name) {
            if self.stack.iter().any(|n| n == name) {
                let start = self.stack.iter().position(|n| n == name).unwrap_or(0);
                let mut chain: Vec<String> = self.stack[start..].to_vec();
                chain.push(name.to_string());
                let head = chain[0].clone();
                self.errors.push(TypeError::coded(
                    "E0370",
                    format!(
                        "the const `{head}` refers to itself: {}",
                        chain.join(" -> ")
                    ),
                    "cycle",
                    span,
                ));
                return Err(Fail);
            }
            return match self.eval_const(name) {
                Some(lit) => Ok(from_lit(&lit)),
                None => Err(Fail),
            };
        }
        if let Some(lit) = self.imported.get(name) {
            return Ok(from_lit(lit));
        }
        Err(self.not_const(owner, span))
    }

    fn eval(&mut self, owner: &str, e: &Expr, want: Option<&InferType>) -> R<(Val, InferType)> {
        match &e.kind {
            ExprKind::Int(v) => match want {
                Some(t) if t.is_float() => Ok((Val::Float(*v as f64), t.clone())),
                Some(t) if t.is_integer() => {
                    Ok((self.check_int(owner, *v as i128, t, e.span)?, t.clone()))
                }
                _ => Ok((Val::Int(*v as i128), InferType::I64)),
            },
            ExprKind::Float(v) => match want {
                Some(InferType::F32) => Ok((Val::Float(*v as f32 as f64), InferType::F32)),
                _ => Ok((Val::Float(*v), InferType::F64)),
            },
            ExprKind::Bool(b) => Ok((Val::Bool(*b), InferType::Bool)),
            ExprKind::Char(c) => Ok((Val::Char(*c), InferType::Char)),
            ExprKind::String(s) => Ok((Val::Str(s.clone()), InferType::StrRef)),
            ExprKind::Identifier(name) => self.lookup_name(owner, name, e.span),
            ExprKind::Member { object, member } => {
                let ExprKind::Identifier(ns) = &object.kind else {
                    return Err(self.not_const(owner, e.span));
                };
                let Some(exports) = self.namespaces.get(ns).cloned() else {
                    return Err(self.not_const(owner, e.span));
                };
                match exports.value(member) {
                    Lookup::Found(item) => match self.imported.get(&item.qualified) {
                        Some(lit) => Ok(from_lit(lit)),
                        None => Err(self.not_const(owner, e.span)),
                    },
                    Lookup::NotPublic => {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::ModuleItemNotPublic {
                                module: exports.path.clone(),
                                item: member.clone(),
                            },
                            span: e.span,
                            reason: ConstraintReason::Other(String::new()),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                        Err(Fail)
                    }
                    Lookup::Missing => Err(self.not_const(owner, e.span)),
                }
            }
            ExprKind::Grouping(inner) => self.eval(owner, inner, want),
            ExprKind::Unary { op, operand } => {
                let (v, t) = self.eval(owner, operand, want)?;
                match (op, v) {
                    (UnaryOp::Neg, Val::Int(v)) => Ok((self.check_int(owner, -v, &t, e.span)?, t)),
                    (UnaryOp::Neg, Val::Float(v)) => Ok((Val::Float(-v), t)),
                    (UnaryOp::BitNot | UnaryOp::Not, Val::Int(v)) => {
                        let r = if signed(&t) { Some(!v) } else { wrap(!v, &t) };
                        let Some(r) = r else {
                            return Err(self.not_const(owner, e.span));
                        };
                        Ok((Val::Int(r), t))
                    }
                    (UnaryOp::Not, Val::Bool(b)) => Ok((Val::Bool(!b), t)),
                    _ => Err(self.not_const(owner, e.span)),
                }
            }
            ExprKind::And { left, right } | ExprKind::Or { left, right } => {
                let is_and = matches!(e.kind, ExprKind::And { .. });
                let (l, lt) = self.eval(owner, left, Some(&InferType::Bool))?;
                let (r, rt) = self.eval(owner, right, Some(&InferType::Bool))?;
                match (l, r) {
                    (Val::Bool(a), Val::Bool(b)) => {
                        Ok((Val::Bool(if is_and { a && b } else { a || b }), InferType::Bool))
                    }
                    _ => {
                        let found = if lt == InferType::Bool { rt } else { lt };
                        self.errors
                            .push(mismatch(&InferType::Bool, &found, e.span));
                        Err(Fail)
                    }
                }
            }
            ExprKind::Binary { left, op, right } => self.binary(owner, e, left, *op, right, want),
            ExprKind::Cast { expr, target } => {
                let Some(to) = scalar_type(target) else {
                    return Err(self.not_const(owner, e.span));
                };
                let hint = self.natural(owner, expr)?;
                let (v, _) = self.eval(owner, expr, hint.as_ref())?;
                let out = match v {
                    Val::Int(v) if to.is_integer() => match wrap(v, &to) {
                        Some(w) => Val::Int(w),
                        None => return Err(self.not_const(owner, e.span)),
                    },
                    Val::Int(v) if to.is_float() => Val::Float(v as f64),
                    Val::Float(f) if to.is_float() => {
                        Val::Float(if to == InferType::F32 { f as f32 as f64 } else { f })
                    }
                    Val::Float(f) if to.is_integer() => {
                        let t = f.trunc();
                        let Some((lo, hi)) = to.int_bounds() else {
                            return Err(self.not_const(owner, e.span));
                        };
                        if !t.is_finite() || t < lo as f64 || t > hi as f64 {
                            return Err(self.fail(owner, &format!("overflows `{to}`"), e.span));
                        }
                        Val::Int(t as i128)
                    }
                    Val::Bool(b) if to.is_integer() => Val::Int(b as i128),
                    Val::Char(c) if to.is_integer() => match wrap(c as i128, &to) {
                        Some(w) => Val::Int(w),
                        None => return Err(self.not_const(owner, e.span)),
                    },
                    Val::Bool(b) if to == InferType::Bool => Val::Bool(b),
                    Val::Char(c) if to == InferType::Char => Val::Char(c),
                    _ => return Err(self.not_const(owner, e.span)),
                };
                Ok((out, to))
            }
            _ => Err(self.not_const(owner, e.span)),
        }
    }

    fn binary(
        &mut self,
        owner: &str,
        e: &Expr,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        want: Option<&InferType>,
    ) -> R<(Val, InferType)> {
        let compare = matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        );
        if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
            let hint = match self.natural(owner, left)? {
                Some(t) => Some(t),
                None => want.filter(|t| t.is_integer()).cloned(),
            };
            let (l, lt) = self.eval(owner, left, hint.as_ref())?;
            let rhint = self.natural(owner, right)?;
            let (r, _) = self.eval(owner, right, rhint.as_ref())?;
            let (Val::Int(l), Val::Int(r)) = (l, r) else {
                return Err(self.not_const(owner, e.span));
            };
            let Some(bits) = width(&lt) else {
                return Err(self.not_const(owner, e.span));
            };
            if r < 0 || r >= bits as i128 {
                return Err(self.fail(owner, &format!("overflows `{lt}`"), e.span));
            }
            let v = if op == BinaryOp::Shl {
                match wrap(l << r, &lt) {
                    Some(w) => w,
                    None => return Err(self.not_const(owner, e.span)),
                }
            } else {
                l >> r
            };
            return Ok((Val::Int(v), lt));
        }
        let hint = match self.natural(owner, left)? {
            Some(t) => Some(t),
            None => match self.natural(owner, right)? {
                Some(t) => Some(t),
                None if compare => None,
                None => want.cloned(),
            },
        };
        let (l, lt) = self.eval(owner, left, hint.as_ref())?;
        let (r, rt) = self.eval(owner, right, Some(&lt))?;
        if lt != rt && !(lt.is_str() && rt.is_str()) {
            self.errors.push(mismatch(&lt, &rt, right.span));
            return Err(Fail);
        }
        if compare {
            let ord = match (&l, &r) {
                (Val::Int(a), Val::Int(b)) => a.partial_cmp(b),
                (Val::Float(a), Val::Float(b)) => a.partial_cmp(b),
                (Val::Bool(a), Val::Bool(b)) => a.partial_cmp(b),
                (Val::Char(a), Val::Char(b)) => a.partial_cmp(b),
                (Val::Str(a), Val::Str(b)) if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                    a.partial_cmp(b)
                }
                _ => return Err(self.not_const(owner, e.span)),
            };
            use std::cmp::Ordering::*;
            let b = match (op, ord) {
                (_, None) => op == BinaryOp::Ne,
                (BinaryOp::Eq, Some(o)) => o == Equal,
                (BinaryOp::Ne, Some(o)) => o != Equal,
                (BinaryOp::Lt, Some(o)) => o == Less,
                (BinaryOp::Le, Some(o)) => o != Greater,
                (BinaryOp::Gt, Some(o)) => o == Greater,
                (_, Some(o)) => o != Less,
            };
            return Ok((Val::Bool(b), InferType::Bool));
        }
        match (l, r) {
            (Val::Int(a), Val::Int(b)) => {
                let v = match op {
                    BinaryOp::Add => a + b,
                    BinaryOp::Sub => a - b,
                    BinaryOp::Mul => match a.checked_mul(b) {
                        Some(v) => v,
                        None => return Err(self.fail(owner, &format!("overflows `{lt}`"), e.span)),
                    },
                    BinaryOp::Div | BinaryOp::Mod if b == 0 => {
                        return Err(self.fail(owner, "performs a division by zero", e.span));
                    }
                    BinaryOp::Div => a / b,
                    BinaryOp::Mod => a % b,
                    BinaryOp::BitAnd => a & b,
                    BinaryOp::BitOr => a | b,
                    BinaryOp::BitXor => a ^ b,
                    _ => return Err(self.not_const(owner, e.span)),
                };
                if op == BinaryOp::Mod
                    && lt.int_bounds().is_some_and(|(lo, _)| a == lo)
                    && b == -1
                    && signed(&lt)
                {
                    return Err(self.fail(owner, &format!("overflows `{lt}`"), e.span));
                }
                Ok((self.check_int(owner, v, &lt, e.span)?, lt))
            }
            (Val::Float(a), Val::Float(b)) => {
                let v = match op {
                    BinaryOp::Add => a + b,
                    BinaryOp::Sub => a - b,
                    BinaryOp::Mul => a * b,
                    BinaryOp::Div => a / b,
                    BinaryOp::Mod => a % b,
                    _ => return Err(self.not_const(owner, e.span)),
                };
                let v = if lt == InferType::F32 { v as f32 as f64 } else { v };
                Ok((Val::Float(v), lt))
            }
            _ => Err(self.not_const(owner, e.span)),
        }
    }
}

fn from_lit(lit: &ConstLit) -> (Val, InferType) {
    let v = match &lit.value {
        ConstScalar::Int(v) if lit.ty == InferType::U64 => Val::Int(*v as u64 as i128),
        ConstScalar::Int(v) => Val::Int(*v as i128),
        ConstScalar::Float(v) => Val::Float(*v),
        ConstScalar::Bool(b) => Val::Bool(*b),
        ConstScalar::Char(c) => Val::Char(*c),
        ConstScalar::Str(s) => Val::Str(s.clone()),
    };
    (v, lit.ty.clone())
}

fn mismatch(expected: &InferType, found: &InferType, span: Span) -> TypeError {
    TypeError {
        kind: TypeErrorKind::Mismatch {
            expected: expected.clone(),
            found: found.clone(),
        },
        span,
        reason: ConstraintReason::Other(String::new()),
        secondary_spans: Vec::new(),
        help: None,
        suggestion: None,
    }
}
