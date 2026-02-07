// lt/le/gt/ge - eq/ne inlined in dispatch

use super::VM;
use super::Value;
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

impl VM {
    /// Less than comparison.
    pub fn compare_lt(&self, left: Value, right: Value) -> Result<bool, RuntimeError> {
        // Fast path: both integers
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return Ok(a < b);
        }

        // Float < Float
        if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
            return Ok(a < b);
        }

        // Int < Float or Float < Int
        if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
            return Ok((a as f64) < b);
        }
        if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
            return Ok(a < (b as f64));
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "comparison",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Less than or equal comparison.
    pub fn compare_le(&self, left: Value, right: Value) -> Result<bool, RuntimeError> {
        // Fast path: both integers
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return Ok(a <= b);
        }

        // Float <= Float
        if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
            return Ok(a <= b);
        }

        // Int <= Float or Float <= Int
        if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
            return Ok((a as f64) <= b);
        }
        if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
            return Ok(a <= (b as f64));
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "comparison",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Greater than comparison.
    pub fn compare_gt(&self, left: Value, right: Value) -> Result<bool, RuntimeError> {
        // Fast path: both integers
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return Ok(a > b);
        }

        // Float > Float
        if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
            return Ok(a > b);
        }

        // Int > Float or Float > Int
        if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
            return Ok((a as f64) > b);
        }
        if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
            return Ok(a > (b as f64));
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "comparison",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Greater than or equal comparison.
    pub fn compare_ge(&self, left: Value, right: Value) -> Result<bool, RuntimeError> {
        // Fast path: both integers
        if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
            return Ok(a >= b);
        }

        // Float >= Float
        if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
            return Ok(a >= b);
        }

        // Int >= Float or Float >= Int
        if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
            return Ok((a as f64) >= b);
        }
        if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
            return Ok(a >= (b as f64));
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "comparison",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }
}
