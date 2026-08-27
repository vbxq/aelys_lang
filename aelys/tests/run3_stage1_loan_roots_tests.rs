use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use tempfile::tempdir;

const LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

static COMPILE_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    COMPILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn accepts(src: &str) {
    for level in LEVELS {
        let dir = tempdir().expect("tempdir");
        let source_path = dir.path().join("module.aelys");
        fs::write(&source_path, src).expect("write source");
        let guard = lock();
        let outcome = lower_file_to_air(&source_path, level);
        drop(guard);
        if let Err(err) = outcome {
            panic!("the borrow checker must accept this program at {level:?}: {err}");
        }
    }
}

fn rejects(src: &str) -> String {
    let mut first = None;
    for level in LEVELS {
        let dir = tempdir().expect("tempdir");
        let source_path = dir.path().join("module.aelys");
        fs::write(&source_path, src).expect("write source");
        let guard = lock();
        let outcome = lower_file_to_air(&source_path, level);
        drop(guard);
        match outcome {
            Ok(_) => panic!("the program must be rejected at {level:?}, but it compiled"),
            Err(err) => {
                if first.is_none() {
                    first = Some(err);
                }
            }
        }
    }
    first.expect("one level at least")
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found")
        || error.contains("failed to run")
        || error.contains("failed with status Some(-1073741819)")
}

const RUN_LEVELS: [OptimizationLevel; 3] = [
    OptimizationLevel::None,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

fn run(src: &str, level: OptimizationLevel, allocator: &str) -> Option<(i32, String, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    let guard = lock();
    let outcome = compile_file_with_llvm(&source_path, level, false);
    drop(guard);
    match outcome {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping exec assertion");
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }
    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        eprintln!("executable not produced (linker unavailable); skipping");
        return None;
    }
    let out = Command::new(&exe)
        .env("AELYS_RC_STATS", "1")
        .env("AELYS_ALLOC", allocator)
        .output()
        .expect("run compiled exe");
    Some((
        out.status.code().expect("exit code"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn accepts_and_runs(src: &str, stdout: &str, exit: i32, stats: &str) {
    accepts(src);
    for level in RUN_LEVELS {
        for allocator in ["immix", "malloc"] {
            let Some((code, got_out, got_err)) = run(src, level, allocator) else {
                return;
            };
            assert_eq!(
                code, exit,
                "exit code at {level:?}/{allocator}; stdout:\n{got_out}stderr:\n{got_err}"
            );
            assert_eq!(
                got_out, stdout,
                "stdout at {level:?}/{allocator}; stderr:\n{got_err}"
            );
            assert!(
                got_err.contains(stats),
                "expected `{stats}` at {level:?}/{allocator}; got stderr:\n{got_err}"
            );
        }
    }
}

const STRUCT_S: &str = "struct S { r: Rc<i64>, a: [i64;3] }\n";

#[test]
fn w1_write_through_a_shared_view_of_a_non_mut_vec_is_e0422() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    s[0] = 9
    return s[0]
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    assert!(err.contains("shared `&[i64]`"), "got: {err}");
    accepts_and_runs(
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    s[0] = 9
    println(s[0])
    return 0
}
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn w2_write_through_a_slice_of_a_non_mut_array_is_e0422() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    s[0] = 9
    return s[0]
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
fn main() -> i64 {
    let mut a: [i64;3] = [1,2,3]
    let s = a[..]
    s[0] = 9
    return s[0]
}
"#,
        "",
        9,
        "allocs=0 frees=0",
    );
}

#[test]
fn w3_reborrow_of_a_shared_vec_ref_is_e0422() {
    let err = rejects(
        r#"
fn get(r: &Vec<i64>) -> i64 {
    let s = (*r)[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts(
        r#"
fn get(r: &mut Vec<i64>) -> i64 {
    let s = (*r)[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return 0 }
"#,
    );
}

#[test]
fn w4_reslice_of_a_shared_view_stays_shared_and_is_e0422() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    let t = s[..]
    t[0] = 9
    return t[0]
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    let t = s[..]
    t[0] = 9
    println(t[0])
    return 0
}
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn w5_copy_of_a_shared_view_is_e0422() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    let t = s
    t[0] = 9
    return t[0]
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    let t = s
    t[0] = 9
    println(t[0])
    return 0
}
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn w6_a_callee_writing_its_shared_slice_parameter_is_e0422() {
    let err = rejects(
        r#"
fn wr(s: &[i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    return wr(s)
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    assert!(err.contains("declare the slice"), "got: {err}");
    accepts_and_runs(
        r#"
fn wr(s: &mut [i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    println(wr(s))
    return 0
}
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn w7_an_array_slice_passed_to_a_shared_writer_is_e0422() {
    let err = rejects(
        r#"
fn wr(s: &[i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    println(wr(s))
    return 0
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
fn wr(s: &mut [i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let mut a: [i64;3] = [1,2,3]
    let s = a[..]
    println(wr(s))
    return 0
}
"#,
        "9\n",
        0,
        "allocs=0 frees=0",
    );
}

#[test]
fn w8_the_write_one_frame_down_is_refused_in_the_callee() {
    let err = rejects(
        r#"
fn rec(s: &[i64], n: i64) -> i64 {
    if n == 0 {
        s[0] = 9
        return s[0]
    }
    return rec(s, n - 1)
}
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    println(rec(s, 2))
    return 0
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
fn rec(s: &mut [i64], n: i64) -> i64 {
    if n == 0 {
        s[0] = 9
        return s[0]
    }
    return rec(s, n - 1)
}
fn main() -> i64 {
    let mut a: [i64;3] = [1,2,3]
    let s = a[..]
    println(rec(s, 2))
    return 0
}
"#,
        "9\n",
        0,
        "allocs=0 frees=0",
    );
}

#[test]
fn w9_vec_slice_passed_to_a_reader_runs() {
    accepts_and_runs(
        r#"
fn rd(s: &[i64]) -> i64 { return s[0] }
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    println(rd(s))
    return 0
}
"#,
        "1\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn w10_write_to_a_borrowed_vec_is_e0711() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let r = &v[0]
    v = vec[4,5,6]
    return *r
}
"#,
    );
    assert!(err.contains("E0711"), "got: {err}");
}

#[test]
fn w11_write_to_a_borrowed_array_is_e0711() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let mut a: [i64;3] = [1,2,3]
    let s = a[..]
    a = [4,5,6]
    return s[0]
}
"#,
    );
    assert!(err.contains("E0711"), "got: {err}");
}

#[test]
fn w12_write_to_a_borrowed_scalar_is_e0711() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let mut x: i64 = 1
    let r = &x
    x = 5
    return *r
}
"#,
    );
    assert!(err.contains("E0711"), "got: {err}");
}

#[test]
fn w13_mut_ref_into_a_vec_element_is_still_e0415() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let r = &mut v[0]
    return 0
}
"#,
    );
    assert!(err.contains("E0415"), "got: {err}");
}

#[test]
fn w14_amp_of_a_slice_expression_is_still_e0421() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = &v[..]
    return 0
}
"#,
    );
    assert!(err.contains("E0421"), "got: {err}");
}

#[test]
fn w16_rc_of_vec_is_still_fenced() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let v: Rc<Vec<i64>> = Rc::new(vec[1,2,3])
    return 0
}
"#,
    );
    assert!(err.contains("E0412"), "got: {err}");
}

#[test]
fn w17_ref_to_rc_at_a_let_is_still_e0410() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let p: &Rc<i64> = &r
    return 0
}
"#,
    );
    assert!(err.contains("E0410"), "got: {err}");
}

#[test]
fn w19_borrow_outliving_its_vec_is_e0722() {
    let err = rejects(
        r#"
fn main() -> i64 {
    let x: i64 = 0
    let mut p: &i64 = &x
    if true {
        let v: Vec<i64> = vec[1,2,3]
        p = &v[0]
    }
    return *p
}
"#,
    );
    assert!(err.contains("E0722"), "got: {err}");
}

#[test]
fn w20_the_write_two_frames_down_is_refused_where_it_is_written() {
    let err = rejects(
        r#"
fn wr(s: &[i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn mid(s: &[i64]) -> i64 { return wr(s) }
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    return mid(s)
}
"#,
    );
    // the refusal is intraprocedural now: it names `wr`'s own body, not the caller's argument,
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
fn wr(s: &mut [i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn mid(s: &mut [i64]) -> i64 { return wr(s) }
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    println(mid(s))
    return 0
}
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn w21_a_recursive_writer_of_a_shared_slice_is_e0422() {
    let err = rejects(
        r#"
fn rec(s: &[i64], n: i64) -> i64 {
    if n == 0 {
        s[0] = 9
        return s[0]
    }
    return rec(s, n - 1)
}
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    return rec(s, 2)
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
fn rec(s: &mut [i64], n: i64) -> i64 {
    if n == 0 {
        s[0] = 9
        return s[0]
    }
    return rec(s, n - 1)
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    println(rec(s, 2))
    return 0
}
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn u1_slice_of_an_array_field_next_to_an_rc_field_needs_a_mut_root() {
    let err = rejects(&format!(
        r#"{STRUCT_S}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}}
"#
    ));
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn main() -> i64 {{
    let mut x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}}
"#
        ),
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn u2_that_slice_passed_to_a_writer_needs_a_mut_view() {
    let err = rejects(&format!(
        r#"{STRUCT_S}
fn w(s: &[i64]) -> i64 {{
    s[0] = 9
    return s[0]
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    println(w(s))
    return 0
}}
"#
    ));
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn w(s: &mut [i64]) -> i64 {{
    s[0] = 9
    return s[0]
}}
fn main() -> i64 {{
    let mut x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    println(w(s))
    return 0
}}
"#
        ),
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn u3_nested_struct_holding_the_rc_needs_a_mut_root() {
    let err = rejects(
        r#"
struct Inner { r: Rc<i64> }
struct Outer { i: Inner, a: [i64;3] }
fn main() -> i64 {
    let x: Outer = Outer { i: Inner { r: Rc::new(5) }, a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
struct Inner { r: Rc<i64> }
struct Outer { i: Inner, a: [i64;3] }
fn main() -> i64 {
    let mut x: Outer = Outer { i: Inner { r: Rc::new(5) }, a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn u5_the_same_struct_without_an_rc_field_runs_without_allocating() {
    let err = rejects(
        r#"
struct T { b: i64, a: [i64;3] }
fn main() -> i64 {
    let x: T = T { b: 1, a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
struct T { b: i64, a: [i64;3] }
fn main() -> i64 {
    let mut x: T = T { b: 1, a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
"#,
        "9\n",
        0,
        "allocs=0 frees=0",
    );
}

#[test]
fn u7_the_struct_as_a_by_value_parameter_needs_a_mut_parameter() {
    let err = rejects(&format!(
        r#"{STRUCT_S}
fn f(x: S) -> i64 {{
    let s = x.a[..]
    s[0] = 9
    return s[0]
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    println(f(x))
    return 0
}}
"#
    ));
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn f(mut x: S) -> i64 {{
    let s = x.a[..]
    s[0] = 9
    return s[0]
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    println(f(x))
    return 0
}}
"#
        ),
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn u8_reslice_off_the_array_field_needs_a_mut_root() {
    let err = rejects(&format!(
        r#"{STRUCT_S}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    let t = s[..]
    t[0] = 9
    println(t[0])
    return 0
}}
"#
    ));
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn main() -> i64 {{
    let mut x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    let t = s[..]
    t[0] = 9
    println(t[0])
    return 0
}}
"#
        ),
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn u9_the_same_field_slice_reached_through_a_shared_reference_is_e0422() {
    let err = rejects(&format!(
        r#"{STRUCT_S}
fn f(w: &S) -> i64 {{
    let s = (*w).a[..]
    s[0] = 9
    println(s[0])
    return 0
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    return f(&x)
}}
"#
    ));
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn f(w: &mut S) -> i64 {{
    let s = (*w).a[..]
    s[0] = 9
    println(s[0])
    return 0
}}
fn main() -> i64 {{
    let mut x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    return f(&mut x)
}}
"#
        ),
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn direct_field_write_on_a_non_mut_binding_is_still_accepted() {
    accepts_and_runs(
        r#"
struct T { b: i64, a: [i64;3] }
fn main() -> i64 {
    let x: T = T { b: 1, a: [4,5,6] }
    x.a[0] = 9
    println(x.a[0])
    return 0
}
"#,
        "9\n",
        0,
        "allocs=0 frees=0",
    );
    let err = rejects(
        r#"
fn main() -> i64 {
    let a: [i64;3] = [4,5,6]
    a[0] = 9
    println(a[0])
    return 0
}
"#,
    );
    assert!(err.contains("E0401"), "got: {err}");
}

#[test]
fn u10_borrowing_the_whole_struct_runs() {
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn take(w: &S) -> i64 {{ return 0 }}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let q = take(&x)
    println(q)
    return 0
}}
"#
        ),
        "0\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn u11_a_ref_peeled_place_with_no_field_projection_runs() {
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn f(w: &S) -> i64 {{
    let q = &*w
    return 0
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    return f(&x)
}}
"#
        ),
        "",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn i1_indirect_call_with_an_rc_argument_needs_a_mut_result() {
    let err = rejects(
        r#"
struct S { a: [i64;3] }
fn pick(p: &Rc<i64>, x: &S) -> &S { return x }
fn go(g: fn(&Rc<i64>, &S) -> &S) -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let x: S = S { a: [4,5,6] }
    let w = g(&r, &x)
    let s = (*w).a[..]
    s[0] = 9
    println(s[0])
    return 0
}
fn main() -> i64 { return go(pick) }
"#,
    );
    assert!(err.contains("E0422"), "got: {err}");
    accepts_and_runs(
        r#"
struct S { a: [i64;3] }
fn pick(p: &Rc<i64>, x: &mut S) -> &mut S { return x }
fn go(g: fn(&Rc<i64>, &mut S) -> &mut S) -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let mut x: S = S { a: [4,5,6] }
    let w = g(&r, &mut x)
    let s = (*w).a[..]
    s[0] = 9
    println(s[0])
    return 0
}
fn main() -> i64 { return go(pick) }
"#,
        "9\n",
        0,
        "allocs=1 frees=1",
    );
}

const IND2_CTL: &str = r#"
struct S { a: [i64;3] }
fn pick(p: &Vec<i64>, x: &S) -> &S { return x }
fn go(g: fn(&Vec<i64>, &S) -> &S) -> i64 {
    let r: Vec<i64> = vec[5,6,7]
    let x: S = S { a: [4,5,6] }
    let w = g(&r, &x)
    let s = (*w).a[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return go(pick) }
"#;

#[test]
fn i2_the_vec_twin_of_that_indirect_call_is_e0422_for_the_same_reason() {
    let err = rejects(IND2_CTL);
    assert!(err.contains("E0422"), "got: {err}");
}

// view's own mutability
#[test]
fn x1_the_vec_in_the_message_is_gone_and_both_spellings_agree() {
    let err = rejects(IND2_CTL);
    assert!(
        !err.contains("it is a slice of a `Vec`"),
        "the message that named a Vec the slice does not come from is back; got: {err}"
    );
    let direct = r#"
struct S { a: [i64;3] }
fn pick(p: &Vec<i64>, x: &S) -> &S { return x }
fn go() -> i64 {
    let r: Vec<i64> = vec[5,6,7]
    let x: S = S { a: [4,5,6] }
    let w = pick(&r, &x)
    let s = (*w).a[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return go() }
"#;
    let direct_err = rejects(direct);
    assert!(direct_err.contains("E0422"), "got: {direct_err}");
    accepts(
        r#"
struct S { a: [i64;3] }
fn pick(p: &Vec<i64>, x: &mut S) -> &mut S { return x }
fn go() -> i64 {
    let r: Vec<i64> = vec[5,6,7]
    let mut x: S = S { a: [4,5,6] }
    let w = pick(&r, &mut x)
    let s = (*w).a[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return go() }
"#,
    );
}

#[test]
fn e1_an_enum_carrying_an_rc_runs_with_a_live_refcount() {
    accepts_and_runs(
        r#"
enum E { A(Rc<i64>) }
fn f(e: &E) -> i64 { return 0 }
fn main() -> i64 {
    let e: E = E::A(Rc::new(7))
    let q = f(&e)
    println(q)
    return 0
}
"#,
        "0\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn e2_the_plain_enum_twin_runs_without_allocating() {
    accepts_and_runs(
        r#"
enum E { A(i64), B(i64) }
fn f(e: &E) -> i64 { return 0 }
fn main() -> i64 {
    let e: E = E::A(1)
    let q = f(&e)
    println(q)
    return 0
}
"#,
        "0\n",
        0,
        "allocs=0 frees=0",
    );
}

#[test]
fn f1_an_array_of_a_struct_holding_an_rc_is_accepted() {
    accepts(
        r#"
struct S { r: Rc<i64> }
fn f(a: [S;2]) -> i64 {
    let p = &a
    return 0
}
fn main() -> i64 { return 0 }
"#,
    );
}

#[test]
fn c1r_a_borrowed_rc_with_no_projection_runs() {
    accepts_and_runs(
        r#"
fn take(p: &Rc<i64>) -> i64 { return 0 }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let q = take(&r)
    println(q)
    return 0
}
"#,
        "0\n",
        0,
        "allocs=1 frees=1",
    );
}

#[test]
fn c3r_a_borrowed_scalar_runs_without_allocating() {
    accepts_and_runs(
        r#"
fn main() -> i64 {
    let x: i64 = 1
    let r = &x
    return *r
}
"#,
        "",
        1,
        "allocs=0 frees=0",
    );
}

#[test]
fn u12_a_used_peeled_ref_runs() {
    accepts_and_runs(
        &format!(
            r#"{STRUCT_S}
fn f(w: &S) -> i64 {{
    let q = &*w
    return (*q).a[0]
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    println(f(&x))
    return 0
}}
"#
        ),
        "4\n",
        0,
        "allocs=1 frees=1",
    );
}

fn dump(src: &str) -> Vec<String> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    let dump_path = dir.path().join("loanroots.txt");
    fs::write(&source_path, src).expect("write source");

    let guard = lock();
    unsafe { std::env::set_var("AELYS_DUMP_LOAN_ROOTS", &dump_path) };
    let outcome = lower_file_to_air(&source_path, OptimizationLevel::None);
    unsafe { std::env::remove_var("AELYS_DUMP_LOAN_ROOTS") };
    drop(guard);

    let text = fs::read_to_string(&dump_path).expect("the dump file must exist");
    let raw: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with("LOANROOT"))
        .map(str::to_string)
        .collect();
    let mut distinct = raw.clone();
    distinct.sort();
    distinct.dedup();
    assert!(!distinct.is_empty(), "the dump must not be empty:\n{text}");
    let expected = if outcome.is_err() {
        distinct.len()
    } else {
        2 * distinct.len()
    };
    assert_eq!(
        raw.len(),
        expected,
        "raw lines vs distinct lines; the dump was:\n{text}"
    );
    distinct
}

fn assert_class(src: &str, expected: &[&str]) {
    assert_eq!(dump(src), expected, "loan root classification");
}

#[test]
fn dump_stays_off_without_the_env_var() {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    let dump_path = dir.path().join("loanroots.txt");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    return s[0]
}
"#,
    )
    .expect("write source");
    let guard = lock();
    let outcome = lower_file_to_air(&source_path, OptimizationLevel::None);
    drop(guard);
    assert!(outcome.is_ok(), "the control program must compile");
    assert!(
        !dump_path.exists(),
        "the dump must write nothing when the env var is unset"
    );
}

#[test]
fn c1r_an_rc_root_with_no_projection_is_managed() {
    assert_class(
        r#"
fn take(p: &Rc<i64>) -> i64 { return 0 }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let q = take(&r)
    println(q)
    return 0
}
"#,
        &[r#"LOANROOT fn=main loan=0 root=%1 proj=[] ty=Rc(I64) class=Managed"#],
    );
}

#[test]
fn c2r_a_slice_root_is_unknown() {
    assert_class(
        &format!(
            r#"{STRUCT_S}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    let t = s[..]
    println(t[0])
    return 0
}}
"#
        ),
        &[
            r#"LOANROOT fn=main loan=0 root=%3 proj=[Field(a)] ty=Struct("S") class=Unknown"#,
            r#"LOANROOT fn=main loan=1 root=%5 proj=[] ty=Slice { elem: I64, mutable: false } class=Unknown"#,
        ],
    );
}

#[test]
fn c3r_an_array_of_scalars_and_a_scalar_are_unmanaged() {
    assert_class(
        r#"
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    return s[0]
}
"#,
        &[r#"LOANROOT fn=main loan=0 root=%1 proj=[] ty=Array(I64, Some(3)) class=Unmanaged"#],
    );
    assert_class(
        r#"
fn main() -> i64 {
    let x: i64 = 1
    let r = &x
    return *r
}
"#,
        &[r#"LOANROOT fn=main loan=0 root=%0 proj=[] ty=I64 class=Unmanaged"#],
    );
}

#[test]
fn u1_class_a_field_projection_is_unknown() {
    assert_class(
        &format!(
            r#"{STRUCT_S}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let s = x.a[..]
    println(s[0])
    return 0
}}
"#
        ),
        &[r#"LOANROOT fn=main loan=0 root=%3 proj=[Field(a)] ty=Struct("S") class=Unknown"#],
    );
}

#[test]
fn u5_class_the_unmanaged_twin_is_unknown_too() {
    assert_class(
        r#"
struct T { b: i64, a: [i64;3] }
fn main() -> i64 {
    let x: T = T { b: 1, a: [4,5,6] }
    let s = x.a[..]
    println(s[0])
    return 0
}
"#,
        &[r#"LOANROOT fn=main loan=0 root=%2 proj=[Field(a)] ty=Struct("T") class=Unknown"#],
    );
}

#[test]
fn u9_class_agrees_across_the_reference_boundary() {
    assert_class(
        &format!(
            r#"{STRUCT_S}
fn f(w: &S) -> i64 {{
    let s = (*w).a[..]
    println(s[0])
    return 0
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    return f(&x)
}}
"#
        ),
        &[
            r#"LOANROOT fn=f loan=0 root=%0 proj=[Deref,Field(a)] ty=Ref { referent: Struct("S"), mutable: false } class=Unknown"#,
            r#"LOANROOT fn=main loan=0 root=%3 proj=[] ty=Struct("S") class=Managed"#,
        ],
    );
}

#[test]
fn u10_class_the_whole_struct_is_managed() {
    assert_class(
        &format!(
            r#"{STRUCT_S}
fn take(w: &S) -> i64 {{ return 0 }}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    let q = take(&x)
    println(q)
    return 0
}}
"#
        ),
        &[r#"LOANROOT fn=main loan=0 root=%3 proj=[] ty=Struct("S") class=Managed"#],
    );
}

#[test]
fn u11_class_a_peeled_ref_with_no_field_is_unknown() {
    assert_class(
        &format!(
            r#"{STRUCT_S}
fn f(w: &S) -> i64 {{
    let q = &*w
    return 0
}}
fn main() -> i64 {{
    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}
    return f(&x)
}}
"#
        ),
        &[
            r#"LOANROOT fn=f loan=0 root=%0 proj=[Deref] ty=Ref { referent: Struct("S"), mutable: false } class=Unknown"#,
            r#"LOANROOT fn=main loan=0 root=%3 proj=[] ty=Struct("S") class=Managed"#,
        ],
    );
}

#[test]
fn i1_class_the_rc_loan_is_managed_and_the_program_still_compiles() {
    assert_class(
        r#"
struct S { a: [i64;3] }
fn pick(p: &Rc<i64>, x: &S) -> &S { return x }
fn go(g: fn(&Rc<i64>, &S) -> &S) -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let x: S = S { a: [4,5,6] }
    let w = g(&r, &x)
    let s = (*w).a[..]
    println(s[0])
    return 0
}
fn main() -> i64 { return go(pick) }
"#,
        &[
            r#"LOANROOT fn=go loan=0 root=%2 proj=[] ty=Rc(I64) class=Managed"#,
            r#"LOANROOT fn=go loan=1 root=%5 proj=[] ty=Struct("S") class=Unmanaged"#,
            r#"LOANROOT fn=go loan=2 root=%9 proj=[Deref,Field(a)] ty=Ref { referent: Struct("S"), mutable: false } class=Unknown"#,
        ],
    );
}

#[test]
fn e1_class_an_enum_holding_an_rc_is_managed() {
    assert_class(
        r#"
enum E { A(Rc<i64>) }
fn f(e: &E) -> i64 { return 0 }
fn main() -> i64 {
    let e: E = E::A(Rc::new(7))
    let q = f(&e)
    println(q)
    return 0
}
"#,
        &[r#"LOANROOT fn=main loan=0 root=%2 proj=[] ty=Enum("E", []) class=Managed"#],
    );
}

#[test]
fn e2_class_the_plain_enum_twin_is_unmanaged() {
    assert_class(
        r#"
enum E { A(i64), B(i64) }
fn f(e: &E) -> i64 { return 0 }
fn main() -> i64 {
    let e: E = E::A(1)
    let q = f(&e)
    println(q)
    return 0
}
"#,
        &[r#"LOANROOT fn=main loan=0 root=%1 proj=[] ty=Enum("E", []) class=Unmanaged"#],
    );
}

#[test]
fn f1_class_an_array_of_a_managed_struct_is_managed() {
    assert_class(
        r#"
struct S { r: Rc<i64> }
fn f(a: [S;2]) -> i64 {
    let p = &a
    return 0
}
fn main() -> i64 { return 0 }
"#,
        &[r#"LOANROOT fn=f loan=0 root=%0 proj=[] ty=Array(Struct("S"), Some(2)) class=Managed"#],
    );
}

#[test]
fn a_rejected_program_dumps_each_loan_once() {
    assert_class(
        r#"
nogc fn f() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    return s[0]
}
fn main() -> i64 { return f() }
"#,
        &[r#"LOANROOT fn=f loan=0 root=%1 proj=[] ty=Vec(I64) class=Managed"#],
    );
}
#[cfg(feature = "asan-invariants")]
mod asan {
    use super::*;

    fn build_asan_archive(dir: &Path) -> Option<PathBuf> {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let core_src = manifest
            .parent()
            .unwrap_or(manifest)
            .join("core")
            .join("src");
        let mut objects = Vec::new();
        for unit in [
            "aelys_core_common.c",
            "aelys_alloc_immix.c",
            "aelys_rc_real.c",
        ] {
            let src = core_src.join(unit);
            if !src.is_file() {
                eprintln!("core source {unit} missing; skipping the ASan tier");
                return None;
            }
            let obj = dir.join(unit).with_extension("o");
            match Command::new("clang")
                .args(["-fsanitize=address", "-g", "-c"])
                .arg(&src)
                .arg(format!("-I{}", core_src.display()))
                .arg("-o")
                .arg(&obj)
                .output()
            {
                Ok(out) if out.status.success() => objects.push(obj),
                Ok(out) => panic!(
                    "instrumented core compile of {unit} failed:\n{}",
                    String::from_utf8_lossy(&out.stderr)
                ),
                Err(_) => {
                    eprintln!("clang unavailable; skipping the ASan tier");
                    return None;
                }
            }
        }
        let archive = dir.join("libaelys-core-rc-asan.a");
        match Command::new("ar")
            .arg("rcs")
            .arg(&archive)
            .args(&objects)
            .output()
        {
            Ok(out) if out.status.success() => Some(archive),
            Ok(out) => panic!("ar failed:\n{}", String::from_utf8_lossy(&out.stderr)),
            Err(_) => {
                eprintln!("ar unavailable; skipping the ASan tier");
                None
            }
        }
    }

    fn link_instrumented(dir: &Path, id: &str, src: &str) -> Option<PathBuf> {
        let path = dir.join(format!("{id}.aelys"));
        fs::write(&path, src).expect("write source");
        let guard = lock();
        let built = compile_file_with_llvm(&path, OptimizationLevel::None, false);
        drop(guard);
        if built.is_err() {
            eprintln!("{id}: toolchain unavailable, skipping");
            return None;
        }
        let object = path.with_extension(if cfg!(windows) { "obj" } else { "o" });
        if !object.is_file() {
            eprintln!("{id}: object not produced, skipping");
            return None;
        }
        let exe = dir.join(format!("{id}_asan_exe"));
        let link = Command::new("clang")
            .args(["-fsanitize=address", "-g"])
            .arg(&object)
            .arg(format!("-L{}", dir.display()))
            .arg("-laelys-core-rc-asan")
            .arg("-o")
            .arg(&exe)
            .output()
            .expect("clang link");
        assert!(
            link.status.success(),
            "{id}: ASan link failed:\n{}",
            String::from_utf8_lossy(&link.stderr)
        );
        Some(exe)
    }

    // a print-free row leaves no allocation behind, so the leak oracle below can stay strict
    fn asan_rows() -> Vec<(&'static str, String, i32)> {
        vec![
            (
                "u1",
                format!(
                    "{STRUCT_S}\nfn main() -> i64 {{\n    let mut x: S = S {{ r: Rc::new(5), a: [4,5,6] }}\n    let s = x.a[..]\n    s[0] = 9\n    return s[0]\n}}\n"
                ),
                9,
            ),
            (
                "u9",
                format!(
                    "{STRUCT_S}\nfn f(w: &mut S) -> i64 {{\n    let s = (*w).a[..]\n    s[0] = 9\n    return s[0]\n}}\nfn main() -> i64 {{\n    let mut x: S = S {{ r: Rc::new(5), a: [4,5,6] }}\n    return f(&mut x)\n}}\n"
                ),
                9,
            ),
            (
                "u12",
                format!(
                    "{STRUCT_S}\nfn f(w: &S) -> i64 {{\n    let q = &*w\n    return (*q).a[0]\n}}\nfn main() -> i64 {{\n    let x: S = S {{ r: Rc::new(5), a: [4,5,6] }}\n    return f(&x)\n}}\n"
                ),
                4,
            ),
            (
                "i1",
                r#"
struct S { a: [i64;3] }
fn pick(p: &Rc<i64>, x: &mut S) -> &mut S { return x }
fn go(g: fn(&Rc<i64>, &mut S) -> &mut S) -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let mut x: S = S { a: [4,5,6] }
    let w = g(&r, &mut x)
    let s = (*w).a[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return go(pick) }
"#
                .to_string(),
                9,
            ),
            (
                "e1",
                r#"
enum E { A(Rc<i64>) }
fn f(e: &E) -> i64 { return 0 }
fn main() -> i64 {
    let e: E = E::A(Rc::new(7))
    return f(&e)
}
"#
                .to_string(),
                0,
            ),
        ]
    }

    #[test]
    fn asan_tier_is_armed_and_the_managed_rows_are_clean() {
        let dir = tempdir().expect("tempdir");
        let Some(_archive) = build_asan_archive(dir.path()) else {
            panic!("the ASan tier must build its archive on a machine with clang and ar");
        };
        let rows = asan_rows();
        let mut armed = false;
        for (id, src, exit) in &rows {
            let Some(exe) = link_instrumented(dir.path(), id, src) else {
                panic!("{id}: the ASan tier must link");
            };
            if !armed {
                let control = Command::new(&exe)
                    .env("ASAN_OPTIONS", "verbosity=1")
                    .output()
                    .expect("run the instrumented exe");
                let control_err = String::from_utf8_lossy(&control.stderr);
                assert!(
                    control_err.contains("AddressSanitizer"),
                    "{id}: the binary is not instrumented; got stderr:\n{control_err}"
                );
                armed = true;
            }
            let run = Command::new(&exe)
                .env("AELYS_RC_STATS", "1")
                .env("ASAN_OPTIONS", "detect_leaks=1")
                .output()
                .expect("run the instrumented exe");
            let err = String::from_utf8_lossy(&run.stderr);
            assert_eq!(
                run.status.code(),
                Some(*exit),
                "{id} under ASan; stderr:\n{err}"
            );
            assert!(
                !err.contains("AddressSanitizer:"),
                "{id}: the sanitizer reports a memory error\nstderr:\n{err}"
            );
            assert!(
                err.contains("allocs=1 frees=1"),
                "{id}: expected a balanced refcount; stderr:\n{err}"
            );
        }
        assert_eq!(rows.len(), 5, "the ASan tier must cover five rows");
    }

    // the leak is in the bootstrap integer-to-string helper and predates this stage, so the row
    #[test]
    fn asan_tier_pins_the_known_println_leak() {
        let dir = tempdir().expect("tempdir");
        let Some(_archive) = build_asan_archive(dir.path()) else {
            panic!("the ASan tier must build its archive on a machine with clang and ar");
        };
        let src = format!(
            "{STRUCT_S}\nfn main() -> i64 {{\n    let mut x: S = S {{ r: Rc::new(5), a: [4,5,6] }}\n    let s = x.a[..]\n    s[0] = 9\n    println(s[0])\n    return 0\n}}\n"
        );
        let Some(exe) = link_instrumented(dir.path(), "known_leak", &src) else {
            panic!("the ASan tier must link");
        };
        let run = Command::new(&exe)
            .env("AELYS_RC_STATS", "1")
            .env("ASAN_OPTIONS", "detect_leaks=1")
            .output()
            .expect("run the instrumented exe");
        let err = String::from_utf8_lossy(&run.stderr);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "9\n",
            "stderr:\n{err}"
        );
        assert!(
            err.contains("allocs=1 frees=1"),
            "the managed refcount must still balance; stderr:\n{err}"
        );
        assert!(
            err.contains("LeakSanitizer: detected memory leaks"),
            "the known println leak stopped reproducing, so this row must be re-decided;\nstderr:\n{err}"
        );
        assert!(
            err.contains("__aelys_to_string_i64"),
            "a leak appeared that is not the known bootstrap one; stderr:\n{err}"
        );
        let stacks = err.matches("allocated from:").count();
        assert_eq!(stacks, 1, "exactly one leak stack is known; stderr:\n{err}");
    }
}

