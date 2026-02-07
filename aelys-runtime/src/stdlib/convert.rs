//! std.convert - Type conversion functions

use crate::stdlib::helpers::{get_int, get_string, make_int_checked, make_string};
use crate::stdlib::{StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut all_exports = Vec::new();
    let mut native_functions = Vec::new();

    macro_rules! reg_fn {
        ($name:expr, $arity:expr, $func:expr) => {{
            register_native(vm, "convert", $name, $arity, $func)?;
            all_exports.push($name.to_string());
            native_functions.push(format!("convert::{}", $name));
        }};
    }

    reg_fn!("parse_int", 1, native_parse_int);
    reg_fn!("parse_int_radix", 2, native_parse_int_radix);
    reg_fn!("parse_float", 1, native_parse_float);
    reg_fn!("parse_bool", 1, native_parse_bool);
    reg_fn!("to_string", 1, native_to_string);
    reg_fn!("to_hex", 1, native_to_hex);
    reg_fn!("to_binary", 1, native_to_binary);
    reg_fn!("to_octal", 1, native_to_octal);
    reg_fn!("to_radix", 2, native_to_radix);
    reg_fn!("to_int", 1, native_to_int);
    reg_fn!("to_float", 1, native_to_float);
    reg_fn!("to_bool", 1, native_to_bool);
    reg_fn!("ord", 1, native_ord);
    reg_fn!("chr", 1, native_chr);
    reg_fn!("type_of", 1, native_type_of);
    reg_fn!("is_int", 1, native_is_int);
    reg_fn!("is_float", 1, native_is_float);
    reg_fn!("is_string", 1, native_is_string);
    reg_fn!("is_bool", 1, native_is_bool);
    reg_fn!("is_null", 1, native_is_null);
    reg_fn!("is_function", 1, native_is_function);

    Ok(StdModuleExports {
        all_exports,
        native_functions,
    })
}

fn convert_error(vm: &VM, op: &'static str, msg: String) -> RuntimeError {
    vm.runtime_error(RuntimeErrorKind::TypeError {
        operation: op,
        expected: "valid conversion",
        got: msg,
    })
}

// Parse string to int. Handles 0x, 0o, 0b prefixes. Returns null on failure.
fn native_parse_int(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "convert.parse_int")?;
    let trimmed = s.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        match i64::from_str_radix(hex, 16) {
            Ok(n) => return Ok(Value::int(n)),
            Err(_) => return Ok(Value::null()),
        }
    }
    if let Some(oct) = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
    {
        match i64::from_str_radix(oct, 8) {
            Ok(n) => return Ok(Value::int(n)),
            Err(_) => return Ok(Value::null()),
        }
    }
    if let Some(bin) = trimmed
        .strip_prefix("0b")
        .or_else(|| trimmed.strip_prefix("0B"))
    {
        match i64::from_str_radix(bin, 2) {
            Ok(n) => return Ok(Value::int(n)),
            Err(_) => return Ok(Value::null()),
        }
    }

    match trimmed.parse::<i64>() {
        Ok(n) => Ok(Value::int(n)),
        Err(_) => {
            // Try parsing as float and truncating (with range check)
            match trimmed.parse::<f64>() {
                Ok(f) => make_int_checked(vm, f as i64, "convert.parse_int"),
                Err(_) => Ok(Value::null()),
            }
        }
    }
}

fn native_parse_int_radix(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "convert.parse_int_radix")?;
    let radix = get_int(vm, args[1], "convert.parse_int_radix")?;

    if !(2..=36).contains(&radix) {
        return Err(convert_error(
            vm,
            "convert.parse_int_radix",
            format!("radix must be 2-36, got {}", radix),
        ));
    }

    match i64::from_str_radix(s.trim(), radix as u32) {
        Ok(n) => Ok(Value::int(n)),
        Err(_) => Ok(Value::null()),
    }
}

fn native_parse_float(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "convert.parse_float")?;
    Ok(s.trim()
        .parse::<f64>()
        .map(Value::float)
        .unwrap_or(Value::null()))
}

// Lenient boolean parsing: true/false, 1/0, yes/no, on/off all work
fn native_parse_bool(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "convert.parse_bool")?;
    match s.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(Value::bool(true)),
        "false" | "0" | "no" | "off" => Ok(Value::bool(false)),
        _ => Ok(Value::null()),
    }
}

fn native_to_string(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    make_string(vm, &vm.value_to_string(args[0]))
}

fn native_to_hex(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let n = get_int(vm, args[0], "convert.to_hex")?;
    let s = if n < 0 {
        format!("-{:x}", -n)
    } else {
        format!("{:x}", n)
    };
    make_string(vm, &s)
}

fn native_to_binary(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let n = get_int(vm, args[0], "convert.to_binary")?;
    make_string(
        vm,
        &if n >= 0 {
            format!("{:b}", n)
        } else {
            format!("-{:b}", -n)
        },
    )
}

fn native_to_octal(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let n = get_int(vm, args[0], "convert.to_octal")?;
    if n >= 0 {
        make_string(vm, &format!("{:o}", n))
    } else {
        make_string(vm, &format!("-{:o}", -n))
    }
}

/// to_radix(n, radix) - Convert integer to string in given radix (2-36).
fn native_to_radix(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let n = get_int(vm, args[0], "convert.to_radix")?;
    let radix = get_int(vm, args[1], "convert.to_radix")?;

    if !(2..=36).contains(&radix) {
        return Err(convert_error(
            vm,
            "convert.to_radix",
            format!("radix must be 2-36, got {}", radix),
        ));
    }

    let digits = "0123456789abcdefghijklmnopqrstuvwxyz";
    let mut result = String::new();
    let mut num = if n < 0 { -(n as i128) } else { n as i128 };
    let radix = radix as i128;

    if num == 0 {
        return make_string(vm, "0");
    }

    while num > 0 {
        let digit = (num % radix) as usize;
        let ch = digits.as_bytes().get(digit).copied().ok_or_else(|| {
            convert_error(
                vm,
                "convert.to_radix",
                format!("digit out of range: {}", digit),
            )
        })?;
        result.insert(0, ch as char);
        num /= radix;
    }

    if n < 0 {
        result.insert(0, '-');
    }

    make_string(vm, &result)
}

/// to_int(value) - Convert to integer.
/// Strings are parsed, floats are truncated, bools become 0/1.
/// Returns an error if the result exceeds 48-bit range.
fn native_to_int(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let value = args[0];

    if let Some(i) = value.as_int() {
        return Ok(Value::int(i));
    }
    if let Some(f) = value.as_float() {
        return make_int_checked(vm, f as i64, "convert.to_int");
    }
    if let Some(b) = value.as_bool() {
        return Ok(Value::int(if b { 1 } else { 0 }));
    }
    if value.is_null() {
        return Ok(Value::int(0));
    }

    // Try string conversion
    if let Ok(s) = get_string(vm, value, "convert.to_int") {
        if let Ok(n) = s.trim().parse::<i64>() {
            return Ok(Value::int(n));
        }
        if let Ok(f) = s.trim().parse::<f64>() {
            return make_int_checked(vm, f as i64, "convert.to_int");
        }
    }

    Err(convert_error(
        vm,
        "convert.to_int",
        format!("cannot convert {} to int", vm.value_type_name(value)),
    ))
}

/// to_float(value) - Convert to float.
fn native_to_float(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let value = args[0];

    if let Some(f) = value.as_float() {
        return Ok(Value::float(f));
    }
    if let Some(i) = value.as_int() {
        return Ok(Value::float(i as f64));
    }
    if let Some(b) = value.as_bool() {
        return Ok(Value::float(if b { 1.0 } else { 0.0 }));
    }
    if value.is_null() {
        return Ok(Value::float(0.0));
    }

    // Try string conversion
    if let Ok(s) = get_string(vm, value, "convert.to_float")
        && let Ok(f) = s.trim().parse::<f64>()
    {
        return Ok(Value::float(f));
    }

    Err(convert_error(
        vm,
        "convert.to_float",
        format!("cannot convert {} to float", vm.value_type_name(value)),
    ))
}

/// to_bool(value) - Convert to boolean.
fn native_to_bool(_vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(args[0].is_truthy()))
}

/// ord(char) - Get Unicode code point of first character.
fn native_ord(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "convert.ord")?;
    match s.chars().next() {
        Some(c) => Ok(Value::int(c as i64)),
        None => Err(convert_error(vm, "convert.ord", "empty string".to_string())),
    }
}

/// chr(code) - Get character from Unicode code point.
fn native_chr(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let code = get_int(vm, args[0], "convert.chr")?;

    if !(0..=0x10FFFF).contains(&code) {
        return Err(convert_error(
            vm,
            "convert.chr",
            format!("invalid code point: {}", code),
        ));
    }

    match char::from_u32(code as u32) {
        Some(c) => Ok(make_string(vm, &c.to_string())?),
        None => Err(convert_error(
            vm,
            "convert.chr",
            format!("invalid code point: {}", code),
        )),
    }
}

/// type_of(value) - Get type name as string.
fn native_type_of(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let type_name = vm.value_type_name(args[0]);
    make_string(vm, type_name)
}

/// is_int(value) - Check if value is an integer.
fn native_is_int(_vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(args[0].is_int()))
}

/// is_float(value) - Check if value is a float.
fn native_is_float(_vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(args[0].is_float()))
}

/// is_string(value) - Check if value is a string.
fn native_is_string(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let is_string = get_string(vm, args[0], "").is_ok();
    Ok(Value::bool(is_string))
}

/// is_bool(value) - Check if value is a boolean.
fn native_is_bool(_vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(args[0].is_bool()))
}

/// is_null(value) - Check if value is null.
fn native_is_null(_vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(args[0].is_null()))
}

/// is_function(value) - Check if value is a function.
fn native_is_function(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    use crate::vm::{GcRef, ObjectKind};

    let is_func = if let Some(ptr) = args[0].as_ptr() {
        if let Some(obj) = vm.heap().get(GcRef::new(ptr)) {
            matches!(
                obj.kind,
                ObjectKind::Function(_) | ObjectKind::Closure(_) | ObjectKind::Native(_)
            )
        } else {
            false
        }
    } else {
        false
    };

    Ok(Value::bool(is_func))
}
