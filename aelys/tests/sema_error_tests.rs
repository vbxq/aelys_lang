use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::{TypeError, TypeInference};
use aelys_syntax::Source;

fn sema_check(code: &str) -> Result<(), Vec<TypeError>> {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).map(|_| ())
}

#[test]
fn rejects_binary_type_mismatch() {
    let result = sema_check("fn f() { let x = 1 + \"hello\" }");
    assert!(result.is_err(), "adding i64 and string should be rejected");
}

#[test]
fn rejects_return_type_mismatch() {
    let result = sema_check(r#"fn f() -> i64 { return "text" }"#);
    assert!(
        result.is_err(),
        "returning string from i64 function should be rejected"
    );
}

#[test]
fn rejects_if_non_bool_condition() {
    let result = sema_check("fn f() { if 5 { } }");
    assert!(result.is_err(), "if condition must be bool");
}

#[test]
fn rejects_while_non_bool_condition() {
    let result = sema_check(r#"fn f() { while "yes" { } }"#);
    assert!(result.is_err(), "while condition must be bool");
}

#[test]
fn rejects_array_mixed_types() {
    let result = sema_check(r#"fn f() { let a = [1, "two", 3] }"#);
    assert!(result.is_err(), "array with mixed types should be rejected");
}

#[test]
fn rejects_undefined_variable() {
    let result = sema_check("fn f() { let y = x + 1 }");
    assert!(result.is_err(), "undefined variable x should be rejected");
}

#[test]
fn rejects_argument_type_mismatch() {
    let result = sema_check(r#"fn f(x: i64) {} fn g() { f("str") }"#);
    assert!(
        result.is_err(),
        "passing string to i64 param should be rejected"
    );
}

#[test]
fn multiple_errors_collected_not_just_first() {
    // two independent type errors in separate functions. Sema should report both, not just the first one.
    let result = sema_check(
        r#"
fn f() -> i64 { return "bad" }
fn g() -> string { return 42 }
"#,
    );
    let errors = result.unwrap_err();
    assert!(
        errors.len() >= 2,
        "expected at least 2 errors, got {}",
        errors.len()
    );
}

#[test]
fn rejects_index_assign_on_i64() {
    let result = sema_check(
        r#"
fn f() {
    let mut x: i64 = 42
    x[0] = 10
}
"#,
    );
    assert!(
        result.is_err(),
        "index assignment on i64 should be rejected"
    );
}

#[test]
fn rejects_index_assign_on_bool() {
    let result = sema_check(
        r#"
fn f() {
    let mut b: bool = true
    b[0] = false
}
"#,
    );
    assert!(
        result.is_err(),
        "index assignment on bool should be rejected"
    );
}

#[test]
fn accepts_index_assign_on_array() {
    let result = sema_check(
        r#"
fn f() {
    let mut arr = [1, 2, 3]
    arr[0] = 10
}
"#,
    );
    assert!(
        result.is_ok(),
        "index assignment on array should be accepted, got {:?}",
        result
    );
}

#[test]
fn accepts_index_assign_on_vec() {
    let result = sema_check(
        r#"
fn f() {
    let mut v = Vec<i64>[1, 2, 3]
    v[0] = 10
}
"#,
    );
    assert!(
        result.is_ok(),
        "index assignment on vec should be accepted, got {:?}",
        result
    );
}

#[test]
fn rejects_index_assign_on_immutable_array() {
    let result = sema_check(
        r#"
fn f() {
    let arr = [1, 2, 3]
    arr[0] = 10
}
"#,
    );
    assert!(
        result.is_err(),
        "index assignment on immutable array should be rejected"
    );
}

#[test]
fn rejects_index_assign_on_immutable_vec() {
    let result = sema_check(
        r#"
fn f() {
    let v = Vec<i64>[1, 2, 3]
    v[0] = 10
}
"#,
    );
    assert!(
        result.is_err(),
        "index assignment on immutable vec should be rejected"
    );
}

#[test]
fn accepts_index_assign_on_mut_param() {
    let result = sema_check(
        r#"
fn f(mut arr: [i64; 3]) {
    arr[0] = 10
}
"#,
    );
    assert!(
        result.is_ok(),
        "index assignment on mut param should be accepted, got {:?}",
        result
    );
}

#[test]
fn rejects_legacy_array_type_annotation() {
    let result = sema_check(
        r#"
fn f(arr: Array<i64>) -> i64 {
    return arr[0]
}
"#,
    );
    assert!(
        result.is_err(),
        "Array<T> syntax should be rejected — use [T; N] instead"
    );
    let errors = result.unwrap_err();
    let has_help = errors
        .iter()
        .any(|e| e.help.as_deref() == Some("use [T; N] syntax instead of Array<T>"));
    assert!(
        has_help,
        "error should include help suggesting [T; N] syntax, got: {:?}",
        errors.iter().map(|e| &e.help).collect::<Vec<_>>()
    );
}

#[test]
fn accepts_bracket_array_annotation() {
    let result = sema_check(
        r#"
fn f(arr: [i64; 3]) -> i64 {
    return arr[0]
}
"#,
    );
    assert!(
        result.is_ok(),
        "[T; N] bracket syntax should be accepted, got {:?}",
        result
    );
}

fn std_module_codes(code: &str) -> Vec<(String, String)> {
    let src = Source::new("<std>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let imports = aelys_sema::ModuleImports {
        is_importable: true,
        module_path: "std.probe".to_string(),
        ..Default::default()
    };
    match TypeInference::infer_program_with_imports(stmts, src, imports, Default::default()) {
        Ok(_) => Vec::new(),
        Err(errors) => errors
            .into_iter()
            .map(|e| match e.kind {
                aelys_sema::TypeErrorKind::Coded { code, message, .. } => {
                    (code.to_string(), message)
                }
                other => ("?".to_string(), format!("{other:?}")),
            })
            .collect(),
    }
}

#[test]
fn a_std_module_accepts_methods_on_builtin_heads() {
    let errors = std_module_codes(
        "impl string { pub fn twice(self) -> string { return self + self } }\n\
         impl<T> [T] { pub fn size(&self) -> i64 { return self.len } }\n\
         impl<T> Vec<T> { pub fn size(&self) -> i64 { return Vec::len(*self) } }\n\
         impl i64 { pub fn inc(self) -> i64 { return self + 1 } }",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_std_method_named_like_a_vec_intrinsic_is_refused() {
    let errors = std_module_codes("impl<T> Vec<T> { pub fn get(&self) -> i64 { return 0 } }");
    assert!(
        errors
            .iter()
            .any(|(c, m)| c == "E0342" && m.contains("`get` is an intrinsic method of `Vec`")),
        "{errors:?}"
    );
}

#[test]
fn a_std_method_named_like_a_builtin_trait_method_is_refused() {
    let errors = std_module_codes("impl string { pub fn cmp(self) -> i64 { return 0 } }");
    assert!(
        errors
            .iter()
            .any(|(c, m)| c == "E0342" && m.contains("`cmp` is a method of a builtin trait")),
        "{errors:?}"
    );
}

#[test]
fn a_std_slice_method_by_value_is_refused() {
    let errors = std_module_codes("impl<T> [T] { pub fn size(self) -> i64 { return 0 } }");
    assert!(
        errors
            .iter()
            .any(|(c, m)| c == "E0341" && m.contains("a slice method takes `&self` or `&mut self`")),
        "{errors:?}"
    );
}

#[test]
fn a_std_len_method_on_string_is_refused() {
    let errors = std_module_codes("impl string { pub fn len(self) -> i64 { return 0 } }");
    assert!(
        errors
            .iter()
            .any(|(c, m)| c == "E0342" && m.contains("`len` is an intrinsic method of `string`")),
        "{errors:?}"
    );
}

#[test]
fn a_user_module_impl_on_a_builtin_head_stays_refused() {
    let src = Source::new(
        "<user>",
        "impl string { pub fn twice(self) -> string { return self } }",
    );
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let errors = TypeInference::infer_program(stmts, src).expect_err("must be refused");
    assert!(
        errors.iter().any(|e| matches!(
            &e.kind,
            aelys_sema::TypeErrorKind::Coded { code: "E0341", .. }
        )),
        "{errors:?}"
    );
}
