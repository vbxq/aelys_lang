use super::Compiler;
use aelys_bytecode::{OpCode, Value};
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;

// fits in 16-bit immediate?
pub(super) fn small_int_immediate(n: i64) -> Option<i16> {
    (n >= i16::MIN as i64 && n <= i16::MAX as i64).then(|| n as i16)
}

impl Compiler {
    pub fn compile_literal_int(&mut self, n: i64, dest: u8, span: Span) -> Result<()> {
        match small_int_immediate(n) {
            Some(imm) => {
                self.emit_b(OpCode::LoadI, dest, imm, span);
            }
            None => {
                let val = Value::int_checked(n).map_err(|_| {
                    CompileError::new(
                        CompileErrorKind::IntegerOverflow {
                            value: n.to_string(),
                            min: Value::INT_MIN,
                            max: Value::INT_MAX,
                        },
                        span,
                        self.source.clone(),
                    )
                })?;
                let k = self.add_constant(val, span)?;
                self.emit_b(OpCode::LoadK, dest, k as i16, span);
            }
        }
        Ok(())
    }

    pub fn compile_literal_float(&mut self, f: f64, dest: u8, span: Span) -> Result<()> {
        let k = self.add_constant(Value::float(f), span)?;
        self.emit_b(OpCode::LoadK, dest, k as i16, span);
        Ok(())
    }

    pub fn compile_literal_string(&mut self, s: &str, dest: u8, span: Span) -> Result<()> {
        let ref_ = self.heap.intern_string(s);
        let k = self.add_constant(Value::ptr(ref_.index()), span)?;
        self.emit_b(OpCode::LoadK, dest, k as i16, span);
        Ok(())
    }

    pub fn compile_literal_bool(&mut self, b: bool, dest: u8, span: Span) -> Result<()> {
        self.emit_a(OpCode::LoadBool, dest, if b { 1 } else { 0 }, 0, span);
        Ok(())
    }

    pub fn compile_literal_null(&mut self, dest: u8, span: Span) -> Result<()> {
        self.emit_a(OpCode::LoadNull, dest, 0, 0, span);
        Ok(())
    }
}
