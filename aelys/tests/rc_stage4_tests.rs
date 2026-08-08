use aelys_driver::{RuntimeVariant, compile_file_with_llvm, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found") || error.contains("failed to run")
}

fn run_with_stats(src: &str, variant: RuntimeVariant) -> Option<(i32, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, OptimizationLevel::None, false, variant) {
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

    let output = Command::new(&exe)
        .env("AELYS_RC_STATS", "1")
        .output()
        .expect("run compiled exe");
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Some((code, stderr))
}

fn rc_reject(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect_err("compilation should be rejected")
        .to_string()
}

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

#[test]
fn p1_copy_then_push_leaves_original_unchanged() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0] + a[1] + a[2] + b[0] + b[1] + b[2] + b[3]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(
        code, 21,
        "a must stay [1,2,3] (6) and b be [1,2,3,9] (15) => 21; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        (allocs, frees),
        (2, 2),
        "shared push must copy the buffer (2 allocs) and free both (2 frees); stderr:\n{stderr}"
    );
}

#[test]
fn p1_sharp_shared_push_with_realloc_keeps_original() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 10)
    Vec::push(b, 20)
    Vec::push(b, 30)
    return a[0] + a[1] + a[2]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(
        code, 6,
        "a must stay [1,2,3] across b's realloc-grows; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "balanced (no double-free / no leak); stderr:\n{stderr}"
    );
}

#[test]
fn p2_unshared_push_does_not_allocate() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    Vec::push(a, 9)
    return a[0] + a[1] + a[2] + a[3]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(code, 15, "in-place push: [1,2,3,9]; stderr:\n{stderr}");
    let (allocs, _) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, 1,
        "fast path: only the initial buffer is allocated, push reuses it; stderr:\n{stderr}"
    );
}

#[test]
fn p2_shared_push_allocates_one_more_than_unshared() {
    let Some((_, unshared)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    Vec::push(a, 9)
    return a[0]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    let Some((_, shared)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    let (ua, _) = parse_stats(&unshared).expect("unshared stats");
    let (sa, _) = parse_stats(&shared).expect("shared stats");
    assert_eq!(
        sa,
        ua + 1,
        "the shared push must allocate exactly one extra buffer (the CoW copy); \
         unshared allocs={ua}, shared allocs={sa}"
    );
}

#[test]
fn p6_asan_shared_push_is_clean() {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0] + a[1] + a[2] + b[3]
}
"#,
    )
    .expect("write source");

    match compile_file_with_llvm_variant(
        &source_path,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
    ) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping ASan probe");
                return;
            }
            panic!("compilation should succeed: {err}");
        }
    }

    let object = source_path.with_extension(if cfg!(windows) { "obj" } else { "o" });
    if !object.is_file() {
        eprintln!("object not produced; skipping ASan probe");
        return;
    }

    // uninstrumented archive only exposes the malloc/free interceptors, not the immix poison net
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let core_src = manifest
        .parent()
        .unwrap_or(manifest)
        .join("core")
        .join("src");
    let asan_lib_dir = dir.path();
    let core_units = [
        "aelys_core_common.c",
        "aelys_alloc_immix.c",
        "aelys_rc_real.c",
    ];
    let mut objects = Vec::new();
    for unit in core_units {
        let src = core_src.join(unit);
        if !src.is_file() {
            eprintln!("core source {unit} missing; skipping ASan probe");
            return;
        }
        let obj = asan_lib_dir.join(unit).with_extension("o");
        let cc = Command::new("clang")
            .arg("-fsanitize=address")
            .arg("-g")
            .arg("-c")
            .arg(&src)
            .arg(format!("-I{}", core_src.display()))
            .arg("-o")
            .arg(&obj)
            .output();
        match cc {
            Ok(out) if out.status.success() => objects.push(obj),
            Ok(out) => panic!(
                "instrumented core compile of {unit} failed:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(_) => {
                eprintln!("clang unavailable; skipping ASan probe");
                return;
            }
        }
    }
    let asan_archive = asan_lib_dir.join("libaelys-core-rc-asan.a");
    let ar = Command::new("ar")
        .arg("rcs")
        .arg(&asan_archive)
        .args(&objects)
        .output();
    match ar {
        Ok(out) if out.status.success() => {}
        Ok(out) => panic!("ar failed:\n{}", String::from_utf8_lossy(&out.stderr)),
        Err(_) => {
            eprintln!("ar unavailable; skipping ASan probe");
            return;
        }
    }

    let asan_exe = dir.path().join("module_asan");
    let link = Command::new("clang")
        .arg("-fsanitize=address")
        .arg("-g")
        .arg(&object)
        .arg(format!("-L{}", asan_lib_dir.display()))
        .arg("-laelys-core-rc-asan")
        .arg("-o")
        .arg(&asan_exe)
        .output();
    let link = match link {
        Ok(out) => out,
        Err(_) => {
            eprintln!("clang unavailable; skipping ASan probe");
            return;
        }
    };
    assert!(
        link.status.success(),
        "ASan link failed:\nstdout:\n{}\nstderr:\n{}",
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
        Some(15),
        "ASan run must exit 15 (no sanitizer abort); stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "ASan must report no errors (no double-free/UAF/leak); stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "every Vec buffer (original + CoW copy) is freed exactly once; stderr:\n{stderr}"
    );
}

#[test]
fn n1_vec_of_struct_bearing_rc_is_rejected() {
    let err = rc_reject(
        r#"
struct S { r: Rc<i64> }
fn main() -> i64 {
    let v = vec[S { r: Rc::new(1) }]
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N1 must carry the marker: {err}"
    );
}

#[test]
fn n2_vec_of_nested_struct_bearing_rc_is_rejected() {
    let err = rc_reject(
        r#"
struct Inner { r: Rc<i64> }
struct Outer { i: Inner }
fn main() -> i64 {
    let v = vec[Outer { i: Inner { r: Rc::new(1) } }]
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N2 must carry the marker: {err}"
    );
}

#[test]
fn n3_array_of_struct_bearing_rc_is_rejected() {
    let err = rc_reject(
        r#"
struct S { r: Rc<i64> }
fn main() -> i64 {
    let a = [S { r: Rc::new(1) }]
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N3 (array) must carry the marker: {err}"
    );
}

#[test]
fn n4_inline_return_vec_of_struct_rc_is_rejected() {
    let err = rc_reject(
        r#"
struct S { r: Rc<i64> }
fn make() -> i64 {
    return Rc::get(vec[S { r: Rc::new(1) }][0].r)
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N4 must carry the marker: {err}"
    );
}

#[test]
fn n5_push_of_struct_bearing_rc_is_rejected() {
    let err = rc_reject(
        r#"
struct S { r: Rc<i64> }
fn main() -> i64 {
    let v = Vec::new()
    Vec::push(v, S { r: Rc::new(1) })
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N5 must carry the marker: {err}"
    );
}

#[test]
fn n_positive_plain_struct_vec_is_accepted() {
    let Some((code, stderr)) = run_with_stats(
        r#"
struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let v = vec[Point { x: 3, y: 4 }]
    return v[0].x + v[0].y
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(
        code, 7,
        "a plain-struct Vec must compile and run; stderr:\n{stderr}"
    );
}

#[test]
fn p8_leak_variant_keeps_value_semantics_frees_nothing() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0] + a[1] + a[2] + b[3]
}
"#,
        RuntimeVariant::Leak,
    ) else {
        return;
    };
    assert_eq!(
        code, 15,
        "leak: a stays [1,2,3] (6) + b[3]=9 => 15; stderr:\n{stderr}"
    );
    let (_, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        frees, 0,
        "the leak variant must never free; stderr:\n{stderr}"
    );
}

#[test]
fn vec_new_then_grow_reads_back() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    Vec::push(v, 4)
    Vec::push(v, 8)
    return v[0] + v[1] + v[2] + v[3]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(
        code, 15,
        "Vec::new + 4 pushes grow & read back: 1+2+4+8=15; stderr:\n{stderr}"
    );
}

#[test]
fn f1_vec_param_mutation_does_not_corrupt_caller() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn f(v: Vec<i64>) {
    Vec::push(v, 99)
}
fn main() -> i64 {
    let a = vec[1, 2, 3]
    Vec::push(a, 4)
    f(a)
    Vec::push(a, 5)
    return a[4]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(
        code, 5,
        "a must be unchanged by the callee's push (a[4]==5); a 231 means the param \
         aliased & mutated a's buffer in place; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        (allocs, frees),
        (2, 2),
        "the callee's push CoW-copies (original buffer + the private copy = 2 allocs), \
         both freed (no leak, no double-free); stderr:\n{stderr}"
    );
}

#[test]
fn f1_read_only_vec_param_is_sound() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn g(v: Vec<i64>) -> i64 {
    return v[0]
}
fn main() -> i64 {
    let a = vec[1, 2, 3]
    return g(a)
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(code, 1, "g reads v[0]==1; stderr:\n{stderr}");
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        (allocs, frees),
        (1, 1),
        "one buffer (a's), retained on entry and released once balanced, no leak; \
         stderr:\n{stderr}"
    );
}

#[test]
fn f1_returned_vec_param_escapes_balanced() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn id(v: Vec<i64>) -> Vec<i64> {
    return v
}
fn main() -> i64 {
    let a = vec[10, 20, 30]
    let b = id(a)
    Vec::push(b, 99)
    return a[0] + a[1] + a[2] + b[3]
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(
        code, 159,
        "a stays [10,20,30] (60); b=[10,20,30,99], b[3]=99 => 159; an over-release of \
         the escaping param would UAF/double-free; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "balanced (escape excluded from release); stderr:\n{stderr}"
    );
}

#[test]
fn f2_struct_with_vec_field_is_rejected() {
    let err = rc_reject(
        r#"
struct H { v: Vec<i64> }
fn main() -> i64 {
    let h = H { v: vec[1, 2, 3] }
    return h.v[0]
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "a struct with a Vec field must be rejected with the stable marker: {err}"
    );
}

#[test]
fn f2_nested_struct_with_vec_field_is_rejected() {
    let err = rc_reject(
        r#"
struct Inner { v: Vec<i64> }
struct Outer { i: Inner }
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "a struct transitively holding a Vec must be rejected: {err}"
    );
}

#[test]
fn f2_forward_referenced_vec_carrier_is_rejected() {
    let err = rc_reject(
        r#"
struct Outer { i: Inner }
struct Inner { v: Vec<i64> }
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "a forward-referenced Vec carrier must still be rejected: {err}"
    );
}

#[test]
fn f2_enum_with_vec_payload_is_rejected() {
    let err = rc_reject(
        r#"
enum E { Has(Vec<i64>), None }
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "an enum variant with a Vec payload must be rejected: {err}"
    );
}

#[test]
fn f2_positive_struct_without_vec_is_accepted() {
    let Some((code, stderr)) = run_with_stats(
        r#"
struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let p = Point { x: 3, y: 4 }
    return p.x + p.y
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(
        code, 7,
        "a plain struct (no Vec field) must compile and run; stderr:\n{stderr}"
    );
}
