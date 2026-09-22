use aelys_driver::{
    RuntimeVariant, compile_file_with_llvm_variant, compile_file_with_llvm_with_warnings,
    lower_file_to_air,
};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::{TempDir, tempdir};

mod common;
use common::{
    Cli, Leg, Outcome, RunResult, exe_path_for, linker_unavailable, slug, warm_core_archive,
};

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

// re-derived at 27b4eea09ae99f0e: 0 partial over 415 corpus files, and the 15 were never partial
const CORPUS_PARTIAL_ACCEPT_BASELINE: usize = 0;

// the one program that carried this baseline is excluded by name below, so a swept program that stops terminating here is a new one
const CORPUS_NONTERMINATING_BASELINE: usize = 0;

// a recursion that prints megabytes cuts wherever the stack runs out, so two runs already disagree
const NONDETERMINISTIC_BY_NATURE: [(&str, &str); 1] = [(
    "torture/torture_test_26.aelys",
    "an unbounded recursion that prints until the stack ends, a different number of bytes on \
     every run",
)];

// the tracked half of the swept population, the only half a fresh clone has, and the floors are measured on it alone
const TRACKED_PROBE_FLOOR: usize = 56;
const TRACKED_COMPARED_FLOOR: usize = 33;

const CORPUS_NONDETERMINISTIC_BASELINE: usize = 0;

const CORPUS_LINK_FAULT_BASELINE: usize = 0;

const CORPUS_WORKERS: usize = 4;

#[derive(Clone, PartialEq, Eq)]
enum LevelResult {
    Rejected,
    Ran(RunResult),
}

impl LevelResult {
    fn render(&self) -> String {
        match self {
            LevelResult::Rejected => "rejected".to_string(),
            LevelResult::Ran(result) => result.render(),
        }
    }
}

// only a missing linker excuses a leg, and nothing excuses a sweep that measured none of them
enum Eval {
    NoLinker,
    Unavailable(String),
    Nondeterministic,
    Levels(Vec<(&'static str, LevelResult)>),
}

struct Harness {
    dir: TempDir,
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
        }
    }

    fn run(exe: &Path) -> Option<RunResult> {
        let _pin = common::pin_legs("run", 1);
        let out = Command::new(exe).output().ok()?;
        common::note_leg();
        Some(RunResult::read(
            &out.status,
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ))
    }

    fn evaluate(&self, id: &str, src: &str) -> Eval {
        let mut results = Vec::with_capacity(LEVELS.len());
        let mut first_exe: Option<PathBuf> = None;
        for (name, opt) in LEVELS {
            let path = self.dir.path().join(format!("{}.aelys", slug(id, name)));
            if fs::write(&path, src).is_err() {
                return Eval::Unavailable(format!("{id}: the fixture could not be written"));
            }
            match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
                Err(err) => {
                    if linker_unavailable(&err.to_string()) {
                        common::require_linker_skip(
                            "a skipped value row carries no runtime evidence at all",
                        );
                        return Eval::NoLinker;
                    }
                    results.push((name, LevelResult::Rejected));
                }
                Ok(()) => {
                    let exe = exe_path_for(&path);
                    let Some(result) = Harness::run(&exe) else {
                        return Eval::Unavailable(format!(
                            "{id} at {name}: the artifact was produced and would not run"
                        ));
                    };
                    if first_exe.is_none() {
                        first_exe = Some(exe);
                    }
                    results.push((name, LevelResult::Ran(result)));
                }
            }
        }

        if let Some(exe) = first_exe {
            let Some(second) = Harness::run(&exe) else {
                return Eval::Unavailable(format!("{id}: the artifact ran once and not twice"));
            };
            let first = results.iter().find_map(|(_, r)| match r {
                LevelResult::Ran(result) => Some(result.clone()),
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
        .filter(|(_, r)| matches!(r, LevelResult::Ran(_)))
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
    (
        vec![format!(
            "  {id}: the levels that produced a runtime disagree:\n      {breakdown}"
        )],
        partial,
    )
}


const FIXTURES: &[(&str, &str)] = &[
    (
        "SI-D01",
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
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::null()
    return Rc::get(r)
}
"#,
    ),
    (
        "SI-D07",
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
        // the net refuses the split dce used to open here, so the verdict is one verdict again
        "SI-D10",
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
        "SI-D11",
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
        "SI-D13",
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

    for (id, src) in FIXTURES {
        match h.evaluate(id, src) {
            Eval::NoLinker => {
                common::require_linker_skip("a skipped fixture compares no opt level to any other");
            }
            Eval::Unavailable(why) => failures.push(format!("  {why}")),
            Eval::Nondeterministic => {
                failures.push(format!("  {id}: two runs of the same binary disagree, so no cross-O comparison is meaningful"));
            }
            Eval::Levels(levels) => {
                measured += 1;
                let (mut lines, partial) = compare(id, &levels);
                failures.append(&mut lines);
                if partial {
                    let breakdown = levels
                        .iter()
                        .map(|(name, r)| format!("{name}: {}", r.render()))
                        .collect::<Vec<_>>()
                        .join("\n      ");
                    failures.push(format!(
                        "  {id}: the verdict changes across -O. the net refuses a split as E0432, \
                         so a partial that reaches here is a rejecting check running downstream of \
                         the compared stage:\n      {breakdown}"
                    ));
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
    assert_eq!(
        measured,
        FIXTURES.len(),
        "OOB-2: every curated fixture must be measured. a sweep that compares nothing passes for \
         the same reason one that stops before running does, so AELYS_ALLOW_LINKER_SKIP excuses a \
         leg and never the comparison."
    );
}

const PREOPT_FIXTURES: &[(&str, &str, &str)] = &[
    (
        "SI-D09",
        "E0204",
        r#"
pub let g: i64 = 1
pub let g: i64 = 2
fn main() -> i64 {
    println(g)
    return 0
}
"#,
    ),
    (
        "SI-D10",
        "E0204",
        r#"
pub let g: i64 = 1
pub let mut g: i64 = 2
fn main() -> i64 {
    println(g)
    g = 99
    println(g)
    return 0
}
"#,
    ),
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

#[test]
fn the_duplicate_global_divergence_would_have_been_caught_by_this_class() {
    let answered = |stdout: &str| {
        LevelResult::Ran(RunResult {
            outcome: common::Outcome::Exit(0),
            stdout: stdout.to_string(),
        })
    };
    let recorded = [
        ("-O0", answered("2\n")),
        ("-O1", answered("2\n")),
        ("-O2", answered("1\n")),
        ("-O3", answered("1\n")),
    ];
    let (failures, _) = compare("SI-D09", &recorded);
    assert!(
        !failures.is_empty(),
        "SI-D09 was measured on the pre-repair compiler at -O0 2, -O1 2, -O2 1, -O3 1; the row \
         asserts E0204 at every level now, so this is the standing proof that the comparison \
         which catches it is the one running, and that a one-level row could not have seen it"
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

// the four directories are ignored by git, so the local half exists on a working copy and on no fresh clone
const LOCAL_CORPUS_DIRS: [&str; 4] = [
    "tests_e2e",
    "torture",
    "examples",
    "aelys/tests/exploration",
];

fn local_corpus_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in LOCAL_CORPUS_DIRS {
        collect_aelys(&root.join(dir), &mut files);
    }
    files.sort();
    files
}

fn local_corpus(root: &Path) -> Vec<(String, String)> {
    local_corpus_files(root)
        .iter()
        .filter_map(|path| {
            let src = fs::read_to_string(path).ok()?;
            let name = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string();
            Some((name, src))
        })
        .collect()
}

fn tracked_probes() -> Vec<(String, String)> {
    tracked_probe_sources()
        .into_iter()
        .map(|(name, src)| (name, src.to_string()))
        .collect()
}

fn population_line(local: usize, probes: usize) -> String {
    if local == 0 {
        format!(
            "population: corpus local absent, 0 files, a fresh clone tracks none of them; probes \
             {probes} programs tracked, the measurement rests on these alone"
        )
    } else {
        format!(
            "population: corpus {local} files, local and untracked, a fresh clone has none of \
             them; probes {probes} programs tracked"
        )
    }
}

#[test]
fn the_aelys_corpus_produces_the_same_answer_at_every_opt_level() {
    let root = workspace_root();
    let probes = tracked_probes();
    let mut local = local_corpus(&root);
    let mut excluded = Vec::new();
    local.retain(|(name, _)| {
        let Some((_, why)) = NONDETERMINISTIC_BY_NATURE.iter().find(|(f, _)| *f == name) else {
            return true;
        };
        excluded.push(format!(
            "  excluded, its own output is not reproducible: {name}: {why}"
        ));
        false
    });
    eprintln!("{}", population_line(local.len(), probes.len()));
    for line in &excluded {
        eprintln!("{line}");
    }
    assert!(
        local.is_empty() || excluded.len() == NONDETERMINISTIC_BY_NATURE.len(),
        "the exclusion list names {} program(s) and {} of them were found in the corpus this run \
         swept; a name that matches no file excludes nothing and says nothing about excluding \
         nothing",
        NONDETERMINISTIC_BY_NATURE.len(),
        excluded.len()
    );
    assert!(
        probes.len() >= TRACKED_PROBE_FLOOR,
        "{} tracked probes, floor is {TRACKED_PROBE_FLOOR}. the local corpus is untracked, so the \
         probes are the half of this population a floor can be asserted on at all, and a sweep \
         over fewer of them is measuring less than it did",
        probes.len()
    );

    let tracked = probes.len();
    let mut population = probes;
    population.extend(local);
    let shapes = shape_census(&population);
    assert!(
        shapes.iter().all(|(_, count)| *count > 0),
        "the swept population cannot express the class it is swept for: {shapes:?}. a census over \
         a population with none of the shapes is an empty instrument reporting a clean world."
    );

    let cli = Cli::located();
    let scratch = tempdir().expect("tempdir");
    let control = scratch.path().join("control");
    fs::create_dir_all(&control).expect("control dir");
    eprintln!("{}", common::opt_level_reaches_the_artifact(&cli, &control));

    let swept = sweep_with_cli(&cli, scratch.path(), &population);

    let mut failures = Vec::new();
    let mut partials = Vec::new();
    let mut nonterminating = Vec::new();
    let mut nondeterministic = Vec::new();
    let mut undiagnosed = Vec::new();
    let mut link_faults = Vec::new();
    let mut no_linker = 0usize;
    let mut compared_tracked = 0usize;
    let mut compared_local = 0usize;

    let recheck = scratch.path().join("recheck");
    fs::create_dir_all(&recheck).expect("recheck dir");

    for (index, ((name, src), legs)) in population.iter().zip(&swept).enumerate() {
        let breakdown = leg_breakdown(legs);
        for (_, leg) in legs {
            if let Some(why) = common::refusal_is_not_a_verdict(leg) {
                undiagnosed.push(format!("  {name}: {why}"));
            }
        }
        if legs
            .iter()
            .any(|(_, l)| l.refusal().is_some_and(|(_, r)| linker_unavailable(r)))
        {
            no_linker += 1;
        } else if legs.iter().any(|(_, l)| common::refusal_is_a_link_fault(l)) {
            link_faults.push(format!("  {name}: {breakdown}"));
        }
        if legs.iter().any(|(_, l)| *l == Leg::CompilerDidNotFinish) {
            failures.push(format!(
                "  {name}: the compiler did not finish: {breakdown}"
            ));
            continue;
        }
        if legs.iter().any(|(_, l)| *l == Leg::DidNotTerminate) {
            nonterminating.push(format!("  {name}: {breakdown}"));
            continue;
        }
        let accepted = legs.iter().filter(|(_, l)| l.compiled()).count();
        if accepted == 0 {
            continue;
        }
        if accepted < legs.len() {
            partials.push(format!("  {name}: {breakdown}"));
            continue;
        }
        if index < tracked {
            compared_tracked += 1;
        } else {
            compared_local += 1;
        }
        let (_, first) = &legs[0];
        let Some((diverging, _)) = legs.iter().find(|(_, l)| l != first) else {
            continue;
        };
        if !disagreement_reproduces(&cli, &recheck, src, [legs[0].0, diverging]) {
            nondeterministic.push(name.clone());
            continue;
        }
        failures.push(format!(
            "  {name}: the levels that produced a runtime disagree: {breakdown}"
        ));
    }

    eprintln!(
        "corpus sweep: {} programs, {compared_tracked} tracked probes and {compared_local} local \
         corpus files compared at four levels, {} partially accepting, {} nonterminating, {} \
         nondeterministic, {} refused by a linker fault",
        population.len(),
        partials.len(),
        nonterminating.len(),
        nondeterministic.len(),
        link_faults.len()
    );
    for line in partials
        .iter()
        .chain(nonterminating.iter())
        .chain(link_faults.iter())
    {
        eprintln!("{line}");
    }
    for name in &nondeterministic {
        eprintln!("  nondeterministic, not compared: {name}");
    }
    if no_linker > 0 {
        common::require_linker_skip(
            "a corpus leg the linker never produced an artifact for compares no value",
        );
    }

    assert!(
        failures.is_empty(),
        "{} corpus files produce a different answer at different -O levels:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        undiagnosed.is_empty(),
        "{} refusal(s) are not a language verdict: a refusal is an exit 1 carrying a diagnostic, \
         and an ice, a signal or a silent failure counted as one is a compiler fault read as a \
         property of the program:\n{}",
        undiagnosed.len(),
        undiagnosed.join("\n")
    );
    assert!(
        link_faults.len() <= CORPUS_LINK_FAULT_BASELINE,
        "{} swept programs are refused by a linker fault, baseline is {}. a `cc` that failed is \
         the environment, not a verdict on the program:\n{}",
        link_faults.len(),
        CORPUS_LINK_FAULT_BASELINE,
        link_faults.join("\n")
    );
    assert!(
        nondeterministic.len() <= CORPUS_NONDETERMINISTIC_BASELINE,
        "{} swept programs disagreed across -O and would not reproduce the disagreement at the \
         two levels that produced it, baseline is {}. every one of them is a disagreement this \
         sweep threw away:\n  {}",
        nondeterministic.len(),
        CORPUS_NONDETERMINISTIC_BASELINE,
        nondeterministic.join("\n  ")
    );
    assert!(
        partials.len() <= CORPUS_PARTIAL_ACCEPT_BASELINE,
        "the partial-acceptance ratchet: {} swept programs change verdict across -O, baseline is \
         {}. the net refuses a split as E0432, so the only way one survives to here is a check \
         downstream of the compared stage; find it rather than raising the number:\n{}",
        partials.len(),
        CORPUS_PARTIAL_ACCEPT_BASELINE,
        partials.join("\n")
    );
    assert!(
        nonterminating.len() <= CORPUS_NONTERMINATING_BASELINE,
        "{} swept programs fail to terminate at some -O level, baseline is {}. each one is a \
         program this sweep compares nothing for:\n{}",
        nonterminating.len(),
        CORPUS_NONTERMINATING_BASELINE,
        nonterminating.join("\n")
    );
    assert!(
        compared_tracked >= TRACKED_COMPARED_FLOOR,
        "{compared_tracked} of {tracked} tracked probes were compared at four levels, floor is \
         {TRACKED_COMPARED_FLOOR}; the {compared_local} local corpus files compared alongside them \
         are a supplement that a fresh clone does not have, so a sweep that compares almost none \
         of the probes is green for a reason that has nothing to do with the compiler"
    );
}

// re-running a disagreement anywhere but at the two levels that produced it measures a third thing
fn disagreement_reproduces(cli: &Cli, dir: &Path, src: &str, levels: [&str; 2]) -> bool {
    levels
        .iter()
        .all(|level| cli.leg(dir, src, level) == cli.leg(dir, src, level))
}

#[test]
fn the_comparison_this_sweep_runs_tells_two_level_results_apart() {
    let mut blind = Vec::new();
    for (what, left, right) in common::fabricated_level_disagreements() {
        let shown = leg_breakdown(&[("-O0", left.clone()), ("-O1", right.clone())]);
        if left == right {
            blind.push(format!("  {what}: compared equal: {shown}"));
            continue;
        }
        if left.render() == right.render() {
            blind.push(format!("  {what}: rendered identically: {shown}"));
        }
    }
    assert!(
        blind.is_empty(),
        "{} fabricated pair(s) of level results are not told apart by the comparison every swept \
         program runs through, so a green sweep would say nothing about the compiler:\n{}",
        blind.len(),
        blind.join("\n")
    );
}

fn leg_breakdown(legs: &[(&'static str, Leg)]) -> String {
    legs.iter()
        .map(|(name, leg)| format!("{name}: {}", leg.render()))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn sweep_with_cli(
    cli: &Cli,
    scratch: &Path,
    population: &[(String, String)],
) -> Vec<Vec<(&'static str, Leg)>> {
    let next = AtomicUsize::new(0);
    let mut collected: Vec<(usize, Vec<(&'static str, Leg)>)> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for worker in 0..CORPUS_WORKERS {
            let dir = scratch.join(format!("w{worker}"));
            fs::create_dir_all(&dir).expect("worker scratch dir");
            let next = &next;
            handles.push(scope.spawn(move || {
                let mut mine = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= population.len() {
                        return mine;
                    }
                    mine.push((i, cli.legs(&dir, &population[i].1)));
                }
            }));
        }
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a corpus sweep worker panicked"))
            .collect()
    });
    collected.sort_by_key(|(i, _)| *i);
    collected.into_iter().map(|(_, legs)| legs).collect()
}

// every program this file tracks, as one population: the shapes the corpus has none of are here
fn tracked_probe_sources() -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for (id, src) in FIXTURES {
        out.push((format!("probe/curated/{id}"), src));
    }
    for (id, _, src) in PREOPT_FIXTURES {
        out.push((format!("probe/pre-opt/{id}"), src));
    }
    for (id, _, src) in S03A_ROWS {
        out.push((format!("probe/s03a/{id}"), src));
    }
    for (id, _, src) in S03B_ROWS {
        out.push((format!("probe/s03b/{id}"), src));
    }
    for (id, _, src) in S03C_ROWS {
        out.push((format!("probe/s03c/{id}"), src));
    }
    out.push((
        "probe/s03a/inline-recursive".to_string(),
        S03A_INLINE_RECURSIVE,
    ));
    out
}

// a plain `[` scan would count `vec[1, 2]` and report a population that carries the shape
fn has_non_literal_array_size(src: &str) -> bool {
    for (at, _) in src.match_indices(';') {
        let Some(open) = src[..at].rfind('[') else {
            continue;
        };
        let Some(close) = src[at..].find(']') else {
            continue;
        };
        let size = src[at + 1..at + close].trim();
        if src[open..at].contains(']') || size.is_empty() {
            continue;
        }
        if !size.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    false
}

fn shape_census(population: &[(String, String)]) -> Vec<(&'static str, usize)> {
    let mut sizes = 0usize;
    let mut inlines = 0usize;
    let mut counts = 0usize;
    for (_, src) in population {
        if has_non_literal_array_size(src) {
            sizes += 1;
        }
        if src.contains("@inline") {
            inlines += 1;
        }
        if src.contains("Rc::") {
            counts += 1;
        }
    }
    vec![
        ("non-literal array size", sizes),
        ("@inline", inlines),
        ("Rc::", counts),
    ]
}

// every level. every row below compiles at -o0/-o1/-o2/-o3, runs what it accepts, and fails the

#[derive(Clone, PartialEq, Eq)]
enum S03aVerdict {
    Refused(String),
    Ran {
        exit: Outcome,
        stdout: String,
        rc: String,
    },
}

impl S03aVerdict {
    fn render(&self) -> String {
        match self {
            S03aVerdict::Refused(rendered) => format!("refused: {}", s03a_first_line(rendered)),
            S03aVerdict::Ran { exit, stdout, rc } => {
                format!("ran {exit:?} stdout={stdout:?} rc={rc}")
            }
        }
    }
}

fn s03a_first_line(rendered: &str) -> String {
    rendered.lines().next().unwrap_or_default().to_string()
}

// the runtime prints its counters on stderr, and a row that touches memory has to read them
fn s03a_rc_line(stderr: &str) -> String {
    stderr
        .lines()
        .find(|line| line.starts_with("[rc] "))
        .unwrap_or("[rc] missing")
        .to_string()
}

fn s03a_run(exe: &Path) -> Option<(Outcome, String, String)> {
    let _pin = common::pin_legs("S0.3a run", 1);
    let out = Command::new(exe).env("AELYS_RC_STATS", "1").output().ok()?;
    common::note_leg();
    Some((
        common::outcome_of(&out.status),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn s03a_evaluate(h: &Harness, id: &str, src: &str) -> Option<Vec<(&'static str, S03aVerdict)>> {
    let mut results = Vec::with_capacity(LEVELS.len());
    for (name, opt) in LEVELS {
        let stem = slug(id, name);
        let path = h.dir.path().join(format!("{stem}.aelys"));
        fs::write(&path, src).expect("write fixture");
        match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
            Err(err) => {
                // the fixture name carries the level, so it has to leave the compared text
                let rendered = err.to_string().replace(&stem, "<fixture>");
                if linker_unavailable(&rendered) {
                    common::require_linker_skip(
                        "a skipped S0.3a row carries no runtime evidence at all",
                    );
                    return None;
                }
                results.push((name, S03aVerdict::Refused(rendered)));
            }
            Ok(()) => {
                let exe = exe_path_for(&path);
                let (exit, stdout, stderr) = s03a_run(&exe)?;
                results.push((
                    name,
                    S03aVerdict::Ran {
                        exit,
                        stdout,
                        rc: s03a_rc_line(&stderr),
                    },
                ));
            }
        }
    }
    Some(results)
}

// the canary: one disagreeing level fails the row, whatever the row expected
fn s03a_disagreement(id: &str, levels: &[(&'static str, S03aVerdict)]) -> Option<String> {
    let (_, first) = &levels[0];
    if levels.iter().all(|(_, v)| v == first) {
        return None;
    }
    let breakdown = levels
        .iter()
        .map(|(name, v)| format!("{name}: {}", v.render()))
        .collect::<Vec<_>>()
        .join("\n      ");
    Some(format!(
        "  {id}: the levels do not agree:\n      {breakdown}"
    ))
}

enum S03aExpect {
    // a refusal names its code and the words the user has to act on
    Refused(&'static str, &'static [&'static str]),
    Ran(i32, &'static str),
}

const S03A_ROWS: &[(&str, S03aExpect, &str)] = &[
    (
        "S0.3a-R1 repeat size is a local binding",
        S03aExpect::Refused(
            "E0902",
            &["unsupported non-constant array size", "compile-time constant"],
        ),
        r#"
fn main() -> i64 {
    let n = 4
    let a = [0; n]
    return a[0]
}
"#,
    ),
    (
        "S0.3a-R2 repeat size is a module global",
        S03aExpect::Refused(
            "E0902",
            &["unsupported non-constant array size", "compile-time constant"],
        ),
        r#"
let N: i64 = 4
fn main() -> i64 {
    let a = [0; N]
    return a[0]
}
"#,
    ),
    (
        "S0.3a-R3 repeat size is a call",
        S03aExpect::Refused(
            "E0902",
            &["unsupported non-constant array size", "compile-time constant"],
        ),
        r#"
fn four() -> i64 { return 4 }
fn main() -> i64 {
    let a = [0; four()]
    return a[0]
}
"#,
    ),
    (
        "S0.3a-R4 repeat size in expression position",
        S03aExpect::Refused(
            "E0902",
            &["unsupported non-constant array size", "compile-time constant"],
        ),
        r#"
fn main() -> i64 {
    let n = 4
    return [7; n][0]
}
"#,
    ),
    (
        "S0.3a-R5 file-scope array with a non-constant repeat size",
        S03aExpect::Refused(
            "E0902",
            &["file-scope let 'g' requires a compile-time constant initializer"],
        ),
        r#"
let N: i64 = 4
let g = [0; N]
fn main() -> i64 { return g[0] }
"#,
    ),
    (
        "S0.3a-R6 literal has more elements than the annotation",
        S03aExpect::Refused("E0301", &["[i64; 4]", "[i64; 2]"]),
        r#"
fn main() -> i64 {
    let a: [i64; 2] = [1, 2, 3, 4]
    return a[0]
}
"#,
    ),
    (
        "S0.3a-R7 repeat is longer than the annotation",
        S03aExpect::Refused("E0301", &["[i64; 4]", "[i64; 2]"]),
        r#"
fn main() -> i64 {
    let a: [i64; 2] = [7; 4]
    return a[0]
}
"#,
    ),
    (
        "S0.3a-R8 repeat is shorter than the annotation",
        S03aExpect::Refused("E0301", &["[i64; 2]", "[i64; 4]"]),
        r#"
fn main() -> i64 {
    let a: [i64; 4] = [7; 2]
    return a[3]
}
"#,
    ),
    (
        "S0.3a-C1 literal repeat size",
        S03aExpect::Ran(0, ""),
        r#"
fn main() -> i64 {
    let a = [0; 4]
    return a[0]
}
"#,
    ),
    (
        "S0.3a-C2 literal repeat size with a fill",
        S03aExpect::Ran(7, "28\n"),
        r#"
fn main() -> i64 {
    let a = [7; 4]
    println(a[0] + a[1] + a[2] + a[3])
    return a[3]
}
"#,
    ),
    (
        "S0.3a-C3 annotated length with a global repeat size",
        S03aExpect::Ran(0, ""),
        r#"
let N: i64 = 4
fn main() -> i64 {
    let a: [i64; 4] = [0; N]
    return a[0]
}
"#,
    ),
    (
        "S0.3a-C4 annotated length fills every slot",
        S03aExpect::Ran(7, "28\n"),
        r#"
let N: i64 = 4
fn main() -> i64 {
    let a: [i64; 4] = [7; N]
    println(a[0] + a[1] + a[2] + a[3])
    return a[3]
}
"#,
    ),
    (
        "S0.3a-C5 annotated length with a local repeat size",
        S03aExpect::Ran(7, "28\n"),
        r#"
fn main() -> i64 {
    let n = 4
    let a: [i64; 4] = [7; n]
    println(a[0] + a[1] + a[2] + a[3])
    return a[3]
}
"#,
    ),
    (
        "S0.3a-G1 closed arithmetic repeat size",
        S03aExpect::Ran(7, "28\n"),
        r#"
fn main() -> i64 {
    let a = [7; 2 + 2]
    println(a[0] + a[1] + a[2] + a[3])
    return a[3]
}
"#,
    ),
    (
        "S0.3a-G2 closed arithmetic file-scope initializer",
        S03aExpect::Ran(4, ""),
        r#"
let g: i64 = 2 + 2
fn main() -> i64 { return g }
"#,
    ),
    (
        "S0.3a-G3 parenthesised file-scope initializer",
        S03aExpect::Ran(7, ""),
        r#"
let g: i64 = (7)
fn main() -> i64 { return g }
"#,
    ),
    (
        "S0.3a-G4 cast file-scope initializer",
        S03aExpect::Ran(3, ""),
        r#"
let g: i64 = 3 as i64
fn main() -> i64 { return g }
"#,
    ),
    (
        "S0.3a-G5 unary file-scope initializer",
        S03aExpect::Ran(5, ""),
        r#"
let g: bool = not false
fn main() -> i64 {
    if g { return 5 }
    return 0
}
"#,
    ),
    (
        "S0.3a-K2 the inline analysis does not change what runs",
        S03aExpect::Ran(120, ""),
        S03A_INLINE_RECURSIVE,
    ),
];

const S03A_INLINE_RECURSIVE: &str = r#"
@inline
fn fact(n: i64) -> i64 {
    if n <= 1 { return 1 }
    return n * fact(n - 1)
}

fn main() -> i64 {
    return fact(5)
}
"#;

#[test]
fn s03a_array_length_and_inline_analysis_answer_the_same_at_every_level() {
    let h = Harness::new();
    let mut failures = Vec::new();
    let mut measured = 0usize;

    for (id, expect, src) in S03A_ROWS {
        let Some(levels) = s03a_evaluate(&h, id, src) else {
            common::require_linker_skip("a skipped S0.3a sweep compares no level to any other");
            continue;
        };
        measured += 1;

        if let Some(line) = s03a_disagreement(id, &levels) {
            failures.push(line);
            continue;
        }

        let (_, verdict) = &levels[0];
        match (expect, verdict) {
            (S03aExpect::Refused(code, fragments), S03aVerdict::Refused(rendered)) => {
                if !rendered.contains(&format!("[{code}]")) {
                    failures.push(format!(
                        "  {id}: refused without {code}:\n{rendered}"
                    ));
                }
                for fragment in *fragments {
                    if !rendered.contains(fragment) {
                        failures.push(format!(
                            "  {id}: the refusal never says {fragment:?}:\n{rendered}"
                        ));
                    }
                }
            }
            (S03aExpect::Ran(exit, stdout), S03aVerdict::Ran { exit: got, stdout: out, rc }) => {
                if *got != Outcome::Exit(*exit) || out != stdout {
                    failures.push(format!(
                        "  {id}: expected exit={exit} stdout={stdout:?}, got {got:?} stdout={out:?}"
                    ));
                }
                let want_rc = "[rc] allocs=0 frees=0";
                if *rc != want_rc {
                    failures.push(format!(
                        "  {id}: a stack array retains and frees nothing and a printed line takes \
                         the frame buffer, so nothing is allocated at all; expected {want_rc}, \
                         got {rc}"
                    ));
                }
            }
            (S03aExpect::Refused(code, _), S03aVerdict::Ran { exit, stdout, .. }) => {
                failures.push(format!(
                    "  {id}: expected {code} at every level, it ran: {exit:?} stdout={stdout:?}"
                ));
            }
            (S03aExpect::Ran(exit, _), S03aVerdict::Refused(rendered)) => {
                failures.push(format!(
                    "  {id}: expected exit={exit} at every level, it was refused:\n{rendered}"
                ));
            }
        }
    }

    assert_eq!(
        measured,
        S03A_ROWS.len(),
        "OOB-2: every S0.3a row must be measured; a sweep that compared nothing must fail here \
         rather than report a clean world"
    );
    assert!(
        failures.is_empty(),
        "{} S0.3a rows are wrong:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn s03a_the_canary_fires_on_the_pre_repair_measurement() {
    let recorded = [
        (
            "-O0",
            S03aVerdict::Refused(
                "error[E0902]: [air-lowering] 1. unsupported non-constant array size".to_string(),
            ),
        ),
        (
            "-O1",
            S03aVerdict::Ran {
                exit: Outcome::Signal(6),
                stdout: String::new(),
                rc: "[rc] allocs=0 frees=0".to_string(),
            },
        ),
        (
            "-O2",
            S03aVerdict::Ran {
                exit: Outcome::Signal(6),
                stdout: String::new(),
                rc: "[rc] allocs=0 frees=0".to_string(),
            },
        ),
        (
            "-O3",
            S03aVerdict::Ran {
                exit: Outcome::Signal(6),
                stdout: String::new(),
                rc: "[rc] allocs=0 frees=0".to_string(),
            },
        ),
    ];
    assert!(
        s03a_disagreement("S0.3a-R1", &recorded).is_some(),
        "`let n = 4; let a = [0; n]` was measured on binary 755468e9fa1ba796 as E0902 at -O0 and \
         rc=134 `index out of bounds` at -O1/-O2/-O3; the canary that catches it is the one the \
         row above runs, and a guard never made to fire is not a guard"
    );
}

#[test]
fn s03a_the_inline_analysis_warns_identically_at_every_level() {
    let h = Harness::new();
    let mut per_level = Vec::with_capacity(LEVELS.len());
    for (name, opt) in LEVELS {
        let path = h
            .dir
            .path()
            .join(format!("{}.aelys", slug("S0.3a-W", name)));
        fs::write(&path, S03A_INLINE_RECURSIVE).expect("write fixture");
        match compile_file_with_llvm_with_warnings(&path, opt, false, RuntimeVariant::Rc) {
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    common::require_linker_skip("a skipped warning row compares no level");
                    continue;
                }
                panic!("{name}: the inline fixture must compile, got:\n{err}");
            }
            Ok(warnings) => {
                let mut kinds: Vec<String> = warnings
                    .iter()
                    .map(|w| format!("{:?}", w.kind))
                    .collect();
                kinds.sort();
                per_level.push((name, kinds));
            }
        }
    }

    assert!(
        !per_level.is_empty(),
        "OOB-2: no level was measured, so the warning sets compared here are no sets at all"
    );
    let (_, baseline) = &per_level[0];
    assert!(
        !baseline.is_empty(),
        "the positive control must warn, or this row proves nothing: {per_level:?}"
    );
    for (name, kinds) in &per_level {
        assert_eq!(
            kinds, baseline,
            "{name} produced a different warning set than -O0: {per_level:?}"
        );
    }

    let werror_rejects: Vec<bool> = per_level
        .iter()
        .map(|(_, kinds)| !kinds.is_empty())
        .collect();
    assert!(
        werror_rejects.iter().all(|rejected| *rejected),
        "under -Werror every level must reject this program, got {werror_rejects:?}"
    );
}

// ---- s0.3b: the rc carrier surface is decided before the optimizer, and a generic instance

enum S03bExpect {
    // a refusal names its code and the words the user has to act on
    Refused(&'static str, &'static [&'static str]),
    Ran(i32, &'static str, &'static str),
}

const S03B_ROWS: &[(&str, S03bExpect, &str)] = &[
    (
        "S0.3b-K01 a carrier field from a call",
        S03bExpect::Refused(
            "E0410",
            &["[rc-stage3a]", "field `W.r`", "transfers its count or lends it"],
        ),
        r#"
struct W { r: Rc<i64> }
fn mkrc() -> Rc<i64> { return Rc::new(7) }
fn main() -> i64 {
    let w: W = W{ r: mkrc() }
    return Rc::get(w.r)
}
"#,
    ),
    (
        "S0.3b-K03 an enum payload from a call",
        S03bExpect::Refused(
            "E0410",
            &["[rc-stage3a]", "payload of `W::V`", "transfers its count or lends it"],
        ),
        r#"
enum W { V(Rc<i64>) }
fn mkrc() -> Rc<i64> { return Rc::new(7) }
fn main() -> i64 { let w: W = W::V(mkrc())
    return match w { W::V(r) => Rc::get(r) } }
"#,
    ),
    (
        "S0.3b-N1 the same call whose callee cannot be inlined away",
        S03bExpect::Refused(
            "E0410",
            &["[rc-stage3a]", "field `W.r`", "transfers its count or lends it"],
        ),
        r#"
struct W { r: Rc<i64> }
fn seven() -> i64 { return 7 }
fn mkrc() -> Rc<i64> { return Rc::new(seven()) }
fn main() -> i64 {
    let w: W = W{ r: mkrc() }
    return Rc::get(w.r)
}
"#,
    ),
    (
        "S0.3b-N2 a callee that lends its count instead of transferring it",
        S03bExpect::Refused(
            "E0410",
            &["[rc-stage3a]", "field `W.r`", "transfers its count or lends it"],
        ),
        r#"
struct W { r: Rc<i64> }
fn get(w: W) -> Rc<i64> { return w.r }
fn main() -> i64 {
    let a: W = W{ r: Rc::new(7) }
    let b: W = W{ r: get(a) }
    return Rc::get(b.r)
}
"#,
    ),
    (
        "S0.3b-N3 the same lend, with an allocation after it to reuse the slab",
        S03bExpect::Refused(
            "E0410",
            &["[rc-stage3a]", "field `W.r`", "transfers its count or lends it"],
        ),
        S03B_LENT_COUNT_THEN_REUSE,
    ),
    (
        "S0.3b-N4 a conditional whose branches disagree on provenance",
        S03bExpect::Refused(
            "E0410",
            &["[rc-stage3a]", "field `W.r`", "both one or both the other"],
        ),
        r#"
struct W { r: Rc<i64> }
fn pick() -> bool { return true }
fn main() -> i64 { let a: Rc<i64> = Rc::new(1)
    let c: bool = pick()
    let w: W = W{ r: if c { a } else { Rc::new(2) } }
    return Rc::get(w.r) }
"#,
    ),
    (
        "S0.3b-N5 two nested functions of one name still collide",
        S03bExpect::Refused("E0427", &["same symbol `dup`"]),
        r#"
fn outer() -> i64 {
    fn dup() -> i64 { return 1 }
    return dup()
}
fn other() -> i64 {
    fn dup() -> i64 { return 2 }
    return dup()
}
fn main() -> i64 { return outer() + other() }
"#,
    ),
    (
        "S0.3b-K02 a conditional whose branches both read a binding",
        S03bExpect::Ran(1, "", "[rc] allocs=2 frees=2"),
        r#"
struct W { r: Rc<i64> }
fn pick() -> bool { return true }
fn main() -> i64 { let a: Rc<i64> = Rc::new(1)
    let b: Rc<i64> = Rc::new(2)
    let c: bool = pick()
    let w: W = W{ r: if c { a } else { b } }
    return Rc::get(w.r) }
"#,
    ),
    (
        "S0.3b-D05 two generic instances that once shared one symbol",
        S03bExpect::Ran(1, "", "[rc] allocs=0 frees=0"),
        r#"
struct A_B { x: i64 }
struct B { x: i64 }
fn f<T>(v: T) -> i64 { return 1 }
fn f_A<T>(v: T) -> i64 { return 2 }
fn main() -> i64 { let p: A_B = A_B{x: 1}
    let r = f(p)
    if false { let q: B = B{x: 2}
        let s = f_A(q) }
    return r }
"#,
    ),
    (
        "S0.3b-G1 a conditional whose branches are both fresh",
        S03bExpect::Ran(1, "", "[rc] allocs=1 frees=1"),
        r#"
struct W { r: Rc<i64> }
fn pick() -> bool { return true }
fn main() -> i64 { let c: bool = pick()
    let w: W = W{ r: if c { Rc::new(1) } else { Rc::new(2) } }
    return Rc::get(w.r) }
"#,
    ),
    (
        "S0.3b-C1 a carrier field from a fresh literal",
        S03bExpect::Ran(7, "", "[rc] allocs=1 frees=1"),
        r#"
struct W { r: Rc<i64> }
fn main() -> i64 {
    let w: W = W{ r: Rc::new(7) }
    return Rc::get(w.r)
}
"#,
    ),
    (
        "S0.3b-C2 a carrier field cloned from a binding",
        S03bExpect::Ran(7, "", "[rc] allocs=1 frees=1"),
        r#"
struct W { r: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let w: W = W{ r: a }
    return Rc::get(w.r)
}
"#,
    ),
    (
        "S0.3b-C3 an enum payload cloned from a binding",
        S03bExpect::Ran(7, "", "[rc] allocs=1 frees=1"),
        r#"
enum W { V(Rc<i64>) }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let w: W = W::V(a)
    return match w { W::V(r) => Rc::get(r) }
}
"#,
    ),
    (
        "S0.3b-C4 a nested carrier literal",
        S03bExpect::Ran(7, "", "[rc] allocs=1 frees=1"),
        r#"
struct Inner { r: Rc<i64> }
struct Outer { i: Inner }
fn main() -> i64 {
    let o: Outer = Outer{ i: Inner{ r: Rc::new(7) } }
    return Rc::get(o.i.r)
}
"#,
    ),
];

// admitting the lend this program spells aborts in malloc: the cell is released twice
const S03B_LENT_COUNT_THEN_REUSE: &str = r#"
struct W { r: Rc<i64> }
fn get(w: W) -> Rc<i64> { return w.r }
fn boom() -> i64 {
    let a: W = W{ r: Rc::new(7) }
    let b: W = W{ r: get(a) }
    return Rc::get(b.r)
}
fn main() -> i64 {
    let x: i64 = boom()
    let p: Rc<i64> = Rc::new(1)
    let q: Rc<i64> = Rc::new(2)
    let r: Rc<i64> = Rc::new(3)
    return x + Rc::get(p) + Rc::get(q) + Rc::get(r)
}
"#;

#[test]
fn s03b_the_rc_carrier_surface_and_the_mono_symbol_answer_the_same_at_every_level() {
    let h = Harness::new();
    let mut failures = Vec::new();
    let mut measured = 0usize;

    for (id, expect, src) in S03B_ROWS {
        let Some(levels) = s03a_evaluate(&h, id, src) else {
            common::require_linker_skip("a skipped S0.3b sweep compares no level to any other");
            continue;
        };
        measured += 1;

        if let Some(line) = s03a_disagreement(id, &levels) {
            failures.push(line);
            continue;
        }

        let (_, verdict) = &levels[0];
        match (expect, verdict) {
            (S03bExpect::Refused(code, fragments), S03aVerdict::Refused(rendered)) => {
                if !rendered.contains(&format!("[{code}]")) {
                    failures.push(format!("  {id}: refused without {code}:\n{rendered}"));
                }
                for fragment in *fragments {
                    if !rendered.contains(fragment) {
                        failures.push(format!(
                            "  {id}: the refusal never says {fragment:?}:\n{rendered}"
                        ));
                    }
                }
            }
            (
                S03bExpect::Ran(exit, stdout, rc),
                S03aVerdict::Ran {
                    exit: got,
                    stdout: out,
                    rc: got_rc,
                },
            ) => {
                if *got != Outcome::Exit(*exit) || out != stdout {
                    failures.push(format!(
                        "  {id}: expected exit={exit} stdout={stdout:?}, got {got:?} stdout={out:?}"
                    ));
                }
                if got_rc != rc {
                    failures.push(format!(
                        "  {id}: expected {rc}, got {got_rc}; an unbalanced count is the defect \
                         this row exists to catch"
                    ));
                }
            }
            (S03bExpect::Refused(code, _), S03aVerdict::Ran { exit, stdout, rc }) => {
                failures.push(format!(
                    "  {id}: expected {code} at every level, it ran: {exit:?} stdout={stdout:?} {rc}"
                ));
            }
            (S03bExpect::Ran(exit, _, _), S03aVerdict::Refused(rendered)) => {
                failures.push(format!(
                    "  {id}: expected exit={exit} at every level, it was refused:\n{rendered}"
                ));
            }
        }
    }

    assert_eq!(
        measured,
        S03B_ROWS.len(),
        "OOB-2: every S0.3b row must be measured; a sweep that compared nothing must fail here \
         rather than report a clean world"
    );
    assert!(
        failures.is_empty(),
        "{} S0.3b rows are wrong:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn s03b_the_canary_fires_on_the_pre_repair_measurement() {
    let k01 = [
        (
            "-O0",
            S03aVerdict::Refused(
                "error[E0902]: [air-lowering] 1. [rc-stage3a] field `W.r` is initialized from a \
                 call returning an `Rc<T>`-bearing value"
                    .to_string(),
            ),
        ),
        (
            "-O1",
            S03aVerdict::Ran {
                exit: Outcome::Exit(7),
                stdout: String::new(),
                rc: "[rc] allocs=1 frees=1".to_string(),
            },
        ),
        (
            "-O2",
            S03aVerdict::Ran {
                exit: Outcome::Exit(7),
                stdout: String::new(),
                rc: "[rc] allocs=1 frees=1".to_string(),
            },
        ),
        (
            "-O3",
            S03aVerdict::Ran {
                exit: Outcome::Exit(7),
                stdout: String::new(),
                rc: "[rc] allocs=1 frees=1".to_string(),
            },
        ),
    ];
    assert!(
        s03a_disagreement("S0.3b-K01", &k01).is_some(),
        "`W{{ r: mkrc() }}` was measured on binary 95094c01364a3a20 as E0902 at -O0 and exit 7 \
         with allocs=1 frees=1 at -O1/-O2/-O3; the canary that catches it is the one the rows \
         above run, and a guard never made to fire is not a guard"
    );

    let d05 = [
        (
            "-O0",
            S03aVerdict::Refused(
                "error[E0427]: [symbol] two functions compile to the same symbol `__mono_f_A_B`"
                    .to_string(),
            ),
        ),
        (
            "-O1",
            S03aVerdict::Refused(
                "error[E0427]: [symbol] two functions compile to the same symbol `__mono_f_A_B`"
                    .to_string(),
            ),
        ),
        (
            "-O2",
            S03aVerdict::Ran {
                exit: Outcome::Exit(1),
                stdout: String::new(),
                rc: "[rc] allocs=0 frees=0".to_string(),
            },
        ),
        (
            "-O3",
            S03aVerdict::Ran {
                exit: Outcome::Exit(1),
                stdout: String::new(),
                rc: "[rc] allocs=0 frees=0".to_string(),
            },
        ),
    ];
    assert!(
        s03a_disagreement("S0.3b-D05", &d05).is_some(),
        "`f<A_B>` and `f_A<B>` were measured on binary 95094c01364a3a20 as E0427 at -O0/-O1 and \
         exit 1 at -O2/-O3, because dead-code elimination removed one instantiation before the \
         symbol check ran"
    );
}

// ---- s0.3c: the net. above -o0 the compared stage runs on the program the user wrote as well

enum S03cExpect {
    Runs(i32, &'static str, &'static str),
    // the -o0 rendering, byte for byte, at all four levels: a re-faulted twin would not match
    Unchanged(&'static str, &'static [&'static str]),
    // the two sides part company from some level up: the code per level, then what E0432 must say
    Split([&'static str; 4], &'static [&'static str]),
}

const S03C_ARRAY_SIZE_IS_A_BINDING: &str = r#"
fn main() -> i64 {
    let n = 4
    let a = [0; n]
    return a[0]
}
"#;

const S03C_ROWS: &[(&str, S03cExpect, &str)] = &[
    (
        "S0.3c-A1 both sides accept a program the optimizer rewrites",
        S03cExpect::Runs(14, "8\n9\n", "[rc] allocs=0 frees=0"),
        r#"
fn double(x: i64) -> i64 { return x + x }
fn side() -> i64 {
    println(9)
    return 3
}
fn main() -> i64 {
    let k = double(2 + 2)
    println(k)
    return double(side()) + k
}
"#,
    ),
    (
        "S0.3c-A2 both sides accept and a count moves into a carrier",
        S03cExpect::Runs(13, "", "[rc] allocs=4 frees=4"),
        S03C_MOVED_COUNT_THEN_REUSE,
    ),
    (
        "S0.3c-B1 both sides refuse a non-constant array size",
        S03cExpect::Unchanged("E0902", &["unsupported non-constant array size"]),
        S03C_ARRAY_SIZE_IS_A_BINDING,
    ),
    (
        "S0.3c-B2 both sides refuse the break guard no enumerated row names",
        S03cExpect::Unchanged(
            "E0902",
            &["an Rc<T> live in a loop body is abandoned by `break`"],
        ),
        r#"
fn main() -> i64 {
    let mut i = 0
    while i < 3 {
        let r: Rc<i64> = Rc::new(7)
        if i == 1 { break }
        i = i + 1
    }
    return 0
}
"#,
    ),
    (
        "S0.3c-C1 dce removes the instantiation the vec surface check would reject",
        S03cExpect::Split(
            ["E0412", "E0412", "E0432", "E0432"],
            &[
                // the actionable refusal leads, in full, and the level split follows it
                "error[E0432]: [E0412] [vec-surface] a temporary of `main`",
                "which holds a `Vec<T>` by value inside another container",
                "that refusal is raised at -O0 and not at -O",
                "a verdict belongs to the language and not to the optimization level",
                "read the refusal above first",
            ],
        ),
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
];

const S03C_MOVED_COUNT_THEN_REUSE: &str = r#"
struct W { r: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let w: W = W{ r: a }
    let x: i64 = Rc::get(w.r)
    let p: Rc<i64> = Rc::new(1)
    let q: Rc<i64> = Rc::new(2)
    let s: Rc<i64> = Rc::new(3)
    return x + Rc::get(p) + Rc::get(q) + Rc::get(s)
}
"#;

const S03B_K02_THEN_REUSE: &str = r#"
struct W { r: Rc<i64> }
fn pick() -> bool { return true }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(1)
    let b: Rc<i64> = Rc::new(2)
    let c: bool = pick()
    let w: W = W{ r: if c { a } else { b } }
    let x: i64 = Rc::get(w.r)
    let p: Rc<i64> = Rc::new(10)
    let q: Rc<i64> = Rc::new(20)
    let s: Rc<i64> = Rc::new(30)
    return x + Rc::get(p) + Rc::get(q) + Rc::get(s)
}
"#;

const S03B_G1_THEN_REUSE: &str = r#"
struct W { r: Rc<i64> }
fn pick() -> bool { return true }
fn main() -> i64 {
    let c: bool = pick()
    let w: W = W{ r: if c { Rc::new(1) } else { Rc::new(2) } }
    let x: i64 = Rc::get(w.r)
    let p: Rc<i64> = Rc::new(10)
    let q: Rc<i64> = Rc::new(20)
    let s: Rc<i64> = Rc::new(30)
    return x + Rc::get(p) + Rc::get(q) + Rc::get(s)
}
"#;

const S03C_REUSE_ROWS: &[(&str, &str, i32, &str)] = &[
    (
        "S0.3c-A2m a moved count",
        S03C_MOVED_COUNT_THEN_REUSE,
        13,
        "[rc] allocs=4 frees=4",
    ),
    (
        "S0.3b-K02 a conditional whose branches both read a binding",
        S03B_K02_THEN_REUSE,
        61,
        "[rc] allocs=5 frees=5",
    ),
    (
        "S0.3b-G1 a conditional whose branches are both fresh",
        S03B_G1_THEN_REUSE,
        61,
        "[rc] allocs=4 frees=4",
    ),
];

fn s03c_code(verdict: &S03aVerdict) -> Option<String> {
    let S03aVerdict::Refused(rendered) = verdict else {
        return None;
    };
    let at = rendered.find("error[")? + "error[".len();
    let end = at + rendered[at..].find(']')?;
    Some(rendered[at..end].to_string())
}

#[test]
fn s03c_the_net_answers_the_same_way_at_every_level() {
    let h = Harness::new();
    let mut failures = Vec::new();
    let mut measured = 0usize;

    for (id, expect, src) in S03C_ROWS {
        let Some(levels) = s03a_evaluate(&h, id, src) else {
            common::require_linker_skip("a skipped S0.3c row compares no level to any other");
            continue;
        };
        measured += 1;

        match expect {
            S03cExpect::Runs(exit, stdout, rc) => {
                if let Some(line) = s03a_disagreement(id, &levels) {
                    failures.push(line);
                    continue;
                }
                match &levels[0].1 {
                    S03aVerdict::Ran {
                        exit: got,
                        stdout: out,
                        rc: got_rc,
                    } => {
                        if *got != Outcome::Exit(*exit) || out != stdout || got_rc != rc {
                            failures.push(format!(
                                "  {id}: expected exit={exit} stdout={stdout:?} {rc}, got \
                                 {got:?} stdout={out:?} {got_rc}"
                            ));
                        }
                    }
                    S03aVerdict::Refused(rendered) => failures.push(format!(
                        "  {id}: expected exit={exit} at every level, it was refused:\n{rendered}"
                    )),
                }
            }
            S03cExpect::Unchanged(code, fragments) => {
                if let Some(line) = s03a_disagreement(id, &levels) {
                    failures.push(line);
                    continue;
                }
                match &levels[0].1 {
                    S03aVerdict::Refused(rendered) => {
                        if !rendered.contains(&format!("[{code}]")) {
                            failures.push(format!("  {id}: refused without {code}:\n{rendered}"));
                        }
                        for fragment in *fragments {
                            if !rendered.contains(fragment) {
                                failures.push(format!(
                                    "  {id}: the refusal never says {fragment:?}:\n{rendered}"
                                ));
                            }
                        }
                    }
                    S03aVerdict::Ran { exit, stdout, .. } => failures.push(format!(
                        "  {id}: expected {code} at every level, it ran: {exit:?} stdout={stdout:?}"
                    )),
                }
            }
            S03cExpect::Split(codes, fragments) => {
                let got: Vec<String> = levels
                    .iter()
                    .map(|(name, verdict)| {
                        format!("{name}={}", s03c_code(verdict).unwrap_or("accept".into()))
                    })
                    .collect();
                let want: Vec<String> = LEVELS
                    .iter()
                    .zip(codes.iter())
                    .map(|((name, _), code)| format!("{name}={code}"))
                    .collect();
                if got != want {
                    failures.push(format!(
                        "  {id}: expected {want:?}, got {got:?}. an accept on either side of a \
                         split is the net not running"
                    ));
                    continue;
                }
                for (name, verdict) in &levels {
                    let S03aVerdict::Refused(rendered) = verdict else {
                        continue;
                    };
                    if !rendered.contains("[E0432]") {
                        continue;
                    }
                    for fragment in *fragments {
                        if !rendered.contains(fragment) {
                            failures.push(format!(
                                "  {id} at {name}: E0432 never says {fragment:?}:\n{rendered}"
                            ));
                        }
                    }
                }
            }
        }
    }

    assert_eq!(
        measured,
        S03C_ROWS.len(),
        "OOB-2: every S0.3c row must be measured; a sweep that compared nothing must fail here \
         rather than report a clean world"
    );
    assert!(
        failures.is_empty(),
        "{} S0.3c rows are wrong:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn s03c_a_refusal_both_sides_reach_is_the_unoptimised_one_byte_for_byte() {
    let h = Harness::new();
    let mut measured = 0usize;
    for (id, expect, src) in S03C_ROWS {
        if !matches!(expect, S03cExpect::Unchanged(_, _)) {
            continue;
        }
        let Some(levels) = s03a_evaluate(&h, id, src) else {
            common::require_linker_skip("a skipped row compares no rendering to any other");
            continue;
        };
        measured += 1;
        let S03aVerdict::Refused(baseline) = &levels[0].1 else {
            panic!("{id}: -O0 must refuse this program");
        };
        for (name, verdict) in &levels[1..] {
            let S03aVerdict::Refused(rendered) = verdict else {
                panic!("{id} at {name}: every level must refuse this program");
            };
            assert_eq!(
                rendered, baseline,
                "{id} at {name}: when both sides refuse, the diagnostic emitted is the \
                 unoptimised one unchanged. a re-faulted or re-anchored twin differs here, and \
                 re-faulting is what group_surface_tests forbids."
            );
        }
    }
    assert!(
        measured > 0,
        "OOB-2: this row compared no rendering at all, which is the shape where a skip reads as a \
         pass. AELYS_ALLOW_LINKER_SKIP excuses a leg, never a whole comparison."
    );
}

fn s03c_run_under(exe: &Path, allocator: &str) -> Option<(Outcome, String, String)> {
    let _pin = common::pin_legs("S0.3c allocator run", 1);
    let out = Command::new(exe)
        .env("AELYS_RC_STATS", "1")
        .env("AELYS_ALLOC", allocator)
        .output()
        .ok()?;
    common::note_leg();
    Some((
        common::outcome_of(&out.status),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        s03a_rc_line(&String::from_utf8_lossy(&out.stderr)),
    ))
}

// aelys_core_asan arms the archive at build time, so a test can only take the reuse half
#[test]
fn s03c_a_moved_count_survives_a_reused_slab_at_every_level() {
    let h = Harness::new();
    let mut measured = 0usize;
    for (id, src, want_exit, want_rc) in S03C_REUSE_ROWS {
        for (name, opt) in LEVELS {
            let path = h.dir.path().join(format!("{}.aelys", slug(id, name)));
            fs::write(&path, src).expect("write fixture");
            match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
                Err(err) => {
                    if linker_unavailable(&err.to_string()) {
                        common::require_linker_skip("a skipped allocator row runs no artifact");
                        continue;
                    }
                    panic!("{id} at {name}: the reuse fixture must compile, got:\n{err}");
                }
                Ok(()) => {
                    let exe = exe_path_for(&path);
                    for allocator in ["immix", "malloc"] {
                        let Some((exit, stdout, rc)) = s03c_run_under(&exe, allocator) else {
                            panic!("{id} at {name}: the artifact must run under AELYS_ALLOC={allocator}");
                        };
                        measured += 1;
                        assert_eq!(
                            (exit, stdout.as_str(), rc.as_str()),
                            (Outcome::Exit(*want_exit), "", *want_rc),
                            "{id} at {name} under AELYS_ALLOC={allocator}: a balanced counter is \
                             not a safety oracle, so the same row is read again with the slab \
                             reused; a release of an already freed cell keeps the count balanced \
                             and aborts here"
                        );
                    }
                }
            }
        }
    }
    assert!(
        measured > 0,
        "OOB-2: no artifact was run at all, so neither allocator was an oracle for anything"
    );
    assert_eq!(
        measured % (S03C_REUSE_ROWS.len() * LEVELS.len() * 2),
        0,
        "OOB-2: {measured} runs is not every row at every level under both allocators, so at \
         least one ownership move was witnessed by the balanced counter alone"
    );
}

#[test]
fn s03c_the_ablation_puts_the_array_length_back_on_the_optimizer_and_the_net_fires() {
    let h = Harness::new();
    aelys_air::ablation::set_array_length_from_size_expr(true);
    let levels = s03a_evaluate(&h, "S0.3c-D1 ablated", S03C_ARRAY_SIZE_IS_A_BINDING);
    aelys_air::ablation::set_array_length_from_size_expr(false);

    let Some(levels) = levels else {
        common::require_linker_skip("a skipped ablation row proves the net fires on nothing");
        panic!(
            "OOB-2: the ablation compared nothing. a guard that has not been made to fire is not \
             a guard, and a skipped ablation is exactly that."
        );
    };

    let got: Vec<String> = levels
        .iter()
        .map(|(name, verdict)| format!("{name}={}", s03c_code(verdict).unwrap_or("accept".into())))
        .collect();
    assert_eq!(
        got,
        vec!["-O0=E0902", "-O1=E0432", "-O2=E0432", "-O3=E0432"],
        "with the array length read back off the size expression the optimizer rewrites, -O0 \
         refuses `[0; n]` and every level above it folds `n` and accepts. the net has to refuse \
         that split rather than pick a side."
    );

    let (_, split) = &levels[2];
    let S03aVerdict::Refused(rendered) = split else {
        panic!("-O2 must refuse the ablated program");
    };
    for fragment in [
        // the refusal leads and the split follows, or the user is told to file a bug for a fixable program
        "error[E0432]: [E0902]",
        "unsupported non-constant array size",
        "that refusal is raised at -O0 and not at -O2",
    ] {
        assert!(
            rendered.contains(fragment),
            "E0432 must say {fragment:?}, got:\n{rendered}"
        );
    }

    let unablated = s03a_evaluate(&h, "S0.3c-D1 repaired", S03C_ARRAY_SIZE_IS_A_BINDING)
        .expect("the repaired compiler must answer");
    let repaired: Vec<String> = unablated
        .iter()
        .map(|(name, verdict)| format!("{name}={}", s03c_code(verdict).unwrap_or("accept".into())))
        .collect();
    assert_eq!(
        repaired,
        vec!["-O0=E0902", "-O1=E0902", "-O2=E0902", "-O3=E0902"],
        "the positive control: with the seam off the same program is refused identically, so the \
         E0432 above is the ablation and not the row"
    );
}

#[test]
fn s03c_the_swept_population_can_express_the_class_the_corpus_cannot() {
    let root = workspace_root();
    let corpus = local_corpus(&root);
    let corpus_shapes = shape_census(&corpus);
    assert!(
        corpus_shapes.iter().all(|(_, count)| *count == 0),
        "the corpus census moved: {corpus_shapes:?} over {} files. it was measured at zero for \
         all three shapes, which is why the probe set is part of the swept population; a corpus \
         that has grown one of them can carry that half of the obligation and this row has to be \
         re-derived rather than left claiming an empty corpus.",
        corpus.len()
    );

    let probes: Vec<(String, String)> = tracked_probe_sources()
        .into_iter()
        .map(|(name, src)| (name, src.to_string()))
        .collect();
    let probe_shapes = shape_census(&probes);
    for (shape, count) in &probe_shapes {
        assert!(
            *count > 0,
            "no tracked probe carries {shape:?}, so the population swept for a level-dependent \
             verdict cannot produce one. that is the defect the corpus row had: an instrument \
             that reports clean because it holds nothing to be dirty about. probe census: \
             {probe_shapes:?} over {} programs",
            probes.len()
        );
    }
    eprintln!(
        "population: corpus {} files {corpus_shapes:?}; probes {} programs {probe_shapes:?}",
        corpus.len(),
        probes.len()
    );
}
