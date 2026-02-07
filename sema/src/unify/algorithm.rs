use super::error::UnifyResult;
use super::occurs::occurs_check;
use super::{Substitution, UnifyError};
use crate::types::InferType;

// standard HM unification
pub fn unify(t1: &InferType, t2: &InferType, subst: &mut Substitution) -> UnifyResult<()> {
    let t1 = subst.apply(t1);
    let t2 = subst.apply(t2);

    match (&t1, &t2) {
        // same concrete types - ok
        (InferType::Int, InferType::Int) => Ok(()),
        (InferType::Float, InferType::Float) => Ok(()),
        (InferType::Bool, InferType::Bool) => Ok(()),
        (InferType::String, InferType::String) => Ok(()),
        (InferType::Null, InferType::Null) => Ok(()),

        // dynamic unifies with anything (gradual typing)
        (InferType::Dynamic, _) | (_, InferType::Dynamic) => Ok(()),

        (InferType::Var(id1), InferType::Var(id2)) if id1 == id2 => Ok(()),

        // type var on left
        (InferType::Var(v), ty) => {
            if *ty != InferType::Dynamic && occurs_check(*v, ty) {
                return Err(UnifyError::InfiniteType(*v, ty.clone()));
            }
            subst.bind(*v, ty.clone());
            Ok(())
        }

        // type var on right - same thing
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
            },
            InferType::Function {
                params: p2,
                ret: r2,
            },
        ) => {
            if p1.len() != p2.len() {
                return Err(UnifyError::ArityMismatch(p1.len(), p2.len()));
            }

            for (param1, param2) in p1.iter().zip(p2.iter()) {
                unify(param1, param2, subst)?;
            }

            unify(r1, r2, subst)
        }

        (InferType::Array(inner1), InferType::Array(inner2)) => unify(inner1, inner2, subst),

        (InferType::Vec(inner1), InferType::Vec(inner2)) => unify(inner1, inner2, subst),

        (InferType::Range, InferType::Range) => Ok(()),

        (InferType::Tuple(elems1), InferType::Tuple(elems2)) => {
            if elems1.len() != elems2.len() {
                return Err(UnifyError::Mismatch(t1.clone(), t2.clone()));
            }

            for (e1, e2) in elems1.iter().zip(elems2.iter()) {
                unify(e1, e2, subst)?;
            }

            Ok(())
        }

        _ => Err(UnifyError::Mismatch(t1.clone(), t2.clone())),
    }
}
