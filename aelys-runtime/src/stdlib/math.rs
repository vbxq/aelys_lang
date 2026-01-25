//! std.math - Mathematical functions and constants

use crate::stdlib::helpers::get_number;
use crate::stdlib::{StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

pub const PI: f64 = std::f64::consts::PI;
pub const E: f64 = std::f64::consts::E;
pub const TAU: f64 = std::f64::consts::TAU; // 2*PI, for the tau manifesto folks

pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut all_exports = Vec::new();
    let mut native_functions = Vec::new();

    // Helper to register a constant
    macro_rules! reg_const {
        ($name:expr, $val:expr) => {{
            register_constant(vm, $name, $val);
            all_exports.push($name.to_string());
        }};
    }

    // Helper to register a native function
    macro_rules! reg_fn {
        ($name:expr, $arity:expr, $func:expr) => {{
            register_native(vm, "math", $name, $arity, $func)?;
            all_exports.push($name.to_string());
            native_functions.push(format!("math::{}", $name));
        }};
    }

    // Constants
    reg_const!("PI", Value::float(PI));
    reg_const!("E", Value::float(E));
    reg_const!("TAU", Value::float(TAU));
    reg_const!("INF", Value::float(f64::INFINITY));
    reg_const!("NEG_INF", Value::float(f64::NEG_INFINITY));

    // Basic functions
    reg_fn!("sqrt", 1, native_sqrt);
    reg_fn!("cbrt", 1, native_cbrt);
    reg_fn!("abs", 1, native_abs);
    reg_fn!("sign", 1, native_sign);

    // Trigonometric functions
    reg_fn!("sin", 1, native_sin);
    reg_fn!("cos", 1, native_cos);
    reg_fn!("tan", 1, native_tan);
    reg_fn!("asin", 1, native_asin);
    reg_fn!("acos", 1, native_acos);
    reg_fn!("atan", 1, native_atan);
    reg_fn!("atan2", 2, native_atan2);

    // Hyperbolic functions
    reg_fn!("sinh", 1, native_sinh);
    reg_fn!("cosh", 1, native_cosh);
    reg_fn!("tanh", 1, native_tanh);

    // Exponential and logarithmic functions
    reg_fn!("exp", 1, native_exp);
    reg_fn!("log", 1, native_log);
    reg_fn!("log10", 1, native_log10);
    reg_fn!("log2", 1, native_log2);
    reg_fn!("pow", 2, native_pow);

    // Rounding functions
    reg_fn!("floor", 1, native_floor);
    reg_fn!("ceil", 1, native_ceil);
    reg_fn!("round", 1, native_round);
    reg_fn!("trunc", 1, native_trunc);

    // Comparison functions
    reg_fn!("min", 2, native_min);
    reg_fn!("max", 2, native_max);
    reg_fn!("clamp", 3, native_clamp);

    // Angle conversion
    reg_fn!("deg_to_rad", 1, native_deg_to_rad);
    reg_fn!("rad_to_deg", 1, native_rad_to_deg);

    // Additional math functions
    reg_fn!("hypot", 2, native_hypot);
    reg_fn!("fmod", 2, native_fmod);
    reg_fn!("is_nan", 1, native_is_nan);
    reg_fn!("is_inf", 1, native_is_inf);
    reg_fn!("is_finite", 1, native_is_finite);

    Ok(StdModuleExports {
        all_exports,
        native_functions,
    })
}

fn register_constant(vm: &mut VM, name: &str, value: Value) {
    crate::stdlib::register_constant(vm, "math", name, value);
}

fn native_sqrt(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "sqrt")?;
    Ok(Value::float(x.sqrt()))
}

fn native_cbrt(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "cbrt")?;
    Ok(Value::float(x.cbrt()))
}

// Preserves type: abs(int) -> int, abs(float) -> float
fn native_abs(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(i) = args[0].as_int() {
        Ok(Value::int(i.abs()))
    } else if let Some(f) = args[0].as_float() {
        Ok(Value::float(f.abs()))
    } else {
        Err(vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "abs",
            expected: "number",
            got: vm.value_type_name(args[0]).to_string(),
        }))
    }
}

fn native_sign(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(i) = args[0].as_int() {
        Ok(Value::int(i.signum()))
    } else if let Some(f) = args[0].as_float() {
        if f.is_nan() {
            Ok(Value::float(f64::NAN))
        } else if f == 0.0 {
            Ok(Value::int(0))
        } else if f > 0.0 {
            Ok(Value::int(1))
        } else {
            Ok(Value::int(-1))
        }
    } else {
        Err(vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "sign",
            expected: "number",
            got: vm.value_type_name(args[0]).to_string(),
        }))
    }
}

// Trig functions - all in radians, obviously
fn native_sin(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "sin")?;
    Ok(Value::float(x.sin()))
}

fn native_cos(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "cos")?;
    Ok(Value::float(x.cos()))
}

fn native_tan(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "tan")?;
    Ok(Value::float(x.tan()))
}

fn native_asin(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "asin")?;
    Ok(Value::float(x.asin()))
}

fn native_acos(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "acos")?;
    Ok(Value::float(x.acos()))
}

fn native_atan(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "atan")?;
    Ok(Value::float(x.atan()))
}

// atan2 is the "proper" arctangent that handles quadrants correctly
fn native_atan2(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let y = get_number(vm, args[0], "atan2")?;
    let x = get_number(vm, args[1], "atan2")?;
    Ok(Value::float(y.atan2(x)))
}
// hyperbolic trig
fn native_sinh(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "sinh")?.sinh()))
}
fn native_cosh(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "cosh")?.cosh()))
}
fn native_tanh(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "tanh")?.tanh()))
}

fn native_exp(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "exp")?.exp()))
}

fn native_log(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "log")?;
    Ok(Value::float(x.ln())) // ln, not log - naming is confusing
}

fn native_log10(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "log10")?.log10()))
}

fn native_log2(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "log2")?.log2()))
}

fn native_pow(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    // try int pow for small exponents - better precision
    if let (Some(b), Some(e)) = (args[0].as_int(), args[1].as_int()) {
        if e >= 0 && e <= 62 {
            if let Some(r) = b.checked_pow(e as u32) { return Ok(Value::int(r)); }
        }
    }
    let base = get_number(vm, args[0], "pow")?;
    let exp = get_number(vm, args[1], "pow")?;
    Ok(Value::float(base.powf(exp)))
}

fn native_floor(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::int(get_number(vm, args[0], "floor")?.floor() as i64))
}

fn native_ceil(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "ceil")?;
    Ok(Value::int(x.ceil() as i64))
}

fn native_round(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let n = get_number(vm, args[0], "round")?;
    Ok(Value::int(n.round() as i64))
}
fn native_trunc(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "trunc")?;
    Ok(Value::int(x.trunc() as i64))
}

fn native_min(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    match (args[0].as_int(), args[1].as_int()) {
        (Some(a), Some(b)) => Ok(Value::int(a.min(b))),
        _ => {
            let a = get_number(vm, args[0], "min")?;
            let b = get_number(vm, args[1], "min")?;
            Ok(Value::float(a.min(b)))
        }
    }
}

fn native_max(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    // int fast path
    if let (Some(a), Some(b)) = (args[0].as_int(), args[1].as_int()) {
        return Ok(Value::int(a.max(b)));
    }
    Ok(Value::float(get_number(vm, args[0], "max")?.max(get_number(vm, args[1], "max")?)))
}

fn native_clamp(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if let (Some(x), Some(lo), Some(hi)) = (args[0].as_int(), args[1].as_int(), args[2].as_int()) {
        return Ok(Value::int(x.max(lo).min(hi)));
    }
    let x = get_number(vm, args[0], "clamp")?;
    let lo = get_number(vm, args[1], "clamp")?;
    let hi = get_number(vm, args[2], "clamp")?;
    Ok(Value::float(x.max(lo).min(hi)))
}

fn native_deg_to_rad(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "deg_to_rad")?.to_radians()))
}

fn native_rad_to_deg(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::float(get_number(vm, args[0], "rad_to_deg")?.to_degrees()))
}
fn native_hypot(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "hypot")?;
    let y = get_number(vm, args[1], "hypot")?;
    Ok(Value::float(x.hypot(y)))
}

/// fmod(x, y) - Floating-point remainder of x/y.
fn native_fmod(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = get_number(vm, args[0], "fmod")?;
    let y = get_number(vm, args[1], "fmod")?;
    Ok(Value::float(x % y))
}

/// is_nan(x) - Check if x is NaN.
fn native_is_nan(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(f) = args[0].as_float() {
        Ok(Value::bool(f.is_nan()))
    } else if args[0].as_int().is_some() {
        Ok(Value::bool(false)) // Integers are never NaN
    } else {
        Err(vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "is_nan",
            expected: "number",
            got: vm.value_type_name(args[0]).to_string(),
        }))
    }
}

/// is_inf(x) - Check if x is infinite.
fn native_is_inf(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(f) = args[0].as_float() {
        Ok(Value::bool(f.is_infinite()))
    } else if args[0].as_int().is_some() {
        Ok(Value::bool(false)) // Integers are never infinite
    } else {
        Err(vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "is_inf",
            expected: "number",
            got: vm.value_type_name(args[0]).to_string(),
        }))
    }
}

/// is_finite(x) - Check if x is finite (not NaN or infinite).
fn native_is_finite(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(f) = args[0].as_float() {
        Ok(Value::bool(f.is_finite()))
    } else if args[0].as_int().is_some() {
        Ok(Value::bool(true)) // Integers are always finite
    } else {
        Err(vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "is_finite",
            expected: "number",
            got: vm.value_type_name(args[0]).to_string(),
        }))
    }
}
