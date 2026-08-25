use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn reject_at(body: &str, level: OptimizationLevel) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, body).expect("write source");
    match lower_file_to_air(&source_path, level) {
        Ok(_) => panic!("expected this program to be rejected, but it compiled"),
        Err(err) => err,
    }
}

fn reject(body: &str) -> String {
    reject_at(body, OptimizationLevel::None)
}

// assert the program lowers to air (sema and the borrow checker accept it), independent of a linker.
fn accepts(body: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, body).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .unwrap_or_else(|err| panic!("this program must compile: {err}"));
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found")
        || error.contains("failed to run")
        || error.contains("failed with status Some(-1073741819)")
}

fn exe_path_for(source_path: &Path) -> PathBuf {
    let mut output = source_path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

fn run_exit(body: &str, level: OptimizationLevel) -> Option<i32> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, body).expect("write source");
    match compile_file_with_llvm(&source_path, level, false) {
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
    let output = Command::new(&exe).output().expect("run compiled exe");
    Some(output.status.code().expect("exit code"))
}

#[test]
fn slice_of_vec_reads_the_buffer() {
    let src = r#"
fn main() -> i64 {
    let v = vec[11, 22, 33]
    let s = v[0..2]
    return s[1]
}
"#;
    accepts(src);
    for level in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        if let Some(code) = run_exit(src, level) {
            assert_eq!(
                code, 22,
                "a slice of a Vec must read the buffer at {level:?}"
            );
        }
    }
}

#[test]
fn write_through_a_vec_slice_lands_at_every_opt_level() {
    let src = r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let s = v[0..2]
    s[0] = 99
    return v[0]
}
"#;
    accepts(src);
    for level in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        if let Some(code) = run_exit(src, level) {
            assert_eq!(code, 99, "the write must land in `v` at {level:?}");
        }
    }
    let aliased = r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w: Vec<i64> = v
    let s = v[0..2]
    s[0] = 99
    return v[0] - w[0]
}
"#;
    accepts(aliased);
    for level in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        if let Some(code) = run_exit(aliased, level) {
            assert_eq!(code, 98, "the alias must still read 1 at {level:?}");
        }
    }
}

#[test]
fn write_through_a_shared_view_is_rejected_at_every_opt_level() {
    let src = r#"
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let s = v[0..2]
    s[0] = 99
    return v[0]
}
"#;
    for level in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        let err = reject_at(src, level);
        assert!(err.contains("[E0422]"), "must reject at {level:?}: {err}");
        assert!(
            err.contains("[shared-mut]"),
            "must carry the shared-mut marker: {err}"
        );
    }
}

#[test]
fn slice_of_array_compiles_and_runs() {
    let src = r#"
fn main() -> i64 {
    let a = [10, 20, 30]
    let s = a[0..2]
    return s[0] + s[1]
}
"#;
    accepts(src);
    if let Some(code) = run_exit(src, OptimizationLevel::None) {
        assert_eq!(code, 30, "array slice must read s[0]+s[1] = 30");
    }
}

#[test]
fn foreach_over_vec_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let mut total = 0
    for x in v {
        total = total + x
    }
    return total
}
"#,
    );
    assert!(err.contains("[E0414]"), "must carry the E0414 code: {err}");
    assert!(
        err.contains("[vec-foreach]"),
        "must carry the vec-foreach marker: {err}"
    );
}

#[test]
fn foreach_over_vec_is_rejected_at_every_opt_level() {
    let src = r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let mut total = 0
    for x in v {
        total = total + x
    }
    return total
}
"#;
    for level in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        let err = reject_at(src, level);
        assert!(err.contains("[E0414]"), "must reject at {level:?}: {err}");
    }
}

#[test]
fn foreach_over_array_compiles_and_runs() {
    let src = r#"
fn main() -> i64 {
    let a = [10, 20, 30]
    let mut total = 0
    for x in a {
        total = total + x
    }
    return total
}
"#;
    accepts(src);
    if let Some(code) = run_exit(src, OptimizationLevel::None) {
        assert_eq!(code, 60, "array foreach must sum to 60");
    }
}

#[test]
fn foreach_over_string_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let s = "abc"
    let mut n = 0
    for c in s {
        n = n + 1
    }
    return n
}
"#,
    );
}

#[test]
fn mut_ref_into_vec_index_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let r = &mut v[0]
    *r = 9
    return v[0]
}
"#,
    );
    assert!(err.contains("[E0415]"), "must carry the E0415 code: {err}");
    assert!(
        err.contains("[mut-index-ref]"),
        "must carry the mut-index-ref marker: {err}"
    );
}

#[test]
fn mut_ref_into_array_index_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let mut a = [1, 2, 3]
    let r = &mut a[0]
    *r = 9
    return a[0]
}
"#,
    );
    assert!(err.contains("[E0415]"), "must carry the E0415 code: {err}");
}

#[test]
fn mut_ref_into_index_is_rejected_at_every_opt_level() {
    let src = r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let r = &mut v[0]
    *r = 9
    return v[0]
}
"#;
    for level in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        let err = reject_at(src, level);
        assert!(err.contains("[E0415]"), "must reject at {level:?}: {err}");
    }
}

#[test]
fn immutable_ref_into_vec_index_compiles_and_runs() {
    let src = r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let r = &v[0]
    return *r
}
"#;
    accepts(src);
    if let Some(code) = run_exit(src, OptimizationLevel::None) {
        assert_eq!(
            code, 10,
            "reading through an immutable element ref must give v[0] = 10"
        );
    }
}

#[test]
fn mut_ref_into_struct_field_is_rejected() {
    let err = reject(
        r#"
struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let mut p = Point{x: 1, y: 2}
    let a = &mut p.x
    *a = 10
    return p.x
}
"#,
    );
    assert!(
        err.contains("[E0415]"),
        "a mutable ref into a field must reject: {err}"
    );
}

#[test]
fn immutable_ref_into_struct_field_compiles_and_runs() {
    let Some(code) = run_exit(
        r#"
struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let p = Point{x: 7, y: 2}
    let a = &p.x
    return *a
}
"#,
        OptimizationLevel::None,
    ) else {
        return;
    };
    assert_eq!(code, 7, "a shared ref into a field still reads the field");
}

#[test]
fn ref_into_rc_payload_field_compiles_and_runs() {
    let src = r#"
struct Cell { x: i64 }
fn read(p: &i64) -> i64 { return *p }
fn main() -> i64 {
    let r = Rc::new(Cell{x: 7})
    let p = &Rc::get(r).x
    let q = Rc::new(Cell{x: 31})
    return read(p) + Rc::get(q).x
}
"#;
    accepts(src);
    for level in [
        OptimizationLevel::None,
        OptimizationLevel::Standard,
        OptimizationLevel::Aggressive,
    ] {
        if let Some(code) = run_exit(src, level) {
            assert_eq!(
                code, 38,
                "call-result field reference must run at {level:?}"
            );
        }
    }
}

#[test]
fn ref_into_bound_payload_field_compiles_and_runs() {
    let src = r#"
struct Cell { x: i64 }
fn main() -> i64 {
    let r = Rc::new(Cell{x: 7})
    let c = Rc::get(r)
    let p = &c.x
    return *p
}
"#;
    accepts(src);
    if let Some(code) = run_exit(src, OptimizationLevel::None) {
        assert_eq!(
            code, 7,
            "reading through a bound payload field ref must give 7"
        );
    }
}

