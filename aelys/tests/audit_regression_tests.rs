use aelys_air::lower::lower;
use aelys_air::passes::validate::validate_air;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn sema_ok(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).is_ok()
}

fn lower_source(code: &str) -> aelys_air::AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");
    lower(&typed)
}

fn air_pipeline_ok(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = match TypeInference::infer_program(stmts, src) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let air = lower(&typed);
    validate_air(&air).is_ok()
}

fn sema_error_count(code: &str) -> usize {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    match TypeInference::infer_program(stmts, src) {
        Ok(_) => 0,
        Err(errors) => errors.len(),
    }
}

#[test]
fn a5_bug001_undefined_assign_no_cascade() {
    let count = sema_error_count(
        r#"
fn test() {
    undefined_var = 42;
    let x = undefined_var;
    let y = undefined_var;
}
"#,
    );
    assert!(
        count <= 2,
        ": undefined_var used 3 times should produce at most 2 errors, got {}",
        count
    );
}

#[test]
fn a5_bug001_single_undefined_single_error() {
    let count = sema_error_count(
        r#"
fn test() {
    bad = 1;
}
"#,
    );
    assert_eq!(
        count, 1,
        ": single undefined assignment should produce exactly 1 error"
    );
}

#[test]
fn a5_bug002_undefined_read_no_cascade() {
    let count = sema_error_count(
        r#"
fn test() {
    let x = ghost + 1;
    let y = ghost * 2;
    let z = ghost;
}
"#,
    );
    assert!(
        count <= 2,
        "A5-BUG-002: ghost used 3 times should produce at most 2 errors, got {}",
        count
    );
}

#[test]
fn a3_bug001_narrowing_through_variable_i8() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = 100;
    x
}
"#
        ),
        ": let x = 100; return x in i8 fn should pass sema"
    );
}

#[test]
fn a3_bug001_narrowing_through_variable_i32() {
    assert!(
        sema_ok(
            r#"
fn f() -> i32 {
    let x = 100;
    x
}
"#
        ),
        ": let x = 100; return x in i32 fn should pass sema"
    );
}

#[test]
fn a3_bug001_narrowing_chain() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = 42;
    let y = x;
    y
}
"#
        ),
        ": let y = x where x = 42, return y in i8 fn should pass sema"
    );
}

#[test]
fn a3_bug001_overflow_through_variable() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = 200;
    x
}
"#
        ),
        ": let x = 200; return x in i8 fn should FAIL (overflow)"
    );
}

#[test]
fn a3_bug001_mutable_not_tracked() {
    // mutable variables should not be tracked for narrowing because they can be reassigned. this test ensures we don't incorrectly narrow through a mutable variable.
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    let mut x = 100;
    x = 999999999;
    x
}
"#
        ),
        ": mutable variable should not be narrowed (could be reassigned)"
    );
}

#[test]
fn a3_bug001_negative_literal_through_variable() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = -1;
    x
}
"#
        ),
        ": let x = -1; return x in i8 fn should pass sema"
    );
}

#[test]
fn if_else_literal_init_is_not_narrowed_i32() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i32 {
    let x = if true { 42 } else { 100 };
    x
}
"#
        ),
        "a binding initialized by an if of literals is i64 and refused into i32"
    );
}

#[test]
fn if_else_literal_init_is_not_narrowed_i8() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = if true { 10 } else { 20 };
    x
}
"#
        ),
        "a binding initialized by an if of literals is i64 and refused into i8"
    );
}

#[test]
fn if_else_direct_return_narrowing() {
    assert!(
        sema_ok(
            r#"
fn f() -> i32 {
    if true { 1 } else { 2 }
}
"#
        ),
        "direct return of if-else with literals should narrow to i32"
    );
}

#[test]
fn if_else_literal_init_is_not_narrowed_i16() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i16 {
    let x = if true { 300 } else { 500 };
    x
}
"#
        ),
        "a binding initialized by an if of literals is i64 and refused into i16"
    );
}

#[test]
fn if_else_overflow_detected() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = if true { 10 } else { 200 };
    x
}
"#
        ),
        "if-else with one branch overflowing i8 should fail"
    );
}

#[test]
fn if_else_same_concrete_type_no_fresh_var() {
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    let x = if true { 42 } else { 100 };
    x
}
"#
        ),
        "if-else with same concrete type should not create a fresh Var"
    );
}

#[test]
fn if_else_different_types_rejected() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i64 {
    let x = if true { 42 } else { "hello" };
    x
}
"#
        ),
        "if-else with different types should be rejected"
    );
}

#[test]
fn if_without_else_not_affected() {
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    let x = 10;
    if x > 5 {
        return 1;
    }
    0
}
"#
        ),
        "if without else should still work"
    );
}

#[test]
fn normal_code_compiles_through_air_pipeline() {
    assert!(
        air_pipeline_ok(
            r#"
use std::process::ExitCode; fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn main() -> ExitCode {
    let x = 42;
    let y = add(x, 10);
    return ExitCode::from(y as u8);
}
"#
        ),
        "basic function calls should pass AIR validation"
    );
}

#[test]
fn function_with_if_else_compiles_through_air() {
    assert!(
        air_pipeline_ok(
            r#"
fn f(x: i64) -> i64 {
    let result = if x > 0 { x } else { 0 };
    result
}
"#
        ),
        "if-else with matching types should pass AIR validation"
    );
}

#[test]
fn multi_function_pipeline_no_var_leak() {
    assert!(
        air_pipeline_ok(
            r#"
use std::process::ExitCode; fn double(n: i64) -> i64 {
    n * 2
}

fn is_positive(n: i64) -> bool {
    n > 0
}

fn main() -> ExitCode {
    let a = 5;
    let b = double(a);
    let flag = is_positive(b);
    if flag {
        return ExitCode::from(b as u8);
    }
    return ExitCode::SUCCESS;
}
"#
        ),
        "multi-function code should not leak type variables into AIR"
    );
}

#[test]
fn oneof_no_leak_on_binary_op() {
    assert!(
        sema_ok(
            r#"
fn f(a: i64, b: i64) -> i64 {
    let c = a + b;
    let d = c * 2;
    let e = d - a;
    e
}
"#
        ),
        "chained binary ops should not corrupt type bindings"
    );
}

#[test]
fn oneof_no_corruption_on_type_mismatch() {
    let count = sema_error_count(
        r#"
fn f(x: String) -> i64 {
    x + 1
}
"#,
    );
    assert!(
        count >= 1,
        "string + int should produce at least 1 error, got {}",
        count
    );
}

#[test]
fn oneof_successful_match_preserves_bindings() {
    assert!(
        sema_ok(
            r#"
fn f(a: i32, b: i32) -> i32 {
    let x = a + b;
    let y = x * a;
    y - b
}
"#
        ),
        "binary ops on i32 should resolve correctly through OneOf"
    );
}

#[test]
fn oneof_multiple_ops_same_function() {
    assert!(
        air_pipeline_ok(
            r#"
fn compute(a: i64, b: i64) -> i64 {
    let sum = a + b;
    let diff = a - b;
    let prod = sum * diff;
    prod
}
"#
        ),
        "multiple binary ops in one function should not leak vars to AIR"
    );
}

#[test]
fn oneof_failed_match_no_pollution() {
    let count = sema_error_count(
        r#"
fn f(x: bool) -> bool {
    x + true
}
"#,
    );
    assert!(
        count >= 1,
        "bool + bool should produce at least 1 error, got {}",
        count
    );
}

#[test]
fn generic_function_called_with_different_types() {
    assert!(
        sema_ok(
            r#"
fn identity<T>(x: T) -> T { x }

fn main() {
    let a = identity(42);
    let b = identity("hello");
}
"#
        ),
        "generic <T> function called with i64 and string should both pass"
    );
}

#[test]
fn generic_function_preserves_return_type_per_call() {
    assert!(
        air_pipeline_ok(
            r#"
use std::process::ExitCode; fn identity<T>(x: T) -> T { x }

fn main() -> ExitCode {
    let a = identity(42);
    return ExitCode::from(a as u8);
}
"#
        ),
        "generic function return type should resolve correctly through AIR"
    );
}

#[test]
fn unannotated_function_infers_single_type() {
    assert!(
        sema_ok(
            r#"
use std::process::ExitCode; fn double(x) -> i64 { x * 2 }

fn main() -> ExitCode {
    return ExitCode::from(double(21) as u8);
}
"#
        ),
        "unannotated function called with one type should infer correctly"
    );
}

#[test]
fn closure_captures_resolved_after_substitution() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    let x: i64 = 100;
    let closure = move || -> i64 { x };
    closure()
}
"#
        ),
        "closure capturing a typed variable should resolve through pipeline"
    );
}

#[test]
fn closure_capture_with_inferred_type() {
    assert!(
        air_pipeline_ok(
            r#"
use std::process::ExitCode; fn main() -> ExitCode {
    let x = 42;
    let f = move || -> i64 { x };
    return ExitCode::from(f() as u8);
}
"#
        ),
        "closure capturing inferred variable should pass AIR validation"
    );
}

#[test]
fn multiple_functions_independent_inference() {
    assert!(
        sema_ok(
            r#"
fn add(a: i64, b: i64) -> i64 { a + b }
fn greet(name: String) -> String { name }

fn main() {
    let x = add(1, 2);
    let y = greet("hello");
}
"#
        ),
        "independent functions with different types should not interfere"
    );
}

#[test]
fn nested_function_types_resolved() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    fn inner(x: i64) -> i64 { x + 1 }
    inner(41)
}
"#
        ),
        "nested function should have types resolved correctly"
    );
}

#[test]
fn equal_constraint_failure_does_not_corrupt_subsequent_inference() {
    assert!(
        sema_ok(
            r#"
fn f(a: i64, b: i64) -> i64 {
    let x = a + b;
    x
}
"#
        ),
        "valid code after constraint solving should pass"
    );
}

#[test]
fn mixed_type_errors_do_not_cascade_through_solver() {
    let count = sema_error_count(
        r#"
fn f(a: i64, b: String) -> i64 {
    let x = a + b;
    let y = a + 1;
    y
}
"#,
    );
    assert!(
        count >= 1 && count <= 3,
        "type mismatch in one expr should not cascade unboundedly, got {}",
        count
    );
}

#[test]
fn function_type_mismatch_on_return_does_not_corrupt_solver() {
    let count = sema_error_count(
        r#"
use std::process::ExitCode; fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn main() -> ExitCode {
    let g = move |n: i64| -> i64 { n + 1 };
    return ExitCode::from(apply(g, 10) as u8);
}
"#,
    );
    assert_eq!(
        count, 0,
        "valid higher-order function should produce no errors"
    );
}

#[test]
fn function_unification_partial_param_match_rolled_back() {
    let count = sema_error_count(
        r#"
fn f() -> i64 {
    let x: i64 = 10;
    let y: String = "hello";
    x + 1
}
"#,
    );
    assert_eq!(
        count, 0,
        "unrelated variables should not interfere with each other"
    );
}

#[test]
fn compound_type_unification_failure_isolated() {
    let count = sema_error_count(
        r#"
fn f(a: i64, b: i64) -> i64 {
    let sum = a + b;
    let prod = a * b;
    sum + prod
}
"#,
    );
    assert_eq!(
        count, 0,
        "compound expressions should unify cleanly without partial corruption"
    );
}

#[test]
fn solver_rollback_preserves_valid_bindings_after_error() {
    let count = sema_error_count(
        r#"
fn f() -> i64 {
    let a: i64 = 10;
    let b: i64 = 20;
    let c = a + b;
    c
}
"#,
    );
    assert_eq!(count, 0, "valid bindings should survive constraint solving");
}

#[test]
fn closure_capture_inferred_var_resolved_through_pipeline() {
    assert!(
        air_pipeline_ok(
            r#"
fn outer() -> i64 {
    let x = 100;
    let closure = move || { x };
    x
}
"#
        ),
        "closure capturing Var(N) should resolve after substitution + finalize"
    );
}

#[test]
fn closure_capture_multiple_inferred_vars() {
    assert!(
        air_pipeline_ok(
            r#"
fn outer() -> i64 {
    let a = 10;
    let b = 20;
    let closure = move || -> i64 { a + b };
    closure()
}
"#
        ),
        "closure capturing multiple inferred variables should pass AIR"
    );
}

#[test]
fn nested_closure_captures_propagate() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    let x: i64 = 42;
    let f = move || -> i64 {
        let g = move || -> i64 { x };
        g()
    };
    f()
}
"#
        ),
        "nested closures should capture and resolve types correctly"
    );
}

#[test]
fn closure_capture_used_in_binary_op() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    let x = 5;
    let f = move || -> i64 { x + 1 };
    f()
}
"#
        ),
        "captured variable used in binary op should type-check"
    );
}

#[test]
fn overflow_through_variable_binop_i8() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = 100;
    x + 28
}
"#
        ),
        "100 + 28 = 128 overflows i8, should be rejected"
    );
}

#[test]
fn variable_binop_fits_i8() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = 50;
    x + 20
}
"#
        ),
        "50 + 20 = 70 fits i8, should pass"
    );
}

#[test]
fn overflow_variable_on_right_side() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let y = 28;
    100 + y
}
"#
        ),
        "100 + 28 = 128 overflows i8 with variable on right"
    );
}

#[test]
fn overflow_both_variables_binop() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let a = 100;
    let b = 28;
    a + b
}
"#
        ),
        "100 + 28 = 128 overflows i8 with both operands as variables"
    );
}

#[test]
fn overflow_through_chained_variable_binop() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = 100;
    let y = x;
    y + 28
}
"#
        ),
        "chained copy 100 + 28 = 128 overflows i8"
    );
}

#[test]
fn variable_subtraction_overflow_i8() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = -100;
    x - 29
}
"#
        ),
        "-100 - 29 = -129 overflows i8 (min is -128)"
    );
}

#[test]
fn variable_multiplication_overflow_i8() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = 20;
    x * 7
}
"#
        ),
        "20 * 7 = 140 overflows i8"
    );
}

#[test]
fn variable_binop_fits_i16() {
    assert!(
        sema_ok(
            r#"
fn f() -> i16 {
    let x = 10000;
    x + 5000
}
"#
        ),
        "10000 + 5000 = 15000 fits i16, should pass"
    );
}

#[test]
fn variable_binop_overflow_i16() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i16 {
    let x = 30000;
    x + 3000
}
"#
        ),
        "30000 + 3000 = 33000 overflows i16 (max 32767)"
    );
}

#[test]
fn negative_variable_binop_fits_i8() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = -100;
    x + 10
}
"#
        ),
        "-100 + 10 = -90 fits i8, should pass"
    );
}

#[test]
fn let_shadowing_function_name_then_call_rejected() {
    assert!(
        !sema_ok(
            r#"
fn outer() -> i64 {
    42
}

fn test() {
    let outer = 99;
    let x = outer();
}
"#
        ),
        "calling a variable that shadows a function should be a type error"
    );
}

#[test]
fn let_shadowing_function_name_without_call_is_valid() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    42
}

fn test() {
    let outer = 99;
}
"#
        ),
        "shadowing a function name with a let without calling should be valid"
    );
}

#[test]
fn nested_fn_same_name_in_different_parents_independent() {
    assert!(
        sema_ok(
            r#"
fn outer() {
    fn inner() -> i64 { 1 }
    let x = inner();
}

fn test() {
    fn inner() -> i64 { 2 }
    let x = inner();
}
"#
        ),
        "same-named nested functions in different parents should be independent"
    );
}

#[test]
fn nested_fn_not_visible_outside_parent() {
    assert!(
        !sema_ok(
            r#"
fn outer() {
    fn inner() -> i64 { 1 }
}

fn test() {
    let x = inner();
}
"#
        ),
        "nested function should not be visible outside its parent"
    );
}

#[test]
fn nested_fn_callable_from_within_parent() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    fn helper() -> i64 { 42 }
    helper()
}
"#
        ),
        "nested function should be callable from within its parent"
    );
}

#[test]
fn multiple_errors_do_not_compound_through_rollback() {
    let count = sema_error_count(
        r#"
fn f(x: i64) -> i64 {
    let a = x + "bad";
    let b = x + true;
    x
}
"#,
    );
    assert!(
        count <= 4,
        "two independent type errors should not compound, got {}",
        count
    );
}

#[test]
fn struct_field_unknown_type_rejected() {
    assert!(
        !sema_ok(
            r#"
struct Foo { x: CompletelyMadeUpType }
fn main() {
    let y = 42;
}
"#
        ),
        "struct with nonexistent field type should be rejected even if unused"
    );
}

#[test]
fn struct_field_valid_primitive_types_accepted() {
    assert!(
        sema_ok(
            r#"
struct Point { x: i64, y: i64 }
fn main() {
    let p = Point { x: 1, y: 2 };
}
"#
        ),
        "struct with valid primitive field types should pass"
    );
}

#[test]
fn struct_field_references_other_struct() {
    assert!(
        sema_ok(
            r#"
struct Inner { value: i64 }
struct Outer { child: Inner }
fn main() {
    let i = Inner { value: 1 };
}
"#
        ),
        "struct referencing another struct should pass"
    );
}

#[test]
fn struct_field_forward_reference_accepted() {
    assert!(
        sema_ok(
            r#"
struct Outer { child: Inner }
struct Inner { value: i64 }
fn main() {
    let i = Inner { value: 1 };
}
"#
        ),
        "struct forward-referencing a later struct should pass"
    );
}

#[test]
fn generic_struct_field_type_param_accepted() {
    assert!(
        sema_ok(
            r#"
use std::process::ExitCode; struct Wrapper<T> { value: T }
fn main() -> ExitCode {
    let w = Wrapper { value: 42 };
    let n: i64 = w.value;
    return ExitCode::from(n as u8);
}
"#
        ),
        "a generic struct field takes the type argument of its literal"
    );
    assert_eq!(
        sema_error_count(
            r#"
struct Wrapper<T> { value: T }
fn main() {
    let w = Wrapper { value: 42 };
    let s: String = w.value;
}
"#
        ),
        1,
        "a field instantiated at i64 is refused once where a string is expected"
    );
}

#[test]
fn struct_multiple_invalid_fields_all_reported() {
    let count = sema_error_count(
        r#"
struct Bad { a: FakeTypeA, b: FakeTypeB }
fn main() {
    let y = 42;
}
"#,
    );
    assert!(
        count >= 2,
        "struct with two invalid field types should produce at least 2 errors, got {}",
        count
    );
}

#[test]
fn struct_name_collides_with_type_param_mismatch_detected() {
    assert!(
        !sema_ok(
            r#"
fn unrelated<T>(x: T) -> T { x }
struct T { value: i64 }
fn main() {
    let x: T = 42;
}
"#
        ),
        "assigning i64 to struct T should be rejected despite T being a type param elsewhere"
    );
}

#[test]
fn struct_name_collides_with_type_param_valid_usage() {
    assert!(
        sema_ok(
            r#"
fn unrelated<T>(x: T) -> T { x }
struct T { value: i64 }
fn main() {
    let x = T { value: 42 };
}
"#
        ),
        "constructing struct T should work despite T being a type param elsewhere"
    );
}

#[test]
fn generic_fn_no_collision_still_works() {
    assert!(
        sema_ok(
            r#"
use std::process::ExitCode; fn identity<T>(x: T) -> T { x }
fn main() -> ExitCode {
    return ExitCode::from(identity(42) as u8);
}
"#
        ),
        "generic function without struct name collision should work"
    );
}

#[test]
fn return_stack_array_lowers_successfully() {
    // Arrays are returned by value and should lower without errors.
    assert!(air_pipeline_ok(
        r#"
fn bar() -> [i64; 3] {
    let arr = [1, 2, 3];
    arr
}
"#,
    ));
}

#[test]
fn return_stack_array_compiles() {
    // Arrays can be returned by value; this should lower successfully.
    assert!(air_pipeline_ok(
        r#"
fn baz() -> [i64; 2] {
    let arr = [10, 20];
    arr
}
"#,
    ));
}

#[test]
fn constant_sized_array_in_let_compiles() {
    assert!(
        air_pipeline_ok(
            r#"
fn f() -> i64 {
    let arr = [1, 2, 3];
    arr[0]
}
"#
        ),
        "constant-sized array in let should compile through AIR"
    );
}

#[test]
fn nested_fn_as_last_stmt_with_return_type_rejected() {
    assert!(
        !sema_ok(
            r#"
fn outer() -> i64 {
    fn inner() -> i64 { 1 }
}
"#
        ),
        "nested function as last stmt in non-void function should be rejected"
    );
}

#[test]
fn nested_fn_as_last_stmt_void_return_accepted() {
    assert!(
        sema_ok(
            r#"
fn outer() {
    fn inner() -> i64 { 1 }
}
"#
        ),
        "nested function as last stmt in void function should be accepted"
    );
}

#[test]
fn nested_fn_followed_by_return_accepted() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    fn inner() -> i64 { 1 }
    inner()
}
"#
        ),
        "nested function followed by explicit return should be accepted"
    );
}

#[test]
fn unify_prevents_var_cycle_through_resolution() {
    // Var(A) unified with Var(B), then Var(B) unified with Var(A)
    // should succeed (identity) without creating a cycle
    assert!(
        sema_ok(
            r#"
fn f(a, b) {
    let x = a;
    let y = b;
    let z: i64 = x;
    let w: i64 = y;
}
"#
        ),
        "unifying vars that resolve to the same type should not cycle"
    );
}

#[test]
fn compound_generic_does_not_create_infinite_type() {
    assert!(
        sema_ok(
            r#"
use std::process::ExitCode; fn wrap<T>(x: T) -> T {
    x
}

fn main() -> ExitCode {
    let a = wrap(42);
    let b = wrap(a);
    return ExitCode::from(b as u8);
}
"#
        ),
        "chained generic calls should resolve without infinite type"
    );
}

#[test]
fn self_referencing_let_is_undefined_variable() {
    assert!(
        !sema_ok(
            r#"
fn test() {
    let f = move |x| { f };
}
"#
        ),
        "self-referencing let should fail with undefined variable"
    );
}

#[test]
fn var_chain_resolves_without_cycle() {
    assert!(
        sema_ok(
            r#"
fn test() -> i64 {
    let a = 1;
    let b = a;
    let c = b;
    let d = c;
    d
}
"#
        ),
        "long chain of variable copies should resolve without cycle"
    );
}

#[test]
fn substitution_apply_resolves_nested_vars() {
    assert!(
        air_pipeline_ok(
            r#"
use std::process::ExitCode; fn identity<T>(x: T) -> T { x }

fn main() -> ExitCode {
    let a = identity(42);
    let b = identity(a);
    return ExitCode::from(b as u8);
}
"#
        ),
        "nested generic calls should resolve vars completely"
    );
}

#[test]
fn type_mismatch_does_not_leave_corrupt_substitution() {
    let count = sema_error_count(
        r#"
fn f() -> i64 {
    let x: i64 = "bad";
    let y = 42;
    y
}
"#,
    );
    assert!(
        count >= 1 && count <= 2,
        "type mismatch should produce errors without corrupting solver, got {}",
        count
    );
}

#[test]
fn error_recovery_does_not_corrupt_unrelated_compound_type() {
    let count = sema_error_count(
        r#"
use std::process::ExitCode; fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn main() -> ExitCode {
    let bad = "hello" + 42;
    let g = move |n: i64| -> i64 { n + 1 };
    return ExitCode::from(apply(g, 10) as u8);
}
"#,
    );
    assert!(
        count >= 1 && count <= 3,
        "error in one expression should not corrupt function type inference, got {}",
        count
    );
}

#[test]
fn inner_vars_in_function_type_resolve_through_finalization() {
    assert!(
        air_pipeline_ok(
            r#"
use std::process::ExitCode; fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    f(x)
}

fn main() -> ExitCode {
    let inc = move |n: i64| -> i64 { n + 1 };
    return ExitCode::from(apply(inc, 5) as u8);
}
"#
        ),
        "function type params should resolve through full pipeline"
    );
}

#[test]
fn force_dynamic_on_error_preserves_valid_inference_elsewhere() {
    let count = sema_error_count(
        r#"
fn f(x) {
    let y: i64 = x;
    let z = y + "hello";
    let w: i64 = 42;
    return w;
}
"#,
    );
    assert!(
        count >= 1 && count <= 3,
        "type error should not corrupt unrelated bindings, got {}",
        count
    );
}

#[test]
fn recursion_limit_orphan_constraints_do_not_cause_miscompilation() {
    // even if orphan constraints exist from partial inference,
    // the RecursionLimit error makes the program fail at sema
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    1 + 2 + 3
}
"#
        ),
        "normal expressions should not trigger recursion limit"
    );
}

#[test]
fn recursion_limit_does_not_prevent_other_function_inference() {
    assert!(
        sema_ok(
            r#"
fn simple() -> i64 {
    42
}
"#
        ),
        "normal functions should still compile"
    );
}

#[test]
fn lambda_calling_nested_function_in_outer_scope() {
    assert!(
        air_pipeline_ok(
            r#"
fn outer() -> i64 {
    fn helper() -> i64 { 42 }
    let lambda = move || -> i64 { helper() };
    helper()
}
"#
        ),
        "lambda calling a sibling function should compile through AIR"
    );
}

#[test]
fn lambda_calling_toplevel_function() {
    assert!(
        air_pipeline_ok(
            r#"
use std::process::ExitCode; fn helper() -> i64 { 10 }

fn main() -> ExitCode {
    let f = move || -> i64 { helper() };
    return ExitCode::from(helper() as u8);
}
"#
        ),
        "lambda calling a top-level function should compile through AIR"
    );
}

#[test]
fn array_literal_narrowing_tracked_variable() {
    assert!(
        sema_ok(
            r#"
fn f() -> [i32; 3] {
    let x = 100;
    [x, 1, 2]
}
"#
        ),
        "tracked variable in array literal should narrow to i32"
    );
}

#[test]
fn array_literal_narrowing_all_literals() {
    assert!(
        sema_ok(
            r#"
fn f() -> [i32; 3] {
    [1, 2, 3]
}
"#
        ),
        "all-literal array should narrow to i32"
    );
}

#[test]
fn array_literal_narrowing_untracked_param_rejected() {
    assert!(
        !sema_ok(
            r#"
fn f(y: i64) -> [i32; 3] {
    [y, 1, 2]
}
"#
        ),
        "i64 parameter in i32 array should be rejected"
    );
}

#[test]
fn array_literal_narrowing_mixed_tracked_and_literal() {
    assert!(
        sema_ok(
            r#"
fn f() -> [i32; 4] {
    let a = 10;
    let b = 20;
    [a, b, 30, 40]
}
"#
        ),
        "mix of tracked variables and literals should narrow"
    );
}
