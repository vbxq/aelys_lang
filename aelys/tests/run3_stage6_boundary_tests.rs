// ! program passes with the formation detach deleted, so it witnesses that the capability exists

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

    fn fenced_row(&self, id: &str, src: &str, code: &str) -> String {
        let mut last = String::new();
        for (tag, opt) in LEVELS {
            let path = self.write(id, tag, src);
            self.legs.set(self.legs.get() + 1);
            let rendered = match lower_file_to_air(&path, *opt) {
                Ok(_) => panic!("{id} at {tag} MUST be rejected, but it was accepted:\n{src}"),
                Err(rendered) => rendered,
            };
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

    fn observe(&self, id: &str, src: &str) -> (String, i64, i64) {
        let mut seen: Option<(String, i64, i64)> = None;
        for (tag, opt) in LEVELS {
            let Some(exe) = self.compile(id, tag, src, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set"
                );
                return (String::new(), -1, -1);
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let o = self.run(&exe, *alloc);
                self.legs.set(self.legs.get() + 1);
                assert_eq!(o.exit, 0, "{id} at {tag}/{alloc_name} must exit 0:\n{src}");
                let (a, f) = o
                    .stats
                    .unwrap_or_else(|| panic!("{id} at {tag}/{alloc_name}: no [rc] stats line"));
                let now = (o.stdout, a, f);
                match &seen {
                    None => seen = Some(now),
                    Some(prev) => assert_eq!(
                        *prev, now,
                        "{id} at {tag}/{alloc_name} disagrees with an earlier leg"
                    ),
                }
            }
        }
        seen.expect("at least one leg")
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

// the headline. both slices derived from managed `vec`s, and a third owner aliasing the

const S6_H1: &str = "struct V3 { x: i64, y: i64, z: i64 }\n\
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

const S6_H2: &str = "nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = src[0]\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut sv: Vec<i64> = vec[7919]\n\
                     \x20   let mut dv: Vec<i64> = vec[0]\n\
                     \x20   let dw: Vec<i64> = dv\n\
                     \x20   let q = cn(sv[0..1], dv[0..1])\n\
                     \x20   println(dv[0])\n\
                     \x20   println(dw[0])\n\
                     \x20   return 0\n\
                     }\n";

const S6_H3: &str = "nogc fn dbl(x: i64) -> i64 { return x + x }\n\
                     nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = dbl(src[0])\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut sv: Vec<i64> = vec[50]\n\
                     \x20   let mut dv: Vec<i64> = vec[0]\n\
                     \x20   let dw: Vec<i64> = dv\n\
                     \x20   let q = cn(sv[0..1], dv[0..1])\n\
                     \x20   println(dv[0])\n\
                     \x20   println(dw[0])\n\
                     \x20   return 0\n\
                     }\n";

#[test]
fn the_headline_a_nogc_fn_writes_a_managed_vec_through_a_mutable_view() {
    let h = Harness::new();
    h.value_row("S6-H1", S6_H1, "101\n0\n7919\n", 3, 3);
    h.value_row("S6-H2", S6_H2, "7919\n0\n", 3, 3);
    h.value_row("S6-H3", S6_H3, "100\n0\n", 3, 3);
    h.assert_legs(24);
}

const S6_H2_NOREGION: &str = "fn main() -> i64 {\n\
                              \x20   let mut sv: Vec<i64> = vec[7919]\n\
                              \x20   let mut dv: Vec<i64> = vec[0]\n\
                              \x20   let dw: Vec<i64> = dv\n\
                              \x20   dv[0] = sv[0]\n\
                              \x20   println(dv[0])\n\
                              \x20   println(dw[0])\n\
                              \x20   return 0\n\
                              }\n";

#[test]
fn the_region_adds_no_allocation_of_its_own() {
    let h = Harness::new();
    let with_region = h.observe("S6-E1", S6_H2);
    let without_region = h.observe("S6-E2", S6_H2_NOREGION);
    assert_eq!(
        with_region, without_region,
        "the `nogc` region must change neither the values nor the allocation totals; the honest \
         claim is NO ADDITIONAL allocation inside the region, and this is what measures it"
    );
    assert_eq!(
        with_region,
        ("7919\n0\n".to_string(), 3, 3),
        "the differential is only worth its zero if both sides are pinned absolutely"
    );
    h.assert_legs(16);
}

const S6_M1: &str = "nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = src[0]\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut sv: Vec<i64> = vec[7919]\n\
                     \x20   let mut dv: Vec<i64> = vec[0]\n\
                     \x20   let q = cn(sv[0..1], dv[0..1])\n\
                     \x20   Vec::push(dv, 101)\n\
                     \x20   Vec::push(sv, 5)\n\
                     \x20   println(dv[0])\n\
                     \x20   println(dv[1])\n\
                     \x20   println(sv[0])\n\
                     \x20   println(sv[1])\n\
                     \x20   return 0\n\
                     }\n";

// the same, with the destination aliased before the call: growing `dv` afterwards must not
const S6_M2: &str = "nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = src[0]\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 {\n\
                     \x20   let mut sv: Vec<i64> = vec[7919]\n\
                     \x20   let mut dv: Vec<i64> = vec[0]\n\
                     \x20   let dw: Vec<i64> = dv\n\
                     \x20   let q = cn(sv[0..1], dv[0..1])\n\
                     \x20   Vec::push(dv, 101)\n\
                     \x20   println(dv[0])\n\
                     \x20   println(dv[1])\n\
                     \x20   println(dw[0])\n\
                     \x20   println(sv[0])\n\
                     \x20   return 0\n\
                     }\n";

#[test]
fn the_callers_vec_is_still_a_vec_after_the_nogc_call() {
    let h = Harness::new();
    h.value_row("S6-M1", S6_M1, "7919\n101\n7919\n5\n", 2, 2);
    h.value_row("S6-M2", S6_M2, "7919\n101\n0\n7919\n", 3, 3);
    h.assert_legs(16);
}


const S6_T1: &str = "nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = src[0]\n\
                     \x20   let r: Rc<i64> = Rc::new(1)\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S6_T2: &str = "nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                     \x20   let w: Vec<i64> = vec[1]\n\
                     \x20   dst[0] = src[0]\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S6_T3: &str = "fn helper(x: i64) -> i64 {\n\
                     \x20   let w: Vec<i64> = vec[x]\n\
                     \x20   return w[0]\n\
                     }\n\
                     nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = helper(src[0])\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S6_T3_KEPT: &str = "fn helper(x: i64) -> i64 {\n\
                          \x20   return x + 1\n\
                          }\n\
                          nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                          \x20   dst[0] = helper(src[0])\n\
                          \x20   return 0\n\
                          }\n\
                          fn main() -> i64 { return 0 }\n";

const S6_T4: &str = "nogc fn cn(src: &[i64], dst: &mut [i64], f: fn(i64) -> i64) -> i64 {\n\
                     \x20   dst[0] = f(src[0])\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S6_T5: &str = "nogc fn cn(src: Vec<i64>, dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = src[0]\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S6_T6: &str = "nogc fn cn(dst: &mut [i64]) -> i64 {\n\
                     \x20   println(dst[0])\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S6_KEPT: &str = "nogc fn cn(src: &[i64], dst: &mut [i64]) -> i64 {\n\
                       \x20   dst[0] = src[0]\n\
                       \x20   return 0\n\
                       }\n\
                       fn main() -> i64 { return 0 }\n";

const TWINS: &[(&str, &str, &str)] = &[
    ("S6-T1", S6_T1, "cn -> Rc::new"),
    ("S6-T2", S6_T2, "cn -> a vec literal"),
    ("S6-T3", S6_T3, "cn -> helper -> a vec literal"),
    ("S6-T4", S6_T4, "cn -> <indirect call>"),
    ("S6-T5", S6_T5, "cn -> the managed parameter `src`"),
    ("S6-T6", S6_T6, "cn -> <indirect call>"),
];

#[test]
fn every_refusal_names_the_operation_and_ships_its_kept_form() {
    let h = Harness::new();
    h.accepts("S6-KEPT", S6_KEPT);
    h.accepts("S6-T3-KEPT", S6_T3_KEPT);
    for (id, src, via) in TWINS {
        let rendered = h.fenced_row(id, src, "E0727");
        assert!(
            rendered.contains(&format!("via `{via}`")),
            "{id} must name the operation as `{via}`, not merely the type\ngot:\n{rendered}"
        );
    }
    h.assert_legs(32);
}

#[test]
fn the_refusal_reason_is_a_path_through_the_program_and_not_a_type_name() {
    let h = Harness::new();
    let rendered = h.fenced_row("S6-T3", S6_T3, "E0727");
    assert!(
        rendered.contains("cn -> helper -> a vec literal"),
        "the path must traverse the callee\ngot:\n{rendered}"
    );
    assert!(
        rendered.contains("managed memory reached here"),
        "the offending operation must be labelled where it is written\ngot:\n{rendered}"
    );
    assert!(
        rendered.contains("calls `helper` here"),
        "the edge that reaches it must be labelled too\ngot:\n{rendered}"
    );
    h.assert_legs(4);
}


const S6_B1: &str = "nogc fn cn(src: &[Rc<i64>], dst: &mut [i64]) -> i64 {\n\
                     \x20   dst[0] = 1\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

const S6_B2: &str = "nogc fn cn(src: &[Rc<i64>], dst: &mut [Rc<i64>]) -> i64 {\n\
                     \x20   dst[0] = src[0]\n\
                     \x20   return 0\n\
                     }\n\
                     fn main() -> i64 { return 0 }\n";

#[test]
fn a_managed_element_type_is_not_itself_the_refusal() {
    let h = Harness::new();
    h.accepts("S6-B1", S6_B1);
    h.fenced_row("S6-B2", S6_B2, "E0727");
    h.assert_legs(8);
}

// `__aelys_alloc` is served out of the immix arena by default and lsan cannot see it individually.
#[cfg(feature = "asan-invariants")]
mod asan {
    use super::*;

    fn build_asan_archive(dir: &Path) -> Option<PathBuf> {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let core_src = manifest
            .parent()
            .unwrap_or(manifest)
            .join("core")
            .join("src");
        let mut objects = Vec::new();
        for unit in [
            "aelys_core_common.c",
            "aelys_alloc_immix.c",
            "aelys_rc_real.c",
        ] {
            let src = core_src.join(unit);
            if !src.is_file() {
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
                Err(_) => return None,
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
            Err(_) => None,
        }
    }

    fn link_instrumented(dir: &Path, id: &str, src: &str) -> PathBuf {
        let path = dir.join(format!("{id}.aelys"));
        fs::write(&path, src).expect("write source");
        compile_file_with_llvm_variant(&path, OptimizationLevel::None, false, RuntimeVariant::Rc)
            .unwrap_or_else(|e| panic!("{id}: the ASan tier must compile its rows: {e}"));
        let object = path.with_extension(if cfg!(windows) { "obj" } else { "o" });
        assert!(object.is_file(), "{id}: no object was produced");
        let exe = dir.join(format!("{id}_asan_exe"));
        let link = Command::new("clang")
            .args(["-fsanitize=address", "-g"])
            .arg(&object)
            .arg(format!("-L{}", dir.display()))
            .arg("-laelys-core-rc-asan")
            .arg("-o")
            .arg(&exe)
            .output()
            .expect("clang link");
        assert!(
            link.status.success(),
            "{id}: ASan link failed:\n{}",
            String::from_utf8_lossy(&link.stderr)
        );
        exe
    }

    struct AsanRun {
        stdout: String,
        stderr: String,
        exit: Option<i32>,
    }

    // leak detection changes the process exit code, so the program's own verdict and the leak
    fn run_asan_with(exe: &Path, malloc: bool, leaks: bool) -> AsanRun {
        let mut cmd = Command::new(exe);
        cmd.env("AELYS_RC_STATS", "1").env(
            "ASAN_OPTIONS",
            if leaks {
                "detect_leaks=1"
            } else {
                "detect_leaks=0"
            },
        );
        if malloc {
            cmd.env("AELYS_ALLOC", "malloc");
        }
        let out = cmd.output().expect("run the instrumented exe");
        AsanRun {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            exit: out.status.code(),
        }
    }

    // bootstrap leak is one stack per `println`, so the expected leak total is exactly that
    fn memory_rows() -> Vec<(&'static str, &'static str, &'static str, usize)> {
        vec![
            ("S6-H1", S6_H1, "101\n0\n7919\n", 3),
            ("S6-H2", S6_H2, "7919\n0\n", 2),
            ("S6-H3", S6_H3, "100\n0\n", 2),
            ("S6-E2", S6_H2_NOREGION, "7919\n0\n", 2),
            ("S6-M1", S6_M1, "7919\n101\n7919\n5\n", 4),
            ("S6-M2", S6_M2, "7919\n101\n0\n7919\n", 4),
        ]
    }

    #[test]
    fn the_sanitizer_is_actually_linked_in() {
        let dir = tempdir().expect("tempdir");
        let Some(_archive) = build_asan_archive(dir.path()) else {
            panic!("the ASan tier must build its archive on a machine with clang and ar");
        };
        let exe = link_instrumented(dir.path(), "armed", S6_H2);
        let out = Command::new(&exe)
            .env("ASAN_OPTIONS", "verbosity=1")
            .output()
            .expect("run");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("AddressSanitizer"),
            "the binary is not instrumented, so every clean row below would be a confident \
             zero\nstderr:\n{err}"
        );
    }

    #[test]
    fn every_memory_touching_row_is_clean_under_asan_and_malloc() {
        let dir = tempdir().expect("tempdir");
        let Some(_archive) = build_asan_archive(dir.path()) else {
            panic!("the ASan tier must build its archive on a machine with clang and ar");
        };
        for (id, src, stdout, printlns) in memory_rows() {
            let exe = link_instrumented(dir.path(), &slug(id, "asan"), src);

            let r = run_asan_with(&exe, true, false);
            assert_eq!(r.exit, Some(0), "{id} under ASan:\n{}", r.stderr);
            assert_eq!(r.stdout, stdout, "{id} under ASan:\n{}", r.stderr);
            assert!(
                !r.stderr.contains("AddressSanitizer:"),
                "{id}: the sanitizer reports a memory error\nstderr:\n{}",
                r.stderr
            );

            // leg 2: the leak accounting. the only leaks allowed are the known bootstrap ones,
            let l = run_asan_with(&exe, true, true);
            assert_eq!(
                l.stacks(),
                printlns,
                "{id}: expected exactly {printlns} leak stack(s), all from the known \
                 `println` helper\nstderr:\n{}",
                l.stderr
            );
            assert!(
                !l.stderr.contains("in __aelys_alloc"),
                "{id}: a buffer allocated through the Aelys allocator leaked\nstderr:\n{}",
                l.stderr
            );
        }
    }

    // the leak oracle above is only worth its zero if a leak would be seen. a capturing closure
    const S6_LEAK: &str = "fn main() -> i64 {\n\
                           \x20   let mut v: Vec<i64> = vec[7919]\n\
                           \x20   let w: Vec<i64> = v\n\
                           \x20   let g = fn() -> i64 {\n\
                           \x20       v[0] = 101\n\
                           \x20       return 0\n\
                           \x20   }\n\
                           \x20   let q = g()\n\
                           \x20   println(v[0])\n\
                           \x20   println(w[0])\n\
                           \x20   return 0\n\
                           }\n";

    #[test]
    fn the_leak_oracle_is_armed_and_only_the_malloc_leg_can_see_a_buffer() {
        let dir = tempdir().expect("tempdir");
        let Some(_archive) = build_asan_archive(dir.path()) else {
            panic!("the ASan tier must build its archive on a machine with clang and ar");
        };
        let exe = link_instrumented(dir.path(), "planted_leak", S6_LEAK);

        let immix = run_asan_with(&exe, false, true);
        assert_eq!(
            immix.stacks(),
            2,
            "under immix only the two `println` leaks are visible\nstderr:\n{}",
            immix.stderr
        );
        assert!(
            !immix.stderr.contains("in __aelys_alloc"),
            "immix serves this buffer from its arena, so LSan cannot name it\nstderr:\n{}",
            immix.stderr
        );

        let malloc = run_asan_with(&exe, true, true);
        assert_eq!(
            malloc.stacks(),
            3,
            "under malloc the leaked buffer is a third stack\nstderr:\n{}",
            malloc.stderr
        );
        assert!(
            malloc.stderr.contains("in __aelys_alloc"),
            "the planted leak stopped reproducing, so every clean row above is now unarmed\n\
             stderr:\n{}",
            malloc.stderr
        );
        assert!(
            malloc.stderr.contains("allocs=3 frees=1"),
            "stderr:\n{}",
            malloc.stderr
        );
    }

    impl AsanRun {
        fn stacks(&self) -> usize {
            self.stderr.matches("allocated from:").count()
        }
    }
}

