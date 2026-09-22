/// when a constraint fails, force_dynamic should only bind top-level Vars to Dynamic.
///
/// it shouldnt recurse into compound types (Array, Function, Tuple, Vec) because nested Vars may be shared with unrelated constraints
/// binding them to Dynamic would poison those other constraints and suppress real error messages
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

fn sema_err(code: &str) -> bool {
    !sema_ok(code)
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
fn force_dynamic_does_not_poison_function_return_type() {
    // The second call to add() is correct; it should not be affected by the
    // error in the first call.
    assert!(
        sema_err(
            r#"
fn add(a: i64, b: i64) -> i64 { return a + b }

fn main() {
    let bad = add(1, "hello")
    let good: i64 = add(2, 3)
}
"#
        ),
        "should have an error from add(1, \"hello\")"
    );
}

// ensures Var(1) remains untouched.
#[test]
fn force_dynamic_does_not_walk_into_array_element_type() {
    // the array [1, 2, 3] has element type that should resolve to i64.
    // assigning the array to a string variable is an error, but the element type should not be poisoned
    assert!(
        sema_err(
            r#"
fn main() {
    let arr = [1, 2, 3]
    let bad: string = arr
    let elem: i64 = arr[0]
}
"#
        ),
        "assigning array to string should be rejected"
    );
}

// two independent type errors should both be reported. with recursive
// force_dynamic, the first error could poison shared Vars and mask the second
#[test]
fn independent_errors_both_reported() {
    let count = sema_error_count(
        r#"
fn main() {
    let a: string = 42
    let b: i64 = "hello"
}
"#,
    );
    assert!(
        count >= 2,
        "both independent type errors should be reported, got {} error(s)",
        count
    );
}

// a valid program should not be affected by the force_dynamic change.
#[test]
fn valid_program_still_passes() {
    assert!(
        sema_ok(
            r#"
fn sum(a: i64, b: i64) -> i64 { return a + b }

fn main() {
    let x: i64 = sum(1, 2)
    let y: i64 = sum(3, 4)
}
"#
        ),
        "valid program should pass"
    );
}
