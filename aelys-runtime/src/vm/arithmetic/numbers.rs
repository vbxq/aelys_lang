use super::super::VM;
use super::super::Value;
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

pub(super) fn try_add_numbers(left: Value, right: Value) -> Option<Value> {
    if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
        return Some(Value::int(a.wrapping_add(b)));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
        return Some(Value::float(a + b));
    }
    if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
        return Some(Value::float(a as f64 + b));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
        return Some(Value::float(a + b as f64));
    }
    None
}

pub(super) fn try_sub_numbers(left: Value, right: Value) -> Option<Value> {
    if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
        return Some(Value::int(a.wrapping_sub(b)));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
        return Some(Value::float(a - b));
    }
    if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
        return Some(Value::float(a as f64 - b));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
        return Some(Value::float(a - b as f64));
    }
    None
}

pub(super) fn try_mul_numbers(left: Value, right: Value) -> Option<Value> {
    if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
        return Some(Value::int(a.wrapping_mul(b)));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
        return Some(Value::float(a * b));
    }
    if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
        return Some(Value::float(a as f64 * b));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
        return Some(Value::float(a * b as f64));
    }
    None
}

pub(super) fn try_div_numbers(
    vm: &VM,
    left: Value,
    right: Value,
) -> Result<Option<Value>, RuntimeError> {
    if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
        if b == 0 {
            return Err(vm.runtime_error(RuntimeErrorKind::DivisionByZero));
        }
        return Ok(Some(Value::int(a / b)));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
        return Ok(Some(Value::float(a / b)));
    }
    if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
        return Ok(Some(Value::float(a as f64 / b)));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
        return Ok(Some(Value::float(a / b as f64)));
    }
    Ok(None)
}

pub(super) fn try_mod_numbers(
    vm: &VM,
    left: Value,
    right: Value,
) -> Result<Option<Value>, RuntimeError> {
    if let (Some(a), Some(b)) = (left.as_int(), right.as_int()) {
        if b == 0 {
            return Err(vm.runtime_error(RuntimeErrorKind::DivisionByZero));
        }
        return Ok(Some(Value::int(a % b)));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
        return Ok(Some(Value::float(a % b)));
    }
    if let (Some(a), Some(b)) = (left.as_int(), right.as_float()) {
        return Ok(Some(Value::float(a as f64 % b)));
    }
    if let (Some(a), Some(b)) = (left.as_float(), right.as_int()) {
        return Ok(Some(Value::float(a % b as f64)));
    }
    Ok(None)
}
