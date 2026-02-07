use super::super::VM;
use super::super::{GcRef, ObjectKind, Value};
use aelys_common::error::RuntimeError;

impl VM {
    pub(super) fn try_concat_strings(
        &mut self,
        left: Value,
        right: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let (Some(left_ptr), Some(right_ptr)) = (left.as_ptr(), right.as_ptr()) else {
            return Ok(None);
        };

        let concatenated = {
            let left_obj = self.heap.get(GcRef::new(left_ptr));
            let right_obj = self.heap.get(GcRef::new(right_ptr));

            if let (Some(l), Some(r)) = (left_obj, right_obj) {
                if let (ObjectKind::String(ls), ObjectKind::String(rs)) = (&l.kind, &r.kind) {
                    Some(format!("{}{}", ls.as_str(), rs.as_str()))
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(result) = concatenated {
            self.maybe_collect();
            let str_ref = self.alloc_string(&result)?;
            return Ok(Some(Value::ptr(str_ref.index())));
        }

        Ok(None)
    }
}
