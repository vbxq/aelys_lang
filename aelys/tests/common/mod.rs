#![allow(dead_code)]

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Once;
use std::time::{Duration, Instant};
use tempfile::tempdir;

static WARM: Once = Once::new();

// the first link in a process builds the core archive, so pay for it before any measured row
pub fn warm_core_archive() {
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

pub fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

pub fn object_path_for(p: &Path) -> PathBuf {
    p.with_extension(if cfg!(windows) { "obj" } else { "o" })
}

pub fn exit_code(status: &std::process::ExitStatus) -> i32 {
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

pub fn slug(id: &str, tag: &str) -> String {
    let mut s = String::with_capacity(id.len() + tag.len() + 1);
    for c in id.chars().chain(std::iter::once('_')).chain(tag.chars()) {
        s.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    s
}

// the union of the three predicates the suites used, so no linker failure escapes the gate
pub fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found")
        || error.contains("failed to run")
        || error.contains("failed with status Some(-1073741819)")
}

pub fn linker_skip_declared() -> bool {
    std::env::var("AELYS_ALLOW_LINKER_SKIP").is_ok()
}

pub fn require_linker_skip(what: &str) {
    assert!(
        linker_skip_declared(),
        "no linker, and AELYS_ALLOW_LINKER_SKIP is not set; {what}"
    );
    note_linker_skip();
}

thread_local! {
    static LEGS: Cell<usize> = const { Cell::new(0) };
    static SKIPS: Cell<usize> = const { Cell::new(0) };
}

pub fn note_leg() {
    LEGS.with(|c| c.set(c.get() + 1));
}

pub fn legs_run() -> usize {
    LEGS.with(|c| c.get())
}

pub fn note_linker_skip() {
    SKIPS.with(|c| c.set(c.get() + 1));
}

pub fn linker_skips() -> usize {
    SKIPS.with(|c| c.get())
}

pub struct LegPin {
    what: String,
    expected: usize,
    legs_at_start: usize,
    skips_at_start: usize,
}

pub fn pin_legs(what: impl Into<String>, expected: usize) -> LegPin {
    LegPin {
        what: what.into(),
        expected,
        legs_at_start: legs_run(),
        skips_at_start: linker_skips(),
    }
}

impl Drop for LegPin {
    fn drop(&mut self) {
        // firing mid-unwind would bury the assertion that actually failed
        if std::thread::panicking() {
            return;
        }
        let ran = legs_run() - self.legs_at_start;
        if ran == self.expected {
            return;
        }
        if linker_skips() > self.skips_at_start && linker_skip_declared() {
            return;
        }
        panic!(
            "{}: must execute exactly {} legs, {ran} ran; a leg that silently stopped running \
             is a confident zero",
            self.what, self.expected
        );
    }
}

// every backend fault renders under e09xx, so a fifth code is caught here without editing a row
pub fn backend_family_code(rendered: &str) -> Option<String> {
    let bytes = rendered.as_bytes();
    (0..bytes.len().saturating_sub(6)).find_map(|i| {
        let window = &bytes[i..i + 7];
        let shaped = window[0] == b'['
            && window[1] == b'E'
            && window[6] == b']'
            && window[2..6].iter().all(u8::is_ascii_digit);
        (shaped && window[2] == b'0' && window[3] == b'9')
            .then(|| String::from_utf8_lossy(&window[1..6]).into_owned())
    })
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Exit(i32),
    Signal(i32),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RunResult {
    pub outcome: Outcome,
    pub stdout: String,
}

// a signal kill reports no status, so the signal is read first to keep 134 apart from sigabrt
pub fn outcome_of(status: &ExitStatus) -> Outcome {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return Outcome::Signal(signal);
        }
    }
    Outcome::Exit(status.code().unwrap_or(-1))
}

impl RunResult {
    pub fn read(status: &ExitStatus, stdout: String) -> RunResult {
        RunResult {
            outcome: outcome_of(status),
            stdout,
        }
    }

    pub fn render(&self) -> String {
        let stdout = rendered_stdout(&self.stdout);
        match self.outcome {
            Outcome::Exit(code) => format!("exit={code} stdout={stdout}"),
            Outcome::Signal(signal) => format!("signal={signal} stdout={stdout}"),
        }
    }
}

// the whole stream is read, so a megabyte artifact does not flood the failure message
fn rendered_stdout(stdout: &str) -> String {
    const HEAD: usize = 160;
    if stdout.len() <= HEAD {
        return format!("{stdout:?}");
    }
    let mut cut = HEAD;
    while !stdout.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{:?} and {} more bytes", &stdout[..cut], stdout.len() - cut)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Leg {
    Rejected(Outcome, String),
    CompilerDidNotFinish,
    DidNotTerminate,
    Ran(RunResult),
}

impl Leg {
    pub fn render(&self) -> String {
        match self {
            Leg::Rejected(outcome, diagnostics) => {
                let head = diagnostics.lines().next().unwrap_or("no diagnostic at all");
                match outcome {
                    Outcome::Exit(code) => format!("rejected exit={code} {head:?}"),
                    Outcome::Signal(signal) => format!("rejected signal={signal} {head:?}"),
                }
            }
            Leg::CompilerDidNotFinish => "the compiler did not finish".to_string(),
            Leg::DidNotTerminate => "the artifact did not terminate".to_string(),
            Leg::Ran(result) => result.render(),
        }
    }

    pub fn compiled(&self) -> bool {
        matches!(self, Leg::DidNotTerminate | Leg::Ran(_))
    }

    pub fn ran(&self) -> bool {
        matches!(self, Leg::Ran(_))
    }

    pub fn refusal(&self) -> Option<(&Outcome, &str)> {
        match self {
            Leg::Rejected(outcome, diagnostics) => Some((outcome, diagnostics.as_str())),
            _ => None,
        }
    }
}

pub const CLI_OPT_LEVELS: [&str; 4] = ["-O0", "-O1", "-O2", "-O3"];

// these three pairs settle a comparator never seen to tell two levels apart
pub fn fabricated_level_disagreements() -> Vec<(&'static str, Leg, Leg)> {
    let ran = |outcome: Outcome, stdout: &str| {
        Leg::Ran(RunResult {
            outcome,
            stdout: stdout.to_string(),
        })
    };
    vec![
        (
            "the same exit code carrying a different stdout",
            ran(Outcome::Exit(0), "1\n"),
            ran(Outcome::Exit(0), "0\n"),
        ),
        (
            "the same stdout carried by a different exit code",
            ran(Outcome::Exit(0), "1\n"),
            ran(Outcome::Exit(1), "1\n"),
        ),
        (
            "an exit 134 against the signal 6 that renders as one",
            ran(Outcome::Exit(134), ""),
            ran(Outcome::Signal(6), ""),
        ),
    ]
}

// the extractor reads this file too, so this program is a member of the swept population and not a fixture beside it
pub const OPTIMIZER_WITNESS: &str = "\
fn optimizer_witness_sum(n: i64) -> i64 {
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < n {
        acc = acc + i * 3
        i = i + 1
    }
    return acc
}

fn main() -> i64 {
    println(optimizer_witness_sum(10))
    return 0
}
";

// four legs under one setting agree by construction, so the sweep waits for a produced artifact
pub fn opt_level_reaches_the_artifact(cli: &Cli, dir: &Path) -> String {
    let built = |level: &str| match cli.artifact(dir, OPTIMIZER_WITNESS, level) {
        Ok(bytes) => bytes,
        Err(leg) => panic!(
            "the artifact control has to compile at {level} to control anything: {}",
            leg.render()
        ),
    };
    let unoptimized = built("-O0");
    let again = built("-O0");
    assert!(
        unoptimized == again,
        "two -O0 compiles of one program produced artifacts of {} and {} bytes that differ, so no \
         later difference between two levels could be attributed to the level",
        unoptimized.len(),
        again.len()
    );
    let optimized = built("-O2");
    assert!(
        unoptimized != optimized,
        "-O0 and -O2 produced the same {} artifact bytes for a program that has a loop to \
         optimize, so nothing above has been shown to compile under four settings rather than \
         four times under one; a cli that maps -O1..-O3 onto -O0 passes every row without this \
         line",
        unoptimized.len()
    );
    format!(
        "artifact control: -O0 produced {} bytes twice byte for byte and -O2 produced a different artifact of {} bytes",
        unoptimized.len(),
        optimized.len()
    )
}

pub fn refusal_is_not_a_verdict(leg: &Leg) -> Option<String> {
    let (outcome, diagnostics) = leg.refusal()?;
    if *outcome != Outcome::Exit(1) {
        return Some(format!("did not exit 1: {}", leg.render()));
    }
    if !diagnostics.contains("[E") {
        return Some(format!("carries no diagnostic: {}", leg.render()));
    }
    None
}

pub fn refusal_is_a_link_fault(leg: &Leg) -> bool {
    leg.refusal()
        .is_some_and(|(_, rendered)| rendered.contains("[llvm-linker]"))
}

const CLI_COMPILE_BUDGET: Duration = Duration::from_secs(60);
const CLI_RUN_BUDGET: Duration = Duration::from_secs(5);

const COMPILER_CRATES: [&str; 9] = [
    "common", "syntax", "frontend", "sema", "opt", "air", "codegen", "driver", "cli",
];

pub struct Cli {
    bin: PathBuf,
    include: PathBuf,
}

impl Cli {
    pub fn located() -> Cli {
        let exe = std::env::current_exe().expect("the test binary has a path");
        let profile = exe
            .parent()
            .and_then(Path::parent)
            .expect("the test binary sits under <target>/<profile>/deps");
        let bin = profile.join(if cfg!(windows) {
            "aelys-cli.exe"
        } else {
            "aelys-cli"
        });
        assert!(
            bin.is_file(),
            "the cross-level net drives {} and it does not exist; build it with \
             `cargo build --workspace --all-targets -j2` in the same profile as this test",
            bin.display()
        );
        let built = fs::metadata(&bin)
            .and_then(|m| m.modified())
            .expect("the cli binary has an mtime");
        let root = repo_root_from_manifest();
        let mut newer = Vec::new();
        for crate_name in COMPILER_CRATES {
            newest_source_after(&root.join(crate_name), built, &mut newer);
        }
        newer.sort();
        assert!(
            newer.is_empty(),
            "{} is older than {} compiler source file(s), so this net would measure a compiler \
             that is no longer in the tree; rebuild with `cargo build --workspace --all-targets -j2`. \
             first few:\n  {}",
            bin.display(),
            newer.len(),
            newer
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        Cli { bin, include: root }
    }

    fn compile(&self, dir: &Path, src: &str, level: &str) -> Result<PathBuf, Leg> {
        let source = dir.join("prog.aelys");
        let exe = exe_path_for(&source);
        let obj = object_path_for(&source);
        let captured = dir.join("prog.stdout");
        let diagnostics = dir.join("prog.stderr");
        // the stem is reused across levels, so a surviving artifact would be run by a compile that just failed
        for stale in [&source, &exe, &obj, &captured, &diagnostics] {
            let _ = fs::remove_file(stale);
        }
        fs::write(&source, src).expect("write the program under measurement");

        let errors = fs::File::create(&diagnostics).expect("create the diagnostic capture file");
        // without this root neither std nor the prelude resolves
        let mut compiler = Command::new(&self.bin)
            .arg("compile")
            .arg(&source)
            .arg(level)
            .arg("-I")
            .arg(&self.include)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(errors))
            .spawn()
            .expect("spawn aelys-cli");
        let Some(status) = wait_within(&mut compiler, CLI_COMPILE_BUDGET) else {
            return Err(Leg::CompilerDidNotFinish);
        };
        if !status.success() || !exe.is_file() {
            let rendered = fs::read(&diagnostics).unwrap_or_default();
            return Err(Leg::Rejected(
                outcome_of(&status),
                String::from_utf8_lossy(&rendered).into_owned(),
            ));
        }
        Ok(exe)
    }

    pub fn artifact(&self, dir: &Path, src: &str, level: &str) -> Result<Vec<u8>, Leg> {
        let exe = self.compile(dir, src, level)?;
        Ok(fs::read(&exe).expect("read the artifact under measurement"))
    }

    pub fn leg(&self, dir: &Path, src: &str, level: &str) -> Leg {
        let exe = match self.compile(dir, src, level) {
            Ok(exe) => exe,
            Err(leg) => return leg,
        };
        let captured = dir.join("prog.stdout");

        let sink = fs::File::create(&captured).expect("create the stdout capture file");
        let mut artifact = Command::new(&exe)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(sink))
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the compiled artifact");
        let Some(status) = wait_within(&mut artifact, CLI_RUN_BUDGET) else {
            return Leg::DidNotTerminate;
        };
        note_leg();
        let stdout = fs::read(&captured).unwrap_or_default();
        Leg::Ran(RunResult::read(
            &status,
            String::from_utf8_lossy(&stdout).into_owned(),
        ))
    }

    pub fn legs(&self, dir: &Path, src: &str) -> Vec<(&'static str, Leg)> {
        CLI_OPT_LEVELS
            .iter()
            .map(|level| (*level, self.leg(dir, src, level)))
            .collect()
    }
}

// a pipe would deadlock the moment a runaway artifact filled it, so stdout goes to a file and the wait is polled
fn wait_within(child: &mut Child, budget: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    let mut nap = Duration::from_micros(200);
    loop {
        match child.try_wait().expect("wait on a spawned child") {
            Some(status) => return Some(status),
            None => {
                if start.elapsed() >= budget {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(nap);
                nap = (nap * 2).min(Duration::from_millis(4));
            }
        }
    }
}

pub fn repo_root_from_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the aelys crate sits inside the workspace")
        .to_path_buf()
}

fn newest_source_after(dir: &Path, cutoff: std::time::SystemTime, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name == "target" || name == "tests" {
                continue;
            }
            newest_source_after(&path, cutoff, out);
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !matches!(ext, "rs" | "c" | "h" | "toml") {
            continue;
        }
        if let Ok(modified) = entry.metadata().and_then(|m| m.modified())
            && modified > cutoff
        {
            out.push(path.display().to_string());
        }
    }
}

// an asan core sees a string header only when the runtime itself makes the count
pub fn with_outline_str_counts<T>(compile: impl FnOnce() -> T) -> T {
    aelys_codegen::set_outline_str_counts(Some(true));
    let out = compile();
    aelys_codegen::set_outline_str_counts(None);
    out
}
