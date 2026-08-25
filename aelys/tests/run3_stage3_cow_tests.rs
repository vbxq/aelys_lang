// ! run 3 · stage 3 · the cow witness corpus.

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

// managed container cannot be involved, so any reappearance of one in the text is a defect.

const S3_12: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
                     \x20   let r = &mut v[0]\n\
                     \x20   *r = 101\n\
                     \x20   return 0\n\
                     }\n";

const S3_13: &str = "fn main() -> i64 {\n\
                     \x20   let mut a: [i64; 3] = [1, 2, 3]\n\
                     \x20   let r = &mut a[0]\n\
                     \x20   *r = 101\n\
                     \x20   return 0\n\
                     }\n";

const S3_14: &str = "struct D { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut d: D = D{n: 1}\n\
                     \x20   let r = &mut d.n\n\
                     \x20   *r = 101\n\
                     \x20   return 0\n\
                     }\n";

#[test]
fn s3_12_the_managed_projection_is_still_fenced() {
    let h = Harness::new();
    let rendered = h.fenced_row("S3-12", S3_12, "E0415");
    assert!(
        rendered.contains("[mut-index-ref]"),
        "the marker is what every other suite in the tree keys on and must survive the message \
         repair:\n{rendered}"
    );
    h.assert_legs(4);
}

#[test]
fn s3_13_an_array_element_is_fenced_by_the_same_code_and_a_true_reason() {
    let h = Harness::new();
    let rendered = h.fenced_row("S3-13", S3_13, "E0415");
    assert!(
        !rendered.contains("Vec"),
        "this program has no managed storage anywhere, so a rendered message naming one is a \
         false reason:\n{rendered}"
    );
    h.assert_legs(4);
}

#[test]
fn s3_14_a_plain_struct_field_is_fenced_by_the_same_code_and_a_true_reason() {
    let h = Harness::new();
    let rendered = h.fenced_row("S3-14", S3_14, "E0415");
    assert!(
        !rendered.contains("Vec"),
        "this program has no managed storage anywhere, so a rendered message naming one is a \
         false reason:\n{rendered}"
    );
    h.assert_legs(4);
}

#[test]
fn e0415_explain_states_a_reason_that_is_true_of_every_program_it_rejects() {
    use aelys_common::diagnostic::registry;
    let info = registry::lookup("E0415").expect("E0415 must have an --explain entry");
    assert!(
        !info.explanation.contains("Vec"),
        "three quarters of E0415's rejections have no managed storage in them, so the \
         explanation must not rest on one:\n{}",
        info.explanation
    );
    assert!(
        !info.explanation.contains("refcount"),
        "the predicate reads no type and no refcount:\n{}",
        info.explanation
    );
    assert!(
        info.explanation.contains("reads no type"),
        "the true reason must be stated, not merely the false one removed:\n{}",
        info.explanation
    );
}

// the detach moves from the air consumer to the air producer. these rows are the ones that

const S3_01: &str = "fn poke(r: &mut Vec<i64>) -> i64 {\n\
                     \x20   (*r)[0] = 101\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   let q = poke(&mut v)\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S3_02: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   v[0] = 101\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S3_09: &str = "nogc fn peek(r: &i64) -> i64 { return *r }\n\
                     fn main() -> i64 {\n\
                     \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   println(peek(&v[0]))\n\
                     \x20   println(peek(&w[0]))\n\
                     \x20   return 0\n\
                     }\n";

const S3_10: &str = "fn grow(r: &mut Vec<i64>) -> i64 {\n\
                     \x20   Vec::push(*r, 4)\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   let q = grow(&mut v)\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S3_11: &str = "fn main() -> i64 {\n\
                     \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   println(v[0])\n\
                     \x20   println(v[2])\n\
                     \x20   return 0\n\
                     }\n";

const S3_27: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   let g = fn() -> i64 {\n\
                     \x20       v[0] = S{n: 101}\n\
                     \x20       return 0\n\
                     \x20   }\n\
                     \x20   let q = g()\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   return 0\n\
                     }\n";

#[test]
fn s3_01_a_mut_view_of_a_whole_owner_still_detaches_at_the_store() {
    let h = Harness::new();
    h.value_row("S3-01", S3_01, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_02_the_depth_one_indexed_store_keeps_its_values_through_the_move() {
    let h = Harness::new();
    h.value_row("S3-02", S3_02, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_09_a_read_through_an_element_reference_must_not_detach() {
    let h = Harness::new();
    h.value_row("S3-09", S3_09, "7919\n7919\n", 1, 1);
    h.assert_legs(8);
}

#[test]
fn s3_10_pushs_own_detach_path_is_untouched_by_the_move() {
    let h = Harness::new();
    h.value_row("S3-10", S3_10, "7919\n7919\n", 2, 2);
    h.assert_legs(8);
}

// head emitted a cow guard for every element of every vec literal, on a buffer it had just
#[test]
fn s3_11_a_vec_literals_element_stores_allocate_nothing() {
    let h = Harness::new();
    h.value_row("S3-11", S3_11, "7919\n3\n", 1, 1);
    h.assert_legs(8);
}

#[test]
fn s3_27_a_capturing_closure_writing_a_whole_element_is_already_by_value() {
    let h = Harness::new();
    h.value_row("S3-27", S3_27, "7919\n7919\n", 3, 1);
    h.assert_legs(8);
}

// joining `generate_vec_runtime_call`'s splice list instead types the detach `void(ptr,ptr,i64)`,

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn intercept_precedes_splice(text: &str) -> bool {
    let Some(intercept) = text.find("name == \"__aelys_vec_detach\"") else {
        return false;
    };
    let Some(splice) = text.find("generate_vec_runtime_call(name, args, &arg_values)") else {
        return false;
    };
    intercept < splice
}

#[test]
fn s3_25_the_detach_is_intercepted_ahead_of_the_vec_runtime_splice_list() {
    const REORDERED: &str = "if name == \"__aelys_vec_init\" {\n\
                             return self.generate_vec_runtime_call(name, args, &arg_values);\n\
                             }\n\
                             if name == \"__aelys_vec_detach\" { }\n";
    assert!(
        !intercept_precedes_splice(REORDERED),
        "the predicate must fail on a source where the splice list comes first, or it proves \
         nothing about the real one"
    );

    let calls = fs::read_to_string(repo_root().join("codegen/src/lowering/calls.rs"))
        .expect("codegen call lowering");
    assert!(
        intercept_precedes_splice(&calls),
        "__aelys_vec_detach must be intercepted in generate_call ahead of the splice list"
    );
    assert!(
        calls.contains("self.emit_vec_detach("),
        "the intercept must route to emit_vec_detach, which is what carries the inline fast path"
    );

    let stores = fs::read_to_string(repo_root().join("codegen/src/lowering/stmts.rs"))
        .expect("codegen store lowering");
    assert!(
        !stores.contains("vec_root_of"),
        "the store-side discriminator is gone; the detach is decided in air lowering now, and \
         two live mechanisms could double-fire"
    );
}

// the e0429 family, discharged. every row below is a program the fence refused; each now

const S3_03: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   v[0].n = 101\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   return 0\n\
                     }\n";

const S3_04: &str = "struct S { n: i64 }\n\
                     fn poke(r: &mut Vec<S>) -> i64 {\n\
                     \x20   (*r)[0].n = 101\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   let q = poke(&mut v)\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   return 0\n\
                     }\n";

const S3_05: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<[i64; 3]> = vec[[1, 2, 3], [4, 5, 6]]\n\
                     \x20   let w: Vec<[i64; 3]> = v\n\
                     \x20   v[0][1] = 101\n\
                     \x20   println(v[0][1])\n\
                     \x20   println(w[0][1])\n\
                     \x20   return 0\n\
                     }\n";

const S3_06: &str = "struct I { k: i64 }\n\
                     struct S { i: I }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{i: I{k: 7919}}, S{i: I{k: 2}}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   v[0].i.k = 101\n\
                     \x20   println(v[0].i.k)\n\
                     \x20   println(w[0].i.k)\n\
                     \x20   return 0\n\
                     }\n";

const S3_07: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   let g = fn(r: &mut Vec<S>) -> i64 {\n\
                     \x20       (*r)[0].n = 101\n\
                     \x20       return 0\n\
                     \x20   }\n\
                     \x20   let q = g(&mut v)\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   return 0\n\
                     }\n";

const S3_08: &str = "struct S { x: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{x: 7919}, S{x: 2}]\n\
                     \x20   v[0].x = 101\n\
                     \x20   println(v[0].x)\n\
                     \x20   return 0\n\
                     }\n";

const S3_28: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   let outer = fn(r: &mut Vec<S>) -> i64 {\n\
                     \x20       let inner = fn(q: &mut Vec<S>) -> i64 {\n\
                     \x20           (*q)[0].n = 101\n\
                     \x20           return 0\n\
                     \x20       }\n\
                     \x20       return inner(r)\n\
                     \x20   }\n\
                     \x20   let z = outer(&mut v)\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   return 0\n\
                     }\n";

const S3_30: &str = "struct S { a: [i64; 3] }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{a: [7919, 2, 3]}, S{a: [4, 5, 6]}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   v[0].a[0] = 101\n\
                     \x20   println(v[0].a[0])\n\
                     \x20   println(w[0].a[0])\n\
                     \x20   return 0\n\
                     }\n";

const S3_31: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   v[0].n += 101\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   return 0\n\
                     }\n";

const S3_32: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 0}, S{n: 0}, S{n: 0}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   let mut i: i64 = 0\n\
                     \x20   while i < 3 {\n\
                     \x20       v[i].n = i + 1\n\
                     \x20       i = i + 1\n\
                     \x20   }\n\
                     \x20   println(v[2].n)\n\
                     \x20   println(w[2].n)\n\
                     \x20   println(i)\n\
                     \x20   return 0\n\
                     }\n";

const S3_33: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   let u: Vec<S> = w\n\
                     \x20   v[0].n = 101\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   println(u[0].n)\n\
                     \x20   return 0\n\
                     }\n";

#[test]
fn s3_03_a_field_under_an_element_detaches_before_the_address_exists() {
    let h = Harness::new();
    h.value_row("S3-03", S3_03, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_04_the_same_store_through_a_reference_spine() {
    let h = Harness::new();
    h.value_row("S3-04", S3_04, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_05_an_index_under_an_index_detaches_once() {
    let h = Harness::new();
    h.value_row("S3-05", S3_05, "101\n2\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_06_two_field_steps_under_an_element() {
    let h = Harness::new();
    h.value_row("S3-06", S3_06, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

// silent wrong answer, because the fence was decided in the bir and a closure body has no bir
#[test]
fn s3_07_the_same_store_inside_a_lambda_body() {
    let h = Harness::new();
    h.value_row("S3-07", S3_07, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_08_a_buffer_that_was_never_shared_allocates_nothing() {
    let h = Harness::new();
    h.value_row("S3-08", S3_08, "101\n", 1, 1);
    h.assert_legs(8);
}

// a second live miscompile at head, `101 | 101` allocs=1, in neither charter a14's table nor
#[test]
fn s3_28_a_lambda_nested_inside_a_lambda() {
    let h = Harness::new();
    h.value_row("S3-28", S3_28, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_30_an_array_element_under_a_field_under_an_element() {
    let h = Harness::new();
    h.value_row("S3-30", S3_30, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

// the read-modify-write reads the pre-detach buffer and the write lands post-detach, so an
#[test]
fn s3_31_a_compound_store_whose_read_straddles_the_detach() {
    let h = Harness::new();
    h.value_row("S3-31", S3_31, "8020\n7919\n", 2, 2);
    h.assert_legs(8);
}

#[test]
fn s3_32_a_loop_carried_element_store_advances_and_stays_private() {
    let h = Harness::new();
    h.value_row("S3-32", S3_32, "3\n0\n3\n", 2, 2);
    h.assert_legs(8);
}

// one detach at refcount 3 must leave both other owners reading the old value
#[test]
fn s3_33_three_way_sharing() {
    let h = Harness::new();
    h.value_row("S3-33", S3_33, "101\n7919\n7919\n", 2, 2);
    h.assert_legs(8);
}

// s3-22 the effect rule and the detach hook must agree.

const S3_22_STORE: &str = "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
                           \x20   (*r)[0] = 101\n\
                           \x20   return 0\n\
                           }\n\
                           fn main() -> i64 { return 0 }\n";

// pick up a detach if the mode gate were wrong. a probe of reads that never form one would
const S3_22_READS: &str = "nogc fn peek(r: &Vec<i64>) -> i64 { return (*r)[0] }\n\
                           nogc fn peekaddr(r: &Vec<i64>) -> i64 {\n\
                           \x20   let p = &(*r)[0]\n\
                           \x20   return *p\n\
                           }\n\
                           fn main() -> i64 {\n\
                           \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                           \x20   let w: Vec<i64> = v\n\
                           \x20   println(peek(&v))\n\
                           \x20   println(peekaddr(&w))\n\
                           \x20   return 0\n\
                           }\n";

fn declared_nogc_fns(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("nogc fn "))
        .filter_map(|r| r.split('(').next())
        .map(|n| n.trim().to_string())
        .collect()
}

#[test]
fn s3_22_a_nogc_function_is_refused_the_store_and_never_lowers_a_detach() {
    let h = Harness::new();
    let rendered = h.fenced_row("S3-22", S3_22_STORE, "E0727");
    assert!(
        rendered.contains("a store into a buffer that may be shared"),
        "the effect witness must name the store clause, not a bare managed type:\n{rendered}"
    );

    let names = declared_nogc_fns(S3_22_READS);
    assert_eq!(
        names.len(),
        2,
        "the probe must reach two declared nogc bodies, or it proves nothing about either"
    );
    for (tag, opt) in LEVELS {
        let path = h.write("S3-22-reads", tag, S3_22_READS);
        let air = lower_file_to_air(&path, *opt)
            .unwrap_or_else(|e| panic!("S3-22 reads at {tag} must compile:\n{e}"));
        h.legs.set(h.legs.get() + 1);
        for name in &names {
            let f = air
                .functions
                .iter()
                .find(|f| &f.name == name)
                .unwrap_or_else(|| panic!("S3-22 at {tag}: no lowered body for nogc fn {name}"));
            let body = aelys_air::print::print_function(f, &air);
            assert!(
                !body.contains("__aelys_vec_detach"),
                "S3-22 at {tag}: `nogc fn {name}` lowered a cow detach:\n{body}"
            );
        }
    }
    h.assert_legs(8);
}


/// every element address realized in this body must be preceded by a detach of the very local
fn detach_precedes_every_element_addr(air: &str) -> Option<String> {
    let mut detached: Vec<&str> = Vec::new();
    for line in air.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("call void __aelys_vec_detach(") {
            if let Some(local) = rest.split(':').next() {
                detached.push(local);
            }
            continue;
        }
        if let Some((_, rhs)) = l.split_once("= addr ") {
            let head = rhs.split(':').next().unwrap_or("");
            if let Some((local, _)) = head.split_once('[') {
                if !detached.contains(&local) {
                    return Some(l.to_string());
                }
            }
        }
    }
    None
}

#[test]
fn s3_23_the_detach_is_emitted_before_the_element_address_it_protects() {
    const REORDERED: &str = "    %8: *S = addr %7[0i64]: S\n\
                             \x20   call void __aelys_vec_detach(%7: *Vec<S>)\n";
    assert!(
        detach_precedes_every_element_addr(REORDERED).is_some(),
        "the predicate must fail on a body where the address comes first, or its green over the \
         real ones means nothing"
    );

    let h = Harness::new();
    for (id, src) in [
        ("S3-03", S3_03),
        ("S3-04", S3_04),
        ("S3-05", S3_05),
        ("S3-06", S3_06),
        ("S3-07", S3_07),
    ] {
        for (tag, opt) in LEVELS {
            let path = h.write(id, tag, src);
            let air = lower_file_to_air(&path, *opt)
                .unwrap_or_else(|e| panic!("{id} at {tag} must compile:\n{e}"));
            let text = aelys_air::print::print_program(&air);
            h.legs.set(h.legs.get() + 1);
            assert!(
                text.contains("__aelys_vec_detach"),
                "{id} at {tag}: no detach emitted at all, so the ordering assertion is vacuous"
            );
            if let Some(line) = detach_precedes_every_element_addr(&text) {
                panic!(
                    "{id} at {tag}: `{line}` realizes an element address with no preceding \
                     detach of that local; the detach repoints the buffer, so the write would \
                     land in the other owner's"
                );
            }
        }
    }
    h.assert_legs(20);
}

#[test]
fn e0429_is_discharged_and_what_replaced_it_is_named() {
    use aelys_common::diagnostic::registry;
    assert!(
        registry::lookup("E0429").is_none(),
        "E0429 must be gone from the registry: the obligation it stood in for is discharged, \
         not deferred"
    );
    let bir = fs::read_to_string(repo_root().join("air/src/bir/loans.rs")).expect("bir loans");
    assert!(
        !bir.contains("E0429"),
        "the bir no longer decides the projection store; if it does again, two mechanisms are \
         live at once"
    );
    // the fences that were not retired, and that a2(2)'s remaining spellings rest on
    for code in ["E0415", "E0714", "E0727"] {
        let info = registry::lookup(code)
            .unwrap_or_else(|| panic!("{code} must still have an --explain entry"));
        assert!(
            !info.explanation.trim().is_empty(),
            "{code}'s --explain entry must not be empty"
        );
    }
}

// it by making depth 2 alias again: a capturing closure cannot mutate an outer `vec`, at any

const S3_26: &str = "struct S { n: i64 }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                     \x20   let w: Vec<S> = v\n\
                     \x20   let g = fn() -> i64 {\n\
                     \x20       v[0].n = 101\n\
                     \x20       return 0\n\
                     \x20   }\n\
                     \x20   let q = g()\n\
                     \x20   println(v[0].n)\n\
                     \x20   println(w[0].n)\n\
                     \x20   return 0\n\
                     }\n";

const S3_29: &str = "fn main() -> i64 {\n\
                     \x20   let mut x: i64 = 7919\n\
                     \x20   let g = fn() -> i64 {\n\
                     \x20       x = 101\n\
                     \x20       return x\n\
                     \x20   }\n\
                     \x20   let q = g()\n\
                     \x20   println(q)\n\
                     \x20   println(x)\n\
                     \x20   return 0\n\
                     }\n";

#[test]
fn s3_26_a_capturing_closure_cannot_mutate_an_outer_vec_at_depth_two_either() {
    let h = Harness::new();
    h.value_row("S3-26", S3_26, "7919\n7919\n", 3, 1);
    h.assert_legs(8);
}

// it, it rests on a measurement over storage that has no cow at all
#[test]
fn s3_29_a_capture_is_by_value_for_storage_the_closure_writes() {
    let h = Harness::new();
    h.value_row("S3-29", S3_29, "101\n7919\n", 1, 0);
    h.assert_legs(8);
}

// the fences that were not retired. each names the code and why that code.

const S3_15: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let s: &mut [i64] = v[0..3]\n\
                     \x20   s[0] = 101\n\
                     \x20   return 0\n\
                     }\n";

const S3_16: &str = "fn view(r: &mut Vec<i64>) -> &mut [i64] { return (*r)[0..3] }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let s = view(&mut v)\n\
                     \x20   s[0] = 101\n\
                     \x20   return 0\n\
                     }\n";

const S3_15_ALIASED: &str = "fn main() -> i64 {\n\
                             \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                             \x20   let w: Vec<i64> = v\n\
                             \x20   let s: &mut [i64] = v[0..3]\n\
                             \x20   s[0] = 101\n\
                             \x20   println(v[0])\n\
                             \x20   println(w[0])\n\
                             \x20   return 0\n\
                             }\n";

const S3_16_ALIASED: &str = "fn view(r: &mut Vec<i64>) -> &mut [i64] { return (*r)[0..3] }\n\
                             fn main() -> i64 {\n\
                             \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                             \x20   let w: Vec<i64> = v\n\
                             \x20   let s = view(&mut v)\n\
                             \x20   s[0] = 101\n\
                             \x20   println(v[0])\n\
                             \x20   println(w[0])\n\
                             \x20   return 0\n\
                             }\n";

const S3_17: &str = "enum Opt { Some(&mut [i64]), None }\n\
                     fn view(r: &mut Vec<i64>) -> Opt {\n\
                     \x20   let s: &mut [i64] = (*r)[0..3]\n\
                     \x20   return Opt::Some(s)\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S3_18: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let r = &mut v\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   (*r)[0] = 101\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S3_19: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let r = &mut v\n\
                     \x20   Vec::push(v, 4)\n\
                     \x20   (*r)[0] = 101\n\
                     \x20   println(v[0])\n\
                     \x20   return 0\n\
                     }\n";

const S3_20: &str = "fn main() -> i64 {\n\
                     \x20   let v: Vec<Vec<i64>> = vec[vec[1, 2], vec[3, 4]]\n\
                     \x20   return 0\n\
                     }\n";

const S3_20B: &str = "struct P { n: i64 }\n\
                      fn main() -> i64 {\n\
                      \x20   let v: Rc<Vec<P>> = Rc::new(vec[P{n: 1}])\n\
                      \x20   return 0\n\
                      }\n";

const S3_20C: &str = "struct S { n: i64 }\n\
                      fn poke<T>(v: T) -> i64 {\n\
                      \x20   v[0].n = 101\n\
                      \x20   return 0\n\
                      }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                      \x20   return poke(v)\n\
                      }\n";

const S3_21_RC: &str = "struct N { n: i64 }\n\
                        fn main() -> i64 {\n\
                        \x20   let v: Vec<Rc<N>> = vec[Rc::new(N{n: 1})]\n\
                        \x20   return 0\n\
                        }\n";

const S3_21_STRUCT: &str = "struct S { v: Vec<i64> }\n\
                            fn main() -> i64 {\n\
                            \x20   let s: S = S{v: vec[1, 2, 3]}\n\
                            \x20   return 0\n\
                            }\n";

// the slice disjunct is discharged: the detach moved off the store, which cannot reach the `vec`
#[test]
fn s3_15_a_write_through_a_slice_of_a_vec_now_runs() {
    let h = Harness::new();
    h.value_row("S3-15", S3_15, "", 1, 1);
    h.value_row("S3-15-aliased", S3_15_ALIASED, "101\n7919\n", 2, 2);
    h.assert_legs(16);
}

#[test]
fn s3_16_the_view_returned_across_an_origin_now_runs() {
    let h = Harness::new();
    h.value_row("S3-16", S3_16, "", 1, 1);
    h.value_row("S3-16-aliased", S3_16_ALIASED, "101\n7919\n", 2, 2);
    h.assert_legs(16);
}

// value is unconstructible, so a user can write it and never inhabit it. this row flips the day
#[test]
fn s3_17_the_typed_failure_channel_is_unconstructible() {
    let h = Harness::new();
    h.fenced_row("S3-17", S3_17, "E0714");
    h.assert_legs(4);
}

// head, so it cannot witness e0713 at all; both spellings are asserted so the day e0415 retires
#[test]
fn s3_18_the_buffer_cannot_be_re_shared_under_a_live_view() {
    let h = Harness::new();
    h.fenced_row("S3-18", S3_18, "E0713");
    h.fenced_row("S3-18-interior", S3_12, "E0415");
    h.assert_legs(8);
}

#[test]
fn s3_19_the_buffer_cannot_be_reallocated_under_a_live_view() {
    let h = Harness::new();
    h.fenced_row("S3-19", S3_19, "E0711");
    h.assert_legs(4);
}

// the nested-vec family never reaches the detach, so `__aelys_vec_detach`'s shallow memcpy is
#[test]
fn s3_20_a_vec_inside_a_vec_never_reaches_the_question() {
    let h = Harness::new();
    h.fenced_row("S3-20", S3_20, "E0412");
    h.assert_legs(4);
}

#[test]
fn s3_20b_an_rc_of_a_vec_is_what_holds_the_narrowing() {
    let h = Harness::new();
    h.fenced_row("S3-20b", S3_20B, "E0412");
    h.assert_legs(4);
}

// the same, via a type parameter: it is only `vec` after monomorphisation, so it misses an
#[test]
fn s3_20c_a_generic_instantiated_at_a_vec_is_the_other_half() {
    let h = Harness::new();
    h.fenced_row("S3-20c", S3_20C, "E0412");
    h.assert_legs(4);
}

#[test]
fn s3_21_the_rc_bearing_and_struct_nested_families_never_reach_it_either() {
    let h = Harness::new();
    h.fenced_row("S3-21-rc", S3_21_RC, "E0410");
    h.fenced_row("S3-21-struct", S3_21_STRUCT, "E0410");
    h.assert_legs(8);
}

// unreachable a slot typed `vec<t>` owns exactly one counted share, so refcount 0 means the

#[test]
fn s3_24_the_c_and_codegen_uniqueness_predicates_stay_complements() {
    let root = repo_root();
    let c = fs::read_to_string(root.join("core/src/aelys_core_common.c")).expect("core runtime");
    let cg =
        fs::read_to_string(root.join("codegen/src/lowering/runtime.rs")).expect("runtime lowering");

    // out of emit_vec_detach's own body or it reads green off the wrong comparison
    let detach = cg
        .split("pub(crate) fn emit_vec_detach(")
        .nth(1)
        .and_then(|s| s.split("\n    pub(crate) fn ").next())
        .expect("emit_vec_detach");

    assert!(
        c.contains("if (__aelys_rc_refcount(v->ptr) <= 1) {"),
        "the C side's count case moved; codegen's inline guard is its complement and nothing \
         but this row keeps the two in step"
    );
    assert!(
        detach.contains("IntPredicate::UGT")
            && detach.contains("i32_ty.const_int(1, false)")
            && detach.contains("\"cow_shared\""),
        "codegen's inline guard must stay `UGT 1 -> slow`, the exact complement of `<= 1 -> return`:\n{detach}"
    );

    let refcount = c
        .split("unsigned __aelys_rc_refcount(void *ptr) {")
        .nth(1)
        .and_then(|s| s.split('}').next())
        .expect("__aelys_rc_refcount");
    assert!(
        refcount.contains("if (!ptr)") && refcount.contains("return 1;"),
        "C's null verdict comes from refcount(NULL) == 1 and NOT from the `<=`, so a guard on \
         the `<=` alone pins the smaller rule:\n{refcount}"
    );
    assert!(
        detach.contains("build_is_not_null(data_ptr, \"cow_nn\")"),
        "codegen's null verdict comes from a separate is-not-null branch, and it is the half a \
         count-only guard cannot see"
    );
}

const S3_34: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   v[0] += 101\n\
                     \x20   println(v[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

const S3_35: &str = "fn main() -> i64 {\n\
                     \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                     \x20   let w: Vec<i64> = v\n\
                     \x20   let r = &mut v\n\
                     \x20   *r = vec[101, 5, 6]\n\
                     \x20   println((*r)[0])\n\
                     \x20   println(w[0])\n\
                     \x20   return 0\n\
                     }\n";

// the index-compound row. `lower_index_assign` emits its detach immediately before its own
#[test]
fn s3_34_an_index_compound_store_reads_before_the_detach_and_writes_after() {
    let h = Harness::new();
    h.value_row("S3-34", S3_34, "8020\n7919\n", 2, 2);
    h.assert_legs(8);
}

// the old buffer and is independent of the detach
#[test]
fn s3_35_a_deref_assign_replacing_the_whole_slot() {
    let h = Harness::new();
    h.value_row("S3-35", S3_35, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}

// - leak legs run under `aelys_alloc=malloc`, because under immix every runtime allocation is

#[cfg(feature = "asan-invariants")]
mod asan {
    use super::*;

    const ASAN_ROWS: &[(&str, &str)] = &[
        ("S3-01", S3_01),
        ("S3-02", S3_02),
        ("S3-03", S3_03),
        ("S3-04", S3_04),
        ("S3-05", S3_05),
        ("S3-06", S3_06),
        ("S3-07", S3_07),
        ("S3-08", S3_08),
        ("S3-09", S3_09),
        ("S3-10", S3_10),
        ("S3-11", S3_11),
        ("S3-26", S3_26),
        ("S3-27", S3_27),
        ("S3-28", S3_28),
        ("S3-29", S3_29),
        ("S3-30", S3_30),
        ("S3-31", S3_31),
        ("S3-32", S3_32),
        ("S3-33", S3_33),
        ("S3-34", S3_34),
        ("S3-35", S3_35),
    ];

    const ERROR_TIERS: &[&str] = &[
        "heap-use-after-free",
        "double-free",
        "attempting free on address which was not malloc",
        "heap-buffer-overflow",
    ];

    fn build_asan_archive(dir: &Path) -> Option<PathBuf> {
        let core_src = repo_root().join("core").join("src");
        let mut objects = Vec::new();
        for unit in [
            "aelys_core_common.c",
            "aelys_alloc_immix.c",
            "aelys_rc_real.c",
        ] {
            let src = core_src.join(unit);
            if !src.is_file() {
                eprintln!("core source {unit} missing; skipping the ASan tier");
                return None;
            }
            let obj = dir.join(unit).with_extension("o");
            match Command::new("clang")
                .args(["-fsanitize=address", "-g", "-c"])
                .arg(&src)
                .arg(format!("-I{}", core_src.display()))
                .arg("-o")
                .arg(&obj)
                .output()
            {
                Ok(out) if out.status.success() => objects.push(obj),
                Ok(out) => panic!(
                    "instrumented core compile of {unit} failed:\n{}",
                    String::from_utf8_lossy(&out.stderr)
                ),
                Err(_) => {
                    eprintln!("clang unavailable; skipping the ASan tier");
                    return None;
                }
            }
        }
        let archive = dir.join("libaelys-core-rc-asan.a");
        match Command::new("ar")
            .arg("rcs")
            .arg(&archive)
            .args(&objects)
            .output()
        {
            Ok(out) if out.status.success() => Some(archive),
            Ok(out) => panic!("ar failed:\n{}", String::from_utf8_lossy(&out.stderr)),
            Err(_) => {
                eprintln!("ar unavailable; skipping the ASan tier");
                None
            }
        }
    }

    fn asan_symbol_count(exe: &Path) -> usize {
        let Ok(out) = Command::new("nm").arg(exe).output() else {
            return 0;
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("asan"))
            .count()
    }

    #[test]
    fn asan_tier_the_corpus_reports_no_memory_error() {
        let h = Harness::new();
        let Some(_archive) = build_asan_archive(h.dir.path()) else {
            return;
        };
        let mut instrumented_and_run = 0usize;

        for (id, src) in ASAN_ROWS {
            let path = h.write(id, "asan", src);
            if compile_file_with_llvm_variant(
                &path,
                OptimizationLevel::None,
                false,
                RuntimeVariant::Rc,
            )
            .is_err()
            {
                eprintln!("{id}: toolchain unavailable, skipping the ASan tier");
                return;
            }
            let object = path.with_extension(if cfg!(windows) { "obj" } else { "o" });
            if !object.is_file() {
                eprintln!("{id}: object not produced, skipping the ASan tier");
                return;
            }
            let exe = h.dir.path().join(format!("{}_asan_exe", slug(id, "")));
            let link = Command::new("clang")
                .args(["-fsanitize=address", "-g"])
                .arg(&object)
                .arg(format!("-L{}", h.dir.path().display()))
                .arg("-laelys-core-rc-asan")
                .arg("-o")
                .arg(&exe)
                .output();
            let Ok(link) = link else {
                eprintln!("{id}: clang unavailable, skipping the ASan tier");
                return;
            };
            assert!(
                link.status.success(),
                "{id}: ASan link failed:\n{}",
                String::from_utf8_lossy(&link.stderr)
            );
            assert!(
                asan_symbol_count(&exe) > 0,
                "{id}: the linked binary carries no asan symbols, so a clean run proves nothing"
            );

            let run = Command::new(&exe)
                .env("AELYS_RC_STATS", "1")
                .env("AELYS_ALLOC", "malloc")
                .env("ASAN_OPTIONS", "detect_leaks=1")
                .output()
                .expect("run the instrumented exe");
            let stderr = String::from_utf8_lossy(&run.stderr);
            instrumented_and_run += 1;
            for tier in ERROR_TIERS {
                assert!(
                    !stderr.contains(tier),
                    "{id}: ASan reports {tier}\nstderr:\n{stderr}"
                );
            }
        }

        assert_eq!(
            instrumented_and_run,
            ASAN_ROWS.len(),
            "an earlier version of this sweep reported 0 against an expected 21 and was caught \
             only by this counter"
        );
    }
}

