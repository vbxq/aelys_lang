use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use tempfile::tempdir;

fn lower(src: &str) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn reject(src: &str) -> String {
    match lower(src) {
        Ok(()) => panic!("expected the nogc check to reject this program, but it compiled"),
        Err(err) => err,
    }
}

fn accepts(src: &str) {
    lower(src).unwrap_or_else(|err| panic!("the nogc check must accept this program: {err}"));
}

fn assert_e0727(err: &str) {
    assert!(err.contains("[E0727]"), "must carry the E0727 code: {err}");
    assert!(err.contains("[nogc]"), "must carry the nogc marker: {err}");
    assert!(
        err.contains("declared nogc but its inferred effects reach managed memory"),
        "must be the nogc-reaches-managed diagnostic: {err}"
    );
}

#[test]
fn reject_direct_managed_intrinsic() {
    let err = reject(
        "\
nogc fn f() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
}

#[test]
fn twin_direct_managed_intrinsic_compiles() {
    accepts(
        "\
fn f() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}
fn main() -> i64 { return f() }
",
    );
}

#[test]
fn reject_transitive_through_general_callee() {
    let err = reject(
        "\
fn g() -> Rc<i64> { return Rc::new(0) }
nogc fn f() -> i64 {
    g()
    return 0
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
}

#[test]
fn twin_transitive_through_general_callee_compiles() {
    accepts(
        "\
fn g() -> Rc<i64> { return Rc::new(0) }
fn f() -> i64 {
    g()
    return 0
}
fn main() -> i64 { return f() }
",
    );
}

// the summary to top (managed included), so the declared-nogc `f` is rejected even though the
#[test]
fn reject_indirect_allocating_closure() {
    let err = reject(
        "\
nogc fn f() -> i64 {
    let c = fn() -> i64 {
        let v = vec[1, 2, 3]
        return v[0]
    }
    return c()
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
}

#[test]
fn twin_indirect_allocating_closure_compiles() {
    accepts(
        "\
fn f() -> i64 {
    let c = fn() -> i64 {
        let v = vec[1, 2, 3]
        return v[0]
    }
    return c()
}
fn main() -> i64 { return f() }
",
    );
}

// bounds-checked slice indexing + arithmetic carries only panic; managed is not in the set, so a
#[test]
fn accept_nogc_slice_sum_panic_only() {
    accepts(
        "\
nogc fn sum3(s: &[i64]) -> i64 { return s[0] + s[1] + s[2] }
fn main() -> i64 { return 0 }
",
    );
}

#[test]
fn accept_nogc_calls_pure_nogc() {
    accepts(
        "\
nogc fn helper(x: i64) -> i64 { return x + 1 }
nogc fn f() -> i64 { return helper(41) }
fn main() -> i64 { return f() }
",
    );
}
