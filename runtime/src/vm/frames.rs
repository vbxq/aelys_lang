use super::{CallFrame, MAX_FRAMES, MAX_REGISTERS, ObjectKind, VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

impl VM {
    pub fn push_frame(&mut self, frame: CallFrame) -> Result<(), RuntimeError> {
        if self.frames.len() >= MAX_FRAMES {
            return Err(self.runtime_error(RuntimeErrorKind::StackOverflow));
        }
        self.frames.push(frame);
        Ok(())
    }

    pub fn pop_frame(&mut self) -> Option<CallFrame> {
        self.frames.pop()
    }

    pub fn current_frame(&self) -> Result<&CallFrame, RuntimeError> {
        if self.frames.is_empty() {
            return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                "no call frame on stack".to_string(),
            )));
        }
        Ok(&self.frames[self.frames.len() - 1])
    }

    pub fn current_frame_mut(&mut self) -> Result<&mut CallFrame, RuntimeError> {
        if self.frames.is_empty() {
            return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                "no call frame on stack".to_string(),
            )));
        }
        Ok(self.frames.last_mut().expect("frame exists"))
    }

    pub fn has_frames(&self) -> bool {
        !self.frames.is_empty()
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn clear_frames(&mut self) {
        self.frames.clear();
    }

    pub fn read_register(&self, reg: u8) -> Result<Value, RuntimeError> {
        let frame = self.current_frame()?;
        let index = frame.register_index(reg).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::InvalidRegister {
                reg: usize::MAX,
                max: MAX_REGISTERS.saturating_sub(1),
            })
        })?;
        Ok(self.registers.get(index).copied().unwrap_or(Value::null()))
    }

    pub fn write_register(&mut self, reg: u8, value: Value) -> Result<(), RuntimeError> {
        let frame = self.current_frame()?;
        let index = frame.register_index(reg).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::InvalidRegister {
                reg: usize::MAX,
                max: MAX_REGISTERS.saturating_sub(1),
            })
        })?;
        if index >= self.registers.len() {
            if index >= MAX_REGISTERS {
                return Err(self.runtime_error(RuntimeErrorKind::InvalidRegister {
                    reg: index,
                    max: MAX_REGISTERS.saturating_sub(1),
                }));
            }
            self.registers.resize(index + 1, Value::null());
        }
        self.registers[index] = value;
        Ok(())
    }

    pub fn register_count(&self) -> usize {
        self.registers.len()
    }

    pub fn shrink_registers(&mut self) {
        if let Some(frame) = self.frames.last() {
            if let Some(obj) = self.heap.get(frame.function())
                && let ObjectKind::Function(func) = &obj.kind
            {
                let needed = frame.base() + func.num_registers() as usize;
                if needed < self.registers.len() {
                    self.registers.truncate(needed);
                }
            }
        } else {
            self.registers.clear();
        }
    }
}
