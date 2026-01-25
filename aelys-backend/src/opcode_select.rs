// pick typed or generic opcode based on resolved types

use aelys_bytecode::OpCode;
use aelys_sema::ResolvedType;
use aelys_syntax::ast::BinaryOp;

pub fn select_opcode(op: BinaryOp, left: &ResolvedType, right: &ResolvedType) -> OpCode {
    let needs_guard = left.needs_guard() || right.needs_guard();
    let left_inner = left.unwrap_uncertain();
    let right_inner = right.unwrap_uncertain();

    match (left_inner, right_inner) {
        (ResolvedType::Int, ResolvedType::Int) => {
            if needs_guard {
                select_guarded_int_opcode(op)
            } else {
                select_typed_int_opcode(op)
            }
        }
        (ResolvedType::Float, ResolvedType::Float) => {
            if needs_guard {
                select_guarded_float_opcode(op)
            } else {
                select_typed_float_opcode(op)
            }
        }
        (ResolvedType::Int, ResolvedType::Float) | (ResolvedType::Float, ResolvedType::Int) => {
            select_guarded_float_opcode(op)
        }
        (ResolvedType::Dynamic, _) | (_, ResolvedType::Dynamic) => select_generic_opcode(op),
        _ => select_generic_opcode(op),
    }
}

fn select_typed_int_opcode(op: BinaryOp) -> OpCode {
    match op {
        BinaryOp::Add => OpCode::AddII,
        BinaryOp::Sub => OpCode::SubII,
        BinaryOp::Mul => OpCode::MulII,
        BinaryOp::Div => OpCode::DivII,
        BinaryOp::Mod => OpCode::ModII,
        BinaryOp::Lt => OpCode::LtII,
        BinaryOp::Le => OpCode::LeII,
        BinaryOp::Gt => OpCode::GtII,
        BinaryOp::Ge => OpCode::GeII,
        BinaryOp::Eq => OpCode::EqII,
        BinaryOp::Ne => OpCode::NeII,
        BinaryOp::Shl => OpCode::ShlII,
        BinaryOp::Shr => OpCode::ShrII,
        BinaryOp::BitAnd => OpCode::AndII,
        BinaryOp::BitOr => OpCode::OrII,
        BinaryOp::BitXor => OpCode::XorII,
    }
}

fn select_typed_float_opcode(op: BinaryOp) -> OpCode {
    match op {
        BinaryOp::Add => OpCode::AddFF,
        BinaryOp::Sub => OpCode::SubFF,
        BinaryOp::Mul => OpCode::MulFF,
        BinaryOp::Div => OpCode::DivFF,
        BinaryOp::Mod => OpCode::ModFF,
        BinaryOp::Lt => OpCode::LtFF,
        BinaryOp::Le => OpCode::LeFF,
        BinaryOp::Gt => OpCode::GtFF,
        BinaryOp::Ge => OpCode::GeFF,
        BinaryOp::Eq => OpCode::EqFF,
        BinaryOp::Ne => OpCode::NeFF,
        BinaryOp::Shl => OpCode::Shl,
        BinaryOp::Shr => OpCode::Shr,
        BinaryOp::BitAnd => OpCode::BitAnd,
        BinaryOp::BitOr => OpCode::BitOr,
        BinaryOp::BitXor => OpCode::BitXor,
    }
}

fn select_guarded_int_opcode(op: BinaryOp) -> OpCode {
    match op {
        BinaryOp::Add => OpCode::AddIIG,
        BinaryOp::Sub => OpCode::SubIIG,
        BinaryOp::Mul => OpCode::MulIIG,
        BinaryOp::Div => OpCode::DivIIG,
        BinaryOp::Mod => OpCode::ModIIG,
        BinaryOp::Lt => OpCode::LtIIG,
        BinaryOp::Le => OpCode::LeIIG,
        BinaryOp::Gt => OpCode::GtIIG,
        BinaryOp::Ge => OpCode::GeIIG,
        BinaryOp::Eq => OpCode::EqIIG,
        BinaryOp::Ne => OpCode::NeIIG,
        BinaryOp::Shl => OpCode::ShlII,
        BinaryOp::Shr => OpCode::ShrII,
        BinaryOp::BitAnd => OpCode::AndII,
        BinaryOp::BitOr => OpCode::OrII,
        BinaryOp::BitXor => OpCode::XorII,
    }
}

fn select_guarded_float_opcode(op: BinaryOp) -> OpCode {
    match op {
        BinaryOp::Add => OpCode::AddFFG,
        BinaryOp::Sub => OpCode::SubFFG,
        BinaryOp::Mul => OpCode::MulFFG,
        BinaryOp::Div => OpCode::DivFFG,
        BinaryOp::Mod => OpCode::ModFFG,
        BinaryOp::Lt => OpCode::LtFFG,
        BinaryOp::Le => OpCode::LeFFG,
        BinaryOp::Gt => OpCode::GtFFG,
        BinaryOp::Ge => OpCode::GeFFG,
        BinaryOp::Eq => OpCode::EqFFG,
        BinaryOp::Ne => OpCode::NeFFG,
        BinaryOp::Shl => OpCode::Shl,
        BinaryOp::Shr => OpCode::Shr,
        BinaryOp::BitAnd => OpCode::BitAnd,
        BinaryOp::BitOr => OpCode::BitOr,
        BinaryOp::BitXor => OpCode::BitXor,
    }
}

fn select_generic_opcode(op: BinaryOp) -> OpCode {
    match op {
        BinaryOp::Add => OpCode::Add,
        BinaryOp::Sub => OpCode::Sub,
        BinaryOp::Mul => OpCode::Mul,
        BinaryOp::Div => OpCode::Div,
        BinaryOp::Mod => OpCode::Mod,
        BinaryOp::Lt => OpCode::Lt,
        BinaryOp::Le => OpCode::Le,
        BinaryOp::Gt => OpCode::Gt,
        BinaryOp::Ge => OpCode::Ge,
        BinaryOp::Eq => OpCode::Eq,
        BinaryOp::Ne => OpCode::Ne,
        BinaryOp::Shl => OpCode::Shl,
        BinaryOp::Shr => OpCode::Shr,
        BinaryOp::BitAnd => OpCode::BitAnd,
        BinaryOp::BitOr => OpCode::BitOr,
        BinaryOp::BitXor => OpCode::BitXor,
    }
}

pub fn compute_result_type(
    op: BinaryOp,
    left: &ResolvedType,
    right: &ResolvedType,
) -> ResolvedType {
    let left_inner = left.unwrap_uncertain();
    let right_inner = right.unwrap_uncertain();
    let make_uncertain = left.needs_guard() || right.needs_guard();

    let result = match op {
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
            match (left_inner, right_inner) {
                (ResolvedType::Int, ResolvedType::Int) => ResolvedType::Int,
                (ResolvedType::Float, ResolvedType::Float) => ResolvedType::Float,
                (ResolvedType::Int, ResolvedType::Float)
                | (ResolvedType::Float, ResolvedType::Int) => ResolvedType::Float,
                (ResolvedType::String, ResolvedType::String) if matches!(op, BinaryOp::Add) => {
                    ResolvedType::String
                }
                _ => ResolvedType::Dynamic,
            }
        }
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::Eq | BinaryOp::Ne => {
            ResolvedType::Bool
        }
        BinaryOp::Shl | BinaryOp::Shr | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
            ResolvedType::Int
        }
    };

    if make_uncertain && !matches!(result, ResolvedType::Bool | ResolvedType::Dynamic) {
        ResolvedType::Uncertain(Box::new(result))
    } else {
        result
    }
}
