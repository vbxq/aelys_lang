use super::error::UnifyResult;
use super::occurs::occurs_check;
use super::{Substitution, UnifyError};
use crate::types::InferType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Exact,
    Flow,
    FlowRev,
}

impl Dir {
    /// `&mut` weakens to `&`; `&` never strengthens to `&mut`
    fn accepts(self, left_mutable: bool, right_mutable: bool) -> bool {
        match self {
            Dir::Exact => left_mutable == right_mutable,
            Dir::Flow => left_mutable || !right_mutable,
            Dir::FlowRev => right_mutable || !left_mutable,
        }
    }

    fn mismatch(self, left: &InferType, right: &InferType, left_mutable: bool) -> UnifyError {
        let (found, required) = match self {
            Dir::Flow => (left, right),
            Dir::FlowRev => (right, left),
            Dir::Exact if left_mutable => (right, left),
            Dir::Exact => (left, right),
        };
        UnifyError::RefMutability(found.clone(), required.clone())
    }
}

pub fn unify(t1: &InferType, t2: &InferType, subst: &mut Substitution) -> UnifyResult<()> {
    unify_dir(t1, t2, subst, Dir::Exact)
}

// never is bottom for control flow, but as a type argument it is a slot the instantiation has to name
fn unify_type_arg(t1: &InferType, t2: &InferType, subst: &mut Substitution) -> UnifyResult<()> {
    match (subst.apply(t1), subst.apply(t2)) {
        (InferType::Var(v), InferType::Never) | (InferType::Never, InferType::Var(v)) => {
            subst.bind(v, InferType::Never);
            Ok(())
        }
        _ => unify(t1, t2, subst),
    }
}

pub fn unify_dir(
    t1: &InferType,
    t2: &InferType,
    subst: &mut Substitution,
    dir: Dir,
) -> UnifyResult<()> {
    let t1 = subst.apply(t1);
    let t2 = subst.apply(t2);

    match (&t1, &t2) {
        (InferType::I8, InferType::I8)
        | (InferType::I16, InferType::I16)
        | (InferType::I32, InferType::I32)
        | (InferType::I64, InferType::I64)
        | (InferType::U8, InferType::U8)
        | (InferType::U16, InferType::U16)
        | (InferType::U32, InferType::U32)
        | (InferType::U64, InferType::U64)
        | (InferType::F32, InferType::F32)
        | (InferType::F64, InferType::F64)
        | (InferType::Bool, InferType::Bool)
        | (InferType::String, InferType::String)
        | (InferType::Null, InferType::Null) => Ok(()),

        (InferType::Struct(a), InferType::Struct(b)) if a == b => Ok(()),
        (InferType::Enum(a, args_a), InferType::Enum(b, args_b)) if a == b => {
            // if one side has type args and the other doesn't (e.g., enum("option", []) from a variant constructor vs enum("option", [i64]) from an annotation), we accept the match, the type args are informational for monomorphization, not for semantic equality.
            if !args_a.is_empty() && !args_b.is_empty() && args_a.len() == args_b.len() {
                for (a_arg, b_arg) in args_a.iter().zip(args_b.iter()) {
                    unify_type_arg(a_arg, b_arg, subst)?;
                }
            }
            Ok(())
        }

        (InferType::Dynamic, _) | (_, InferType::Dynamic) => Ok(()),

        // never is the bottom type (diverging control flow). it unifies with
        (InferType::Never, _) | (_, InferType::Never) => Ok(()),

        (InferType::Var(id1), InferType::Var(id2)) if id1 == id2 => Ok(()),

        (InferType::Var(v), ty) => {
            if *ty != InferType::Dynamic && occurs_check(*v, ty) {
                return Err(UnifyError::InfiniteType(*v, ty.clone()));
            }
            subst.bind(*v, ty.clone());
            Ok(())
        }

        (ty, InferType::Var(v)) => {
            if *ty != InferType::Dynamic && occurs_check(*v, ty) {
                return Err(UnifyError::InfiniteType(*v, ty.clone()));
            }
            subst.bind(*v, ty.clone());
            Ok(())
        }

        (
            InferType::Function {
                params: p1,
                ret: r1,
                ..
            },
            InferType::Function {
                params: p2,
                ret: r2,
                ..
            },
        ) => {
            if p1.len() != p2.len() {
                return Err(UnifyError::ArityMismatch(p1.len(), p2.len()));
            }

            for (param1, param2) in p1.iter().zip(p2.iter()) {
                unify_dir(param1, param2, subst, dir)?;
            }

            unify_dir(r1, r2, subst, dir)
        }

        (InferType::Array(inner1, len1), InferType::Array(inner2, len2)) => {
            match (len1, len2) {
                (Some(n1), Some(n2)) if n1 != n2 => {
                    return Err(UnifyError::Mismatch(t1.clone(), t2.clone()));
                }
                _ => {}
            }
            unify(inner1, inner2, subst)
        }

        (InferType::Vec(inner1), InferType::Vec(inner2)) => unify(inner1, inner2, subst),

        (InferType::Rc(inner1), InferType::Rc(inner2)) => unify(inner1, inner2, subst),

        (
            InferType::Ref {
                referent: r1,
                mutable: m1,
            },
            InferType::Ref {
                referent: r2,
                mutable: m2,
            },
        ) => {
            if !dir.accepts(*m1, *m2) {
                return Err(dir.mismatch(&t1, &t2, *m1));
            }
            unify_dir(r1, r2, subst, Dir::Exact)
        }

        (
            InferType::Slice {
                elem: e1,
                mutable: m1,
            },
            InferType::Slice {
                elem: e2,
                mutable: m2,
            },
        ) => {
            if !dir.accepts(*m1, *m2) {
                return Err(dir.mismatch(&t1, &t2, *m1));
            }
            unify_dir(e1, e2, subst, Dir::Exact)
        }

        (InferType::Range, InferType::Range) => Ok(()),

        (InferType::Tuple(elems1), InferType::Tuple(elems2)) => {
            if elems1.len() != elems2.len() {
                return Err(UnifyError::Mismatch(t1.clone(), t2.clone()));
            }

            for (e1, e2) in elems1.iter().zip(elems2.iter()) {
                unify_dir(e1, e2, subst, dir)?;
            }

            Ok(())
        }

        _ => Err(UnifyError::Mismatch(t1.clone(), t2.clone())),
    }
}
