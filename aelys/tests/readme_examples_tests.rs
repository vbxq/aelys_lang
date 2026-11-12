use aelys_common::{ColorConfig, render_summary};
use aelys_driver::{
    LinkRequirement, RuntimeVariant, SourceOptions, compile_file_with_llvm_sources,
    executable_path_for,
};
use aelys_opt::OptimizationLevel;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
];

const RUN_LIMIT: Duration = Duration::from_secs(20);

const PROGRAMS: usize = 27;
const REFUSALS: usize = 19;
const REFUSALS_WITH_DIAGNOSTIC: usize = 10;
const MODULES: usize = 5;
const FRAGMENTS: usize = 0;
const SHELLS: usize = 1;
const INTERNAL: [&str; 3] = ["E0901", "E0432", "E0903"];
const UNVERIFIED: &[(&str, &str)] = &[("text", "the compiler pipeline drawn as a diagram")];

#[derive(Clone, Debug)]
struct Block {
    line: usize,
    info: Vec<String>,
    body: String,
}

impl Block {
    fn kind(&self) -> &str {
        self.info.first().map(String::as_str).unwrap_or("")
    }

    fn word(&self, i: usize) -> &str {
        self.info.get(i).map(String::as_str).unwrap_or("")
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.info
            .iter()
            .find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
    }

    fn has(&self, word: &str) -> bool {
        self.info.iter().any(|w| w == word)
    }

    fn file(&self) -> &str {
        self.value("file").unwrap_or("main.aelys")
    }
}

fn readme() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn blocks_of(text: &str) -> Result<Vec<Block>, String> {
    let mut blocks = Vec::new();
    let mut open: Option<Block> = None;
    for (i, line) in text.lines().enumerate() {
        let at = i + 1;
        match open.as_mut() {
            None => {
                if line.trim_start().starts_with("~~~") {
                    return Err(format!("line {at}: a ~~~ fence"));
                }
                if line.contains("<pre") || line.contains("<code") {
                    return Err(format!("line {at}: an html code block"));
                }
                if (line.starts_with("    ") || line.starts_with('\t')) && !line.trim().is_empty() {
                    return Err(format!("line {at}: an indented code block"));
                }
                if let Some(rest) = line.trim_start().strip_prefix("```") {
                    if line != line.trim_start() {
                        return Err(format!("line {at}: an indented fence"));
                    }
                    open = Some(Block {
                        line: at,
                        info: rest.split_whitespace().map(str::to_string).collect(),
                        body: String::new(),
                    });
                }
            }
            Some(block) => {
                if line.trim_start().starts_with("```") {
                    if line != "```" {
                        return Err(format!("line {at}: a fence inside a block"));
                    }
                    blocks.push(open.take().expect("open block"));
                } else {
                    block.body.push_str(line);
                    block.body.push('\n');
                }
            }
        }
    }
    if open.is_some() {
        return Err("a fence is never closed".to_string());
    }
    Ok(blocks)
}

fn is_code(word: &str) -> bool {
    word.len() == 5 && word.starts_with('E') && word[1..].bytes().all(|b| b.is_ascii_digit())
}

fn known_word(kind: &str, word: &str) -> bool {
    let keyed = |key: &str| {
        word.strip_prefix(key)
            .and_then(|r| r.strip_prefix('='))
            .is_some_and(|v| !v.is_empty())
    };
    match kind {
        "run" => {
            runtime_of(word).is_some()
                || word == "stats"
                || keyed("file")
                || keyed("include")
                || word
                    .strip_prefix("exit=")
                    .is_some_and(|v| v.parse::<i32>().is_ok())
        }
        "refused" => is_code(word) || word == "no-prelude" || keyed("file") || keyed("include"),
        "module" => keyed("file"),
        _ => false,
    }
}

fn runtimes_of(b: &Block) -> Vec<&str> {
    let named: Vec<&str> = b
        .info
        .iter()
        .map(String::as_str)
        .filter(|w| runtime_of(w).is_some())
        .collect();
    if named.is_empty() { vec!["rc"] } else { named }
}

fn listed_codes(b: &Block) -> Vec<&str> {
    b.info
        .iter()
        .map(String::as_str)
        .filter(|w| is_code(w))
        .collect()
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Census {
    programs: usize,
    refusals: usize,
    refusals_with_diagnostic: usize,
    modules: usize,
    fragments: usize,
    shells: usize,
    unverified: Vec<String>,
}

fn kind_at(blocks: &[Block], i: usize) -> &str {
    blocks.get(i).map(Block::kind).unwrap_or("")
}

fn census(blocks: &[Block]) -> Result<Census, String> {
    let mut c = Census::default();
    let mut i = 0;
    while i < blocks.len() {
        let b = &blocks[i];
        let line = b.line;
        if b.kind() == "rust"
            && let Some(bad) = b.info.iter().skip(2).find(|w| !known_word(b.word(1), w))
        {
            return Err(format!(
                "line {line}: an unknown word `{bad}` in ```{}",
                b.info.join(" ")
            ));
        }
        let bare = |k: usize| blocks.get(k).is_some_and(|x| x.info.len() == 1);
        match (b.kind(), b.word(1)) {
            ("rust", "run") => {
                c.programs += 1;
                if kind_at(blocks, i + 1) != "output" || !bare(i + 1) {
                    return Err(format!("line {line}: a program without its output block"));
                }
                let runtimes = runtimes_of(b);
                let mut keys: Vec<&str> = Vec::new();
                i += 2;
                while kind_at(blocks, i) == "stderr" {
                    let s = &blocks[i];
                    let key = match s.info.len() {
                        1 => "",
                        2 if runtimes.contains(&s.word(1)) => s.word(1),
                        _ => {
                            return Err(format!(
                                "line {}: ```{} names no runtime the program runs under",
                                s.line,
                                s.info.join(" ")
                            ));
                        }
                    };
                    if keys.contains(&key)
                        || (key.is_empty() && !keys.is_empty())
                        || keys.contains(&"")
                    {
                        return Err(format!(
                            "line {}: a second stderr block for one run",
                            s.line
                        ));
                    }
                    keys.push(key);
                    i += 1;
                }
                continue;
            }
            ("rust", "refused") => {
                c.refusals += 1;
                if listed_codes(b).is_empty() || !is_code(b.word(2)) {
                    return Err(format!("line {line}: a refusal without its codes"));
                }
                if kind_at(blocks, i + 1) == "diagnostic" {
                    if !bare(i + 1) {
                        return Err(format!("line {line}: words after ```diagnostic"));
                    }
                    c.refusals_with_diagnostic += 1;
                    i += 1;
                }
            }
            ("rust", "module") => {
                c.modules += 1;
                if b.value("file").is_none() {
                    return Err(format!("line {line}: a module without its file"));
                }
            }
            ("rust", "fragment") if b.info.len() == 2 => c.fragments += 1,
            ("sh", _) if b.info.len() == 1 => {
                c.shells += 1;
                if kind_at(blocks, i + 1) == "output" {
                    if !bare(i + 1) {
                        return Err(format!("line {line}: words after ```output"));
                    }
                    i += 1;
                }
            }
            (kind, _) if b.info.len() == 1 && UNVERIFIED.iter().any(|(k, _)| *k == kind) => {
                c.unverified.push(kind.to_string())
            }
            _ => {
                return Err(format!(
                    "line {line}: a block the test does not recognise: ```{}",
                    b.info.join(" ")
                ));
            }
        }
        i += 1;
    }
    Ok(c)
}

fn runtime_of(word: &str) -> Option<RuntimeVariant> {
    match word {
        "rc" => Some(RuntimeVariant::Rc),
        "rc+cycles" => Some(RuntimeVariant::RcCycles),
        _ => None,
    }
}

fn sources_of(block: &Block, stage: &Path) -> SourceOptions {
    let include = block
        .value("include")
        .map(|list| list.split(',').map(|d| stage.join(d)).collect())
        .unwrap_or_default();
    let mut sources = SourceOptions::with_include(include);
    if block.has("no-prelude") {
        sources.prelude = None;
    }
    sources
}

fn write_stage(stage: &Path, modules: &[(String, String)], file: &str, src: &str) -> PathBuf {
    fs::create_dir_all(stage).expect("stage dir");
    for (path, text) in modules {
        let target = stage.join(path);
        fs::create_dir_all(target.parent().expect("module parent")).expect("module dir");
        fs::write(target, text).expect("write module");
    }
    let root = stage.join(file);
    fs::write(&root, src).expect("write root");
    root
}

struct Ran {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_bounded(exe: &Path, env: &[(&str, &str)]) -> Result<Ran, String> {
    let mut command = Command::new(exe);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot spawn {}: {e}", exe.display()))?;
    let mut out = child.stdout.take().expect("child stdout");
    let mut err = child.stderr.take().expect("child stderr");
    let out_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut text);
        text
    });
    let err_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut err, &mut text);
        text
    });
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            let exit = status.code().unwrap_or_else(|| {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    128 + status.signal().unwrap_or(0)
                }
                #[cfg(not(unix))]
                {
                    -1
                }
            });
            return Ok(Ran {
                stdout: out_reader.join().expect("stdout reader"),
                stderr: err_reader.join().expect("stderr reader"),
                exit,
            });
        }
        if start.elapsed() > RUN_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} did not terminate", exe.display()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn rendered_refusal(err: &aelys_common::AelysError, stage: &Path) -> String {
    let diagnostics = err.to_diagnostics();
    let mut text: String = diagnostics
        .iter()
        .map(|d| d.render(&ColorConfig::never()))
        .collect();
    text.push_str(&render_summary(&diagnostics));
    text.replace(&format!("{}/", stage.display()), "")
}

fn take_modules(pending: &mut Vec<(String, String)>) -> Vec<(String, String)> {
    std::mem::take(pending)
}

fn check_program(
    dir: &Path,
    blocks: &[Block],
    i: usize,
    modules: &[(String, String)],
) -> Vec<String> {
    let b = &blocks[i];
    let mut failures = Vec::new();
    let runtimes = runtimes_of(b);
    let exit: i32 = b.value("exit").map_or(0, |e| e.parse().expect("exit code"));
    let env: &[(&str, &str)] = if b.has("stats") {
        &[("AELYS_RC_STATS", "1")]
    } else {
        &[]
    };
    let stdout_wanted = &blocks[i + 1].body;
    let mut stderr_wanted: BTreeMap<&str, &str> = BTreeMap::new();
    let mut k = i + 2;
    while blocks.get(k).map(Block::kind) == Some("stderr") {
        let s = &blocks[k];
        match s.word(1) {
            "" => {
                for rt in &runtimes {
                    stderr_wanted.insert(rt, &s.body);
                }
            }
            rt => {
                stderr_wanted.insert(rt, &s.body);
            }
        }
        k += 1;
    }
    for rt in &runtimes {
        let runtime = runtime_of(rt).expect("runtime word");
        for (tag, level) in LEVELS {
            let stage = dir.join(format!(
                "p{}_{}_{}",
                b.line,
                rt.replace('+', "_"),
                &tag[1..]
            ));
            let root = write_stage(&stage, modules, b.file(), &b.body);
            let at = format!("line {} {rt} {tag}", b.line);
            match compile_file_with_llvm_sources(
                &root,
                *level,
                false,
                runtime,
                &LinkRequirement::default(),
                &sources_of(b, &stage),
                None,
            ) {
                Err(err) => {
                    failures.push(format!("{at}: refused\n{}", rendered_refusal(&err, &stage)));
                    continue;
                }
                Ok(warnings) if !warnings.is_empty() => {
                    failures.push(format!("{at}: warnings {warnings:?}"));
                }
                Ok(_) => {}
            }
            match run_bounded(&executable_path_for(&root), env) {
                Err(e) => failures.push(format!("{at}: {e}")),
                Ok(ran) => {
                    let stderr = stderr_wanted.get(rt).copied().unwrap_or("");
                    if ran.stdout != *stdout_wanted || ran.stderr != stderr || ran.exit != exit {
                        failures.push(format!(
                            "{at}: exit={} stdout={:?} stderr={:?}, the README says exit={exit} stdout={:?} stderr={:?}",
                            ran.exit, ran.stdout, ran.stderr, stdout_wanted, stderr
                        ));
                    }
                }
            }
        }
    }
    failures
}

fn check_refusal(
    dir: &Path,
    blocks: &[Block],
    i: usize,
    modules: &[(String, String)],
) -> Vec<String> {
    let b = &blocks[i];
    let code = b.word(2);
    let listed = listed_codes(b);
    let shown = blocks
        .get(i + 1)
        .filter(|d| d.kind() == "diagnostic")
        .map(|d| d.body.as_str());
    let mut failures = Vec::new();
    for (tag, level) in LEVELS {
        let stage = dir.join(format!("r{}_{}", b.line, &tag[1..]));
        let root = write_stage(&stage, modules, b.file(), &b.body);
        let at = format!("line {} {tag}", b.line);
        match compile_file_with_llvm_sources(
            &root,
            *level,
            false,
            RuntimeVariant::Rc,
            &LinkRequirement::default(),
            &sources_of(b, &stage),
            None,
        ) {
            Ok(_) => failures.push(format!("{at}: accepted, the README says {code}")),
            Err(err) => {
                let text = rendered_refusal(&err, &stage);
                let emitted: Vec<String> = err
                    .to_diagnostics()
                    .iter()
                    .filter_map(|d| d.code.clone())
                    .collect();
                if emitted != listed {
                    failures.push(format!(
                        "{at}: the compiler emits {emitted:?}, the README lists {listed:?}:\n{text}"
                    ));
                }
                for internal in INTERNAL {
                    if emitted.iter().any(|c| c == internal) {
                        failures.push(format!("{at}: an internal error {internal}:\n{text}"));
                    }
                }
                if let Some(shown) = shown
                    && text != shown
                {
                    failures.push(format!(
                        "{at}: the compiler prints\n{text:?}\nthe README shows\n{shown:?}"
                    ));
                }
            }
        }
    }
    failures
}

fn shell_path(stage: &Path, word: &str) -> String {
    stage.join(word).to_string_lossy().into_owned()
}

fn check_shell(dir: &Path, blocks: &[Block], i: usize, files: &[(String, String)]) -> Vec<String> {
    let b = &blocks[i];
    let stage = dir.join(format!("s{}", b.line));
    fs::create_dir_all(&stage).expect("shell stage");
    for (path, text) in files {
        let target = stage.join(path);
        fs::create_dir_all(target.parent().expect("file parent")).expect("file dir");
        fs::write(target, text).expect("write file");
    }
    let mut failures = Vec::new();
    let mut printed = String::new();
    for line in b.body.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let at = format!("line {} `{line}`", b.line);
        match words.as_slice() {
            ["aelys-cli", rest @ ..] => {
                let mut args = vec!["aelys-cli".to_string()];
                let mut path_next = false;
                for w in rest {
                    let is_path = path_next || w.ends_with(".aelys");
                    path_next = matches!(*w, "-o" | "-I");
                    args.push(if is_path {
                        shell_path(&stage, w)
                    } else {
                        w.to_string()
                    });
                }
                match aelys_cli::cli::run_with_args(&args) {
                    Ok(0) => {}
                    other => failures.push(format!("{at}: {other:?}")),
                }
            }
            [exe] if exe.starts_with("./") => match run_bounded(&stage.join(&exe[2..]), &[]) {
                Ok(ran) if ran.exit == 0 && ran.stderr.is_empty() => printed.push_str(&ran.stdout),
                Ok(ran) => {
                    failures.push(format!("{at}: exit={} stderr={:?}", ran.exit, ran.stderr))
                }
                Err(e) => failures.push(format!("{at}: {e}")),
            },
            ["ls"] => {
                let mut names: Vec<String> = fs::read_dir(&stage)
                    .expect("read shell stage")
                    .map(|e| {
                        e.expect("dir entry")
                            .file_name()
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect();
                names.sort();
                for name in names {
                    printed.push_str(&name);
                    printed.push('\n');
                }
            }
            _ => failures.push(format!("{at}: a command the test cannot run")),
        }
    }
    let wanted = blocks
        .get(i + 1)
        .filter(|o| o.kind() == "output")
        .map_or("", |o| o.body.as_str());
    if printed != wanted {
        failures.push(format!(
            "line {}: the commands print {printed:?}, the README says {wanted:?}",
            b.line
        ));
    }
    failures
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Programs,
    Refusals,
    Shells,
}

fn sweep(pass: Pass) {
    let blocks = blocks_of(&readme()).unwrap_or_else(|e| panic!("README: {e}"));
    census(&blocks).unwrap_or_else(|e| panic!("README: {e}"));
    let dir = tempdir().expect("tempdir");
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut files: Vec<(String, String)> = Vec::new();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (i, b) in blocks.iter().enumerate() {
        if b.kind() == "rust" && b.value("file").is_some() {
            files.push((b.file().to_string(), b.body.clone()));
        }
        match (b.kind(), b.word(1)) {
            ("rust", "module") => pending.push((b.file().to_string(), b.body.clone())),
            ("rust", "run") => {
                let modules = take_modules(&mut pending);
                if pass == Pass::Programs {
                    failures.extend(check_program(dir.path(), &blocks, i, &modules));
                    checked += 1;
                }
            }
            ("rust", "refused") => {
                let modules = take_modules(&mut pending);
                if pass == Pass::Refusals {
                    failures.extend(check_refusal(dir.path(), &blocks, i, &modules));
                    checked += 1;
                }
            }
            ("sh", _) if pass == Pass::Shells => {
                failures.extend(check_shell(dir.path(), &blocks, i, &files));
                checked += 1;
            }
            _ => {}
        }
    }
    assert!(
        pending.is_empty(),
        "README: a module block with no program after it"
    );
    let expected = match pass {
        Pass::Programs => PROGRAMS,
        Pass::Refusals => REFUSALS,
        Pass::Shells => SHELLS,
    };
    assert_eq!(checked, expected, "README: blocks checked");
    assert!(failures.is_empty(), "README:\n{}", failures.join("\n\n"));
}

#[test]
fn readme_every_fence_is_classified_and_counted() {
    let blocks = blocks_of(&readme()).unwrap_or_else(|e| panic!("README: {e}"));
    let c = census(&blocks).unwrap_or_else(|e| panic!("README: {e}"));
    assert_eq!(
        c,
        Census {
            programs: PROGRAMS,
            refusals: REFUSALS,
            refusals_with_diagnostic: REFUSALS_WITH_DIAGNOSTIC,
            modules: MODULES,
            fragments: FRAGMENTS,
            shells: SHELLS,
            unverified: UNVERIFIED.iter().map(|(k, _)| k.to_string()).collect(),
        }
    );
}

#[test]
fn readme_census_rejects_an_unclassified_fence() {
    let text = "```rust\nfn main() -> i64 { return 0 }\n```\n";
    assert!(blocks_of(text).and_then(|b| census(&b)).is_err());
    let text = "```rust run\nfn main() -> i64 { return 0 }\n```\n";
    assert!(blocks_of(text).and_then(|b| census(&b)).is_err());
    let main = "fn main() -> i64 { return 0 }\n";
    for text in [
        format!("~~~rust run\n{main}~~~\n\n~~~output\n~~~\n"),
        format!("Text:\n\n    {main}"),
        format!("```rust run rc+cyles\n{main}```\n```output\n```\n"),
        format!("```rust run\n{main}```\n```output\n```\n```stderr rc+cycles\n```\n"),
        format!("```rust run\n{main}```\n```output extra\n```\n"),
        format!("```rust refused\n{main}```\n"),
    ] {
        assert!(blocks_of(&text).and_then(|b| census(&b)).is_err(), "{text}");
    }
    let good =
        format!("```rust run rc rc+cycles\n{main}```\n```output\n```\n```stderr rc+cycles\n```\n");
    assert!(blocks_of(&good).and_then(|b| census(&b)).is_ok());
}

#[test]
fn readme_programs_print_what_the_readme_says() {
    sweep(Pass::Programs);
}

#[test]
fn readme_refusals_read_as_the_readme_shows() {
    sweep(Pass::Refusals);
}

#[test]
fn readme_shell_commands_run_as_quoted() {
    sweep(Pass::Shells);
}
