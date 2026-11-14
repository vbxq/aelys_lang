use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const LABS: &str = "unsafe extern fn labs(x: i64) -> i64\n";

fn run_at(dir: &Path, name: &str, source: &str, opt: OptimizationLevel) -> i32 {
    let source_path = dir.join(format!("{name}.aelys"));
    fs::write(&source_path, source).expect("source should be written");
    compile_file_with_llvm(&source_path, opt, false)
        .unwrap_or_else(|err| panic!("{name} must compile:\n{source}\n{err}"));
    let exe = source_path.with_extension("");
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("{} >/dev/null 2>&1; echo $?", exe.display()))
        .output()
        .expect("shell should run the compiled executable");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<i32>()
        .expect("a shell status is an integer below 256")
}

fn answers(id: &str, source: &str, want: i32) {
    let dir = tempdir().expect("tempdir");
    let got = run_at(dir.path(), "d", source, OptimizationLevel::None);
    assert_eq!(got, want, "{id}: the executed value is the assertion\n{source}");
}

fn answers_at_every_level(id: &str, source: &str, want: i32) {
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let got = run_at(dir.path(), "d", source, *opt);
        assert_eq!(got, want, "{id} at {level}: same value at every -O\n{source}");
    }
}

#[test]
fn s3_d1_an_i64_tail_return_under_unsafe() {
    answers(
        "S3-D1",
        "fn g(x: i64) -> i64 { unsafe { return x } }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d2_an_i32_tail_return_under_unsafe() {
    answers(
        "S3-D2",
        "fn g(x: i32) -> i32 { unsafe { return x } }\nfn main() -> i64 {\n    let v: i32 = g(7)\n    return v as i64\n}\n",
        7,
    );
}

#[test]
fn s3_d3_a_bool_tail_return_under_unsafe() {
    answers(
        "S3-D3",
        "fn g(x: bool) -> bool { unsafe { return x } }\nfn main() -> i64 {\n    if g(true) { return 7 }\n    return 0\n}\n",
        7,
    );
}

#[test]
fn s3_d4_an_f64_tail_return_under_unsafe() {
    answers(
        "S3-D4",
        "fn g(x: f64) -> f64 { unsafe { return x } }\nfn main() -> i64 {\n    let v: f64 = g(7.0)\n    return v as i64\n}\n",
        7,
    );
}

#[test]
fn s3_d5_a_string_tail_return_under_unsafe() {
    answers(
        "S3-D5",
        "fn g(x: string) -> string { unsafe { return x } }\nfn main() -> i64 {\n    let s = g(\"abcdefg\")\n    if s == \"abcdefg\" { return 7 }\n    return 0\n}\n",
        7,
    );
}

#[test]
fn s3_d6_a_vec_tail_return_under_unsafe() {
    answers(
        "S3-D6",
        "fn g(x: vec<i64>) -> vec<i64> { unsafe { return x } }\nfn main() -> i64 {\n    let v = g(vec[7, 1])\n    return v[0]\n}\n",
        7,
    );
}

#[test]
fn s3_d7_a_reference_tail_return_did_not_have_to_change() {
    answers(
        "S3-D7",
        "fn g(x: &i64) -> &i64 { unsafe { return x } }\nfn main() -> i64 {\n    let a: i64 = 7\n    let r = g(&a)\n    return *r\n}\n",
        7,
    );
}

#[test]
fn s3_d8_a_nested_unsafe_tail_return() {
    answers(
        "S3-D8",
        "fn g(x: i64) -> i64 { unsafe { unsafe { return x } } }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d9_a_tail_return_in_a_lambda_body() {
    answers(
        "S3-D9",
        "fn main() -> i64 {\n    let f = fn(x: i64) -> i64 { unsafe { return x } }\n    return f(7)\n}\n",
        7,
    );
}

#[test]
fn s3_d10_both_arms_of_a_tail_if() {
    answers(
        "S3-D10",
        "fn g(x: i64) -> i64 {\n    if x > 0 { unsafe { return x } } else { unsafe { return 0 } }\n}\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d11_the_bare_block_control_does_not_move() {
    answers(
        "S3-D11",
        "fn g(x: i64) -> i64 { { return x } }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d12_the_diverging_match_control_does_not_move() {
    answers(
        "S3-D12",
        "enum Result<T, E> { Ok(T), Err(E) }\nenum E { X }\nfn mk() -> Result<i64, E> { return Result::Ok(7) }\nfn g() -> i64 { match mk() { Result::Ok(v) => { return v }  Result::Err(e) => { return 0 } } }\nfn main() -> i64 { return g() }\n",
        7,
    );
}

#[test]
fn s3_d13_a_void_body_still_takes_the_return_none_path() {
    answers(
        "S3-D13",
        "fn g(x: i64) { unsafe { let y: i64 = x } }\nfn main() -> i64 {\n    g(7)\n    return 7\n}\n",
        7,
    );
}

#[test]
fn s3_d14_the_repair_is_the_same_at_every_opt_level() {
    answers_at_every_level(
        "S3-D14",
        "fn g(x: i64) -> i64 { unsafe { return x } }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d15_a_struct_tail_return_is_the_thirteenth_form() {
    answers(
        "S3-D15",
        "struct P { a: i64 }\nfn g(x: i64) -> P {\n    unsafe {\n        let q = P { a: x }\n        return q\n    }\n}\nfn main() -> i64 {\n    let p = g(7)\n    return p.a\n}\n",
        7,
    );
}

#[test]
fn s3_d16_a_parenthesised_bare_block_writes_no_unsafe() {
    answers(
        "S3-D16",
        "fn g(x: i64) -> i64 { ({ return x }) }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d17_a_negated_bare_block_writes_no_unsafe() {
    answers(
        "S3-D17",
        "fn g(x: i64) -> i64 { -({ return x }) }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d18_a_bare_block_as_an_argument() {
    answers(
        "S3-D18",
        "fn id(v: i64) -> i64 { return v }\nfn g(x: i64) -> i64 { id({ return x }) }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d19_a_bare_block_as_a_binop_operand() {
    answers(
        "S3-D19",
        "fn g(x: i64) -> i64 { ({ return x }) + 0 }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d20_an_unsafe_block_as_an_argument() {
    answers(
        "S3-D20",
        "fn id(v: i64) -> i64 { return v }\nfn g(x: i64) -> i64 { id(unsafe { return x }) }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d21_an_unsafe_block_as_a_binop_operand() {
    answers(
        "S3-D21",
        "fn g(x: i64) -> i64 { (unsafe { return x }) + 0 }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_d22_the_ffi_form_as_an_argument() {
    answers(
        "S3-D22",
        &format!("{LABS}fn id(v: i64) -> i64 {{ return v }}\nfn main() -> i64 {{ id(unsafe {{ return labs(-7) }}) }}\n"),
        7,
    );
}

#[test]
fn s3_d23_the_banal_accepted_twin_of_e0617() {
    answers(
        "S3-D23",
        &format!("{LABS}fn main() -> i64 {{ unsafe {{ return labs(-7) }} }}\n"),
        7,
    );
}

#[test]
fn s3_d24_the_other_banal_accepted_twin() {
    answers(
        "S3-D24",
        &format!("{LABS}fn main() -> i64 {{\n    unsafe {{\n        let r: i64 = labs(-7)\n        return r\n    }}\n}}\n"),
        7,
    );
}

#[test]
fn s3_dc1_a_plain_return_control() {
    answers("S3-DC1", "fn main() -> i64 { return 7 }\n", 7);
}

#[test]
fn s3_dc2_a_non_terminal_unsafe_return_control() {
    answers(
        "S3-DC2",
        "fn g(x: i64) -> i64 {\n    unsafe { return x }\n    return 0\n}\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_dc3_an_unsafe_return_under_an_if_control() {
    answers(
        "S3-DC3",
        "fn g(x: i64) -> i64 {\n    if x > 0 { unsafe { return x } }\n    return 0\n}\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_dc4_an_unsafe_return_under_a_while_control() {
    answers(
        "S3-DC4",
        "fn g(x: i64) -> i64 {\n    while true { unsafe { return x } }\n    return 0\n}\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_dc5_an_unsafe_tail_call_control() {
    answers(
        "S3-DC5",
        "fn h(x: i64) -> i64 { return x }\nfn g(x: i64) -> i64 {\n    unsafe { h(x) }\n    return x\n}\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_dc6_an_empty_unsafe_block_control() {
    answers("S3-DC6", "fn main() -> i64 {\n    unsafe { }\n    return 7\n}\n", 7);
}

#[test]
fn s3_dc7_an_empty_toplevel_unsafe_block_control() {
    answers("S3-DC7", "unsafe { }\nfn main() -> i64 { return 8 }\n", 8);
}

#[test]
fn s3_dc8_an_unsafe_operand_of_a_return_control() {
    answers(
        "S3-DC8",
        "fn g(x: i64) -> i64 { return unsafe { x } }\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_dc9_the_stage_one_ffi_witness_still_answers_eleven() {
    answers(
        "S3-DC9",
        "unsafe extern fn ffsl(x: i64) -> i64\nfn main() -> i64 { unsafe { return ffsl(1024) } }\n",
        11,
    );
}

#[test]
fn s3_dc10_a_for_loop_control() {
    answers(
        "S3-DC10",
        "fn g(x: i64) -> i64 {\n    let mut s: i64 = 0\n    for i in 0..x {\n        s = s + 1\n    }\n    return s\n}\nfn main() -> i64 { return g(7) }\n",
        7,
    );
}

#[test]
fn s3_dp1_a_divergent_if_condition_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP1",
        "fn main() -> i64 {\n    if unsafe { return 7 } > 0 { return 1 }\n    return 0\n}\n",
        7,
    );
}

#[test]
fn s3_dp2_a_divergent_while_condition_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP2",
        "fn main() -> i64 {\n    while unsafe { return 7 } > 0 { return 1 }\n    return 0\n}\n",
        7,
    );
}

#[test]
fn s3_dp3_a_divergent_for_bound_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP3",
        "fn main() -> i64 {\n    for i in 0..(unsafe { return 7 }) { return 1 }\n    return 0\n}\n",
        7,
    );
}

#[test]
fn s3_dp4_a_divergent_argument_of_a_call_in_a_condition_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP4",
        &format!(
            "{LABS}fn main() -> i64 {{\n    if unsafe {{ labs(unsafe {{ return 7 }}) }} > 0 {{ return 1 }}\n    return 0\n}}\n"
        ),
        7,
    );
}

#[test]
fn s3_dp5_a_divergent_if_expression_condition_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP5",
        "fn main() -> i64 {\n    let x = if unsafe { return 7 } > 0 { 1 } else { 2 }\n    return x\n}\n",
        7,
    );
}

#[test]
fn s3_dp6_a_divergent_left_operand_of_and_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP6",
        "fn main() -> i64 {\n    if (unsafe { return 7 } > 0) && true { return 1 }\n    return 0\n}\n",
        7,
    );
}

#[test]
fn s3_dp7_a_divergent_match_scrutinee_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP7",
        "enum E {\n    A(i64),\n    B(i64),\n}\n\nfn mk(x: i64) -> E { return E::A(x) }\nfn main() -> i64 {\n    let r = match mk(unsafe { return 7 }) {\n        E::A(v) => v,\n        E::B(v) => 0 - v,\n    }\n    return r\n}\n",
        7,
    );
}

#[test]
fn s3_dp8_a_divergent_foreach_iterable_answers_at_every_level() {
    answers_at_every_level(
        "S3-DP8",
        "fn mk(x: i64) -> [i64; 3] { return [x, x, x] }\nfn main() -> i64 {\n    for v in mk(unsafe { return 7 }) { return 1 }\n    return 0\n}\n",
        7,
    );
}
