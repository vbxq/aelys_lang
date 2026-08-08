
use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
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

#[test]
fn use_after_move_renders_moved_here_caret() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    println(a.id)
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(err.contains("after it was moved"), "must be a use-after-move diagnostic: {err}");
    assert!(err.contains("[E0701]"), "must carry the E0701 code: {err}");
    assert!(err.contains("module.aelys:6:5"), "primary caret must anchor at the use: {err}");
    assert!(err.contains("moved here"), "must show the move-site secondary caret: {err}");
}

#[test]
fn double_move_renders_first_moved_here_caret() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    let c = a
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(err.contains("already moved"), "must be a double-move diagnostic: {err}");
    assert!(err.contains("[E0702]"), "must carry the E0702 code: {err}");
    assert!(err.contains("module.aelys:6:5"), "primary caret must anchor at the second move: {err}");
    assert!(err.contains("first moved here"), "must show the first-move secondary caret: {err}");
}

#[test]
fn use_of_maybe_moved_renders_branch_caret() {
    let err = reject(
        r#"
fn main() -> i64 {
    let cond = true
    let a = Resource{id: 1}
    if cond {
        let b = a
    }
    println(a.id)
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(
        err.contains("may have been moved") || err.contains("earlier branch"),
        "must be a use-of-maybe-moved diagnostic: {err}"
    );
    assert!(err.contains("[E0703]"), "must carry the E0703 code: {err}");
    assert!(err.contains("module.aelys:9:5"), "primary caret must anchor at the use: {err}");
    assert!(
        err.contains("moved here on one branch"),
        "must show the conditional-move secondary caret: {err}"
    );
}

#[test]
fn maybe_move_at_scope_end_renders_declared_here_caret() {
    let err = reject(
        r#"
fn f(c: bool) {
    let a = Resource{id: 1}
    if c {
        let b = a
    }
}
fn main() -> i64 {
    f(true)
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(
        err.contains("destroyed at scope end"),
        "must be the scope-exit maybe-moved diagnostic: {err}"
    );
    assert!(err.contains("[E0704]"), "must carry the E0704 code: {err}");
    assert!(err.contains("`a` declared here"), "must show the owner-decl secondary caret: {err}");
}

#[test]
fn maybe_move_at_reassignment_renders_declared_here_caret() {
    let err = reject(
        r#"
fn f(c: bool) {
    let mut a = Resource{id: 1}
    if c {
        let b = a
    }
    a = Resource{id: 2}
}
fn main() -> i64 {
    f(true)
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(
        err.contains("on reassignment"),
        "must be the reassignment maybe-moved diagnostic: {err}"
    );
    assert!(err.contains("[E0704]"), "must carry the E0704 code: {err}");
    assert!(err.contains("module.aelys:8:5"), "primary caret must anchor at the reassignment: {err}");
    assert!(err.contains("`a` declared here"), "must show the owner-decl secondary caret: {err}");
}

// ============================ borrow rejections (e071x) ============================

#[test]
fn write_while_borrowed_renders_three_point() {
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
    assert!(err.contains("[E0711]"), "must carry the E0711 code: {err}");
    assert!(err.contains("module.aelys:6:5"), "primary caret must anchor at the write: {err}");
    assert!(err.contains("borrow created here"), "must show the loan-creation caret: {err}");
    assert!(err.contains("borrow last used here"), "must show the loan-last-use caret: {err}");
}

// the row-6 marquee, and the three-point proof: creation, conflict, last-use on distinct lines.
#[test]
fn push_while_element_borrowed_renders_three_point() {
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
    assert!(err.contains("[E0711]"), "must carry the E0711 code: {err}");
    assert!(err.contains("module.aelys:6:5"), "primary caret must anchor at the push: {err}");
    assert!(err.contains("borrow created here"), "must show the loan-creation caret: {err}");
    assert!(err.contains("borrow last used here"), "must show the loan-last-use caret: {err}");
}

#[test]
fn move_while_borrowed_renders_three_point() {
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
    assert!(err.contains("[E0712]"), "must carry the E0712 code: {err}");
    assert!(err.contains("module.aelys:8:5"), "primary caret must anchor at the move: {err}");
    assert!(err.contains("borrow created here"), "must show the loan-creation caret: {err}");
    assert!(err.contains("borrow last used here"), "must show the loan-last-use caret: {err}");
}

#[test]
fn two_overlapping_mut_renders_as_mutable_code() {
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
    assert!(err.contains("[E0713]"), "must carry the E0713 code: {err}");
    assert!(err.contains("module.aelys:6:14"), "primary caret must anchor at the second borrow: {err}");
    assert!(err.contains("borrow created here"), "must show the loan-creation caret: {err}");
}

#[test]
fn shared_while_mut_renders_as_shared_code() {
    let err = reject(
        r#"
fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    let r2 = &x
    *r1 = 5
    return *r2
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("as shared"), "must be a shared-while-mut conflict: {err}");
    assert!(err.contains("[E0713]"), "must carry the E0713 code: {err}");
    assert!(err.contains("borrow created here"), "must show the loan-creation caret: {err}");
    assert!(err.contains("borrow last used here"), "must show the loan-last-use caret: {err}");
}

#[test]
fn read_while_mut_renders_cannot_use_code() {
    let err = reject(
        r#"
fn main() -> i64 {
    let mut x = 3
    let r = &mut x
    let y = x
    *r = 5
    return y
}
"#,
    );
    assert!(err.contains("[borrow]"), "must carry the borrow marker: {err}");
    assert!(err.contains("cannot use"), "must be a read-while-mut conflict: {err}");
    assert!(err.contains("[E0713]"), "must carry the E0713 code: {err}");
    assert!(err.contains("module.aelys:6:5"), "primary caret must anchor at the read: {err}");
    assert!(err.contains("borrow created here"), "must show the loan-creation caret: {err}");
}

#[test]
fn ref_in_array_literal_renders_this_reference_caret() {
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
    assert!(err.contains("[E0714]"), "must carry the E0714 code: {err}");
    assert!(err.contains("module.aelys:6:15"), "primary caret must anchor at the aggregate: {err}");
    assert!(err.contains("this reference"), "must show the offending-operand secondary caret: {err}");
}

// ============================ escape rejections (e072x) ============================

#[test]
fn return_ref_to_local_renders_destroyed_on_return_caret() {
    let err = reject(
        r#"
fn get() -> &i64 {
    let x = 5
    return &x
}
fn main() -> i64 {
    return 0
}
"#,
    );
    assert!(err.contains("[escape]"), "must carry the escape marker: {err}");
    assert!(err.contains("returns a reference to local"), "must be the D1 return-ref diagnostic: {err}");
    assert!(err.contains("[E0721]"), "must carry the E0721 code: {err}");
    assert!(err.contains("module.aelys:5:5"), "primary caret must anchor at the return: {err}");
    assert!(
        err.contains("destroyed on return"),
        "must show the owner-decl death secondary caret: {err}"
    );
}

#[test]
fn scope_death_renders_owner_decl_and_scope_end_carets() {
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
    assert!(err.contains("[escape]"), "must carry the escape marker: {err}");
    assert!(err.contains("does not live long enough"), "must be the D2 scope-death diagnostic: {err}");
    assert!(err.contains("[E0722]"), "must carry the E0722 code: {err}");
    assert!(err.contains("`x` declared here"), "must show the owner-decl secondary caret: {err}");
    assert!(err.contains("scope of `x` ends here"), "must show the death-point secondary caret: {err}");
}

#[test]
fn origin_floor_renders_escape_caret() {
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
    assert!(err.contains("[escape]"), "must carry the escape marker: {err}");
    assert!(err.contains("cannot infer the origin"), "must be the origin-floor diagnostic: {err}");
    assert!(err.contains("[E0723]"), "must carry the E0723 code: {err}");
    assert!(err.contains("module.aelys:7:5"), "primary caret must anchor at the return: {err}");
}

#[test]
fn projected_ref_store_renders_escape_caret() {
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
    assert!(err.contains("[escape]"), "must carry the escape marker: {err}");
    assert!(err.contains("aggregate containers"), "must be the container-boundary diagnostic: {err}");
    assert!(err.contains("[E0724]"), "must carry the E0724 code: {err}");
    assert!(err.contains("module.aelys:6:14"), "primary caret must anchor at the stored reference: {err}");
}

#[test]
fn closure_capturing_reference_renders_escape_caret() {
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
    assert!(err.contains("[escape]"), "must carry the escape marker: {err}");
    assert!(err.contains("closure"), "must be the closure-capture diagnostic: {err}");
    assert!(err.contains("[E0725]"), "must carry the E0725 code: {err}");
    assert!(err.contains("module.aelys:6:13"), "primary caret must anchor at the closure: {err}");
}

#[test]
fn nested_ref_renders_escape_caret() {
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
    assert!(err.contains("[escape]"), "must carry the escape marker: {err}");
    assert!(err.contains("references to references"), "must be the nested-ref diagnostic: {err}");
    assert!(err.contains("[E0726]"), "must carry the E0726 code: {err}");
    assert!(err.contains("module.aelys:6:14"), "primary caret must anchor at the nested borrow: {err}");
}

#[test]
fn all_e07xx_codes_are_registered() {
    let codes = [
        "E0701", "E0702", "E0703", "E0704", "E0711", "E0712", "E0713", "E0714", "E0721", "E0722",
        "E0723", "E0724", "E0725", "E0726", "E0727", "E0728", "E0729", "E0730", "E0731",
    ];
    for code in codes {
        let info = aelys_common::registry::lookup(code)
            .unwrap_or_else(|| panic!("E07xx code {code} must be registered for --explain"));
        assert!(!info.title.is_empty(), "{code} must have a title");
        assert!(!info.explanation.is_empty(), "{code} must have an explanation");
    }
}

