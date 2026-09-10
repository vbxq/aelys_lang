use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
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

#[test]
fn p1_rc_new_allocates_header_plus_data() {
    let ir = rc_ir(
        r#"
fn make() -> Rc<i64> {
    let a: Rc<i64> = Rc::new(42)
    return a
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        ir.contains("call ptr @__aelys_alloc(i64 24)"),
        "expected an Rc allocation of 16+8=24 bytes; got:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr inbounds i8, ptr") && ir.contains("i64 16"),
        "expected the data pointer to be exposed at base+16; got:\n{ir}"
    );
}

#[test]
fn p2_rc_new_writes_refcount_one() {
    let ir = rc_ir(
        r#"
fn make() -> Rc<i64> {
    let a: Rc<i64> = Rc::new(1)
    return a
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        ir.contains("store i32 1, ptr %rc_alloc_raw"),
        "expected refcount=1 stored at the header base; got:\n{ir}"
    );
    assert!(
        ir.contains("store i8 0, ptr") && ir.contains("store i32 1, ptr %rc_hdr_ptr1"),
        "expected flags=0 and the (real) 1-based type_id=1 stored @8; got:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_rc_type_table"),
        "the Stage 3b RC pointer-map table must be emitted; got:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_rc_type_table")
            && ir.contains("constant [5 x i32] [i32 2, i32 0, i32 5, i32 0, i32 5]"),
        "Rc<i64> must be type_id 1 with count 0; reserved sentinel at index 0 \
         (table blob [2,0,5,0,5]); got:\n{ir}"
    );
}

#[test]
fn p3_clone_emits_retain() {
    let ir = rc_ir(
        r#"
fn use_rc(a: Rc<i64>) -> i64 { return 0 }
fn make() -> i64 {
    let a: Rc<i64> = Rc::new(42)
    let b: Rc<i64> = a
    return use_rc(b)
}
fn main() -> i64 { return make() }
"#,
    );
    assert!(
        ir.contains("call void @__aelys_rc_retain(ptr"),
        "expected a retain on the clone `let b = a`; got:\n{ir}"
    );
}

#[test]
fn p4_scope_exit_emits_release() {
    let ir = rc_ir(
        r#"
fn use_rc(a: Rc<i64>) -> i64 { return 0 }
fn make() -> i64 {
    let a: Rc<i64> = Rc::new(42)
    return use_rc(a)
}
fn main() -> i64 { return make() }
"#,
    );
    assert!(
        ir.contains("call void @__aelys_rc_release(ptr"),
        "expected a release at scope exit for the non-returned Rc; got:\n{ir}"
    );
}

#[test]
fn p5_return_does_not_release_the_returned_handle() {
    let ir = rc_ir(
        r#"
fn make() -> Rc<i64> {
    let a: Rc<i64> = Rc::new(9)
    return a
}
fn main() -> i64 { return 0 }
"#,
    );
    let make_body = function_body(&ir, "@make");
    assert!(
        !make_body.contains("__aelys_rc_release"),
        "the returned Rc must not be released in its creating function; got:\n{make_body}"
    );
    assert!(
        make_body.contains("ret ptr"),
        "expected the Rc data pointer to be returned; got:\n{make_body}"
    );
}

#[test]
fn p6_pure_value_program_emits_no_rc_calls() {
    let ir = rc_ir(
        r#"
struct P { x: i64 }
fn make() -> i64 {
    let y: P = P { x: 5 }
    let z: P = y
    return z.x
}
fn main() -> i64 { return make() }
"#,
    );
    assert!(
        !ir.contains("__aelys_rc_retain") && !ir.contains("__aelys_rc_release"),
        "a pure value program must emit no rc_* calls (LD-1: nogc emits nothing); got:\n{ir}"
    );
}

#[test]
fn p7_arc_stubs_present_and_inert() {
    let archives = find_core_archives();
    assert!(
        !archives.is_empty(),
        "expected at least one libaelys-core.a to exist (build the workspace first)"
    );
    let mut found_retain = false;
    let mut found_release = false;
    for ar in &archives {
        if let Ok(out) = Command::new("nm").arg(ar).output() {
            let syms = String::from_utf8_lossy(&out.stdout);
            if syms.contains("__aelys_arc_retain") {
                found_retain = true;
            }
            if syms.contains("__aelys_arc_release") {
                found_release = true;
            }
        }
    }
    assert!(
        found_retain && found_release,
        "expected __aelys_arc_retain/__aelys_arc_release symbols in the runtime archive(s)"
    );
}

#[test]
fn p8_rc_program_compiles_links_and_runs() {
    let _pin = common::pin_legs("p8_rc_program_compiles_links_and_runs", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn touch(a: Rc<i64>) -> i64 { return 1 }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    let c: Rc<i64> = b
    let n: i64 = touch(c)
    println("rc ok")
    return n + 41
}
"#,
    )
    .expect("write source");

    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped value row carries no runtime evidence at all",
                );
                return;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        common::require_linker_skip("a skipped value row carries no runtime evidence at all");
        return;
    }
    common::note_leg();
    let output = Command::new(&exe).output().expect("run compiled exe");
    assert_eq!(
        output.status.code(),
        Some(42),
        "expected deterministic exit code 42, got {:?}; stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout == "rc ok\n" || stdout == "rc ok\r\n",
        "expected deterministic stdout 'rc ok', got {stdout:?}"
    );
}

#[test]
fn p9_no_release_on_closure_env_only_on_rc() {
    let ir = rc_ir(
        r#"
fn use_rc(a: Rc<i64>) -> i64 { return 0 }
fn make() -> i64 {
    let a: Rc<i64> = Rc::new(9)
    let cap: i64 = 5
    let f: fn() -> i64 = fn() -> i64 { return cap }
    let n: i64 = use_rc(a)
    return f()
}
fn main() -> i64 { return make() }
"#,
    );
    let body = function_body(&ir, "@make");
    assert!(
        body.contains("call ptr @__aelys_alloc(i64 8)"),
        "expected the closure env to be heap-allocated; got:\n{body}"
    );
    assert_eq!(
        call_count(&body, "@__aelys_rc_release"),
        1,
        "expected exactly one rc_release (on the Rc, not the closure env); got:\n{body}"
    );
}

#[test]
fn p10_return_releases_other_rc_not_the_returned() {
    let ir = rc_ir(
        r#"
fn make() -> Rc<i64> {
    let a: Rc<i64> = Rc::new(1)
    let b: Rc<i64> = Rc::new(2)
    return a
}
fn main() -> i64 { return 0 }
"#,
    );
    let body = function_body(&ir, "@make");
    assert_eq!(
        call_count(&body, "@__aelys_rc_release"),
        1,
        "expected exactly one release (on b, the non-returned Rc); got:\n{body}"
    );
    let rel = body.find("__aelys_rc_release").expect("a release exists");
    let ret = body.find("ret ptr").expect("a ptr return exists");
    assert!(
        rel < ret,
        "the release on b must precede the Return of a; got:\n{body}"
    );
}

#[test]
fn p11_passes_preserve_retain_release_with_param() {
    let ir = rc_ir(
        r#"
fn keep(p: Rc<i64>) -> i64 {
    let q: Rc<i64> = p
    return 0
}
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(3)
    return keep(a)
}
"#,
    );
    let body = function_body(&ir, "@keep");
    assert!(
        body.contains("__aelys_rc_retain"),
        "the clone's retain must survive copy_elim/dead_locals; got:\n{body}"
    );
    assert!(
        body.contains("__aelys_rc_release"),
        "the clone's release must survive copy_elim/dead_locals; got:\n{body}"
    );
    let retain_arg = arg_of(&body, "__aelys_rc_retain");
    let release_arg = arg_of(&body, "__aelys_rc_release");
    assert_eq!(
        retain_arg, release_arg,
        "retain and release must target the same handle after aliasing; got retain={retain_arg:?} release={release_arg:?}\n{body}"
    );
}

#[test]
fn p12_rc_get_reads_value_through_handle() {
    let _pin = common::pin_legs("p12_rc_get_reads_value_through_handle", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(42)
    return Rc::get(r)
}
"#,
    )
    .expect("write source");

    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped value row carries no runtime evidence at all",
                );
                return;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        common::require_linker_skip("a skipped value row carries no runtime evidence at all");
        return;
    }
    common::note_leg();
    let output = Command::new(&exe).output().expect("run compiled exe");
    assert_eq!(
        output.status.code(),
        Some(42),
        "expected the value read through the Rc handle (42), got {:?}; stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn p13_rc_get_emits_a_load_no_extra_refcount_op() {
    let ir = rc_ir(
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(42)
    return Rc::get(r)
}
"#,
    );
    let body = function_body(&ir, "@__aelys_main");
    assert!(
        body.contains("load i64,"),
        "Rc::get must lower to an i64 load from the data pointer; got:\n{body}"
    );
    assert_eq!(
        call_count(&body, "@__aelys_rc_retain"),
        0,
        "Rc::get must not emit any rc_retain; got:\n{body}"
    );
    assert_eq!(
        call_count(&body, "@__aelys_rc_release"),
        1,
        "only the scope-exit release of the handle is expected, none from get; got:\n{body}"
    );
}

#[test]
fn p14_rc_get_reads_through_borrowed_param() {
    let _pin = common::pin_legs("p14_rc_get_reads_through_borrowed_param", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn val(a: Rc<i64>) -> i64 { return Rc::get(a) }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(7)
    return val(r)
}
"#,
    )
    .expect("write source");

    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped value row carries no runtime evidence at all",
                );
                return;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        common::require_linker_skip("a skipped value row carries no runtime evidence at all");
        return;
    }
    common::note_leg();
    let output = Command::new(&exe).output().expect("run compiled exe");
    assert_eq!(
        output.status.code(),
        Some(7),
        "expected the value read through the borrowed Rc param (7), got {:?}; stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn n1_rc_mut_reassigned_is_rejected() {
    let err = rc_reject(
        r#"
fn main() -> i64 {
    let mut a: Rc<i64> = Rc::new(0)
    a = Rc::new(1)
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
fn n2_indirect_rc_init_is_rejected() {
    let err = rc_reject(
        r#"
fn main() -> i64 {
    let c: bool = true
    let a: Rc<i64> = if c { Rc::new(0) } else { Rc::new(1) }
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
fn n3_rc_field_in_struct_now_declares_stage3a() {
    let ir = rc_ir(
        r#"
struct S { r: Rc<i64> }
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        !ir.is_empty(),
        "Stage 3a: an Rc field in a nominal struct must now declare; got empty IR"
    );
}

#[test]
fn n3_rc_field_in_enum_now_declares_stage3a() {
    let ir = rc_ir(
        r#"
enum E { V(Rc<i64>) }
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        !ir.is_empty(),
        "Stage 3a: an Rc payload in a nominal enum must now declare; got empty IR"
    );
}

#[test]
fn n3bis_array_literal_of_rc_is_rejected() {
    let err = rc_reject(
        r#"
fn main() -> i64 {
    let v = [Rc::new(1), Rc::new(2)]
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N3-bis must carry the marker: {err}"
    );
}

#[test]
fn n4_rc_abandoned_by_break_is_rejected() {
    let err = rc_reject(
        r#"
fn main() -> i64 {
    let mut i: i64 = 0
    while i < 3 {
        let r: Rc<i64> = Rc::new(i)
        if i == 1 { break }
        i = i + 1
    }
    return 0
}
"#,
    );
    assert!(
        err.contains("[air-lowering]"),
        "N4 must carry the marker: {err}"
    );
}

#[test]
fn n5_closure_capturing_rc_is_rejected() {
    let err = rc_reject(
        r#"
fn deref_use(x: Rc<i64>) -> i64 { return 0 }
fn make() -> i64 {
    let a: Rc<i64> = Rc::new(42)
    let f: fn() -> i64 = fn() -> i64 { return deref_use(a) }
    return f()
}
fn main() -> i64 { return make() }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N5 must carry the marker: {err}"
    );
}

#[test]
fn n6_generic_struct_holding_rc_is_rejected() {
    let err = rc_reject(
        r#"
struct Box<T> { v: T }
fn make() -> Box<Rc<i64>> {
    let a: Rc<i64> = Rc::new(7)
    let b: Box<Rc<i64>> = Box { v: a }
    return b
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N6 (generic struct holding Rc) must carry the marker: {err}"
    );
}

#[test]
fn n6_nested_generic_struct_holding_rc_is_rejected() {
    let err = rc_reject(
        r#"
struct Box<T> { v: T }
fn make() -> Box<Box<Rc<i64>>> {
    return Box { v: Box { v: Rc::new(7) } }
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N6(nested) must carry the marker: {err}"
    );
}

#[test]
fn n7_generic_enum_holding_rc_in_let_is_rejected() {
    let err = rc_reject(
        r#"
enum Opt<T> { Some(T), None }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let x = Opt::Some(a)
    return 0
}
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N7(enum let) must carry the marker: {err}"
    );
}

#[test]
fn n7_generic_enum_holding_rc_in_return_is_rejected() {
    let err = rc_reject(
        r#"
enum Opt<T> { Some(T), None }
fn make() -> Opt<Rc<i64>> {
    let a: Rc<i64> = Rc::new(7)
    return Opt::Some(a)
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N7(enum return) must carry the marker: {err}"
    );
}

#[test]
fn n8_array_of_rc_returned_is_rejected() {
    let err = rc_reject(
        r#"
fn make() -> [Rc<i64>; 2] {
    let a: Rc<i64> = Rc::new(1)
    let b: Rc<i64> = Rc::new(2)
    return [a, b]
}
fn main() -> i64 { return 0 }
"#,
    );
    assert!(
        err.contains("[rc-stage1]"),
        "N8(array return) must carry the marker: {err}"
    );
}

fn function_body(ir: &str, name: &str) -> String {
    let mut out = String::new();
    let mut in_fn = false;
    for line in ir.lines() {
        if line.starts_with("define") && line.contains(name) {
            in_fn = true;
        }
        if in_fn {
            out.push_str(line);
            out.push('\n');
            if line == "}" {
                break;
            }
        }
    }
    out
}

fn arg_of(body: &str, callee: &str) -> Option<String> {
    for line in body.lines() {
        if line.contains(callee) {
            if let Some(open) = line.find('(') {
                let inside = &line[open + 1..];
                let inside = inside.trim_end_matches(')').trim_end_matches(", !");
                return Some(inside.trim().trim_end_matches(')').to_string());
            }
        }
    }
    None
}

fn find_core_archives() -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap_or(manifest);
    let target = root.join("target");
    walk_for(&target, "libaelys-core-leak.a", &mut found);
    walk_for(&target, "libaelys-core-rc.a", &mut found);
    found
}

fn walk_for(dir: &std::path::Path, file: &str, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_for(&path, file, out);
        } else if path.file_name().and_then(|s| s.to_str()) == Some(file) {
            out.push(path);
        }
    }
}
