use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const RESOURCE: &str = "struct Resource { id: i64 }\n";

// compile only up to air lowering (where the escape check lives): needs no linker, always runs.
fn reject(body: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");
    match lower_file_to_air(&source_path, OptimizationLevel::None) {
        Ok(_) => panic!("expected the escape checker to reject this program, but it compiled"),
        Err(err) => err,
    }
}

fn accepts(body: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .unwrap_or_else(|err| panic!("the escape checker must accept this program: {err}"));
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

fn run_exit(body: &str) -> Option<i32> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");

    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
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

// a function returning a reference to one of its own locals escapes; the local dies on return.
#[test]
fn return_ref_to_local_is_rejected() {
    let err = reject(
        r#"
fn bad() -> &i64 {
    let x = 5
    return &x
}
fn main() -> i64 {
    return 0
}
"#,
    );
    assert!(
        err.contains("[escape]"),
        "must carry the escape marker: {err}"
    );
    assert!(
        err.contains("returns a reference to local"),
        "must be the return-ref-to-local diagnostic: {err}"
    );
}

#[test]
fn return_ref_to_param_compiles() {
    accepts(
        r#"
fn good(r: &i64) -> &i64 {
    return r
}
fn main() -> i64 {
    return 0
}
"#,
    );
}

#[test]
fn stage3_return_ref_gap_is_rejected() {
    let err = reject(
        r#"
fn touch(r: &Resource) {
}
fn get(r: &Resource) -> &Resource {
    return r
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = get(&a)
    let b = a
    touch(r)
    return 0
}
"#,
    );
    assert!(
        err.contains("[borrow]"),
        "must carry the borrow marker: {err}"
    );
    assert!(
        err.contains("cannot move"),
        "the move while the returned ref is live must reject: {err}"
    );
}

#[test]
fn stage3_gap_twin_borrow_dead_before_move_compiles() {
    accepts(
        r#"
fn touch(r: &Resource) {
}
fn get(r: &Resource) -> &Resource {
    return r
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = get(&a)
    touch(r)
    let b = a
    return 0
}
"#,
    );
}

// r is repointed to an inner-scope borrow via a temp; the loan on `x` flows
#[test]
fn temp_repoint_uaf_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = 10
    let mut r = &a
    {
        let x = 5
        let tmp = &x
        r = tmp
    }
    return *r
}
"#,
    );
    assert!(
        err.contains("[escape]"),
        "must carry the escape marker: {err}"
    );
    assert!(
        err.contains("does not live long enough"),
        "must be the scope-death escape diagnostic: {err}"
    );
}

// the reborrow twin `r = &*tmp` rejects the same way, via reborrow inheritance.
#[test]
fn temp_repoint_reborrow_twin_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = 10
    let mut r = &a
    {
        let x = 5
        let tmp = &x
        r = &*tmp
    }
    return *r
}
"#,
    );
    assert!(
        err.contains("[escape]"),
        "must carry the escape marker: {err}"
    );
    assert!(
        err.contains("does not live long enough"),
        "must be the scope-death escape diagnostic: {err}"
    );
}

// discriminator: a same-scope borrow whose last use precedes the scope death compiles.
#[test]
fn same_scope_borrow_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let a = 10
    let r = &a
    {
        let x = 5
        let s = &x
        let y = *s
    }
    return *r
}
"#,
    );
}

#[test]
fn safe_reference_repoint_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let a = 10
    let b = 20
    let mut r = &a
    r = &b
    return *r
}
"#,
    );
}

// projected ref-store uaf: a reference reaches a container through a projected index store, which
// the aggregate-literal guard never sees. rejected at build time.
#[test]
fn projected_ref_store_uaf_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let mut arr = []
    let x = 5
    arr[0] = &x
    return 0
}
"#,
    );
    assert!(
        err.contains("[escape]"),
        "must carry the escape marker: {err}"
    );
    assert!(
        err.contains("aggregate containers"),
        "must be the container-boundary diagnostic: {err}"
    );
}

#[test]
fn projected_value_store_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let mut arr = [1, 2, 3]
    arr[0] = 5
    return arr[0]
}
"#,
    );
}

// d3: a closure capturing a reference is rejected (conservative, no bir trace to check later).
#[test]
fn closure_capturing_reference_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = 10
    let r = &a
    let f = fn() -> i64 {
        return *r
    }
    return 0
}
"#,
    );
    assert!(
        err.contains("[escape]"),
        "must carry the escape marker: {err}"
    );
    assert!(
        err.contains("closure"),
        "must be the closure-capture diagnostic: {err}"
    );
}

#[test]
fn closure_capturing_value_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let a = 10
    let f = fn() -> i64 {
        return a
    }
    return 0
}
"#,
    );
}

// labelled over-conservative-but-sound, distinct from the unsafe rejects.
#[test]
fn returned_call_result_hits_floor() {
    let err = reject(
        r#"
fn id(r: &i64) -> &i64 {
    return r
}
fn via(r: &i64) -> &i64 {
    return id(r)
}
fn main() -> i64 {
    return 0
}
"#,
    );
    assert!(
        err.contains("[escape]"),
        "must carry the escape marker: {err}"
    );
    assert!(
        err.contains("cannot infer the origin"),
        "must be the origin-floor diagnostic: {err}"
    );
}

// returning the still-valid borrowed value (10).
#[test]
fn id_passthrough_compiles_and_runs() {
    let src = r#"
fn id(r: &i64) -> &i64 {
    return r
}
fn main() -> i64 {
    let a = 10
    let r = id(&a)
    return *r
}
"#;
    accepts(src);
    let Some(code) = run_exit(src) else {
        return;
    };
    assert_eq!(
        code, 10,
        "the returned reference borrows `a`, so *r is a = 10"
    );
}

#[test]
fn first_of_slice_ref_compiles() {
    accepts(
        r#"
fn first(v: &[i64]) -> &i64 {
    return &v[0]
}
fn main() -> i64 {
    return 0
}
"#,
    );
}

#[test]
fn nll_across_call_compiles() {
    accepts(
        r#"
fn id(r: &i64) -> &i64 {
    return r
}
fn read(r: &i64) -> i64 {
    return *r
}
fn main() -> i64 {
    let mut a = 10
    let r = id(&a)
    let t = read(r)
    a = 20
    return t
}
"#,
    );
}

// c1 (senior review): a reference to a reference lets a loan escape a scope via a deref-copy that
// whole-local provenance never sees; forming the nested ref is rejected so the whole route is dead.
// this uaf compiled and ran to exit 5 before the fix.
#[test]
fn nested_ref_deref_copy_uaf_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = 10
    let mut outer = &a
    {
        let x = 5
        let r = &x
        let pp = &r
        outer = *pp
    }
    return *outer
}
"#,
    );
    assert!(
        err.contains("references to references"),
        "expected the nested-ref escape reject, got: {err}"
    );
}

#[test]
fn nested_ref_construction_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let x = 5
    let r = &x
    let pp = &r
    return 0
}
"#,
    );
    assert!(
        err.contains("references to references"),
        "expected the nested-ref construction reject, got: {err}"
    );
}
