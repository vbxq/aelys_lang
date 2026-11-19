use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

mod common;
use common::{Cli, Leg};

const POPULATION_FLOOR: usize = 1800;
// 891 compiled before the include root reached the legs, so a floor of 900 is what reds if std and the prelude stop resolving again
const COMPILES_EVERYWHERE_FLOOR: usize = 900;

// the two programs that carried this baseline are excluded by origin below, so a swept program that stops terminating here is a new one
const NONTERMINATING_BASELINE: usize = 0;

// a tail call turns stack exhaustion at -o0 into a loop at -o1..3, so these two diverge across levels
const DIVERGES_BEYOND_THIS_COMPARISON: [(&str, &str); 2] = [
    (
        "aelys/tests/air_lower_tests.rs:291",
        "a top level function value that calls itself through two aliases, signal 11 at -O0 \
         against an endless loop at -O1..3",
    ),
    (
        "aelys/tests/llvm_global_tests.rs:108",
        "the same shape through one alias, signal 11 at -O0 against an endless loop at -O1..3",
    ),
];

// the extractor reads this file too, so a population that has stopped carrying this program has stopped reading the tree
const EXTRACTOR_SELF_CHECK: &str = "\
fn value_net_extractor_self_check() -> i64 {
    return 0
}

fn main() -> i64 {
    return value_net_extractor_self_check()
}
";

// every one of these declares a c symbol the net links nothing against, so `cc` failing is the population and not a verdict on the program
const LINK_FAULT_RATCHET: &[(&str, usize)] = &[
    ("aelys/tests/ffi_stage1_c_abi_tests.rs", 2),
    ("aelys/tests/ffi_stage1_effects_tests.rs", 1),
    ("aelys/tests/ffi_stage4_abi_tests.rs", 1),
    ("aelys/tests/ffi_stage4_link_tests.rs", 3),
    ("aelys/tests/group_surface_tests.rs", 1),
    ("aelys/tests/s4c_sweep_fixes_tests.rs", 1),
    ("aelys/tests/semantic_invariants_tests.rs", 2),
    ("aelys/tests/v01b_stage1_string_header_tests.rs", 4),
];

const WORKERS: usize = 4;

struct Program {
    origin: String,
    src: String,
}

fn repo_root() -> PathBuf {
    common::repo_root_from_manifest()
}

fn tracked_test_files(root: &Path) -> Vec<PathBuf> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--", "aelys/tests", "cli/tests", "driver/tests"])
        .output()
        .expect("run `git ls-files`");
    assert!(
        out.status.success(),
        "`git ls-files` exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let listing = String::from_utf8_lossy(&out.stdout);
    let files: Vec<PathBuf> = listing
        .lines()
        .filter(|line| line.ends_with(".rs"))
        .map(|line| root.join(line))
        .collect();
    assert!(
        files.len() > 100,
        "the population is derived from the tracked test sources and `git ls-files` returned {} \
         of them; a population nobody measured is a list in disguise",
        files.len()
    );
    files
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn decode_rust_escapes(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&raw[start..i]);
            continue;
        }
        i += 1;
        let escape = *bytes.get(i)?;
        i += 1;
        match escape {
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'0' => out.push('\0'),
            b'\\' => out.push('\\'),
            b'\'' => out.push('\''),
            b'"' => out.push('"'),
            b'x' => {
                let hex = raw.get(i..i + 2)?;
                out.push(char::from(u8::from_str_radix(hex, 16).ok()?));
                i += 2;
            }
            b'u' => {
                if *bytes.get(i)? != b'{' {
                    return None;
                }
                let close = raw[i..].find('}')? + i;
                let value =
                    u32::from_str_radix(raw[i + 1..close].replace('_', "").as_str(), 16).ok()?;
                out.push(char::from_u32(value)?);
                i = close + 1;
            }
            // a backslash before a newline eats the newline and the indentation that follows it
            b'\n' => {
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

fn rust_string_literals(src: &str) -> Vec<(usize, String)> {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = bytes[i];
        if c == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // `'a'` closes on a quote and `'static` never does, so reading a lifetime as a string start would swallow the rest of the file
        if c == b'\'' {
            if i + 1 < n && bytes[i + 1] == b'\\' {
                let mut j = i + 2;
                while j < n && bytes[j] != b'\'' {
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            if i + 2 < n && bytes[i + 2] == b'\'' {
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            while i < n && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let word = &src[start..i];
            if (word == "r" || word == "br") && i < n && (bytes[i] == b'#' || bytes[i] == b'"') {
                let mut j = i;
                let mut hashes = 0usize;
                while j < n && bytes[j] == b'#' {
                    hashes += 1;
                    j += 1;
                }
                if j < n && bytes[j] == b'"' {
                    let mut terminator = String::with_capacity(hashes + 1);
                    terminator.push('"');
                    for _ in 0..hashes {
                        terminator.push('#');
                    }
                    if let Some(rel) = src[j + 1..].find(&terminator) {
                        let end = j + 1 + rel;
                        out.push((j + 1, src[j + 1..end].to_string()));
                        i = end + terminator.len();
                        continue;
                    }
                }
            }
            continue;
        }
        if c == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < n {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'"' {
                    break;
                }
                j += 1;
            }
            let end = j.min(n);
            if let Some(text) = decode_rust_escapes(&src[start..end]) {
                out.push((start, text));
            }
            i = end + 1;
            continue;
        }
        i += 1;
    }
    out
}

fn line_of(src: &str, offset: usize) -> usize {
    src[..offset.min(src.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

fn population(root: &Path) -> Vec<Program> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut files = tracked_test_files(root);
    files.sort();
    for file in files {
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (offset, text) in rust_string_literals(&src) {
            if !text.contains("fn main(") || !seen.insert(text.clone()) {
                continue;
            }
            out.push(Program {
                origin: format!("{rel}:{}", line_of(&src, offset)),
                src: text,
            });
        }
    }
    out
}

fn breakdown(legs: &[(&'static str, Leg)]) -> String {
    legs.iter()
        .map(|(name, leg)| format!("{name}: {}", leg.render()))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn sweep(cli: &Cli, programs: &[Program]) -> Vec<Vec<(&'static str, Leg)>> {
    let scratch = tempdir().expect("tempdir");
    let next = AtomicUsize::new(0);
    let mut collected: Vec<(usize, Vec<(&'static str, Leg)>)> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for worker in 0..WORKERS {
            let dir = scratch.path().join(format!("w{worker}"));
            std::fs::create_dir_all(&dir).expect("worker scratch dir");
            let next = &next;
            handles.push(scope.spawn(move || {
                let mut mine = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= programs.len() {
                        return mine;
                    }
                    mine.push((i, cli.legs(&dir, &programs[i].src)));
                }
            }));
        }
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a sweep worker panicked"))
            .collect()
    });
    collected.sort_by_key(|(i, _)| *i);
    collected.into_iter().map(|(_, legs)| legs).collect()
}

#[test]
fn every_tracked_aelys_program_answers_the_same_at_every_opt_level() {
    let sampled = std::env::var("AELYS_VALUE_NET_LIMIT").ok().map(|raw| {
        raw.parse::<usize>()
            .expect("AELYS_VALUE_NET_LIMIT is a number")
    });

    let root = repo_root();
    let mut programs = population(&root);

    let mut excluded = Vec::new();
    programs.retain(|program| {
        let Some((_, why)) = DIVERGES_BEYOND_THIS_COMPARISON
            .iter()
            .find(|(origin, _)| *origin == program.origin)
        else {
            return true;
        };
        excluded.push(format!(
            "  excluded, it diverges and this net cannot compare the two sides: {}: {why}",
            program.origin
        ));
        false
    });
    for line in &excluded {
        eprintln!("{line}");
    }
    assert_eq!(
        excluded.len(),
        DIVERGES_BEYOND_THIS_COMPARISON.len(),
        "the exclusion list names {} program(s) by origin and the population carries {} of them; \
         an origin is a file and a line, so a literal that moved excludes nothing, and re-deriving \
         the line is the fix rather than dropping the entry",
        DIVERGES_BEYOND_THIS_COMPARISON.len(),
        excluded.len()
    );

    for (what, program) in [
        ("the self check", EXTRACTOR_SELF_CHECK),
        (
            "the program the artifact control compiles",
            common::OPTIMIZER_WITNESS,
        ),
    ] {
        let carriers = programs.iter().filter(|p| p.src == program).count();
        assert_eq!(
            carriers, 1,
            "{what} lives in a tracked literal and the extractor found it {carriers} time(s); an \
             extractor that stopped seeing its own program is measuring nothing"
        );
    }

    if let Some(limit) = sampled {
        eprintln!(
            "AELYS_VALUE_NET_LIMIT={limit}: SAMPLE, NOT THE NET. {} programs extracted, {limit} measured.",
            programs.len()
        );
        programs.truncate(limit);
    } else {
        assert!(
            programs.len() >= POPULATION_FLOOR,
            "{} programs extracted from the tracked test sources, floor is {POPULATION_FLOOR}. \
             the population is derived at test time, so a drop means the extractor stopped \
             extracting, and a net over nothing is green for a false reason",
            programs.len()
        );
    }

    let cli = Cli::located();
    let control = tempdir().expect("tempdir");
    eprintln!(
        "{}",
        common::opt_level_reaches_the_artifact(&cli, control.path())
    );

    let started = std::time::Instant::now();
    let swept = sweep(&cli, &programs);
    let elapsed = started.elapsed();

    let mut rejected_everywhere = 0usize;
    let mut compiles_everywhere = 0usize;
    let mut split = Vec::new();
    let mut nonterminating = Vec::new();
    let mut stuck_compiler = Vec::new();
    let mut disagreements = Vec::new();
    let mut undiagnosed = Vec::new();
    let mut link_faults = Vec::new();
    let mut no_linker = 0usize;

    for (program, legs) in programs.iter().zip(&swept) {
        let shown = breakdown(legs);
        for (_, leg) in legs {
            if let Some(why) = common::refusal_is_not_a_verdict(leg) {
                undiagnosed.push(format!("  {}: {why}", program.origin));
            }
        }
        if legs
            .iter()
            .any(|(_, l)| l.refusal().is_some_and(|(_, r)| common::linker_unavailable(r)))
        {
            no_linker += 1;
        } else if legs.iter().any(|(_, l)| common::refusal_is_a_link_fault(l)) {
            link_faults.push((program.origin.clone(), shown.clone()));
        }
        if legs.iter().any(|(_, l)| *l == Leg::CompilerDidNotFinish) {
            stuck_compiler.push(format!("  {}: {shown}", program.origin));
            continue;
        }
        if legs.iter().any(|(_, l)| *l == Leg::DidNotTerminate) {
            nonterminating.push(format!("  {}: {shown}", program.origin));
            continue;
        }
        let compiled = legs.iter().filter(|(_, l)| l.compiled()).count();
        if compiled == 0 {
            rejected_everywhere += 1;
            continue;
        }
        if compiled < legs.len() {
            split.push(format!("  {}: {shown}", program.origin));
            continue;
        }
        compiles_everywhere += 1;
        let (_, first) = &legs[0];
        if legs.iter().all(|(_, l)| l == first) {
            continue;
        }
        disagreements.push(format!("  {}: {shown}", program.origin));
    }

    eprintln!(
        "value net: {} programs, {compiles_everywhere} compile at every level, \
         {rejected_everywhere} at none, {} split, {} nonterminating, {} excluded, in {:.1}s",
        programs.len(),
        split.len(),
        nonterminating.len(),
        excluded.len(),
        elapsed.as_secs_f64()
    );
    for line in &nonterminating {
        eprintln!("{line}");
    }
    for (origin, shown) in &link_faults {
        eprintln!("  refused by a linker fault, not compared: {origin}: {shown}");
    }
    if no_linker > 0 {
        common::require_linker_skip(
            "programs the linker never produced an artifact for carry no value to compare",
        );
    }

    assert!(
        undiagnosed.is_empty(),
        "{} refusal(s) are not a language verdict: a refusal is an exit 1 carrying a diagnostic, \
         and an ice, a signal or a silent failure counted as one is a compiler fault read as a \
         property of the program:\n{}",
        undiagnosed.len(),
        undiagnosed.join("\n")
    );
    let mut faults_per_file: BTreeMap<&str, usize> = BTreeMap::new();
    for (origin, _) in &link_faults {
        let file = origin.rsplit_once(':').map_or(origin.as_str(), |(f, _)| f);
        *faults_per_file.entry(file).or_default() += 1;
    }
    let unratcheted: Vec<String> = faults_per_file
        .iter()
        .filter(|(file, count)| {
            let allowed = LINK_FAULT_RATCHET
                .iter()
                .find(|(f, _)| f == *file)
                .map_or(0, |(_, n)| *n);
            **count > allowed
        })
        .map(|(file, count)| format!("  {file}: {count}"))
        .collect();
    assert!(
        unratcheted.is_empty(),
        "{} program(s) are refused by a linker fault and the ratchet names {} of them. a `cc` \
         that failed is the environment, not a verdict on the program; the files over their \
         ratcheted count are:\n{}",
        link_faults.len(),
        LINK_FAULT_RATCHET.iter().map(|(_, n)| n).sum::<usize>(),
        unratcheted.join("\n")
    );
    assert!(
        stuck_compiler.is_empty(),
        "the compiler did not finish on {} program(s); that is not a verdict and not a value:\n{}",
        stuck_compiler.len(),
        stuck_compiler.join("\n")
    );
    assert!(
        split.is_empty(),
        "{} program(s) compile at some -O levels and are refused at others. a verdict that \
         depends on the optimizer is a divergence, not a skip:\n{}",
        split.len(),
        split.join("\n")
    );
    assert!(
        disagreements.is_empty(),
        "{} program(s) produce a different answer at different -O levels:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
    assert!(
        nonterminating.len() <= NONTERMINATING_BASELINE,
        "{} program(s) fail to terminate at some -O level, baseline is {NONTERMINATING_BASELINE}. \
         they carry no comparable value, so every one of them is a hole in this net:\n{}",
        nonterminating.len(),
        nonterminating.join("\n")
    );
    if sampled.is_none() {
        assert!(
            compiles_everywhere >= COMPILES_EVERYWHERE_FLOOR,
            "{compiles_everywhere} of {} programs compile at all four levels, floor is \
             {COMPILES_EVERYWHERE_FLOOR}. a population that stopped compiling is a net that \
             stopped comparing",
            programs.len()
        );
    }

    assert!(
        sampled.is_none(),
        "AELYS_VALUE_NET_LIMIT was set, so this row measured {} programs out of the tracked \
         population and is a sample, not the net. every other assertion above passed on that \
         sample. unset the variable for a verdict",
        programs.len()
    );
}

#[test]
fn the_comparison_this_net_runs_tells_two_level_results_apart() {
    let mut blind = Vec::new();
    for (what, left, right) in common::fabricated_level_disagreements() {
        let shown = breakdown(&[("-O0", left.clone()), ("-O1", right.clone())]);
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
        "{} fabricated pair(s) of level results are not told apart by the comparison every row of \
         this net runs, so a green sweep would say nothing about the compiler:\n{}",
        blind.len(),
        blind.join("\n")
    );
}
