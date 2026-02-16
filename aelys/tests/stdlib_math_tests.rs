mod common;
use common::*;

#[test]
fn math_constants_defined() {
    let code = r#"
if PI > 3.14 and PI < 3.15 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn math_e_constant() {
    let code = r#"
if E > 2.71 and E < 2.72 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn math_tau_is_two_pi() {
    let code = r#"
if TAU > 6.28 and TAU < 6.29 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn sqrt_basic() {
    let code = r#"
let r = sqrt(16.0)
if r > 3.9 and r < 4.1 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn sqrt_negative_is_nan() {
    let code = r#"
let r = sqrt(-1.0)
if is_nan(r) { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn cbrt_basic() {
    let code = r#"
let r = cbrt(27.0)
if r > 2.9 and r < 3.1 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn abs_int_preserves_type() {
    let code = r#"
abs(-42)
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn abs_float() {
    let code = r#"
let r = abs(-3.14)
if r > 3.13 and r < 3.15 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn sign_positive() {
    let code = r#"
sign(42)
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn sign_negative() {
    let code = r#"
sign(-10)
"#;
    assert_aelys_int(code, -1);
}

#[test]
fn sign_zero() {
    let code = r#"
sign(0)
"#;
    assert_aelys_int(code, 0);
}

#[test]
fn sin_zero() {
    let code = r#"
let r = sin(0.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn cos_zero() {
    let code = r#"
let r = cos(0.0)
if r > 0.99 and r < 1.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn tan_zero() {
    let code = r#"
let r = tan(0.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn asin_zero() {
    let code = r#"
let r = asin(0.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn acos_one() {
    let code = r#"
let r = acos(1.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn atan_zero() {
    let code = r#"
let r = atan(0.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn atan2_basic() {
    let code = r#"
let r = atan2(0.0, 1.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn sinh_zero() {
    let code = r#"
let r = sinh(0.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn cosh_zero() {
    let code = r#"
let r = cosh(0.0)
if r > 0.99 and r < 1.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn tanh_zero() {
    let code = r#"
let r = tanh(0.0)
if r > -0.01 and r < 0.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn exp_zero() {
    let code = r#"
let r = exp(0.0)
if r > 0.99 and r < 1.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn log_e() {
    let code = r#"
let r = log(E)
if r > 0.99 and r < 1.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn log10_hundred() {
    let code = r#"
let r = log10(100.0)
if r > 1.99 and r < 2.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn log2_eight() {
    let code = r#"
let r = log2(8.0)
if r > 2.99 and r < 3.01 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn pow_int_small_exp() {
    let code = r#"
pow(2, 10)
"#;
    assert_aelys_int(code, 1024);
}

#[test]
fn pow_float() {
    let code = r#"
let r = pow(2.0, 3.0)
if r > 7.9 and r < 8.1 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn floor_positive() {
    let code = r#"
floor(3.7)
"#;
    assert_aelys_int(code, 3);
}

#[test]
fn floor_negative() {
    let code = r#"
floor(-2.3)
"#;
    assert_aelys_int(code, -3);
}

#[test]
fn ceil_positive() {
    let code = r#"
ceil(3.2)
"#;
    assert_aelys_int(code, 4);
}

#[test]
fn ceil_negative() {
    let code = r#"
ceil(-2.7)
"#;
    assert_aelys_int(code, -2);
}

#[test]
fn round_half_up() {
    let code = r#"
round(3.5)
"#;
    assert_aelys_int(code, 4);
}

#[test]
fn round_half_down() {
    let code = r#"
round(3.4)
"#;
    assert_aelys_int(code, 3);
}

#[test]
fn trunc_positive() {
    let code = r#"
trunc(3.9)
"#;
    assert_aelys_int(code, 3);
}

#[test]
fn trunc_negative() {
    let code = r#"
trunc(-3.9)
"#;
    assert_aelys_int(code, -3);
}

#[test]
fn min_ints() {
    let code = r#"
min(5, 3)
"#;
    assert_aelys_int(code, 3);
}

#[test]
fn min_floats() {
    let code = r#"
let r = min(5.5, 3.3)
if r > 3.2 and r < 3.4 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn max_ints() {
    let code = r#"
max(5, 3)
"#;
    assert_aelys_int(code, 5);
}

#[test]
fn max_floats() {
    let code = r#"
let r = max(5.5, 3.3)
if r > 5.4 and r < 5.6 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn clamp_below_range() {
    let code = r#"
clamp(1, 5, 10)
"#;
    assert_aelys_int(code, 5);
}

#[test]
fn clamp_above_range() {
    let code = r#"
clamp(15, 5, 10)
"#;
    assert_aelys_int(code, 10);
}

#[test]
fn clamp_in_range() {
    let code = r#"
clamp(7, 5, 10)
"#;
    assert_aelys_int(code, 7);
}

#[test]
fn deg_to_rad() {
    let code = r#"
let r = deg_to_rad(180.0)
if r > 3.14 and r < 3.15 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn rad_to_deg() {
    let code = r#"
let r = rad_to_deg(PI)
if r > 179.9 and r < 180.1 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn hypot_3_4_5() {
    let code = r#"
let r = hypot(3.0, 4.0)
if r > 4.9 and r < 5.1 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn fmod_basic() {
    let code = r#"
let r = fmod(7.5, 2.0)
if r > 1.4 and r < 1.6 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn is_nan_on_nan() {
    let code = r#"
let nan = sqrt(-1.0)
if is_nan(nan) { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn is_nan_on_int() {
    let code = r#"
if is_nan(42) { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn is_inf_on_infinity() {
    let code = r#"
if is_inf(INF) { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn is_inf_on_normal() {
    let code = r#"
if is_inf(42.0) { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn is_finite_on_normal() {
    let code = r#"
if is_finite(42.0) { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn is_finite_on_nan() {
    let code = r#"
let nan = sqrt(-1.0)
if is_finite(nan) { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn is_finite_on_infinity() {
    let code = r#"
if is_finite(INF) { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn neg_infinity_constant() {
    let code = r#"
if is_inf(NEG_INF) { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn sign_nan() {
    let code = r#"
let s = sign(sqrt(-1.0))
if is_nan(s) { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn pow_overflow() {
    let code = r#"
let r = pow(2, 100)
if r > 0.0 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn randint_in_range() {
    let code = r#"
let val = randint(1, 10)
if val >= 1 and val <= 10 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn randint_single_value() {
    let code = r#"
randint(42, 42)
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn randint_negative_range() {
    let code = r#"
let val = randint(-10, -5)
if val >= -10 and val <= -5 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn randint_large_range() {
    let code = r#"
let val = randint(0, 1000)
if val >= 0 and val <= 1000 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}
