use super::super::TypeInference;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart, TypedStmt, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{BinaryOp, Span, UnaryOp};

fn ident(name: &str, ty: &InferType, span: Span) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Identifier(name.to_string()),
        ty: ty.clone(),
        span,
    }
}

fn element(of: &TypedExpr, i: usize, ty: &InferType) -> TypedExpr {
    TypedExpr {
        kind: TypedExprKind::Member {
            object: Box::new(of.clone()),
            member: i.to_string(),
        },
        ty: ty.clone(),
        span: of.span,
    }
}

fn bind(name: &str, value: TypedExpr) -> TypedStmt {
    let span = value.span;
    TypedStmt {
        kind: TypedStmtKind::Let {
            name: name.to_string(),
            mutable: false,
            var_type: value.ty.clone(),
            initializer: value,
            is_pub: false,
        },
        span,
    }
}

fn block(stmts: Vec<TypedStmt>, tail: TypedExpr, span: Span) -> TypedExpr {
    let ty = tail.ty.clone();
    TypedExpr {
        kind: TypedExprKind::Block {
            stmts,
            tail: Box::new(tail),
        },
        ty,
        span,
    }
}

fn leaf_pairs(l: &TypedExpr, r: &TypedExpr, out: &mut Vec<(TypedExpr, TypedExpr)>) {
    match (&l.ty, &r.ty) {
        (InferType::Tuple(ls), InferType::Tuple(rs)) if ls.len() == rs.len() => {
            for (i, (lt, rt)) in ls.iter().zip(rs.iter()).enumerate() {
                leaf_pairs(&element(l, i, lt), &element(r, i, rt), out);
            }
        }
        _ => out.push((l.clone(), r.clone())),
    }
}

impl TypeInference {
    pub(super) fn tuple_equality(
        &self,
        left: TypedExpr,
        op: BinaryOp,
        right: TypedExpr,
        span: Span,
    ) -> Option<TypedExpr> {
        if !matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
            return None;
        }
        let (InferType::Tuple(ls), InferType::Tuple(rs)) = (&left.ty, &right.ty) else {
            return None;
        };
        if ls.len() != rs.len() {
            return None;
        }
        let lname = format!("__eq${}${}$l", span.start, span.end);
        let rname = format!("__eq${}${}$r", span.start, span.end);
        let lid = ident(&lname, &left.ty, span);
        let rid = ident(&rname, &right.ty, span);
        let mut pairs = Vec::new();
        leaf_pairs(&lid, &rid, &mut pairs);
        let mut chain: Option<TypedExpr> = None;
        for (a, b) in pairs {
            let test = TypedExpr {
                kind: TypedExprKind::Binary {
                    left: Box::new(a),
                    op: BinaryOp::Eq,
                    right: Box::new(b),
                },
                ty: InferType::Bool,
                span,
            };
            chain = Some(match chain {
                None => test,
                Some(prev) => TypedExpr {
                    kind: TypedExprKind::And {
                        left: Box::new(prev),
                        right: Box::new(test),
                    },
                    ty: InferType::Bool,
                    span,
                },
            });
        }
        let mut tail = chain?;
        if op == BinaryOp::Ne {
            tail = TypedExpr {
                kind: TypedExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(tail),
                },
                ty: InferType::Bool,
                span,
            };
        }
        Some(block(vec![bind(&lname, left), bind(&rname, right)], tail, span))
    }

    fn printable_leaves(&self, ty: &InferType) -> bool {
        match ty {
            InferType::Tuple(elems) => elems.iter().all(|e| self.printable_leaves(e)),
            InferType::Struct(name, _) => self.type_table.has_struct(name),
            InferType::Var(_) | InferType::Dynamic => false,
            _ => true,
        }
    }

    fn tuple_parts(&self, value: &TypedExpr, out: &mut Vec<TypedFmtStringPart>) {
        match &value.ty {
            InferType::Tuple(elems) => {
                out.push(TypedFmtStringPart::Literal("(".to_string()));
                for (i, ty) in elems.iter().enumerate() {
                    if i > 0 {
                        out.push(TypedFmtStringPart::Literal(", ".to_string()));
                    }
                    self.tuple_parts(&element(value, i, ty), out);
                }
                out.push(TypedFmtStringPart::Literal(")".to_string()));
            }
            InferType::String | InferType::StrRef => {
                out.push(TypedFmtStringPart::Literal("\"".to_string()));
                out.push(TypedFmtStringPart::Expr(Box::new(value.clone())));
                out.push(TypedFmtStringPart::Literal("\"".to_string()));
            }
            InferType::Char => {
                out.push(TypedFmtStringPart::Literal("'".to_string()));
                out.push(TypedFmtStringPart::Expr(Box::new(value.clone())));
                out.push(TypedFmtStringPart::Literal("'".to_string()));
            }
            _ => out.push(TypedFmtStringPart::Expr(Box::new(value.clone()))),
        }
    }

    pub(super) fn tuple_format(
        &self,
        parts: Vec<TypedFmtStringPart>,
        span: Span,
    ) -> Result<TypedExpr, Vec<TypedFmtStringPart>> {
        let has_tuple = parts.iter().any(|p| {
            matches!(p, TypedFmtStringPart::Expr(e)
                if matches!(e.ty, InferType::Tuple(_)) && self.printable_leaves(&e.ty))
        });
        if !has_tuple {
            return Err(parts);
        }
        Ok(self.format_block(parts, span))
    }

    fn format_block(&self, parts: Vec<TypedFmtStringPart>, span: Span) -> TypedExpr {
        let mut stmts = Vec::new();
        let mut out = Vec::new();
        for (i, part) in parts.into_iter().enumerate() {
            match part {
                TypedFmtStringPart::Expr(e) => {
                    let name = format!("__fmt${}${}${i}", span.start, span.end);
                    let id = ident(&name, &e.ty, e.span);
                    stmts.push(bind(&name, *e));
                    self.tuple_parts(&id, &mut out);
                }
                other => out.push(other),
            }
        }
        let tail = TypedExpr {
            kind: TypedExprKind::FmtString(out),
            ty: InferType::String,
            span,
        };
        block(stmts, tail, span)
    }

    pub(super) fn tuple_print_arg(&self, arg: TypedExpr) -> TypedExpr {
        if !matches!(arg.ty, InferType::Tuple(_)) || !self.printable_leaves(&arg.ty) {
            return arg;
        }
        let span = arg.span;
        self.format_block(vec![TypedFmtStringPart::Expr(Box::new(arg))], span)
    }
}
