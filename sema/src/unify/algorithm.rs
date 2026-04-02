use super::error::UnifyResult;
use super::occurs::occurs_check;
use super::{Substitution, UnifyError};
use crate::types::InferType;

pub fn unify(t1: &InferType, t2: &InferType, subst: &mut Substitution) -> UnifyResult<()> {
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
            // For generic enums, unify type arguments pairwise.
            // If one side has type args and the other doesn't (e.g., Enum("Option", []) from a variant constructor vs Enum("Option", [I64]) from an annotation), we accept  the match, the type args are informational for monomorphization, not for semantic equality.
            if !args_a.is_empty() && !args_b.is_empty() && args_a.len() == args_b.len() {
                for (a_arg, b_arg) in args_a.iter().zip(args_b.iter()) {
                    unify(a_arg, b_arg, subst)?;
                }
            }
            Ok(())
        }

        (InferType::Dynamic, _) | (_, InferType::Dynamic) => Ok(()),

        // Never is the bottom type (diverging control flow). It unifies with
        // any type T without binding type variables, because a Never-typed
        // expression never produces a value. 
        // 
        // So it's placed before the Var arms so that `unify(Never, Var(v))` succeeds without binding v, letting other constraints determine the variable's actual type.
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

        (InferType::Array(inner1, len1), InferType::Array(inner2, len2)) => {
            // both known lengths must match; if either is None (unsized), just unify inner
            match (len1, len2) {
                (Some(n1), Some(n2)) if n1 != n2 => {
                    return Err(UnifyError::Mismatch(t1.clone(), t2.clone()));
                }
                _ => {}
            }
            unify(inner1, inner2, subst)
        }

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
