use aelys_driver::{RuntimeVariant, compile_file_with_llvm, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

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

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

fn compile(src: &str, opt: OptimizationLevel) -> Option<PathBuf> {
    let dir = tempdir().expect("tempdir");
    // leak the dir so the exe survives past this function
    let dir = Box::leak(Box::new(dir));
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    match compile_file_with_llvm_variant(&source_path, opt, false, RuntimeVariant::Rc) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                return None;
            }
            panic!("compile should succeed:\n{src}\nerror: {err}");
        }
    }
    let exe = exe_path_for(&source_path);
    if exe.is_file() { Some(exe) } else { None }
}

fn run(exe: &Path, alloc: Option<&str>) -> (i32, String) {
    let mut cmd = Command::new(exe);
    cmd.env("AELYS_RC_STATS", "1");
    if let Some(a) = alloc {
        cmd.env("AELYS_ALLOC", a);
    }
    let out = cmd.output().expect("run exe");
    (
        out.status.code().expect("exit code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn assert_exit(label: &str, src: &str, expected: i32) {
    for (name, opt) in LEVELS {
        let Some(exe) = compile(src, *opt) else {
            eprintln!("{label}: linker unavailable, skipping");
            return;
        };
        for alloc in [None, Some("malloc")] {
            let (code, stderr) = run(&exe, alloc);
            let a = alloc.unwrap_or("immix");
            assert_eq!(
                code, expected,
                "{label} at {name} under {a}: expected exit {expected}, got {code}\nstderr:\n{stderr}"
            );
        }
    }
}

// exit oracle plus aelys_rc_stats allocs==frees under both allocators (no leak, no double free).
fn assert_exit_balanced(label: &str, src: &str, expected: i32) {
    for (name, opt) in LEVELS {
        let Some(exe) = compile(src, *opt) else {
            eprintln!("{label}: linker unavailable, skipping");
            return;
        };
        for alloc in [None, Some("malloc")] {
            let (code, stderr) = run(&exe, alloc);
            let a = alloc.unwrap_or("immix");
            assert_eq!(
                code, expected,
                "{label} at {name}/{a}: exit; stderr:\n{stderr}"
            );
            let (allocs, frees) = parse_stats(&stderr).expect("stats line");
            assert_eq!(
                allocs, frees,
                "{label} at {name}/{a}: every buffer freed exactly once (allocs={allocs} frees={frees})"
            );
        }
    }
}

fn reject_e0412(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect_err("must be rejected")
        .to_string()
}

#[test]
fn v01_let_copy_then_index_assign_original() {
    // let w = v; v[0] = 9; the copy must not see the write
    assert_exit_balanced(
        "v01",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    v[0] = 9
    return w[0]
}
"#,
        1,
    );
}

#[test]
fn v02_compound_assign_is_a_write() {
    assert_exit_balanced(
        "v02",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    v[0] += 8
    return w[0]
}
"#,
        1,
    );
}

#[test]
fn v05_param_boundary_has_value_semantics() {
    assert_exit_balanced(
        "v05",
        r#"
fn poke(mut x: Vec<i64>) -> i64 {
    x[0] = 9
    return x[0]
}
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let _ = poke(v)
    return v[0]
}
"#,
        1,
    );
}

#[test]
fn v07_write_in_loop_after_copy() {
    assert_exit_balanced(
        "v07",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    let mut i = 0
    while i < 3 {
        v[i] = 9
        i = i + 1
    }
    return w[0] + w[1] + w[2]
}
"#,
        6,
    );
}

#[test]
fn v10_plain_assign_takes_a_share() {
    assert_exit_balanced(
        "v10",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let mut w = vec[7, 7, 7]
    w = v
    v[0] = 9
    return w[0]
}
"#,
        1,
    );
}

#[test]
fn v17_both_directions_of_the_aliasing_relation() {
    assert_exit_balanced(
        "v17",
        r#"
fn main() -> i64 {
    let mut a = vec[1, 2, 3]
    let b = a
    a[0] = 9
    let mut c = vec[4, 5, 6]
    let d = c
    c[1] = 8
    return a[0] + b[0] + c[1] + d[1]
}
"#,
        23,
    );
}

#[test]
fn v18_two_aliases_mutated_independently() {
    assert_exit_balanced(
        "v18",
        r#"
fn main() -> i64 {
    let mut a = vec[5, 5, 5]
    let mut b = a
    let mut c = a
    b[0] = 1
    c[0] = 2
    return a[0]
}
"#,
        5,
    );
}

#[test]
fn v20_copy_taken_inside_the_loop_body() {
    assert_exit_balanced(
        "v20",
        r#"
fn main() -> i64 {
    let mut total = 0
    let mut i = 0
    while i < 3 {
        let mut v = vec[1, 2, 3]
        let w = v
        v[0] = 9
        total = total + w[0]
        i = i + 1
    }
    return total
}
"#,
        3,
    );
}

#[test]
fn v30_deref_store_of_a_vec_releases_the_old_buffer() {
    // *r = u overwrites the pointee slot: retain-new, release-old-through-ptr (n-3, s5)
    assert_exit_balanced(
        "v30",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let u = vec[9, 9, 9]
    let r = &mut v
    *r = u
    return v[0]
}
"#,
        9,
    );
}

// ----- regression guards: shapes that already worked and must not move ---

#[test]
fn v09_shared_push_still_produces_21() {
    assert_exit_balanced(
        "v09",
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0] + a[1] + a[2] + b[0] + b[1] + b[2] + b[3]
}
"#,
        21,
    );
}

#[test]
fn v11_self_assign_does_not_free() {
    // v = v with retain-first ordering: rc 1 -> 2 -> 1, no free, no uaf
    assert_exit_balanced(
        "v11",
        r#"
fn main() -> i64 {
    let mut v = vec[7, 2, 3]
    v = v
    v[0] = v[0]
    return v[0]
}
"#,
        7,
    );
}

#[test]
fn v15_unshared_write_never_allocates() {
    let src = r#"
fn main() -> i64 {
    let mut v = vec[0, 0, 0, 0]
    let mut i = 0
    while i < 1000000 {
        v[i % 4] = i
        i = i + 1
    }
    return v[0]
}
"#;
    let Some(exe) = compile(src, OptimizationLevel::None) else {
        eprintln!("v15: linker unavailable, skipping");
        return;
    };
    let (code, stderr) = run(&exe, None);
    assert_eq!(code, 60, "v15 exit; stderr:\n{stderr}");
    let (allocs, _) = parse_stats(&stderr).expect("stats");
    assert_eq!(
        allocs, 1,
        "fast path: no copy on an unshared write; stderr:\n{stderr}"
    );
}

#[test]
fn v16_plain_array_is_inline_storage() {
    assert_exit(
        "v16",
        r#"
fn main() -> i64 {
    let a = [1, 2, 3]
    let mut b = a
    b[0] = 9
    return a[0]
}
"#,
        1,
    );
}

#[test]
fn v32_call_transfers_its_share() {
    assert_exit_balanced(
        "v32",
        r#"
fn id(x: Vec<i64>) -> Vec<i64> { return x }
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let w = id(v)
    return w[0] + v[0]
}
"#,
        2,
    );
}

#[test]
fn v12_escaping_closure_reads_a_live_buffer() {
    // stale-but-correct read cannot pass by luck. the env leaks by design, so a leak is expected.
    let src = r#"
fn make() -> fn() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    Vec::push(v, 3)
    let f = fn() -> i64 { return v[0] }
    return f
}
fn main() -> i64 {
    let g = make()
    let junk = vec[77, 77, 77]
    return g()
}
"#;
    assert_exit("v12", src, 1);
    // the captured buffer leaks with the env
    let Some(exe) = compile(src, OptimizationLevel::None) else {
        return;
    };
    let (_, stderr) = run(&exe, None);
    let (allocs, frees) = parse_stats(&stderr).expect("stats");
    assert!(
        frees < allocs,
        "the captured buffer leaks with the deliberately-leaked env (allocs={allocs} frees={frees})"
    );
}

#[test]
fn v06_write_from_a_closure_over_a_shared_vec() {
    // the env slot owns a share, so the closure's v[0]=9 detaches and the outer copy w keeps [1,2,3]
    assert_exit(
        "v06",
        r#"
fn make() -> fn() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    let f = fn() -> i64 {
        v[0] = 9
        return w[0]
    }
    return f
}
fn main() -> i64 {
    let g = make()
    return g()
}
"#,
        1,
    );
}

#[test]
fn v29_closure_capture_leak_is_per_creation_not_a_wrap() {
    // m-1: a closure created in a loop leaks one env + one buffer per creation. the count must not
    // wrap; the value stays correct (acc = 100, 100 % 7 = 2) and the leak is named, not "balanced".
    let src = r#"
fn main() -> i64 {
    let mut i = 0
    let mut acc = 0
    while i < 100 {
        let v = vec[1, 2, 3]
        let f = fn() -> i64 { return v[0] }
        acc = acc + f()
        i = i + 1
    }
    return acc % 7
}
"#;
    assert_exit("v29", src, 2);
    let Some(exe) = compile(src, OptimizationLevel::None) else {
        return;
    };
    let (_, stderr) = run(&exe, None);
    let (allocs, frees) = parse_stats(&stderr).expect("stats");
    assert!(
        allocs >= 200 && frees == 0,
        "100 envs + 100 buffers leak per creation, none freed (allocs={allocs} frees={frees})"
    );
}

#[test]
fn v28_shadowed_capture_does_not_corrupt_the_env() {
    assert_exit(
        "v28",
        r#"
fn make() -> fn() -> i64 {
    let x = 5
    let f = fn() -> i64 {
        let r = x
        let mut x = 100
        x = 200
        return r
    }
    return f
}
fn main() -> i64 {
    let g = make()
    let a = g()
    let b = g()
    return a + b
}
"#,
        10,
    );
}

#[test]
fn v34_nested_closures_do_not_collide_on_localid() {
    // localid restarts at 0 per function, so an outer capture_slots entry must not false-
    assert_exit(
        "v34",
        r#"
fn make() -> fn() -> i64 {
    let outer = 3
    let f = fn() -> i64 {
        let inner = 4
        let h = fn() -> i64 { return inner }
        return outer + h()
    }
    return f
}
fn main() -> i64 {
    let g = make()
    return g()
}
"#,
        7,
    );
}

#[test]
fn v34_vec_nested_closure_with_forced_slab_reuse() {
    assert_exit(
        "v34_vec",
        r#"
fn make() -> fn() -> i64 {
    let v = vec[10, 20, 30]
    let f = fn() -> i64 {
        let k = 5
        let h = fn() -> i64 { return k }
        return v[0] + h()
    }
    return f
}
fn main() -> i64 {
    let g = make()
    let junk = vec[77, 77, 77]
    return g()
}
"#,
        15,
    );
}

// ----- the surface boundary: forms outside vpf reject, they never miscompile ---

#[test]
fn v22_parenthesised_producer_rejects_e0412() {
    // not a compile-to-exit-1: an indirect producer aliases a slot the acquire cannot see.
    let err = reject_e0412(
        r#"
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let mut w = (v)
    w[0] = 9
    return v[0]
}
"#,
    );
    assert!(
        err.contains("E0412"),
        "paren producer must reject E0412: {err}"
    );
}

#[test]
fn v35_return_of_an_indirect_producer_rejects_e0412() {
    // `return if c { a } else { a }` at vec return type is s6/e0412; keeping the escape filter
    let err = reject_e0412(
        r#"
fn pick(a: Vec<i64>, c: i64) -> Vec<i64> {
    return if c == 1 { a } else { a }
}
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let w = pick(v, 1)
    return w[0]
}
"#,
    );
    assert!(
        err.contains("E0412"),
        "return of an `if` must reject E0412: {err}"
    );
}
