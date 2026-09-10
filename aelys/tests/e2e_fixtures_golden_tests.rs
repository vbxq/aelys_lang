use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use tempfile::tempdir;

mod common;
use common::{exe_path_for as executable_path_for, linker_unavailable};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Reject,
    CompileFail,
    CompileOk,
    Exit(i32),
    Signal(i32),
}

impl Verdict {
    fn serialize(&self) -> String {
        match self {
            Verdict::Reject => "reject".to_string(),
            Verdict::CompileFail => "compile_fail".to_string(),
            Verdict::CompileOk => "compile_ok".to_string(),
            Verdict::Exit(code) => format!("exit:{code}"),
            Verdict::Signal(sig) => format!("signal:{sig}"),
        }
    }

    fn parse(text: &str) -> Option<Verdict> {
        Some(match text {
            "reject" => Verdict::Reject,
            "compile_fail" => Verdict::CompileFail,
            "compile_ok" => Verdict::CompileOk,
            other => {
                if let Some(code) = other.strip_prefix("exit:") {
                    Verdict::Exit(code.parse().ok()?)
                } else if let Some(sig) = other.strip_prefix("signal:") {
                    Verdict::Signal(sig.parse().ok()?)
                } else {
                    return None;
                }
            }
        })
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("aelys crate has a parent workspace dir")
        .to_path_buf()
}

fn fixtures_dir() -> PathBuf {
    workspace_root().join("tests_e2e")
}

fn golden_path() -> PathBuf {
    fixtures_dir().join("EXPECTED.tsv")
}

fn is_reject_name(stem: &str) -> bool {
    stem.ends_with("_reject") || stem.starts_with("never")
}

#[cfg(unix)]
fn classify_exit(status: ExitStatus) -> Verdict {
    use std::os::unix::process::ExitStatusExt;
    if let Some(code) = status.code() {
        Verdict::Exit(code)
    } else if let Some(sig) = status.signal() {
        Verdict::Signal(sig)
    } else {
        Verdict::Exit(-1)
    }
}

#[cfg(not(unix))]
fn classify_exit(status: ExitStatus) -> Verdict {
    Verdict::Exit(status.code().unwrap_or(-1))
}

fn linker_available() -> bool {
    let dir = tempdir().expect("tempdir");
    let probe = dir.path().join("canary.aelys");
    fs::write(&probe, "fn main() -> i64 { return 0 }").expect("write canary");
    match compile_file_with_llvm(&probe, OptimizationLevel::Standard, false) {
        Ok(()) => true,
        Err(err) => !linker_unavailable(&err.to_string()),
    }
}

const GOLDEN_LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
];

fn evaluate_at(fixture: &Path, opt: OptimizationLevel) -> Verdict {
    let stem = fixture
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("fixture has a utf-8 stem")
        .to_string();
    let reject = is_reject_name(&stem);

    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join(format!("{stem}.aelys"));
    fs::copy(fixture, &source_path).expect("copy fixture into tempdir");

    match compile_file_with_llvm(&source_path, opt, false) {
        Err(_) => {
            if reject {
                Verdict::Reject
            } else {
                Verdict::CompileFail
            }
        }
        Ok(()) => {
            let exe = executable_path_for(&source_path);
            if !exe.is_file() {
                return Verdict::CompileOk;
            }
            common::note_leg();
            let output = Command::new(&exe)
                .output()
                .expect("compiled executable should run");
            classify_exit(output.status)
        }
    }
}

fn collect_fixtures() -> Vec<PathBuf> {
    let mut fixtures: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .expect("tests_e2e/ dir is readable")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().map(|ext| ext == "aelys").unwrap_or(false))
        .collect();
    fixtures.sort();
    fixtures
}

fn write_golden(results: &BTreeMap<String, Verdict>) {
    let mut out = String::new();
    out.push_str("# golden snapshop of *.aelys tests on the green state.\n");
    out.push_str("# oracle of records, captures behaviour verbatim: mod-256 exit truncation and\n");
    out.push_str(
        "# the runtime SIGABRT guards (div/0, out-of-bounds). not a list of \"correct\" answers.\n",
    );
    out.push_str("# regenerate only when the green behaviour legitimately changes:\n");
    out.push_str("#   AELYS_REGEN_GOLDEN=1 cargo test -p aelys --test e2e_fixtures_golden_tests -- --nocapture\n");
    out.push_str("# Format: <fixture.aelys>\\t<verdict>  (reject | compile_fail | compile_ok | exit:N | signal:N)\n");
    for (name, verdict) in results {
        out.push_str(name);
        out.push('\t');
        out.push_str(&verdict.serialize());
        out.push('\n');
    }
    fs::write(golden_path(), out).expect("write golden EXPECTED.tsv");
}

fn read_golden() -> BTreeMap<String, Verdict> {
    let text = match fs::read_to_string(golden_path()) {
        Ok(text) => text,
        Err(_) => return BTreeMap::new(),
    };
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, verdict) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("malformed golden line (missing tab): {line:?}"));
        let verdict = Verdict::parse(verdict)
            .unwrap_or_else(|| panic!("unknown verdict in golden: {verdict:?}"));
        map.insert(name.to_string(), verdict);
    }
    map
}

#[test]
fn e2e_fixtures_match_golden() {
    if !linker_available() {
        common::require_linker_skip("a skipped golden net compares no fixture against anything");
        return;
    }

    let fixtures = collect_fixtures();
    assert!(
        !fixtures.is_empty(),
        "no .aelys fixtures found in {:?}",
        fixtures_dir()
    );

    let runs = read_golden()
        .values()
        .filter(|v| matches!(v, Verdict::Exit(_) | Verdict::Signal(_)))
        .count();
    let _pin = std::env::var_os("AELYS_REGEN_GOLDEN")
        .is_none()
        .then(|| common::pin_legs("golden net", runs * GOLDEN_LEVELS.len()));

    let mut per_level: Vec<(&str, BTreeMap<String, Verdict>)> = Vec::new();
    for (level, opt) in GOLDEN_LEVELS {
        per_level.push((
            level,
            fixtures
                .iter()
                .map(|path| {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .expect("fixture has a utf-8 name")
                        .to_string();
                    (name, evaluate_at(path, *opt))
                })
                .collect(),
        ));
    }
    let actual: BTreeMap<String, Verdict> = per_level
        .iter()
        .find(|(l, _)| *l == "-O2")
        .map(|(_, m)| m.clone())
        .expect("-O2 leg present");

    if std::env::var_os("AELYS_REGEN_GOLDEN").is_some() {
        write_golden(&actual);
        eprintln!(
            "regenerated golden: {} fixtures -> {:?}",
            actual.len(),
            golden_path()
        );
        return;
    }

    let expected = read_golden();
    assert!(
        !expected.is_empty(),
        "golden {:?} is missing or empty; generate it with AELYS_REGEN_GOLDEN=1",
        golden_path()
    );

    let mut diffs: Vec<String> = Vec::new();
    for name in actual.keys() {
        if !expected.contains_key(name) {
            diffs.push(format!(
                "{name}: present on disk but absent from golden (regen needed)"
            ));
        }
    }
    for name in expected.keys() {
        if !actual.contains_key(name) {
            diffs.push(format!("{name}: in golden but missing on disk"));
        }
    }
    for (level, observed) in &per_level {
        for (name, verdict) in observed {
            if let Some(frozen) = expected.get(name) {
                if verdict != frozen {
                    diffs.push(format!(
                        "{name} at {level}: golden {}, observed {}",
                        frozen.serialize(),
                        verdict.serialize()
                    ));
                }
            }
        }
    }

    assert!(
        diffs.is_empty(),
        "e2e regression net detected {} change(s) vs frozen golden:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

fn extract_should_be(src: &str) -> Option<i64> {
    let mut found = None;
    for line in src.lines() {
        if !line.contains("return") {
            continue;
        }
        let Some(comment) = line.split("//").nth(1) else {
            continue;
        };
        if let Some(rest) = comment.trim_start().strip_prefix("should be ") {
            if let Some(value) = parse_leading_int(rest) {
                found = Some(value);
            }
        }
    }
    found
}

fn parse_leading_int(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut sign = 1i64;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        if bytes[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return None;
    }
    text[start..i].parse::<i64>().ok().map(|n| sign * n)
}

#[test]
fn golden_vs_should_be_annotations() {
    let golden = read_golden();
    if golden.is_empty() {
        eprintln!("golden not generated yet; skipping annotation cross-check");
        return;
    }

    let mut mismatches: Vec<String> = Vec::new();
    for (name, verdict) in &golden {
        let Verdict::Exit(code) = verdict else {
            continue;
        };
        let src = match fs::read_to_string(fixtures_dir().join(name)) {
            Ok(src) => src,
            Err(_) => continue,
        };
        if let Some(annotation) = extract_should_be(&src) {
            let expected = annotation.rem_euclid(256) as i32;
            if *code != expected {
                mismatches.push(format!(
                    "{name}: golden exit {code}, annotation 'should be {annotation}' (-> {expected} mod 256)"
                ));
            }
        }
    }

    if mismatches.is_empty() {
        eprintln!(
            "annotation cross-check: all '// should be N' annotations agree with the golden (mod 256)"
        );
    } else {
        eprintln!(
            "[soft] {} fixture(s) where the golden exit code disagrees with the '// should be N' annotation:",
            mismatches.len()
        );
        for line in &mismatches {
            eprintln!("  {line}");
        }
    }
}
