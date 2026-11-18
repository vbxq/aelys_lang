use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

const HEAD: &str = "enum Result<T, E> { Ok(T), Err(E) }\nenum E { X }\nfn mk() -> Result<i64, E> { Result::Ok(7) }\n";

fn rejected(id: &str, body: &str) {
    let source = format!("{HEAD}{body}");
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, &source).expect("write fixture");
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("{id}: the permission of a block does not reach a body\n{source}"),
        Err(rendered) => rendered,
    };
    assert!(
        rendered.contains("[E0620]"),
        "{id}: the rejection MUST be E0620\n{source}\nrendered:\n{rendered}"
    );
    assert!(
        rendered.contains("`.unwrap_unchecked()` requires an `unsafe` block"),
        "{id}: the message MUST name the missing block\nrendered:\n{rendered}"
    );
}

fn answers(id: &str, body: &str, want: i32) {
    let source = format!("{HEAD}{body}");
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, &source).expect("write fixture");
    compile_file_with_llvm(&root, OptimizationLevel::None, false)
        .unwrap_or_else(|err| panic!("{id} must compile:\n{source}\n{err}"));
    let exe = root.with_extension("");
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("{} >/dev/null 2>&1; echo $?", exe.display()))
        .output()
        .expect("shell should run the compiled executable");
    let got = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<i32>()
        .expect("a shell status is an integer below 256");
    assert_eq!(got, want, "{id}: the executed value is the assertion\n{source}");
}

#[test]
fn s3_s1_a_lambda_written_inside_an_unsafe_block_does_not_inherit_it() {
    rejected(
        "S3-S1",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let f = unsafe { move || -> i64 { mk().unwrap_unchecked() } };\n    return ExitCode::from(f() as u8);\n}\n",
    );
}

#[test]
fn s3_s2_a_lambda_assigned_inside_an_unsafe_block_does_not_inherit_it() {
    rejected(
        "S3-S2",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let mut f = move || -> i64 { 0 };\n    unsafe { f = move || -> i64 { mk().unwrap_unchecked() }; }\n    return ExitCode::from(f() as u8);\n}\n",
    );
}

#[test]
fn s3_s3_a_named_function_declared_under_a_while_inside_unsafe_does_not_inherit_it() {
    rejected(
        "S3-S3",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let mut r: i64 = 0;\n    unsafe {\n        while true {\n            fn g() -> i64 { mk().unwrap_unchecked() }\n            r = g();\n            break;\n        }\n    }\n    return ExitCode::from(r as u8);\n}\n",
    );
}

#[test]
fn s3_s4_the_same_named_function_escaped_and_called_outside_the_block() {
    rejected(
        "S3-S4",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let mut f = move || -> i64 { 0 };\n    unsafe {\n        while true {\n            fn g() -> i64 { mk().unwrap_unchecked() }\n            f = g;\n            break;\n        }\n    }\n    return ExitCode::from(f() as u8);\n}\n",
    );
}

#[test]
fn s3_s5_the_for_route_into_declaration() {
    rejected(
        "S3-S5",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let mut r: i64 = 0;\n    unsafe {\n        for i in 0..1 {\n            fn g() -> i64 { mk().unwrap_unchecked() }\n            r = g();\n        }\n    }\n    return ExitCode::from(r as u8);\n}\n",
    );
}

#[test]
fn s3_s6_the_same_route_inside_a_lambda_body() {
    rejected(
        "S3-S6",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let f = move || -> i64 {\n        let mut r: i64 = 0;\n        unsafe {\n            while true {\n                fn g() -> i64 { mk().unwrap_unchecked() }\n                r = g();\n                break;\n            }\n        }\n        r\n    };\n    return ExitCode::from(f() as u8);\n}\n",
    );
}

#[test]
fn s3_s7_the_block_written_inside_the_body_is_the_accepted_twin() {
    answers(
        "S3-S7",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let f = move || -> i64 { unsafe { return mk().unwrap_unchecked(); } };\n    return ExitCode::from(f() as u8);\n}\n",
        7,
    );
}

#[test]
fn s3_s8_a_named_function_carrying_its_own_block_is_accepted() {
    answers(
        "S3-S8",
        "use std::process::ExitCode; fn g() -> i64 { unsafe { return mk().unwrap_unchecked(); } }\nfn main() -> ExitCode { return ExitCode::from(g() as u8); }\n",
        7,
    );
}

#[test]
fn s3_s9_the_bare_form_stays_rejected() {
    rejected(
        "S3-S9",
        "use std::process::ExitCode; fn main() -> ExitCode { return ExitCode::from(mk().unwrap_unchecked() as u8); }\n",
    );
}

#[test]
fn s3_s10_a_block_in_the_calling_body_still_reaches_its_own_call() {
    answers(
        "S3-S10",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    unsafe {\n        let v: i64 = mk().unwrap_unchecked();\n        return ExitCode::from(v as u8);\n    }\n}\n",
        7,
    );
}
