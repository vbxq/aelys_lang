use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const RESOURCE: &str = "struct Resource { id: i64 }\n";

fn reject(body: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");
    match lower_file_to_air(&source_path, OptimizationLevel::None) {
        Ok(_) => panic!("expected the borrow checker to reject this program, but it compiled"),
        Err(err) => err,
    }
}

// assert the program lowers to air (the borrow checker accepts it), independent of a linker.
fn accepts(body: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .unwrap_or_else(|err| panic!("the borrow checker must accept this program: {err}"));
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

// shared borrow live, then the referent is written: mut-xor-shared write conflict.
#[test]
fn shared_then_mutate_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let mut x = 3
    let r = &x
    x = 5
    return *r
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("cannot write"), "must be a write-while-borrowed diagnostic: {err}");
}

// discriminator: the same shape with a value copy instead of a borrow compiles.
#[test]
fn shared_then_mutate_copy_twin_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let mut x = 3
    let r = x
    x = 5
    return r
}
"#,
    );
}

// two overlapping &mut of the same local, both live: exclusive-borrow conflict.
#[test]
fn two_overlapping_mut_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    let r2 = &mut x
    *r1 = 5
    *r2 = 6
    return x
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("as mutable"), "must be a mutable-reborrow conflict: {err}");
}

#[test]
fn two_sequential_mut_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    *r1 = 5
    let r2 = &mut x
    *r2 = 6
    return x
}
"#,
    );
}

// an affine local is moved while a shared borrow of it is still live: move conflict. the move
#[test]
fn move_while_borrowed_is_rejected() {
    let err = reject(
        r#"
fn touch(r: &Resource) {
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = &a
    let b = a
    touch(r)
    return 0
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("cannot move"), "must be a move-while-borrowed diagnostic: {err}");
}

// discriminator: with the borrow dead before the move (nll), the move is accepted.
#[test]
fn move_after_borrow_dead_compiles() {
    accepts(
        r#"
fn touch(r: &Resource) {
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = &a
    touch(r)
    let b = a
    return 0
}
"#,
    );
}

// the row-6 marquee: vec::push mutates the receiver, so pushing while an element borrow is live
#[test]
fn push_while_element_borrowed_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let r = &v[0]
    Vec::push(v, 40)
    return *r
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("cannot write"), "push must be modeled as a write of v: {err}");
}

// a reference stored into an array literal is rejected (charter-15 v1 boundary).
#[test]
fn ref_in_array_literal_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let a = &v[0]
    let arr = [a]
    return 0
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("aggregate containers"), "must be the container-boundary diagnostic: {err}");
}

#[test]
fn value_array_literal_compiles() {
    accepts(
        r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let a = v[0]
    let arr = [a]
    return 0
}
"#,
    );
}

// a reference stored into an enum payload is rejected (same container boundary).
#[test]
fn ref_in_enum_payload_is_rejected() {
    let err = reject(
        r#"
enum Box { Of(&i64), Empty }
fn main() -> i64 {
    let mut x = 5
    let o = Box::Of(&x)
    return 0
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("aggregate containers"), "must be the container-boundary diagnostic: {err}");
}

#[test]
fn value_enum_payload_compiles() {
    accepts(
        r#"
enum Box { Of(i64), Empty }
fn main() -> i64 {
    let mut x = 5
    let o = Box::Of(x)
    return 0
}
"#,
    );
}

// a reborrow &mut *r suspends r: using the base r while the reborrow is live conflicts.
#[test]
fn reborrow_then_use_base_is_rejected() {
    let err = reject(
        r#"
fn f(r: &mut i64) {
    let r2 = &mut *r
    *r = 6
    *r2 = 5
}
fn main() -> i64 {
    let mut x = 0
    f(&mut x)
    return x
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
}

// nll: the element borrow is dead before the push, so the push is accepted. it also runs and
// returns the still-valid borrowed element (10), proving the loan really died before the push.
#[test]
fn nll_borrow_dead_before_push_compiles_and_runs() {
    let src = r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let r = &v[0]
    let y = *r
    Vec::push(v, 40)
    return y
}
"#;
    accepts(src);
    let Some(code) = run_exit(src) else {
        return;
    };
    assert_eq!(code, 10, "the borrow dies before the push, so it runs and returns v[0] = 10");
}

// with a value oracle rather than a compile check: borrowing one field must not block writing another.
#[test]
fn disjoint_field_places_do_not_conflict() {
    let Some(code) = run_exit(
        r#"
struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let mut p = Point{x: 1, y: 2}
    let a = &p.x
    p.y = 20
    return *a + p.y
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 21, "borrowing p.x must not block writing p.y, and *a must still read 1");
}

// the discriminating twin: the same place, so the write must be rejected. without this a
// place-insensitive checker and a checker that never fires would both pass the test above.
#[test]
fn same_field_place_write_while_borrowed_is_rejected() {
    let err = reject(
        r#"
struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let mut p = Point{x: 1, y: 2}
    let a = &p.x
    p.x = 20
    return *a
}
"#,
    );
    assert!(err.contains("[E0711]"), "writing the borrowed field itself must reject: {err}");
}

// a reborrow &mut *r that never touches the base r again is accepted (no self-conflict).
#[test]
fn reborrow_mut_compiles() {
    accepts(
        r#"
fn f(r: &mut i64) {
    let r2 = &mut *r
    *r2 = 5
}
fn main() -> i64 {
    let mut x = 0
    f(&mut x)
    return x
}
"#,
    );
}

