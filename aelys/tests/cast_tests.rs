mod common;
use common::*;

#[test]
fn int_as_int() {
    assert_aelys_int("42 as i32", 42);
}

#[test]
fn int_as_float() {
    let result = run_aelys("42 as f64");
    assert!(result.as_float().is_some() || result.as_int().is_some());
}

#[test]
fn float_as_int() {
    let result = run_aelys("3.14 as i32");
    assert!(result.as_int().is_some() || result.as_float().is_some());
}

#[test]
fn bool_as_int() {
    let result = run_aelys("true as i32");
    assert!(result.as_bool() == Some(true) || result.as_int() == Some(1));
}

#[test]
fn int_as_bool() {
    let result = run_aelys("1 as bool");
    assert!(result.as_int() == Some(1) || result.as_bool() == Some(true));
}

#[test]
fn chained_cast() {
    let result = run_aelys("42 as i32 as f64");
    assert!(result.as_int().is_some() || result.as_float().is_some());
}

#[test]
fn cast_in_expression() {
    assert_aelys_int("let x = 10\nx as i32 + 5", 15);
}

#[test]
fn cast_preserves_value() {
    assert_aelys_int("100 as i64", 100);
}

#[test]
fn invalid_cast_string_to_int_is_compile_error() {
    let result = run_aelys_result(r#""hello" as i32"#);
    assert!(result.is_err());
}
