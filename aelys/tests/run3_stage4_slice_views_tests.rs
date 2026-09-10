// ! detach and `e0426` fenced the whole family.

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

static WARM: Once = Once::new();

fn warm_core_archive() {
    WARM.call_once(|| {
        let Ok(dir) = tempdir() else { return };
        let path = dir.path().join("warmup.aelys");
        if fs::write(&path, "fn main() -> i64 { return 0 }\n").is_err() {
            return;
        }
        let _ = compile_file_with_llvm_variant(
            &path,
            OptimizationLevel::None,
            false,
            RuntimeVariant::Rc,
        );
    });
}

struct Outcome {
    exit: i32,
    stdout: String,
    stderr: String,
    stats: Option<(i64, i64)>,
}

struct Harness {
    dir: TempDir,
    legs: Cell<usize>,
    linker_skips: Cell<usize>,
}

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

fn exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

fn linker_skip_declared() -> bool {
    std::env::var("AELYS_ALLOW_LINKER_SKIP").is_ok()
}

fn slug(id: &str, tag: &str) -> String {
    let mut s = String::with_capacity(id.len() + tag.len() + 1);
    for c in id.chars().chain(std::iter::once('_')).chain(tag.chars()) {
        s.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    s
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
            legs: Cell::new(0),
            linker_skips: Cell::new(0),
        }
    }

    fn write(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let path = self.dir.path().join(format!("{}.aelys", slug(id, tag)));
        fs::write(&path, src).expect("write fixture");
        path
    }

    fn reject(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> String {
        let path = self.write(id, tag, src);
        match lower_file_to_air(&path, opt) {
            Ok(_) => panic!("{id} at {tag} MUST be rejected, but it was accepted:\n{src}"),
            Err(rendered) => rendered,
        }
    }

    fn fenced_row(&self, id: &str, src: &str, code: &str) -> String {
        let mut last = String::new();
        for (tag, opt) in LEVELS {
            let rendered = self.reject(id, tag, src, *opt);
            self.legs.set(self.legs.get() + 1);
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {tag} MUST be refused by {code} specifically\nsource:\n{src}\ngot:\n{rendered}"
            );
            last = rendered;
        }
        last
    }

    fn accepts(&self, id: &str, src: &str) {
        for (tag, opt) in LEVELS {
            let path = self.write(id, tag, src);
            self.legs.set(self.legs.get() + 1);
            if let Err(rendered) = lower_file_to_air(&path, *opt) {
                panic!("{id} at {tag} MUST be accepted:\nsource:\n{src}\ngot:\n{rendered}");
            }
        }
    }

    fn compile(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> Option<PathBuf> {
        let path = self.write(id, tag, src);
        match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
            Ok(()) => {}
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    self.linker_skips.set(self.linker_skips.get() + 1);
                    return None;
                }
                panic!("{id} at {tag} must compile:\n{src}\nerror: {err}");
            }
        }
        let exe = exe_path_for(&path);
        exe.is_file().then_some(exe)
    }

    fn run(&self, exe: &Path, alloc: Option<&str>) -> Outcome {
        let mut cmd = Command::new(exe);
        cmd.env("AELYS_RC_STATS", "1");
        if let Some(a) = alloc {
            cmd.env("AELYS_ALLOC", a);
        }
        let out = cmd.output().expect("run compiled exe");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Outcome {
            exit: exit_code(&out.status),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stats: parse_stats(&stderr),
            stderr,
        }
    }

    fn value_row(&self, id: &str, src: &str, stdout: &str, allocs: i64, frees: i64) {
        for (tag, opt) in LEVELS {
            let Some(exe) = self.compile(id, tag, src, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped value \
                     row carries no runtime evidence at all"
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let o = self.run(&exe, *alloc);
                self.legs.set(self.legs.get() + 1);
                assert_eq!(
                    o.exit, 0,
                    "{id} at {tag}/{alloc_name} must exit 0\nsource:\n{src}\nstderr:\n{}",
                    o.stderr
                );
                assert_eq!(
                    o.stdout, stdout,
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}\nsource:\n{src}"
                );
                let (a, f) = o.stats.unwrap_or_else(|| {
                    panic!(
                        "{id} at {tag}/{alloc_name}: no [rc] stats line\nstderr:\n{}",
                        o.stderr
                    )
                });
                assert_eq!(
                    (a, f),
                    (allocs, frees),
                    "{id} at {tag}/{alloc_name}: MUST be allocs={allocs} frees={frees}, got \
                     allocs={a} frees={f}\nsource:\n{src}"
                );
            }
        }
    }

    fn assert_legs(&self, expected: usize) {
        if self.linker_skips.get() > 0 && linker_skip_declared() {
            return;
        }
        assert_eq!(
            self.legs.get(),
            expected,
            "this test must execute exactly {expected} legs; a leg that silently stopped running \
             is the confident zero this run keeps hitting"
        );
    }
}

// 🔴 the booking rule. s4-h1 is evidence that the capability exists. it is not evidence that it

const S4_H1: &str = "struct V3 { x: i64, y: i64, z: i64 }\n\
                     nogc fn cn(src: &[V3], dst: &mut [V3]) -> i64 {\n\
                     \x20   dst[0].x = src[0].y\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut sv: Vec<V3> = vec[V3 { x: 7919, y: 101, z: 3 }]\n\
                     \x20   let mut dv: Vec<V3> = vec[V3 { x: 0, y: 0, z: 0 }]\n\
                     \x20   let q = cn(sv[0..1], dv[0..1])\n\
                     \x20   println(dv[0].x)\n\
                     \x20   println(sv[0].x)\n\
                     \x20   return 0\n\
                     }\n";

// the quoted witness. the same headline with a third owner aliasing `dst`, so the formation detach
const S4_H1X: &str = "struct V3 { x: i64, y: i64, z: i64 }\n\
                      nogc fn cn(src: &[V3], dst: &mut [V3]) -> i64 {\n\
                      \x20   dst[0].x = src[0].y\n\
                      \x20   return 0\n\
                      }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut sv: Vec<V3> = vec[V3 { x: 7919, y: 101, z: 3 }]\n\
                      \x20   let mut dv: Vec<V3> = vec[V3 { x: 0, y: 0, z: 0 }]\n\
                      \x20   let dw: Vec<V3> = dv\n\
                      \x20   let q = cn(sv[0..1], dv[0..1])\n\
                      \x20   println(dv[0].x)\n\
                      \x20   println(dw[0].x)\n\
                      \x20   println(sv[0].x)\n\
                      \x20   return 0\n\
                      }\n";

const S4_H1N: &str = "struct V3 { x: i64, y: i64, z: i64 }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut sv: Vec<V3> = vec[V3 { x: 7919, y: 101, z: 3 }]\n\
                      \x20   let mut dv: Vec<V3> = vec[V3 { x: 0, y: 0, z: 0 }]\n\
                      \x20   dv[0].x = sv[0].y\n\
                      \x20   println(dv[0].x)\n\
                      \x20   println(sv[0].x)\n\
                      \x20   return 0\n\
                      }\n";

const S4_H2: &str = "struct V3 { x: i64, y: i64, z: i64 }\n\
                     nogc fn cn(src: &[V3], dst: &mut [V3]) -> i64 {\n\
                     \x20   dst[0].x = src[0].y\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut sv: Vec<V3> = vec[V3 { x: 7919, y: 101, z: 3 }]\n\
                     \x20   let mut dv: [V3; 1] = [V3 { x: 0, y: 0, z: 0 }]\n\
                     \x20   let q = cn(sv[0..1], dv[0..1])\n\
                     \x20   println(dv[0].x)\n\
                     \x20   println(sv[0].x)\n\
                     \x20   return 0\n\
                     }\n";

const S4_H3: &str = "struct V3 { x: i64, y: i64, z: i64 }\n\
                     nogc fn cn(src: &[V3], dst: &mut [V3]) -> i64 {\n\
                     \x20   dst[0].x = src[0].y\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut sv: [V3; 1] = [V3 { x: 7919, y: 101, z: 3 }]\n\
                     \x20   let mut dv: [V3; 1] = [V3 { x: 0, y: 0, z: 0 }]\n\
                     \x20   let q = cn(sv[0..1], dv[0..1])\n\
                     \x20   println(dv[0].x)\n\
                     \x20   println(sv[0].x)\n\
                     \x20   return 0\n\
                     }\n";

// the destination `vec` is aliased before the view is formed, and there is exactly one `vec` in
const S4_H4: &str = "nogc fn cn(dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = 101\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut dv: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let dw: Vec<i64> = dv\n\
                     \x20   let q = cn(dv[0..3])\n\
                     \x20   println(dv[0])\n\
                     \x20   println(dw[0])\n\
                     \x20   return 0\n\
                     }\n";

const S4_H5: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   let s = v[0..2]\n\
                     \x20   s[0] = 99\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S4_H6: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                     \x20   let s = v[0..2]\n\
                     \x20   s[0] = 99\n\
                     \x20   println(v[0])\n\
                     \x20   return 0\n\
                     }\n";

const S4_H7: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   let s = v[0..3]\n\
                     \x20   let f = fn(p: &mut [i64]) -> i64 {\n\
                     \x20       p[0] = 101\n\
                     \x20       return 0\n\
                     \x20   }\n\
                     \x20   let q = f(s)\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S4_H8: &str = "nogc fn rd(s: &[i64]) -> i64 { return s[0] }\n\
                     nogc fn fwd(s: &[i64]) -> i64 { return rd(s) }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   println(fwd(v[0..3]))\n\
                     \x20   return 0\n\
                     }\n";

const S4_H9: &str = "fn f(r: &mut Vec<i64>) -> i64 {\n\
                     \x20   let s = (*r)[0..3]\n\
                     \x20   s[0] = 101\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   let q = f(&mut v)\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

// five mutable views over one aliased `vec`. the cost is one copy per shared buffer, not one per
const S4_H10: &str = "fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[100, 2, 3]\n\
                      \x20   let w: Vec<i64> = v\n\
                      \x20   for i in 0..5 {\n\
                      \x20       let s = v[0..3]\n\
                      \x20       s[0] = s[0] + 1\n\
                      \x20   }\n\
                      \x20   println(v[0])\n\
                      \x20   println(w[0])\n\
                      \x20   return 0\n\
                      }\n";

const S4_H11: &str = "nogc fn peek(s: &[i64]) -> i64 { return s[0] }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                      \x20   let w: Vec<i64> = v\n\
                      \x20   println(peek(v[0..3]))\n\
                      \x20   println(w[0])\n\
                      \x20   return 0\n\
                      }\n";

const S4_H12: &str = "fn view(r: &mut Vec<i64>) -> &mut [i64] { return (*r)[0..3] }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                      \x20   let w: Vec<i64> = v\n\
                      \x20   let s = view(&mut v)\n\
                      \x20   s[0] = 101\n\
                      \x20   println(v[0])\n\
                      \x20   println(w[0])\n\
                      \x20   return 0\n\
                      }\n";

#[test]
fn s4_h1_the_headline_lands_with_both_slices_from_managed_vecs() {
    let h = Harness::new();
    h.value_row("S4-H1", S4_H1, "101\n7919\n", 2, 2);
    // the spelling that is safe to quote: three owners, so the detach is load-bearing here
    h.value_row("S4-H1x", S4_H1X, "101\n0\n7919\n", 3, 3);
    h.assert_legs(16);
}

#[test]
fn s4_h1_carries_no_allocation_the_no_region_twin_does_not() {
    let h = Harness::new();
    h.value_row("S4-H1", S4_H1, "101\n7919\n", 2, 2);
    h.value_row("S4-H1n", S4_H1N, "101\n7919\n", 2, 2);
    h.assert_legs(16);
}

#[test]
fn s4_h2_h3_the_array_bases_still_cost_what_they_did() {
    let h = Harness::new();
    h.value_row("S4-H2", S4_H2, "101\n7919\n", 1, 1);
    h.value_row("S4-H3", S4_H3, "101\n7919\n", 0, 0);
    h.assert_legs(16);
}

#[test]
fn s4_h4_h5_the_aliased_bases_keep_their_value_semantics() {
    let h = Harness::new();
    h.value_row("S4-H4", S4_H4, "101\n7919\n", 2, 2);
    h.value_row("S4-H5", S4_H5, "99\n1\n", 2, 2);
    h.value_row("S4-H6", S4_H6, "99\n", 1, 1);
    h.assert_legs(24);
}

#[test]
fn s4_h7_a_mutable_view_reaches_a_lambda_parameter() {
    let h = Harness::new();
    h.value_row("S4-H7", S4_H7, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s4_h8_forwarding_a_read_only_view_to_a_reader_runs() {
    let h = Harness::new();
    h.value_row("S4-H8", S4_H8, "7919\n", 1, 1);
    h.assert_legs(8);
}

#[test]
fn s4_h9_h12_the_managed_side_forms_and_returns_mutable_views() {
    let h = Harness::new();
    h.value_row("S4-H9", S4_H9, "101\n7919\n", 2, 2);
    h.value_row("S4-H12", S4_H12, "101\n7919\n", 2, 2);
    h.assert_legs(16);
}

#[test]
fn s4_h10_five_views_over_one_aliased_buffer_cost_one_copy() {
    let h = Harness::new();
    h.value_row("S4-H10", S4_H10, "105\n100\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s4_h11_a_read_only_view_of_a_mut_bound_aliased_vec_copies_nothing() {
    let h = Harness::new();
    h.value_row("S4-H11", S4_H11, "7919\n7919\n", 1, 1);
    h.assert_legs(8);
}

// capability is claimed, these four are co-booked by name, each annotated with the single

#[test]
fn the_booking_rule_is_honoured() {
    // d-1, the formation detach: alias formed before the view, so deleting the detach changes the
    let d1 = ["S4-H1x", "S4-H4", "S4-H5"];
    // d-2, the mutable loan: alias formed after the view, which no d-1 row can see
    let d2 = ["S4-N6", "S4-N14"];
    assert_eq!(
        (d1.len(), d2.len()),
        (3, 2),
        "the two halves of A2(2) cover disjoint program shapes and each half is invisible to the \
         other's witness, so neither list may empty"
    );
    // the headline discriminates neither, by construction: one owner each and never aliased
    let h = Harness::new();
    h.value_row("S4-H1", S4_H1, "101\n7919\n", 2, 2);
    h.value_row("S4-H1x", S4_H1X, "101\n0\n7919\n", 3, 3);
    h.assert_legs(16);
    assert!(
        !d1.contains(&"S4-H1") && !d2.contains(&"S4-H1"),
        "S4-H1 is evidence the capability exists, not that it is safe; a capability claim citing \
         only S4-H1 is a claim that cannot fail. quote S4-H1x instead: same program, one more \
         owner, and it fails when the detach is deleted"
    );
}

// to `&[t]`. without it a pure read of a `mut`-bound aliased `vec` pays a buffer copy, so every

const R2_PRE: &str = "nogc fn rd(s: &[i64]) -> i64 { return s[0] }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                      \x20   let w: Vec<i64> = v\n";
const R2_POST: &str = "    println(w[0])\n\
                       \x20   return 0\n\
                       }\n";

fn r2_row(middle: &str) -> String {
    format!("{R2_PRE}{middle}{R2_POST}")
}

const R2_RETURN: &str = "nogc fn rd(s: &[i64]) -> i64 { return s[0] }\n\
                         fn pick(v: &Vec<i64>) -> &[i64] { return (*v)[0..3] }\n\
                         fn main() -> i64 {\n\
                         \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                         \x20   let w: Vec<i64> = v\n\
                         \x20   println(rd(pick(&v)))\n\
                         \x20   println(w[0])\n\
                         \x20   return 0\n\
                         }\n";

const R2_MATCH: &str = "enum W { A, B }\n\
                        fn main() -> i64 {\n\
                        \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                        \x20   let w: Vec<i64> = v\n\
                        \x20   let e = W::A\n\
                        \x20   let s: &[i64] = match e {\n\
                        \x20       W::A => v[0..3],\n\
                        \x20       W::B => v[0..3],\n\
                        \x20   }\n\
                        \x20   println(s[0])\n\
                        \x20   println(w[0])\n\
                        \x20   return 0\n\
                        }\n";

const R2_NESTED: &str = "fn main() -> i64 {\n\
                         \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                         \x20   let w: Vec<i64> = v\n\
                         \x20   let c: bool = true\n\
                         \x20   let s: &[i64] = if c { if c { v[0..3] } else { v[0..3] } } else { v[0..3] }\n\
                         \x20   println(s[0])\n\
                         \x20   println(w[0])\n\
                         \x20   return 0\n\
                         }\n";

const R2_MIXED: &str = "enum W { A, B }\n\
                        fn main() -> i64 {\n\
                        \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                        \x20   let w: Vec<i64> = v\n\
                        \x20   let e = W::A\n\
                        \x20   let s: &[i64] = {\n\
                        \x20       match e {\n\
                        \x20           W::A => (v[0..3]),\n\
                        \x20           W::B => { v[0..3] },\n\
                        \x20       }\n\
                        \x20   }\n\
                        \x20   println(s[0])\n\
                        \x20   println(w[0])\n\
                        \x20   return 0\n\
                        }\n";

#[test]
fn every_position_a_view_can_be_narrowed_in_demotes() {
    let h = Harness::new();
    let arg = r2_row("    println(rd(v[0..3]))\n");
    let annotated = r2_row("    let s: &[i64] = v[0..3]\n    println(s[0])\n");
    let branch = r2_row(
        "    let c: bool = true\n    let s: &[i64] = if c { v[0..3] } else { v[0..3] }\n    \
         println(s[0])\n",
    );
    let tail = r2_row("    let s: &[i64] = { v[0..3] }\n    println(s[0])\n");
    for (id, src) in [
        ("S4-R1-call-argument", arg.as_str()),
        ("S4-R2-annotated-let", annotated.as_str()),
        ("S4-R3-return-type", R2_RETURN),
        ("S4-R4-if-arms", branch.as_str()),
        ("S4-R5-match-arm", R2_MATCH),
        ("S4-R6-block-tail", tail.as_str()),
        ("S4-R7-nested-if", R2_NESTED),
        ("S4-R8-mixed-composites", R2_MIXED),
    ] {
        h.value_row(id, src, "7919\n7919\n", 1, 1);
    }
    h.assert_legs(8 * 8);
}

// read-only view through a composite over a global becomes e0424 and over an aliased `vec`
const R2_GLOBAL_READ: &str = "let mut g: [i64; 3] = [7919, 2, 3]\n\
                              fn main() -> i64 {\n\
                              \x20   let c: bool = true\n\
                              \x20   let s: &[i64] = if c { g[0..3] } else { g[0..3] }\n\
                              \x20   println(s[0])\n\
                              \x20   return 0\n\
                              }\n";
const R2_ALIASED_READ: &str = "fn main() -> i64 {\n\
                               \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                               \x20   let c: bool = true\n\
                               \x20   let s: &[i64] = if c { v[0..3] } else { v[0..3] }\n\
                               \x20   let w: Vec<i64> = v\n\
                               \x20   println(s[0])\n\
                               \x20   println(w[0])\n\
                               \x20   return 0\n\
                               }\n";

#[test]
fn a_missed_demotion_shows_up_as_a_verdict_and_not_as_a_count() {
    let h = Harness::new();
    h.value_row("S4-R9-global-read", R2_GLOBAL_READ, "7919\n", 0, 0);
    h.value_row("S4-R10-aliased-read", R2_ALIASED_READ, "7919\n7919\n", 1, 1);
    h.assert_legs(16);
}

#[test]
fn a_struct_field_cannot_hold_a_view_at_all() {
    let h = Harness::new();
    const FIELD: &str = "struct H { s: &[i64] }\n\
                         fn main() -> i64 {\n\
                         \x20   let a: [i64; 3] = [1, 2, 3]\n\
                         \x20   let h = H { s: a[0..3] }\n\
                         \x20   return 0\n\
                         }\n";
    h.fenced_row("S4-R11-struct-field", FIELD, "E0714");
    h.assert_legs(4);
}

// so the corpus records that the refusal is a spelling problem and not a lost capability.

const S4_N1: &str = "fn bump(s: &[i64]) -> i64 {\n\
                     \x20   s[0] = 8\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let a: [i64; 3] = [7, 2, 3]\n\
                     \x20   let q = bump(a[..])\n\
                     \x20   println(a[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_N1T: &str = "fn bump(s: &mut [i64]) -> i64 {\n\
                      \x20   s[0] = 8\n\
                      \x20   return 0\n\
                      }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut a: [i64; 3] = [7, 2, 3]\n\
                      \x20   let q = bump(a[..])\n\
                      \x20   println(a[0])\n\
                      \x20   return 0\n\
                      }\n";

const S4_N2: &str = "fn main() -> i64 {\n\
                     \x20   let a: [i64; 3] = [1, 2, 3]\n\
                     \x20   let s = a[..]\n\
                     \x20   s[0] = 9\n\
                     \x20   return s[0]\n\
                     }\n";
const S4_N2T: &str = "fn main() -> i64 {\n\
                      \x20   let mut a: [i64; 3] = [1, 2, 3]\n\
                      \x20   let s = a[..]\n\
                      \x20   s[0] = 9\n\
                      \x20   println(s[0])\n\
                      \x20   return 0\n\
                      }\n";

const S4_N3: &str = "struct Ar { d: [i64; 3] }\n\
                     fn poke(r: &Ar) -> i64 {\n\
                     \x20   let s = (*r).d[0..3]\n\
                     \x20   s[0] = 99\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut a: Ar = Ar { d: [1, 2, 3] }\n\
                     \x20   let q = poke(&a)\n\
                     \x20   println(a.d[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_N3T: &str = "struct Ar { d: [i64; 3] }\n\
                      fn poke(r: &mut Ar) -> i64 {\n\
                      \x20   let s = (*r).d[0..3]\n\
                      \x20   s[0] = 99\n\
                      \x20   return 0\n\
                      }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut a: Ar = Ar { d: [1, 2, 3] }\n\
                      \x20   let q = poke(&mut a)\n\
                      \x20   println(a.d[0])\n\
                      \x20   return 0\n\
                      }\n";

const S4_N4: &str = "fn w(s: &mut [i64]) -> i64 {\n\
                     \x20   s[0] = 101\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let a: [i64; 3] = [1, 2, 3]\n\
                     \x20   let q = w(a[0..3])\n\
                     \x20   return 0\n\
                     }\n";

// the sixth spelling of twin 1, and it is new capability made fail-closed rather than a
const S4_N5: &str = "nogc fn f(r: &mut Vec<i64>) -> i64 {\n\
                     \x20   let s = (*r)[0..3]\n\
                     \x20   s[0] = 101\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";
const S4_N5T: &str = "nogc fn f(r: &Vec<i64>) -> i64 {\n\
                      \x20   let s = (*r)[0..3]\n\
                      \x20   return s[0]\n\
                      }\n\
                      fn main() -> i64 { return 0 }\n";

const S4_N6: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let s = v[0..3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   s[0] = 101\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S4_N7: &str = "nogc fn f(s: &[Rc<i64>]) -> Rc<i64> { return s[0] }\n\
                     fn main() -> i64 { return 0 }\n";
const S4_N7T: &str = "struct C { n: i64 }\n\
                      nogc fn f(s: &[Rc<C>]) -> i64 { return s[0].n }\n\
                      fn main() -> i64 { return 0 }\n";

const S4_N8: &str = "nogc fn f(s: &mut [Rc<i64>], r: &Rc<i64>) -> i64 {\n\
                     \x20   s[0] = *r\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S4_N9: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                     \x20   let s = v[0..3]\n\
                     \x20   Vec::push(v, 4)\n\
                     \x20   println(s[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_N9T: &str = "fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                      \x20   Vec::push(v, 4)\n\
                      \x20   let s = v[0..3]\n\
                      \x20   println(s[0])\n\
                      \x20   return 0\n\
                      }\n";

const S4_N10: &str = "fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                      \x20   let f = fn() -> i64 {\n\
                      \x20       let s = v[0..3]\n\
                      \x20       return s[0]\n\
                      \x20   }\n\
                      \x20   return 0\n\
                      }\n";

const S4_N11: &str = "fn apply(f: fn(&mut [i64]) -> i64, s: &mut [i64]) -> i64 { return f(s) }\n\
                      fn main() -> i64 { return 0 }\n";

const S4_N12: &str = "struct H { v: Vec<i64> }\n\
                      nogc fn f(h: H) -> i64 { return 0 }\n\
                      fn main() -> i64 { return 0 }\n";

const S4_N13: &str = "fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                      \x20   let x = v[0..3]\n\
                      \x20   let y = v[0..3]\n\
                      \x20   x[0] = 9\n\
                      \x20   y[0] = 8\n\
                      \x20   return 0\n\
                      }\n";
const S4_N13A: &str = "fn main() -> i64 {\n\
                       \x20   let mut a: [i64; 3] = [1, 2, 3]\n\
                       \x20   let x = a[0..3]\n\
                       \x20   let y = a[0..3]\n\
                       \x20   x[0] = 9\n\
                       \x20   y[0] = 8\n\
                       \x20   return 0\n\
                       }\n";

const S4_N14: &str = "struct V3 { x: i64, y: i64, z: i64 }\n\
                      nogc fn cn(src: &[V3], dst: &mut [V3]) -> i64 {\n\
                      \x20   dst[0].x = src[0].y\n\
                      \x20   return 0\n\
                      }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut v: Vec<V3> = vec[V3 { x: 7919, y: 101, z: 3 }]\n\
                      \x20   let q = cn(v[0..1], v[0..1])\n\
                      \x20   return 0\n\
                      }\n";

// views alias with nothing to notice, while the local twin below is refused
const S4_N15: &str = "let mut g: [i64; 3] = [1, 2, 3]\n\
                      fn main() -> i64 {\n\
                      \x20   let x = g[0..3]\n\
                      \x20   let y = g[0..3]\n\
                      \x20   x[0] = 42\n\
                      \x20   println(y[0])\n\
                      \x20   return 0\n\
                      }\n";
const S4_N15L: &str = "fn main() -> i64 {\n\
                       \x20   let mut g: [i64; 3] = [1, 2, 3]\n\
                       \x20   let x = g[0..3]\n\
                       \x20   let y = g[0..3]\n\
                       \x20   x[0] = 42\n\
                       \x20   println(y[0])\n\
                       \x20   return 0\n\
                       }\n";
const S4_N15T1: &str = "let mut g: [i64; 3] = [7919, 2, 3]\n\
                        fn main() -> i64 {\n\
                        \x20   let s: &[i64] = g[0..3]\n\
                        \x20   println(s[0])\n\
                        \x20   return 0\n\
                        }\n";
const S4_N15T2: &str = "let g: [i64; 3] = [7919, 2, 3]\n\
                        fn main() -> i64 {\n\
                        \x20   let s = g[0..3]\n\
                        \x20   println(s[0])\n\
                        \x20   return 0\n\
                        }\n";
const S4_N15T3: &str = "nogc fn rd(s: &[i64]) -> i64 { return s[0] }\n\
                        let mut g: [i64; 3] = [7919, 2, 3]\n\
                        fn main() -> i64 {\n\
                        \x20   println(rd(g[0..3]))\n\
                        \x20   return 0\n\
                        }\n";

#[test]
fn s4_n1_n3_a_write_through_a_shared_view_is_e0422_with_its_repair() {
    let h = Harness::new();
    for (id, src) in [("S4-N1", S4_N1), ("S4-N2", S4_N2), ("S4-N3", S4_N3)] {
        let rendered = h.fenced_row(id, src, "E0422");
        assert!(
            rendered.contains("shared `&[i64]`") && rendered.contains("declare the slice"),
            "{id}: the refusal must name the view and the repair, not a bare `&`:\n{rendered}"
        );
    }
    h.value_row("S4-N1t", S4_N1T, "8\n", 0, 0);
    h.value_row("S4-N2t", S4_N2T, "9\n", 0, 0);
    h.value_row("S4-N3t", S4_N3T, "99\n", 0, 0);
    h.assert_legs(3 * 4 + 3 * 8);
}

#[test]
fn s4_n4_a_shared_view_into_a_mutable_parameter_is_e0416() {
    let h = Harness::new();
    let rendered = h.fenced_row("S4-N4", S4_N4, "E0416");
    assert!(
        rendered.contains("`&[i64]`") && rendered.contains("`&mut [i64]`"),
        "the flow refusal must name both slice types:\n{rendered}"
    );
    h.assert_legs(4);
}

#[test]
fn s4_n5_a_mutable_view_of_a_vec_inside_nogc_is_e0727() {
    let h = Harness::new();
    let rendered = h.fenced_row("S4-N5", S4_N5, "E0727");
    assert!(
        rendered.contains("a mutable view of a buffer that may be shared"),
        "the effect witness must name the operation, not just the function:\n{rendered}"
    );
    h.accepts("S4-N5t", S4_N5T);
    h.assert_legs(8);
}

#[test]
fn s4_n6_n14_a_view_formed_before_its_base_is_aliased_is_e0713() {
    let h = Harness::new();
    h.fenced_row("S4-N6", S4_N6, "E0713");
    h.fenced_row("S4-N13", S4_N13, "E0713");
    h.fenced_row("S4-N13a", S4_N13A, "E0713");
    h.fenced_row("S4-N14", S4_N14, "E0713");
    // the repaired shape for s4-n6 is s4-h4: alias first, then form the view
    h.value_row("S4-H4", S4_H4, "101\n7919\n", 2, 2);
    h.assert_legs(4 * 4 + 8);
}

#[test]
fn s4_n7_n8_the_managed_element_operations_are_held_by_the_effect_system() {
    let h = Harness::new();
    for (id, src) in [("S4-N7", S4_N7), ("S4-N8", S4_N8)] {
        let rendered = h.fenced_row(id, src, "E0727");
        assert!(
            rendered.contains("a managed value"),
            "{id}: §7's slice arm is narrow and correct, and what holds the OPERATION is the \
             materialization rule; if that stops naming it the arm is vacuous:\n{rendered}"
        );
    }
    h.accepts("S4-N7t", S4_N7T);
    h.assert_legs(2 * 4 + 4);
}

#[test]
fn s4_n9_n10_the_fences_this_design_depends_on_but_does_not_own_still_fire() {
    let h = Harness::new();
    // realloc invalidation is held by the borrow checker's write conflict
    h.fenced_row("S4-N9", S4_N9, "E0711");
    h.value_row("S4-N9t", S4_N9T, "1\n", 1, 1);
    h.fenced_row("S4-N10", S4_N10, "E0423");
    h.assert_legs(4 + 8 + 4);
}

#[test]
fn s4_n15_a_mutable_view_of_a_global_is_e0424_and_the_reads_are_not() {
    let h = Harness::new();
    let rendered = h.fenced_row("S4-N15", S4_N15, "E0424");
    assert!(
        rendered.contains("would alias unchecked"),
        "E0424's own text is the specification here, and it describes exactly this program:\n\
         {rendered}"
    );
    h.fenced_row("S4-N15L", S4_N15L, "E0713");
    h.value_row("S4-N15t1", S4_N15T1, "7919\n", 0, 0);
    h.value_row("S4-N15t2", S4_N15T2, "7919\n", 0, 0);
    h.value_row("S4-N15t3", S4_N15T3, "7919\n", 0, 0);
    h.assert_legs(4 + 4 + 3 * 8);
}


const SPINE_ROOTS: &[(&str, &str)] = &[
    (
        "SP1-identifier",
        "let mut g: [i64; 3] = [1, 2, 3]\n\
         fn main() -> i64 {\n\
         \x20   let x = g[0..3]\n\
         \x20   let y = g[0..3]\n\
         \x20   x[0] = 42\n\
         \x20   println(y[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP2-grouping",
        "let mut g: [i64; 3] = [1, 2, 3]\n\
         fn main() -> i64 {\n\
         \x20   let x = (g)[0..3]\n\
         \x20   let y = (g)[0..3]\n\
         \x20   x[0] = 42\n\
         \x20   println(y[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP3-member",
        "struct Ar { d: [i64; 3] }\n\
         let mut g: Ar = Ar { d: [1, 2, 3] }\n\
         fn main() -> i64 {\n\
         \x20   let x = g.d[0..3]\n\
         \x20   let y = g.d[0..3]\n\
         \x20   x[0] = 42\n\
         \x20   println(y[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP4-index",
        "let mut g: [[i64; 3]; 2] = [[1, 2, 3], [4, 5, 6]]\n\
         fn main() -> i64 {\n\
         \x20   let x = g[0][0..3]\n\
         \x20   let y = g[0][0..3]\n\
         \x20   x[0] = 42\n\
         \x20   println(y[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP5-reslice",
        "let mut g: [i64; 3] = [1, 2, 3]\n\
         fn main() -> i64 {\n\
         \x20   let x = g[0..3][0..2]\n\
         \x20   println(x[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP6-deref",
        "let mut g: [i64; 3] = [1, 2, 3]\n\
         fn main() -> i64 {\n\
         \x20   let p = &g\n\
         \x20   let x = (*p)[0..3]\n\
         \x20   println(x[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP7-nested-member",
        "struct In { d: [i64; 3] }\n\
         struct Out { i: In }\n\
         let mut g: Out = Out { i: In { d: [1, 2, 3] } }\n\
         fn main() -> i64 {\n\
         \x20   let x = g.i.d[0..3]\n\
         \x20   let y = g.i.d[0..3]\n\
         \x20   x[0] = 42\n\
         \x20   println(y[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP8-member-under-index",
        "struct Ar { d: [i64; 3] }\n\
         let mut g: [Ar; 2] = [Ar { d: [1, 2, 3] }, Ar { d: [4, 5, 6] }]\n\
         fn main() -> i64 {\n\
         \x20   let x = g[0].d[0..3]\n\
         \x20   let y = g[0].d[0..3]\n\
         \x20   x[0] = 42\n\
         \x20   println(y[0])\n\
         \x20   return 0\n\
         }\n",
    ),
    (
        "SP9-index-under-member",
        "struct Ar { d: [[i64; 3]; 2] }\n\
         let mut g: Ar = Ar { d: [[1, 2, 3], [4, 5, 6]] }\n\
         fn main() -> i64 {\n\
         \x20   let x = g.d[0][0..3]\n\
         \x20   let y = g.d[0][0..3]\n\
         \x20   x[0] = 42\n\
         \x20   println(y[0])\n\
         \x20   return 0\n\
         }\n",
    ),
];

const SP_LOCAL_TWIN: &str = "struct Ar { d: [i64; 3] }\n\
                             fn main() -> i64 {\n\
                             \x20   let mut g: Ar = Ar { d: [1, 2, 3] }\n\
                             \x20   let x = g.d[0..3]\n\
                             \x20   let y = g.d[0..3]\n\
                             \x20   x[0] = 42\n\
                             \x20   println(y[0])\n\
                             \x20   return 0\n\
                             }\n";

const SP_LOCAL_WRITES: &str = "struct Ar { d: [i64; 3] }\n\
                               fn main() -> i64 {\n\
                               \x20   let mut a: Ar = Ar { d: [1, 2, 3] }\n\
                               \x20   let s = a.d[0..3]\n\
                               \x20   s[0] = 42\n\
                               \x20   println(a.d[0])\n\
                               \x20   return 0\n\
                               }\n";

const SP_SHARED_HATCH: &str = "struct Ar { d: [i64; 3] }\n\
                               let mut g: Ar = Ar { d: [7919, 2, 3] }\n\
                               fn main() -> i64 {\n\
                               \x20   let s: &[i64] = g.d[0..3]\n\
                               \x20   println(s[0])\n\
                               \x20   return 0\n\
                               }\n";

const SP_NONMUT_GLOBAL: &str = "struct Ar { d: [i64; 3] }\n\
                                let g: Ar = Ar { d: [7919, 2, 3] }\n\
                                fn main() -> i64 {\n\
                                \x20   let s = g.d[0..3]\n\
                                \x20   println(s[0])\n\
                                \x20   return 0\n\
                                }\n";

#[test]
fn every_spine_shape_that_roots_at_a_global_is_e0424() {
    let h = Harness::new();
    for (id, src) in SPINE_ROOTS {
        h.fenced_row(id, src, "E0424");
    }
    h.assert_legs(SPINE_ROOTS.len() * 4);
}

#[test]
fn the_projected_global_and_its_local_twin_point_the_same_way() {
    let h = Harness::new();
    h.fenced_row("SP-local-twin", SP_LOCAL_TWIN, "E0713");
    h.value_row("SP-local-writes", SP_LOCAL_WRITES, "42\n", 0, 0);
    h.value_row("SP-shared-hatch", SP_SHARED_HATCH, "7919\n", 0, 0);
    h.value_row("SP-nonmut-global", SP_NONMUT_GLOBAL, "7919\n", 0, 0);
    h.assert_legs(4 + 3 * 8);
}

#[test]
fn s4_n11_n12_the_two_pending_rows_name_what_will_move_them() {
    let h = Harness::new();
    h.fenced_row("S4-N11", S4_N11, "E0301");
    h.fenced_row("S4-N12", S4_N12, "E0410");
    h.assert_legs(8);
}

// alias of it. one view plus one base read is enough, and that is the common shape, not `s4-o1`'s

const S4_O3: &str = "fn main() -> i64 {\n\
                     \x20   let mut a: [i64; 3] = [7919, 2, 3]\n\
                     \x20   let s = a[0..3]\n\
                     \x20   println(a[0])\n\
                     \x20   println(s[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_O3H: &str = "fn main() -> i64 {\n\
                      \x20   let mut a: [i64; 3] = [7919, 2, 3]\n\
                      \x20   let s: &[i64] = a[0..3]\n\
                      \x20   println(a[0])\n\
                      \x20   println(s[0])\n\
                      \x20   return 0\n\
                      }\n";
const S4_O4: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let s = v[0..3]\n\
                     \x20   println(v[0])\n\
                     \x20   println(s[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_O4H: &str = "fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                      \x20   let s: &[i64] = v[0..3]\n\
                      \x20   println(v[0])\n\
                      \x20   println(s[0])\n\
                      \x20   return 0\n\
                      }\n";
const S4_O5: &str = "let mut g: [i64; 3] = [7919, 2, 3]\n\
                     fn main() -> i64 {\n\
                     \x20   let s = g[0..3]\n\
                     \x20   println(s[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_O5H: &str = "let mut g: [i64; 3] = [7919, 2, 3]\n\
                      fn main() -> i64 {\n\
                      \x20   let s: &[i64] = g[0..3]\n\
                      \x20   println(s[0])\n\
                      \x20   return 0\n\
                      }\n";
const S4_O6: &str = "let mut g: [i64; 3] = [7919, 2, 3]\n\
                     fn main() -> i64 {\n\
                     \x20   let s: &mut [i64] = g[0..3]\n\
                     \x20   s[0] = 42\n\
                     \x20   println(g[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_O7: &str = "fn main() -> i64 {\n\
                     \x20   let mut a: [i64; 3] = [7919, 2, 3]\n\
                     \x20   let s = a[0..3]\n\
                     \x20   for i in 0..3 {\n\
                     \x20       println(s[i])\n\
                     \x20   }\n\
                     \x20   return 0\n\
                     }\n";

#[test]
fn the_over_rejection_reaches_one_view_plus_any_use_of_the_base() {
    let h = Harness::new();
    h.fenced_row("S4-O3", S4_O3, "E0713");
    h.fenced_row("S4-O4", S4_O4, "E0713");
    h.value_row("S4-O3h", S4_O3H, "7919\n7919\n", 0, 0);
    h.value_row("S4-O4h", S4_O4H, "7919\n7919\n", 1, 1);
    h.value_row("S4-O7", S4_O7, "7919\n2\n3\n", 0, 0);
    h.assert_legs(2 * 4 + 3 * 8);
}

#[test]
fn on_a_global_the_over_rejection_reaches_a_single_read_only_view() {
    let h = Harness::new();
    let rendered = h.fenced_row("S4-O5", S4_O5, "E0424");
    assert!(
        rendered.contains("copy it into a local"),
        "the refusal must say what the user does instead, because there is no annotation that \
         makes a WRITE through a global view legal:\n{rendered}"
    );
    h.fenced_row("S4-O6", S4_O6, "E0424");
    h.value_row("S4-O5h", S4_O5H, "7919\n", 0, 0);
    h.assert_legs(2 * 4 + 8);
}

const S4_O1: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let s = v[0..3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   println(s[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_O1H: &str = "fn main() -> i64 {\n\
                      \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                      \x20   let s: &[i64] = v[0..3]\n\
                      \x20   let w: Vec<i64> = v\n\
                      \x20   println(s[0])\n\
                      \x20   println(w[0])\n\
                      \x20   return 0\n\
                      }\n";
const S4_O2: &str = "fn main() -> i64 {\n\
                     \x20   let mut a: [i64; 3] = [7919, 2, 3]\n\
                     \x20   let x = a[0..3]\n\
                     \x20   let y = a[0..3]\n\
                     \x20   println(x[0])\n\
                     \x20   println(y[0])\n\
                     \x20   return 0\n\
                     }\n";
const S4_O2H: &str = "fn main() -> i64 {\n\
                      \x20   let mut a: [i64; 3] = [7919, 2, 3]\n\
                      \x20   let x: &[i64] = a[0..3]\n\
                      \x20   let y: &[i64] = a[0..3]\n\
                      \x20   println(x[0])\n\
                      \x20   println(y[0])\n\
                      \x20   return 0\n\
                      }\n";

#[test]
fn the_declared_over_rejections_carry_their_post_flip_oracles() {
    let h = Harness::new();
    h.fenced_row("S4-O1", S4_O1, "E0713");
    h.fenced_row("S4-O2", S4_O2, "E0713");
    h.value_row("S4-O1h", S4_O1H, "7919\n7919\n", 1, 1);
    h.value_row("S4-O2h", S4_O2H, "7919\n7919\n", 0, 0);
    h.assert_legs(2 * 4 + 2 * 8);
}

#[test]
fn e0426_is_discharged_and_what_replaced_it_is_named() {
    assert!(
        aelys_common::diagnostic::registry::lookup("E0426").is_none(),
        "E0426 must be gone from the registry: the obligation moved to the formation of the view, \
         where the `Vec` header is still in hand, and no store through a slice detaches under any \
         design"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root is the parent of this crate");
    let loans = fs::read_to_string(root.join("air/src/bir/loans.rs")).expect("bir loans");
    assert!(
        !loans.contains("E0426") && !loans.contains("check_vec_slice_mutation"),
        "the bir still decides the slice store; two live mechanisms would double-fire"
    );
    let origins = fs::read_to_string(root.join("air/src/bir/origins.rs")).expect("bir origins");
    assert!(
        !origins.contains("SliceWrites") && !origins.contains("slice_param_writes"),
        "the interprocedural write summary existed only for E0426; leaving it live leaves an \
         over-approximation nobody reads"
    );
    let lower = fs::read_to_string(root.join("air/src/lower/expr.rs")).expect("air lower expr");
    assert!(
        lower.contains("emit_cow_detach"),
        "the formation detach is what discharges the obligation; without it the fence came down \
         over nothing"
    );
    // e0415 fences a `&mut t` into an element, which is a different spelling of a mutable
    assert!(
        aelys_common::diagnostic::registry::lookup("E0415").is_some(),
        "E0415 has been found holding things nobody chose in five consecutive stages; this design \
         takes no position on it and must not retire it as a side effect"
    );
}

#[test]
fn the_detach_immediately_precedes_the_view_it_protects() {
    let h = Harness::new();
    // the ordering the correctness rests on: the detach repoints `v->ptr` and `slice_from_parts`
    const FORMS_A_MUTABLE_VIEW: &str = "fn main() -> i64 {\n\
                                        \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                                        \x20   let s = v[0..3]\n\
                                        \x20   println(s[0])\n\
                                        \x20   return 0\n\
                                        }\n";
    const FORMS_A_SHARED_VIEW: &str = "fn main() -> i64 {\n\
                                       \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                                       \x20   let s: &[i64] = v[0..3]\n\
                                       \x20   println(s[0])\n\
                                       \x20   return 0\n\
                                       }\n";
    for (tag, opt) in LEVELS {
        let path = h.write("detach-order", tag, FORMS_A_MUTABLE_VIEW);
        let air = lower_file_to_air(&path, *opt).expect("the mutable view must compile");
        let text = aelys_air::print::print_program(&air);
        h.legs.set(h.legs.get() + 1);
        let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
        let at = lines
            .iter()
            .position(|l| l.starts_with("call void __aelys_vec_detach("))
            .unwrap_or_else(|| panic!("no formation detach at {tag}:\n{text}"));
        let ptr = lines[at]
            .trim_start_matches("call void __aelys_vec_detach(")
            .split(':')
            .next()
            .expect("the detach names its pointer")
            .to_string();
        assert!(
            lines[at + 1].contains("slice_from_parts") && lines[at + 1].contains(&ptr),
            "the detach must be immediately followed by the view it protects, on the same \
             pointer, at {tag}:\n{}\n{}",
            lines[at],
            lines[at + 1]
        );

        let path = h.write("no-detach-order", tag, FORMS_A_SHARED_VIEW);
        let air = lower_file_to_air(&path, *opt).expect("the shared view must compile");
        let text = aelys_air::print::print_program(&air);
        h.legs.set(h.legs.get() + 1);
        assert!(
            !text.contains("__aelys_vec_detach"),
            "a `&[T]` over a `Vec` never writes, so it must detach nothing, at {tag}:\n{text}"
        );
    }
    h.assert_legs(8);
}

const S4_R1_VEC_READS: &str = r#"
nogc fn read(v: &Vec<i64>) -> i64 {
    let s: &[i64] = Vec::as_slice(*v)
    return Vec::len(*v) + s[0] - 7919
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    println(read(&v))
    println(v[0])
    return 0
}
"#;

#[test]
fn s4_r1_vec_len_and_as_slice_are_effect_free_reads() {
    let h = Harness::new();
    h.value_row("S4-R1", S4_R1_VEC_READS, "3\n7919\n", 1, 1);
    h.assert_legs(8);
}

const S4_R2_COMPUTE_NORMALS: &str = r#"
struct Vec3 { x: i64, y: i64, z: i64 }
nogc fn compute_normals(vertices: &[Vec3], normals: &mut [Vec3]) -> i64 {
    normals[0].x = vertices[0].y
    return 0
}
fn main() -> i64 {
    let mut vertices: Vec<Vec3> = vec[Vec3 { x: 7919, y: 101, z: 3 }]
    let mut normals: Vec<Vec3> = vec[Vec3 { x: 0, y: 0, z: 0 }]
    let src: &[Vec3] = Vec::as_slice(vertices)
    let dst: &mut [Vec3] = Vec::try_as_unique_mut_slice(normals)
    compute_normals(src, dst)
    println(normals[0].x)
    println(vertices[0].x)
    return 0
}
"#;

const S4_N_VEC_TOUCH: &str = r#"
nogc fn bad(v: &mut Vec<i64>) -> i64 {
    Vec::push(*v, 101)
    return 0
}
fn main() -> i64 { return 0 }
"#;

const S4_N_CALLBACK: &str = r#"
nogc fn invoke(f: fn(i64) -> i64, x: i64) -> i64 {
    return f(x)
}
fn inc(x: i64) -> i64 { return x + 1 }
fn main() -> i64 { return invoke(inc, 1) }
"#;

const S4_N_VEC_BY_VALUE: &str = r#"
nogc fn owns(v: Vec<i64>) -> i64 {
    return Vec::len(v)
}
fn main() -> i64 { return 0 }
"#;

#[test]
fn s4_r2_compute_normals_uses_vec_read_and_unique_write_views() {
    let h = Harness::new();
    h.value_row("S4-R2", S4_R2_COMPUTE_NORMALS, "101\n7919\n", 2, 2);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let primary = fs::read_to_string(root.join("sema/src/infer/expr/primary.rs")).expect("primary");
    assert!(primary.contains("variant == \"len\" || variant == \"as_slice\""));
    let lower = fs::read_to_string(root.join("air/src/lower/expr.rs")).expect("lower");
    assert!(lower.contains("Rvalue::SliceFromParts") && lower.contains("Rvalue::Len"));
    let build = fs::read_to_string(root.join("air/src/bir/build.rs")).expect("bir build");
    assert!(build.contains("variant.as_str() == \"as_slice\"") && build.contains("mutable: false"));
    let effects = fs::read_to_string(root.join("air/src/bir/effects.rs")).expect("effects");
    assert!(
        effects.contains("variant == \"len\" || variant == \"as_slice\"")
            && effects.contains("Pos::Base")
    );
    h.assert_legs(8);
}

#[test]
fn s4_r3_negative_twins_keep_managed_and_indirect_effect_fences() {
    let h = Harness::new();
    h.fenced_row("S4-N-vec-touch", S4_N_VEC_TOUCH, "E0727");
    h.fenced_row("S4-N-callback", S4_N_CALLBACK, "E0727");
    for (tag, opt) in LEVELS {
        let rendered = h.reject("S4-N-vec-by-value", tag, S4_N_VEC_BY_VALUE, *opt);
        h.legs.set(h.legs.get() + 1);
        assert!(
            rendered.contains("[E0727]"),
            "by-value Vec must remain outside nogc: {rendered}"
        );
    }
    h.assert_legs(12);
}

const S2B_FE_SHARED: &str = "fn sum(s: &[i64]) -> i64 {\n\
                             \x20   let mut t: i64 = 0\n\
                             \x20   for x in s { t = t + x }\n\
                             \x20   return t\n\
                             }\n\
                             fn main() -> i64 {\n\
                             \x20   let a: [i64; 3] = [1, 2, 3]\n\
                             \x20   println(sum(a[..]))\n\
                             \x20   return 0\n\
                             }\n";

const S2B_FE_MUT: &str = "fn sum(s: &mut [i64]) -> i64 {\n\
                          \x20   let mut t: i64 = 0\n\
                          \x20   for x in s { t = t + x }\n\
                          \x20   return t\n\
                          }\n\
                          fn main() -> i64 {\n\
                          \x20   let mut a: [i64; 3] = [1, 2, 3]\n\
                          \x20   println(sum(a[..]))\n\
                          \x20   return 0\n\
                          }\n";

const S2B_FE_EMPTY: &str = "fn count(s: &[i64]) -> i64 {\n\
                            \x20   let mut t: i64 = 0\n\
                            \x20   for x in s { t = t + 100 }\n\
                            \x20   return t\n\
                            }\n\
                            fn main() -> i64 {\n\
                            \x20   let a: [i64; 3] = [1, 2, 3]\n\
                            \x20   println(count(a[0..0]))\n\
                            \x20   return 0\n\
                            }\n";

const S2B_FE_RESLICE: &str = "fn sum(s: &[i64]) -> i64 {\n\
                              \x20   let mut t: i64 = 0\n\
                              \x20   for x in s { t = t + x }\n\
                              \x20   return t\n\
                              }\n\
                              fn main() -> i64 {\n\
                              \x20   let a: [i64; 4] = [1, 2, 3, 4]\n\
                              \x20   let s: &[i64] = a[0..3]\n\
                              \x20   let t: &[i64] = s[0..2]\n\
                              \x20   println(sum(t))\n\
                              \x20   return 0\n\
                              }\n";

const S2B_FE_BYTES: &str = "fn main() -> i64 {\n\
                            \x20   let s: string = \"abc\"\n\
                            \x20   let mut t: i64 = 0\n\
                            \x20   for b in s.bytes { t = t + (b as i64) }\n\
                            \x20   println(t)\n\
                            \x20   return 0\n\
                            }\n";

const S2B_FE_ESCAPES: &str = "fn main() -> i64 {\n\
                              \x20   let anchor: [i64; 1] = [9]\n\
                              \x20   let mut s: &[i64] = anchor[..]\n\
                              \x20   {\n\
                              \x20       let base: [i64; 3] = [1, 2, 3]\n\
                              \x20       s = base[..]\n\
                              \x20   }\n\
                              \x20   let mut t: i64 = 0\n\
                              \x20   for x in s { t = t + x }\n\
                              \x20   println(t)\n\
                              \x20   return 0\n\
                              }\n";

const S2B_FE_SCALAR: &str = "fn main() -> i64 {\n\
                             \x20   let n: i64 = 3\n\
                             \x20   for x in n { println(x) }\n\
                             \x20   return 0\n\
                             }\n";

const S2B_FE_VEC: &str = "fn main() -> i64 {\n\
                          \x20   let v: Vec<i64> = vec[1, 2, 3]\n\
                          \x20   let mut t: i64 = 0\n\
                          \x20   for x in v { t = t + x }\n\
                          \x20   println(t)\n\
                          \x20   return 0\n\
                          }\n";

#[test]
fn s2b_a_slice_iterates_by_value_at_every_level() {
    let h = Harness::new();
    h.value_row("S2B-FE-shared", S2B_FE_SHARED, "6\n", 0, 0);
    h.value_row("S2B-FE-mut", S2B_FE_MUT, "6\n", 0, 0);
    h.value_row("S2B-FE-empty", S2B_FE_EMPTY, "0\n", 0, 0);
    h.value_row("S2B-FE-reslice", S2B_FE_RESLICE, "3\n", 0, 0);
    h.value_row("S2B-FE-bytes", S2B_FE_BYTES, "294\n", 0, 0);
    h.assert_legs(40);
}

#[test]
fn s2b_iterating_a_slice_takes_a_loan_on_it() {
    let h = Harness::new();
    h.fenced_row("S2B-FE-escapes", S2B_FE_ESCAPES, "E0722");
    h.assert_legs(4);
}

// a `for` header is not a member access, so the refusal left e0304's family
#[test]
fn s2b_a_non_iterable_for_header_names_its_own_code() {
    let h = Harness::new();
    let rendered = h.fenced_row("S2B-FE-scalar", S2B_FE_SCALAR, "E0431");
    assert!(
        rendered.contains("this expression cannot be iterated") && !rendered.contains("E0304"),
        "the refusal must carry its own label rather than `invalid member access`:\n{rendered}"
    );
    assert!(
        rendered.contains("(`&[T]` or `&mut [T]`)"),
        "the message must name the slice forms that do iterate now:\n{rendered}"
    );
    // parked: a vec keeps its own code and does not fall out of the slice lowering
    h.fenced_row("S2B-FE-vec", S2B_FE_VEC, "E0414");
    h.assert_legs(8);
}
