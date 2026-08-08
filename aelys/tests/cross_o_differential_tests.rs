use aelys_driver::{compile_file_with_llvm_variant, lower_file_to_air, RuntimeVariant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use tempfile::{tempdir, TempDir};

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const CORPUS_PARTIAL_ACCEPT_BASELINE: usize = 15;

static WARM: Once = Once::new();

fn warm_core_archive() {
    WARM.call_once(|| {
        let Ok(dir) = tempdir() else { return };
        let path = dir.path().join("warmup.aelys");
        if fs::write(&path, "fn main() -> i64 { return 0 }\n").is_err() {
            return;
        }
        let _ =
            compile_file_with_llvm_variant(&path, OptimizationLevel::None, false, RuntimeVariant::Rc);
    });
}

#[derive(Clone, PartialEq, Eq)]
enum LevelResult {
    Rejected,
    Ran { exit: i32, stdout: String },
}

impl LevelResult {
    fn render(&self) -> String {
        match self {
            LevelResult::Rejected => "rejected".to_string(),
            LevelResult::Ran { exit, stdout } => format!("exit={exit} stdout={stdout:?}"),
        }
    }
}

#[derive(Clone, Copy)]
enum Expect {
    Invariant,
// a dead construct is removed by dce, so the post-opt check has nothing to compare
    SeamDivergent(&'static str),
}

// a fixture that could not be measured is reported, not silently passed
enum Eval {
    Unavailable,
    Nondeterministic,
    Levels(Vec<(&'static str, LevelResult)>),
}

struct Harness {
    dir: TempDir,
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
        Harness { dir: tempdir().expect("tempdir") }
    }

    fn run(exe: &Path) -> Option<(i32, String)> {
        let out = Command::new(exe).output().ok()?;
        Some((exit_code(&out.status), String::from_utf8_lossy(&out.stdout).into_owned()))
    }

    fn evaluate(&self, id: &str, src: &str) -> Eval {
        let mut results = Vec::with_capacity(LEVELS.len());
        let mut first_exe: Option<PathBuf> = None;
        for (name, opt) in LEVELS {
            let path = self.dir.path().join(format!("{}.aelys", slug(id, name)));
            if fs::write(&path, src).is_err() {
                return Eval::Unavailable;
            }
            match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
                Err(err) => {
                    if linker_unavailable(&err.to_string()) {
                        return Eval::Unavailable;
                    }
                    results.push((name, LevelResult::Rejected));
                }
                Ok(()) => {
                    let exe = exe_path_for(&path);
                    let Some((exit, stdout)) = Harness::run(&exe) else {
                        return Eval::Unavailable;
                    };
                    if first_exe.is_none() {
                        first_exe = Some(exe);
                    }
                    results.push((name, LevelResult::Ran { exit, stdout }));
                }
            }
        }

        if let Some(exe) = first_exe {
            let Some(second) = Harness::run(&exe) else {
                return Eval::Unavailable;
            };
            let first = results.iter().find_map(|(_, r)| match r {
                LevelResult::Ran { exit, stdout } => Some((*exit, stdout.clone())),
                LevelResult::Rejected => None,
            });
            if first != Some(second) {
                return Eval::Nondeterministic;
            }
        }

        Eval::Levels(results)
    }
}

fn compare(id: &str, levels: &[(&'static str, LevelResult)]) -> (Vec<String>, bool) {
    let accepted: Vec<&(&'static str, LevelResult)> = levels
        .iter()
        .filter(|(_, r)| matches!(r, LevelResult::Ran { .. }))
        .collect();
    let partial = !accepted.is_empty() && accepted.len() < levels.len();

    if accepted.len() < 2 {
        return (Vec::new(), partial);
    }
    let (_, baseline) = accepted[0];
    if accepted.iter().all(|(_, r)| r == baseline) {
        return (Vec::new(), partial);
    }
    let breakdown = levels
        .iter()
        .map(|(name, r)| format!("{name}: {}", r.render()))
        .collect::<Vec<_>>()
        .join("\n      ");
    (vec![format!("  {id}: the levels that produced a runtime disagree:\n      {breakdown}")], partial)
}

// every divergence class this project has seen, plus the two value-semantics shapes the run was built around

const FIXTURES: &[(&str, Expect, &str)] = &[
    (
// the inliner duplicated the argument to every parameter occurrence, so an effectful arg ran more than once
        "SI-D01",
        Expect::Invariant,
        r#"
fn side() -> i64 {
    println(9)
    return 2
}
fn twice(x: i64) -> i64 {
    return x + x
}
fn main() -> i64 {
    return twice(side())
}
"#,
    ),
    (
        "SI-D02",
        Expect::Invariant,
        r#"
fn side() -> i64 {
    println(9)
    return 2
}
fn thrice(x: i64) -> i64 {
    return x + x + x
}
fn main() -> i64 {
    return thrice(side())
}
"#,
    ),
    (
// int_min / -1 was undefined in the backend: sigfpe at -o0, a folded constant at -o2
        "SI-D05",
        Expect::Invariant,
        r#"
fn main() -> i64 {
    let a = 0 - 9223372036854775807 - 1
    let b = 0 - 1
    return a / b
}
"#,
    ),
    (
        "SI-D06",
        Expect::Invariant,
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::null()
    return Rc::get(r)
}
"#,
    ),
    (
        "SI-D07",
        Expect::Invariant,
        r#"
struct Resource { id: i64 }
fn use_it() {
    let a = Resource{id: 7}
}
fn main() -> i64 {
    use_it()
    return 0
}
"#,
    ),
    (
        "SI-D08",
        Expect::Invariant,
        r#"
fn make(x: i64) -> Rc<i64> {
    println(9)
    return Rc::new(x)
}
fn main() -> i64 {
    discard make(3)
    return 0
}
"#,
    ),
    (
// value stability across a real reallocation: the alias is taken while the buffer is small
        "SI-D09",
        Expect::Invariant,
        r#"
fn main() -> i64 {
    let mut v = Vec::new()
    let mut i = 0
    while i < 40 {
        Vec::push(v, i)
        i = i + 1
    }
    let w = v
    Vec::push(v, 99)
    let junk = vec[77, 77, 77]
    return w[0] + w[39] + v[40]
}
"#,
    ),
    (
// dce removes it at -o2/-o3 and the post-mono vec-surface check has nothing to reject
        "SI-D10",
        Expect::SeamDivergent("dead generic-enum-with-Vec: dce removes the instantiation the post-mono vec-surface check would reject"),
        r#"
enum Opt<T> { Some(T), Nil }
fn main() -> i64 {
    let mut i1 = Vec::new()
    Vec::push(i1, 1)
    let a = Opt::Some(i1)
    return 0
}
"#,
    ),
    (
// a stdout-shaped divergence: interpolation allocates and formats per iteration, so a pass can reorder it
        "SI-D11",
        Expect::Invariant,
        r#"
fn main() -> i64 {
    let mut i = 0
    let mut total = 0
    while i < 3 {
        println("row {i} of 3")
        total = total + i
        i = i + 1
    }
    return total
}
"#,
    ),
    (
        "SI-D12",
        Expect::Invariant,
        r#"
fn main() -> i64 {
    let base = 4
    let f = fn(x: i64) -> i64 {
        println(x)
        return x + base
    }
    return f(3) + f(2)
}
"#,
    ),
    (
// defect number one, carried here for its -o1 leg: the invariants suite fixes the absolute bound
        "SI-D13",
        Expect::Invariant,
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    v[0] = 9
    return w[0]
}
"#,
    ),
    (
        "SI-D14",
        Expect::Invariant,
        r#"
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    Vec::push(v, 3)
    let w = v
    Vec::push(v, 4)
    Vec::push(v, 5)
    let junk = vec[77, 77, 77]
    return w[0]
}
"#,
    ),
    (
// `?` short-circuits on the error path, so the second marker must not print at any level
        "SI-D15",
        Expect::Invariant,
        r#"
enum Result<T, E> { Ok(T), Err(E) }
fn probe(x: i64) -> Result<i64, i64> {
    println("probe {x}")
    if x < 0 {
        return Result::Err(4)
    }
    return Result::Ok(x + 1)
}
fn chain(x: i64) -> Result<i64, i64> {
    let a = probe(x)?
    let b = probe(a - 3)?
    return Result::Ok(a + b)
}
fn main() -> i64 {
    return match chain(1) { Result::Ok(v) => v, Result::Err(e) => e }
}
"#,
    ),
];

#[test]
fn curated_fixtures_produce_the_same_answer_at_every_opt_level() {
    let h = Harness::new();
    let mut failures = Vec::new();
    let mut measured = 0usize;

    for (id, expect, src) in FIXTURES {
        match h.evaluate(id, src) {
            Eval::Unavailable => {
                eprintln!("{id}: toolchain unavailable, skipping");
                return;
            }
            Eval::Nondeterministic => {
                failures.push(format!("  {id}: two runs of the same binary disagree, so no cross-O comparison is meaningful"));
            }
            Eval::Levels(levels) => {
                measured += 1;
                let (mut lines, partial) = compare(id, &levels);
                failures.append(&mut lines);
                let breakdown = || {
                    levels
                        .iter()
                        .map(|(name, r)| format!("{name}: {}", r.render()))
                        .collect::<Vec<_>>()
                        .join("\n      ")
                };
                match expect {
                    Expect::Invariant if partial => failures.push(format!(
                        "  {id}: declared Invariant but the verdict changes across -O. only a fixture \
                         declared SeamDivergent may do that, and only for a DEAD post-opt-detectable \
                         construct:\n      {}",
                        breakdown()
                    )),
                    Expect::SeamDivergent(reason) if !partial => failures.push(format!(
                        "  {id}: declared SeamDivergent ({reason}) but every level now reaches the same \
                         verdict. the seam moved, so the declaration must be re-derived:\n      {}",
                        breakdown()
                    )),
                    _ => {}
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "a program's runtime behaviour must not depend on the -O level, but {} of {measured} curated \
         fixtures broke that:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(measured, FIXTURES.len(), "every curated fixture must be measured");
}

const PREOPT_FIXTURES: &[(&str, &str, &str)] = &[
    (
        "SI-D03",
        "E0702",
        r#"
struct Resource { id: i64 }
fn main() -> i64 {
    let a = Resource{id: 1}
    if false {
        let b = a
        let c = a
    }
    return 0
}
"#,
    ),
    (
        "SI-D04",
        "E0711",
        r#"
fn main() -> i64 {
    let mut x = 3
    let r = &x
    x = 5
    if false {
        return *r
    }
    return 0
}
"#,
    ),
];

#[test]
fn pre_opt_verdicts_hold_at_every_opt_level_even_when_dead() {
    let h = Harness::new();
    let mut failures = Vec::new();
    for (id, code, src) in PREOPT_FIXTURES {
        for (name, opt) in LEVELS {
            let path = h.dir.path().join(format!("{}.aelys", slug(id, name)));
            fs::write(&path, src).expect("write fixture");
            match lower_file_to_air(&path, opt) {
                Ok(_) => failures.push(format!("  {id} at {name}: accepted, but {code} is a pre-opt verdict and must hold at every level")),
                Err(rendered) => {
                    if !rendered.contains(&format!("[{code}]")) {
                        failures.push(format!("  {id} at {name}: rejected without {code}:\n{rendered}"));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "a pre-optimization verdict must not depend on the -O level:\n{}",
        failures.join("\n")
    );
}

// the curated table above holds every known divergence class, so the default path loses nothing we curate

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the aelys crate has a parent workspace dir")
        .to_path_buf()
}

fn collect_aelys(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_aelys(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("aelys") {
            out.push(path);
        }
    }
}

#[test]
// miscompiled at -o0 only (llvm repaired them at -o1+), so this sweep would have flagged them
#[ignore = "STAGE GATE: run scripts/corpus_sweep.sh instead -- in-process this is OOM-killed"]
fn the_aelys_corpus_produces_the_same_answer_at_every_opt_level() {
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in ["tests_e2e", "torture", "examples", "aelys/tests/exploration"] {
        collect_aelys(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(files.len() > 300, "the corpus should be the repo's .aelys files, found {}", files.len());

    let h = Harness::new();
    let mut failures = Vec::new();
    let mut partials = Vec::new();
    let mut nondeterministic = Vec::new();

    for (i, path) in files.iter().enumerate() {
        let name = path.strip_prefix(&root).unwrap_or(path).display().to_string();
        let Ok(src) = fs::read_to_string(path) else { continue };
        let id = format!("c{i}");
        match h.evaluate(&id, &src) {
            Eval::Unavailable => {
                eprintln!("corpus sweep: toolchain unavailable, skipping");
                return;
            }
            Eval::Nondeterministic => nondeterministic.push(name),
            Eval::Levels(levels) => {
                let (lines, partial) = compare(&name, &levels);
                for line in lines {
                    failures.push(line);
                }
                if partial {
                    let breakdown = levels
                        .iter()
                        .map(|(n, r)| format!("{n}={}", r.render()))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    partials.push(format!("  {name}: {breakdown}"));
                }
            }
        }
    }

    eprintln!("corpus sweep: {} files, {} partially accepting, {} nondeterministic", files.len(), partials.len(), nondeterministic.len());
    for line in &partials {
        eprintln!("{line}");
    }
    for name in &nondeterministic {
        eprintln!("  nondeterministic, not compared: {name}");
    }

    assert!(
        failures.is_empty(),
        "{} corpus files produce a different answer at different -O levels:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        partials.len() <= CORPUS_PARTIAL_ACCEPT_BASELINE,
        "the partial-acceptance ratchet: {} corpus files change verdict across -O, baseline is {}. a \
         NEW one is a regression unless it is a dead post-opt-detectable construct, in which case \
         lower nothing and re-derive the baseline deliberately:\n{}",
        partials.len(),
        CORPUS_PARTIAL_ACCEPT_BASELINE,
        partials.join("\n")
    );
}

