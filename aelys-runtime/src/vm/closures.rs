use super::VM;
use super::{AelysUpvalue, GcObject, GcRef, ObjectKind, UpvalueLocation, Value};
use aelys_common::error::RuntimeError;

impl VM {
    pub fn close_upvalues_from(&mut self, start_reg: usize) {
        let mut i = 0;
        while i < self.open_upvalues.len() {
            let upval_ref = self.open_upvalues[i];
            let should_close = if let Some(obj) = self.heap.get(upval_ref) {
                if let ObjectKind::Upvalue(upval) = &obj.kind {
                    if let UpvalueLocation::Open {
                        frame_base,
                        register,
                    } = upval.location
                    {
                        frame_base + register as usize >= start_reg
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if should_close {
                let value = self.get_upvalue_value(upval_ref);
                if let Some(obj) = self.heap.get_mut(upval_ref) {
                    if let ObjectKind::Upvalue(upval) = &mut obj.kind {
                        upval.location = UpvalueLocation::Closed(value);
                    }
                }
                self.open_upvalues.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Capture an upvalue for a local variable in the given frame.
    /// If an open upvalue for this location already exists, returns it.
    /// Otherwise creates a new open upvalue.
    pub fn capture_upvalue(
        &mut self,
        frame_base: usize,
        register: u8,
    ) -> Result<GcRef, RuntimeError> {
        // Check if we already have an open upvalue for this location
        for &upval_ref in &self.open_upvalues {
            if let Some(obj) = self.heap.get(upval_ref) {
                if let ObjectKind::Upvalue(upval) = &obj.kind {
                    if let UpvalueLocation::Open {
                        frame_base: fb,
                        register: reg,
                    } = upval.location
                    {
                        if fb == frame_base && reg == register {
                            return Ok(upval_ref);
                        }
                    }
                }
            }
        }

        // Create a new open upvalue
        let upvalue = AelysUpvalue::new_open(frame_base, register);
        let upval_ref = self.alloc_object(GcObject::new(ObjectKind::Upvalue(upvalue)))?;

        // Add to open upvalues list
        self.open_upvalues.push(upval_ref);

        Ok(upval_ref)
    }

    /// Get the value from an upvalue (handles both open and closed).
    pub fn get_upvalue_value(&self, upval_ref: GcRef) -> Value {
        if let Some(obj) = self.heap.get(upval_ref) {
            if let ObjectKind::Upvalue(upval) = &obj.kind {
                match &upval.location {
                    UpvalueLocation::Open {
                        frame_base,
                        register,
                    } => {
                        // Read from the live register
                        let idx = *frame_base + *register as usize;
                        if idx < self.registers.len() {
                            return self.registers[idx];
                        }
                    }
                    UpvalueLocation::Closed(value) => {
                        return *value;
                    }
                }
            }
        }
        Value::null()
    }

    /// Set the value of an upvalue (handles both open and closed).
    pub fn set_upvalue_value(&mut self, upval_ref: GcRef, value: Value) {
        // First check if it's open or closed
        let location = if let Some(obj) = self.heap.get(upval_ref) {
            if let ObjectKind::Upvalue(upval) = &obj.kind {
                upval.location.clone()
            } else {
                return;
            }
        } else {
            return;
        };

        match location {
            UpvalueLocation::Open {
                frame_base,
                register,
            } => {
                // Write to the live register
                let idx = frame_base + register as usize;
                if idx < self.registers.len() {
                    self.registers[idx] = value;
                }
            }
            UpvalueLocation::Closed(_) => {
                // Update the closed value
                if let Some(obj) = self.heap.get_mut(upval_ref) {
                    if let ObjectKind::Upvalue(upval) = &mut obj.kind {
                        upval.location = UpvalueLocation::Closed(value);
                    }
                }
            }
        }
    }

    /// Close all open upvalues that point to registers >= from_reg in the given frame.
    pub fn close_upvalues(&mut self, frame_base: usize, from_reg: u8) {
        let mut i = 0;
        while i < self.open_upvalues.len() {
            let upval_ref = self.open_upvalues[i];
            let should_close = if let Some(obj) = self.heap.get(upval_ref) {
                if let ObjectKind::Upvalue(upval) = &obj.kind {
                    if let UpvalueLocation::Open {
                        frame_base: fb,
                        register,
                    } = upval.location
                    {
                        fb == frame_base && register >= from_reg
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if should_close {
                // Read the current value from the register
                let value = self.get_upvalue_value(upval_ref);

                // Close the upvalue by storing the value
                if let Some(obj) = self.heap.get_mut(upval_ref) {
                    if let ObjectKind::Upvalue(upval) = &mut obj.kind {
                        upval.location = UpvalueLocation::Closed(value);
                    }
                }

                // Remove from open_upvalues list
                self.open_upvalues.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}
