use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

const LABS: &str = "extern \"C\" { fn labs(x: i64) -> i64; }\n";
const FREE: &str = "extern \"C\" { fn free(p: &i64); }\n";

fn rejects(id: &str, source: &str, code: &str, hits: usize) {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("{id}: MUST be rejected\n{source}"),
        Err(rendered) => rendered,
    };
    assert!(
        rendered.contains(code),
        "{id}: the rejection MUST be {code}\n{source}\nrendered:\n{rendered}"
    );
    assert_eq!(
        rendered.matches(&format!("error[{code}]")).count(),
        hits,
        "{id}: {hits} rejection(s) expected\nrendered:\n{rendered}"
    );
}

fn answers(id: &str, source: &str, want: i32) {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
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
fn s3_a3_a_nested_empty_unsafe_block_stays_accepted() {
    answers(
        "S3-A3",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    unsafe { unsafe { } }\n    return ExitCode::from(7);\n}\n",
        7,
    );
}

#[test]
fn s3_a4_an_empty_unsafe_block_in_a_lambda_stays_accepted() {
    answers(
        "S3-A4",
        "use std::process::ExitCode; fn main() -> ExitCode {\n    let f = move || -> i64 {\n        unsafe { }\n        7\n    };\n    return ExitCode::from(f() as u8);\n}\n",
        7,
    );
}

#[test]
fn s3_b1_a_bare_direct_call_is_rejected() {
    rejects("S3-B1", &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{ return ExitCode::from(labs(-7) as u8); }}\n"), "E0617", 1);
}

#[test]
fn s3_b1_bis_the_accepted_twin_answers() {
    answers("S3-B1'", &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{ unsafe {{ return ExitCode::from(labs(-7) as u8); }} }}\n"), 7);
}

#[test]
fn s3_b2_the_non_terminal_accepted_twin_answers() {
    answers(
        "S3-B2",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let mut r: i64 = 0;\n    unsafe {{ r = labs(-7); }}\n    return ExitCode::from(r as u8);\n}}\n"),
        7,
    );
}

#[test]
fn s3_b3_the_entry_witness_of_the_stage_is_rejected() {
    rejects(
        "S3-B3",
        &format!("{FREE}fn main() {{\n    let x: i64 = 7;\n    free(&x);\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b3_bis_the_accepted_twin_crashes_because_the_program_says_so() {
    let source = format!("{FREE}fn main() {{\n    let x: i64 = 7;\n    unsafe {{ free(&x); }}\n}}\n");
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, &source).expect("write fixture");
    compile_file_with_llvm(&root, OptimizationLevel::None, false)
        .unwrap_or_else(|err| panic!("S3-B3' must compile:\n{source}\n{err}"));
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
    assert!(
        got > 128,
        "S3-B3': the double free is the contract of the program, not of the compiler, got {got}"
    );
}

#[test]
fn s3_b4_a_bare_call_in_a_lambda_body_is_rejected() {
    rejects(
        "S3-B4",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let f = move || -> i64 {{ labs(-7) }};\n    return ExitCode::from(f() as u8);\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b4_bis_the_block_inside_the_lambda_body_is_the_accepted_twin() {
    answers(
        "S3-B4'",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let f = move || -> i64 {{ unsafe {{ return labs(-7); }} }};\n    return ExitCode::from(f() as u8);\n}}\n"),
        7,
    );
}

#[test]
fn s3_b5_the_block_at_the_writing_site_does_not_reach_the_lambda_body() {
    rejects(
        "S3-B5",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let f = unsafe {{ move || -> i64 {{ labs(-7) }} }};\n    return ExitCode::from(f() as u8);\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b6_an_argument_position_call_is_rejected() {
    rejects(
        "S3-B6",
        &format!("use std::process::ExitCode; {LABS}fn id(v: i64) -> i64 {{ v }}\nfn main() -> ExitCode {{ return ExitCode::from(id(labs(-7)) as u8); }}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b6_bis_the_accepted_twin_answers() {
    answers(
        "S3-B6'",
        &format!("use std::process::ExitCode; {LABS}fn id(v: i64) -> i64 {{ v }}\nfn main() -> ExitCode {{\n    unsafe {{\n        let t: i64 = labs(-7);\n        return ExitCode::from(id(t) as u8);\n    }}\n}}\n"),
        7,
    );
}

#[test]
fn s3_b7_a_nested_call_is_rejected_once_per_call() {
    rejects(
        "S3-B7",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{ return ExitCode::from(labs(labs(-7)) as u8); }}\n"),
        "E0617",
        2,
    );
}

#[test]
fn s3_b8_a_call_in_an_if_condition_is_rejected() {
    rejects(
        "S3-B8",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    if labs(-7) > 0 {{ return ExitCode::from(7); }}\n    return ExitCode::SUCCESS;\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b9_a_call_as_an_and_operand_is_rejected() {
    rejects(
        "S3-B9",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    if labs(-1) > 0 && labs(-2) > 0 {{ return ExitCode::from(7); }}\n    return ExitCode::SUCCESS;\n}}\n"),
        "E0617",
        2,
    );
}

#[test]
fn s3_b10_a_call_in_a_for_body_is_rejected() {
    rejects(
        "S3-B10",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let mut s: i64 = 0;\n    for i in 0..2 {{\n        s = s + labs(-1);\n    }}\n    return ExitCode::from(s as u8);\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b11_a_call_in_an_array_element_is_rejected() {
    rejects(
        "S3-B11",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let a = [labs(-7), 0];\n    return ExitCode::from(a[0] as u8);\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b12_a_call_in_a_struct_field_is_rejected() {
    rejects(
        "S3-B12",
        &format!("use std::process::ExitCode; struct P {{ a: i64 }}\n{LABS}fn main() -> ExitCode {{\n    let p = P {{ a: labs(-7) }};\n    return ExitCode::from(p.a as u8);\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b13_a_call_in_a_formatted_string_is_rejected() {
    rejects(
        "S3-B13",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let s: String = format!(\"v=${{}}\", labs(-7));\n    if s == \"v=7\" {{ return ExitCode::from(7); }}\n    return ExitCode::SUCCESS;\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b14_a_discarded_call_is_rejected() {
    rejects(
        "S3-B14",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let _ = labs(-7);\n    return ExitCode::from(7);\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b15_a_call_in_a_match_arm_is_rejected() {
    rejects(
        "S3-B15",
        &format!("use std::process::ExitCode; enum E {{ X, Y }}\n{LABS}fn main() -> ExitCode {{\n    let e = E::X;\n    match e {{\n        E::X => {{ return ExitCode::from(labs(-7) as u8); }}\n        E::Y => {{ return ExitCode::SUCCESS; }}\n    }}\n}}\n"),
        "E0617",
        1,
    );
}

#[test]
fn s3_b16_a_parenthesised_callee_stays_e0616_not_e0617() {
    rejects(
        "S3-B16",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{ return ExitCode::from((labs)(-7) as u8); }}\n"),
        "E0616",
        1,
    );
}

#[test]
fn s3_b17_the_name_as_a_value_stays_e0616() {
    rejects(
        "S3-B17",
        &format!("use std::process::ExitCode; {LABS}fn main() -> ExitCode {{\n    let f = labs;\n    return ExitCode::from(f(-7) as u8);\n}}\n"),
        "E0616",
        1,
    );
}

#[test]
fn s3_b18_bis_an_aelys_call_beside_a_declared_extern_stays_accepted() {
    answers(
        "S3-B18'",
        &format!("use std::process::ExitCode; {LABS}fn id(v: i64) -> i64 {{ v }}\nfn main() -> ExitCode {{ return ExitCode::from(id(7) as u8); }}\n"),
        7,
    );
}

#[test]
fn s3_b19_a_nogc_extern_called_from_a_nogc_body_inside_a_block_is_accepted() {
    answers(
        "S3-B19",
        "use std::process::ExitCode; extern \"C\" { nogc fn labs(x: i64) -> i64; }\n\nnogc fn c() -> i64 {\n    unsafe { return labs(-7); }\n}\nfn main() -> ExitCode { return ExitCode::from(c() as u8); }\n",
        7,
    );
}

#[test]
fn s3_b20_a_plain_extern_called_from_a_nogc_body_inside_a_block_is_still_e0727() {
    rejects(
        "S3-B20",
        "use std::process::ExitCode; extern \"C\" { fn labs(x: i64) -> i64; }\n\nnogc fn c() -> i64 {\n    unsafe { return labs(-7); }\n}\nfn main() -> ExitCode { return ExitCode::from(c() as u8); }\n",
        "E0727",
        1,
    );
}

#[test]
fn s3_b21_a_parameter_spelling_an_extern_is_the_callee_and_answers() {
    answers(
        "S3-B21",
        &format!(
            "use std::process::ExitCode; {LABS}fn g(labs: fn(i64) -> i64) -> i64 {{ labs(3) }}\n             fn seven(x: i64) -> i64 {{ 7 }}\n             fn main() -> ExitCode {{ return ExitCode::from(g(seven) as u8); }}\n"
        ),
        7,
    );
}

#[test]
fn s3_b22_a_let_spelling_an_extern_is_the_callee_and_answers() {
    answers(
        "S3-B22",
        &format!(
            "use std::process::ExitCode; {LABS}fn seven(x: i64) -> i64 {{ 7 }}\n             fn main() -> ExitCode {{\n    let labs = seven;\n    return ExitCode::from(labs(3) as u8);\n}}\n"
        ),
        7,
    );
}

#[test]
fn s3_b23_the_extern_beyond_the_shadowing_scope_is_still_rejected() {
    rejects(
        "S3-B23",
        &format!(
            "use std::process::ExitCode; {LABS}fn g(labs: fn(i64) -> i64) -> i64 {{ labs(3) }}\n             fn main() -> ExitCode {{ return ExitCode::from(labs(-7) as u8); }}\n"
        ),
        "E0617",
        1,
    );
}
