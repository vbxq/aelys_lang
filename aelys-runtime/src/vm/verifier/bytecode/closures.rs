use crate::vm::{Function, OpCode};

use super::{verify_const, verify_reg, verify_upval};

pub(super) fn verify(
    func: &Function,
    opcode: OpCode,
    a: usize,
    b: usize,
    c: usize,
    num_regs: usize,
    constants_len: usize,
    upvalues_len: usize,
) -> Result<bool, String> {
    match opcode {
        OpCode::MakeClosure => {
            verify_reg(a, num_regs, "MakeClosure")?;
            verify_const(b, constants_len, "MakeClosure")?;
            let constant = func.constants[b];
            let func_idx = match constant.as_nested_fn_marker() {
                Some(idx) => idx,
                None => {
                    return Err(format!(
                        "MakeClosure constant {} is not a nested function marker",
                        b
                    ));
                }
            };
            if func_idx >= func.nested_functions.len() {
                return Err(format!(
                    "MakeClosure nested function index {} out of bounds",
                    func_idx
                ));
            }
            let nested = &func.nested_functions[func_idx];
            if nested.upvalue_descriptors.len() != c {
                return Err(format!(
                    "MakeClosure upvalue count {} does not match descriptors {}",
                    c,
                    nested.upvalue_descriptors.len()
                ));
            }
        }
        OpCode::GetUpval => {
            verify_reg(a, num_regs, "GetUpval")?;
            verify_upval(b, upvalues_len, "GetUpval")?;
        }
        OpCode::SetUpval => {
            verify_upval(a, upvalues_len, "SetUpval")?;
            verify_reg(b, num_regs, "SetUpval")?;
        }
        OpCode::CloseUpvals => {
            verify_reg(a, num_regs, "CloseUpvals")?;
        }
        _ => return Ok(false),
    }

    Ok(true)
}
