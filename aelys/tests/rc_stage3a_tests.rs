use aelys_driver::{RuntimeVariant, compile_file_with_llvm, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

mod common;
use common::{exe_path_for, linker_unavailable};

fn rc_ir(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect("llvm backend compilation should succeed");
    fs::read_to_string(source_path.with_extension("ll")).expect("ir file")
}

fn rc_reject(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect_err("compilation should be rejected")
        .to_string()
}

fn call_count(ir: &str, needle: &str) -> usize {
    ir.lines()
        .filter(|l| l.contains(needle) && !l.trim_start().starts_with("declare"))
        .count()
}

fn run_with_stats(src: &str) -> Option<(i32, String)> {
    let _pin = common::pin_legs("run_with_stats", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(
        &source_path,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
    ) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped value row carries no runtime evidence at all",
                );
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        common::require_linker_skip("a skipped value row carries no runtime evidence at all");
        return None;
    }

    common::note_leg();
    let output = Command::new(&exe)
        .env("AELYS_RC_STATS", "1")
        .output()
        .expect("run compiled exe");
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Some((code, stderr))
}

fn find_core_archive(file: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, file: &str) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, file) {
                    return Some(found);
                }
            } else if path.file_name().and_then(|s| s.to_str()) == Some(file) {
                return Some(path);
            }
        }
        None
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap_or(manifest);
    walk(&root.join("target"), file)
}

#[test]
fn t1_carrier_copy_frees_once() {
    let Some((code, stderr)) = run_with_stats(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let n: Node = Node { next: a }
    let m: Node = n
    return Rc::get(m.next)
}
"#,
    ) else {
        return;
    };
    assert_eq!(
        code, 7,
        "value read through the copied carrier; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "a copied carrier must free its Rc exactly once; got stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("frees=2"),
        "a carrier's Rc must NOT be freed per copy; got stderr:\n{stderr}"
    );
}

#[test]
fn t2_drop_releases_the_field() {
    let ir = rc_ir(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    {
        let a: Rc<i64> = Rc::new(7)
        let n: Node = Node { next: a }
    }
    return 0
}
"#,
    );
    assert!(
        call_count(&ir, "@__aelys_rc_release") >= 1,
        "dropping a carrier must release its Rc field at scope exit; got:\n{ir}"
    );

    let Some((code, stderr)) = run_with_stats(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    {
        let a: Rc<i64> = Rc::new(7)
        let n: Node = Node { next: a }
    }
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "the dropped carrier must free its Rc once; got stderr:\n{stderr}"
    );
}

#[test]
fn t3_construction_from_live_emits_one_retain() {
    let ir = rc_ir(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let n: Node = Node { next: a }
    return Rc::get(n.next)
}
"#,
    );
    assert_eq!(
        call_count(&ir, "@__aelys_rc_retain"),
        1,
        "constructing a carrier from a live Rc must retain its field exactly once; got:\n{ir}"
    );
}

#[test]
fn t3bis_construction_from_fresh_rc_new_has_no_retain() {
    let ir = rc_ir(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let n: Node = Node { next: Rc::new(7) }
    return Rc::get(n.next)
}
"#,
    );
    assert_eq!(
        call_count(&ir, "@__aelys_rc_retain"),
        0,
        "a fresh `Rc::new` field needs no construction retain (refcount=1); got:\n{ir}"
    );
    let Some((code, stderr)) = run_with_stats(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let n: Node = Node { next: Rc::new(7) }
    return Rc::get(n.next)
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 7, "stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "fresh-Rc carrier: one alloc, one free; got stderr:\n{stderr}"
    );
}

#[test]
fn t4_pure_value_carrier_emits_no_rc_calls() {
    let ir = rc_ir(
        r#"
struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let p: Point = Point { x: 3, y: 4 }
    let q: Point = p
    return q.x
}
"#,
    );
    assert_eq!(
        call_count(&ir, "@__aelys_rc_retain"),
        0,
        "a pure value type must emit no retain (LD-1 zero runtime); got:\n{ir}"
    );
    assert_eq!(
        call_count(&ir, "@__aelys_rc_release"),
        0,
        "a pure value type must emit no release (LD-1 zero runtime); got:\n{ir}"
    );
}

#[test]
fn t5_returned_carrier_field_is_not_released() {
    let Some((code, stderr)) = run_with_stats(
        r#"
struct Node { next: Rc<i64> }
fn make() -> Node {
    let r: Rc<i64> = Rc::new(7)
    return Node { next: r }
}
fn main() -> i64 {
    let n: Node = make()
    return Rc::get(n.next)
}
"#,
    ) else {
        return;
    };
    assert_eq!(
        code, 7,
        "the escaped carrier's field must be live in the caller; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "escape must not double-release nor leak; got stderr:\n{stderr}"
    );
}

#[test]
fn t6_borrowed_carrier_arg_has_no_callee_release() {
    let Some((code, stderr)) = run_with_stats(
        r#"
struct Node { next: Rc<i64> }
fn touch(n: Node) -> i64 { return 0 }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(7)
    let n: Node = Node { next: r }
    let unused: i64 = touch(n)
    return Rc::get(n.next)
}
"#,
    ) else {
        return;
    };
    assert_eq!(
        code, 7,
        "borrow must not free the caller's Rc early; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "a borrowed carrier arg must not be released callee-side; got stderr:\n{stderr}"
    );
}

#[test]
fn t7_nested_transitive_carrier_frees_once() {
    let ir = rc_ir(
        r#"
struct B { r: Rc<i64> }
struct A { b: B }
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(9)
    let inner: B = B { r: x }
    let a: A = A { b: inner }
    let a2: A = a
    return Rc::get(a2.b.r)
}
"#,
    );
    assert!(
        ir.contains("field") || ir.contains("getelementptr"),
        "transitive carrier must GEP through `.b` then `.r`; got:\n{ir}"
    );
    assert!(
        call_count(&ir, "@__aelys_rc_retain") >= 2,
        "a nested transitive carrier copied must retain its leaf at each new owner; got:\n{ir}"
    );

    let Some((code, stderr)) = run_with_stats(
        r#"
struct B { r: Rc<i64> }
struct A { b: B }
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(9)
    let inner: B = B { r: x }
    let a: A = A { b: inner }
    let a2: A = a
    return Rc::get(a2.b.r)
}
"#,
    ) else {
        return;
    };
    assert_eq!(
        code, 9,
        "value read through the nested transitive path; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "a nested transitive carrier must free its leaf exactly once; got stderr:\n{stderr}"
    );
}

#[test]
fn t8_enum_carrier_frees_once() {
    let Some((code, stderr)) = run_with_stats(
        r#"
enum Holder { Cell(Rc<i64>) }
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(5)
    let e: Holder = Holder::Cell(x)
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "an enum carrier must release its Rc payload exactly once; got stderr:\n{stderr}"
    );
}

#[test]
fn trej_a_generic_struct_of_rc_in_field_is_rejected() {
    let err = rc_reject(
        r#"
struct Wrap<T> { v: T }
struct S { w: Wrap<Rc<i64>> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(1)
    let s: S = S { w: Wrap { v: a } }
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "T-Rej(a): a generic struct of Rc in a field must be rejected; got:\n{err}"
    );
}

#[test]
fn trej_b_array_of_rc_in_field_is_rejected() {
    let err = rc_reject(
        r#"
struct S { arr: [Rc<i64>; 2] }
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "T-Rej(b): an array of Rc in a field must be rejected; got:\n{err}"
    );
}

#[test]
fn trej_b_vec_of_rc_in_field_is_rejected() {
    let err = rc_reject(
        r#"
struct S { v: vec<Rc<i64>> }
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "T-Rej(b): a vec of Rc in a field must be rejected; got:\n{err}"
    );
}

#[test]
fn trej_c_field_assign_rc_field_is_rejected() {
    let err = rc_reject(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(1)
    let b: Rc<i64> = Rc::new(2)
    let mut n: Node = Node { next: a }
    n.next = b
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "T-Rej(c): in-place reassign of an Rc field must be rejected; got:\n{err}"
    );
}

#[test]
fn trej_d_field_assign_carrier_field_is_rejected() {
    let err = rc_reject(
        r#"
struct B { r: Rc<i64> }
struct A { b: B }
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(1)
    let y: Rc<i64> = Rc::new(2)
    let mut a: A = A { b: B { r: x } }
    a.b = B { r: y }
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "T-Rej(d): in-place reassign of a carrier field must be rejected; got:\n{err}"
    );
}

#[test]
fn trej_e_carrier_abandoned_by_break_is_rejected() {
    let err = rc_reject(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let mut i: i64 = 0
    while i < 3 {
        let n: Node = Node { next: Rc::new(i) }
        if i == 1 { break }
        i = i + 1
    }
    return 0
}
"#,
    );
    assert!(
        err.contains("[air-lowering]"),
        "T-Rej(e): a carrier abandoned by break must be rejected; got:\n{err}"
    );
}

#[test]
fn trej_f_carrier_captured_by_closure_is_rejected() {
    let err = rc_reject(
        r#"
struct Node { next: Rc<i64> }
fn read_node(n: Node) -> i64 { return 0 }
fn make() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let n: Node = Node { next: a }
    let f: fn() -> i64 = fn() -> i64 { return read_node(n) }
    return f()
}
fn main() -> i64 { return make() }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "T-Rej(f): a carrier captured by a closure must be rejected; got:\n{err}"
    );
}

fn asan_probe(src: &str, expected_exit: i32, label: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(
        &source_path,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
    ) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped asan probe proves nothing about memory cleanliness",
                );
                return;
            }
            panic!("[{label}] compilation should succeed: {err}");
        }
    }

    let object = source_path.with_extension(if cfg!(windows) { "obj" } else { "o" });
    if !object.is_file() {
        eprintln!("[{label}] object not produced; skipping ASan probe");
        return;
    }

    let Some(archive) = find_core_archive("libaelys-core-rc.a") else {
        eprintln!("[{label}] rc archive not found; skipping ASan probe");
        return;
    };
    let lib_dir = archive.parent().expect("archive has a parent");

    let asan_exe = dir.path().join("module_asan");
    let link = Command::new("clang")
        .arg("-fsanitize=address,leak")
        .arg("-g")
        .arg(&object)
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-laelys-core-rc")
        .arg("-o")
        .arg(&asan_exe)
        .output();
    let link = match link {
        Ok(out) => out,
        Err(_) => {
            eprintln!("[{label}] clang unavailable; skipping ASan probe");
            return;
        }
    };
    assert!(
        link.status.success(),
        "[{label}] ASan link failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&link.stdout),
        String::from_utf8_lossy(&link.stderr)
    );

    let run = Command::new(&asan_exe)
        .env("AELYS_RC_STATS", "1")
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run ASan-instrumented exe");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(
        run.status.code(),
        Some(expected_exit),
        "[{label}] ASan run must exit {expected_exit} (no sanitizer abort); stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "[{label}] ASan must report no error (no double-free/UAF/leak); stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "[{label}] under ASan the carrier's Rc must free exactly once; stderr:\n{stderr}"
    );
}

#[test]
fn t_asan_carrier_copy_is_clean() {
    asan_probe(
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let n: Node = Node { next: a }
    let m: Node = n
    return Rc::get(m.next)
}
"#,
        7,
        "T1",
    );
}

#[test]
fn t_asan_nested_transitive_is_clean() {
    asan_probe(
        r#"
struct B { r: Rc<i64> }
struct A { b: B }
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(9)
    let inner: B = B { r: x }
    let a: A = A { b: inner }
    let a2: A = a
    return Rc::get(a2.b.r)
}
"#,
        9,
        "T7",
    );
}

#[test]
fn f1_rej_multivariant_rc_enum_is_rejected() {
    let err = rc_reject(
        r#"
enum E { A(Rc<i64>), B(i64) }
fn main() -> i64 {
    let e: E = E::B(99)
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "F1-rej: a multi-variant Rc-bearing enum must be rejected (no SEGV); got:\n{err}"
    );
}

#[test]
fn f1_rej_multivariant_rc_enum_transitive_in_struct_is_rejected() {
    let err = rc_reject(
        r#"
enum E { A(Rc<i64>), B(i64) }
struct S { e: E }
fn main() -> i64 {
    let s: S = S { e: E::B(99) }
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "F1-rej(transitive): a struct carrier of a multi-variant Rc enum must be rejected; got:\n{err}"
    );
}

#[test]
fn f1_ok_mono_variant_rc_enum_still_compiles_and_frees_once() {
    let Some((code, stderr)) = run_with_stats(
        r#"
enum Holder { Cell(Rc<i64>) }
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(5)
    let e: Holder = Holder::Cell(x)
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "F1-ok: a mono-variant Rc enum must still free its payload exactly once; got stderr:\n{stderr}"
    );
}

#[test]
fn f1_ok_mono_variant_rc_enum_is_asan_clean() {
    asan_probe(
        r#"
enum Holder { Cell(Rc<i64>) }
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(5)
    let e: Holder = Holder::Cell(x)
    return 0
}
"#,
        0,
        "F1-ok/T8",
    );
}

#[test]
fn f1_no_overreject_multivariant_enum_without_rc_compiles() {
    let ir = rc_ir(
        r#"
enum Color { Red, Green }
fn main() -> i64 {
    let c: Color = Color::Red
    return 0
}
"#,
    );
    assert_eq!(
        call_count(&ir, "@__aelys_rc_retain"),
        0,
        "F1-no-overreject: a pure value enum must emit no rc_* (no over-reject); got:\n{ir}"
    );
    assert_eq!(
        call_count(&ir, "@__aelys_rc_release"),
        0,
        "F1-no-overreject: a pure value enum must emit no rc_* (no over-reject); got:\n{ir}"
    );
}

#[test]
fn f1_no_overreject_multivariant_enum_with_payload_but_no_rc_compiles() {
    let ir = rc_ir(
        r#"
enum Opt { Some(i64), None }
fn main() -> i64 {
    let o: Opt = Opt::Some(7)
    return 0
}
"#,
    );
    assert_eq!(
        call_count(&ir, "@__aelys_rc_retain") + call_count(&ir, "@__aelys_rc_release"),
        0,
        "F1-no-overreject: a multi-variant non-Rc enum must emit no rc_*; got:\n{ir}"
    );
}
