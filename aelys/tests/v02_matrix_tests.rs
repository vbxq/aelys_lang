use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod common;

const LEVELS: [&str; 4] = ["-O0", "-O1", "-O2", "-O3"];

#[derive(Debug)]
enum Expect {
    Run(String),
    Balanced(String),
    Refuse(Vec<String>, Vec<String>),
}

struct Case {
    name: String,
    src: PathBuf,
    modules: Option<PathBuf>,
    args: Vec<String>,
    expect: Expect,
}

fn parse_expect(text: &str) -> Expect {
    let (head, body) = text.split_once('\n').unwrap_or((text, ""));
    let mut words = head.split_whitespace();
    match words.next() {
        Some("run") => Expect::Run(body.to_string()),
        Some("balanced") => Expect::Balanced(body.to_string()),
        Some("refuse") => Expect::Refuse(
            words.map(str::to_string).collect(),
            body.lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string)
                .collect(),
        ),
        other => panic!("unknown expectation head {other:?}"),
    }
}

fn cases_of(track: &str) -> Vec<Case> {
    let dir = common::repo_root_from_manifest()
        .join("aelys/tests/v02")
        .join(track);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let filter = std::env::var("V02_CASE").ok();
    let mut cases = Vec::new();
    for entry in entries.flatten() {
        let src = entry.path();
        if src.extension().and_then(|e| e.to_str()) != Some("case") {
            continue;
        }
        let name = src.file_stem().unwrap().to_string_lossy().into_owned();
        if let Some(f) = &filter
            && !name.contains(f.as_str())
        {
            continue;
        }
        let expect_path = src.with_extension("expect");
        let text = fs::read_to_string(&expect_path)
            .unwrap_or_else(|_| panic!("{} has no .expect", src.display()));
        let args = fs::read_to_string(src.with_extension("args"))
            .map(|a| a.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        let modules = src.with_extension("mod");
        cases.push(Case {
            name,
            modules: modules.is_dir().then_some(modules),
            src,
            args,
            expect: parse_expect(&text),
        });
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

fn cli_bin() -> PathBuf {
    let _ = common::Cli::located();
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().join("aelys-cli")
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().replace(".amod", ".aelys");
        let target = to.join(name);
        if entry.path().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn wait(mut child: std::process::Child, budget: Duration) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return Some(s);
        }
        if start.elapsed() > budget {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn leg(bin: &Path, case: &Case, level: &str) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    if let Some(m) = &case.modules {
        copy_tree(m, dir.path());
    }
    let prog = dir.path().join("prog.aelys");
    fs::copy(&case.src, &prog).unwrap();
    let exe = dir.path().join("prog");
    let diag = dir.path().join("diag");
    let compile = Command::new(bin)
        .arg("compile")
        .arg(&prog)
        .arg(level)
        .arg("-o")
        .arg(&exe)
        .arg("-I")
        .arg(common::repo_root_from_manifest())
        .args(&case.args)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&diag).unwrap())
        .spawn()
        .unwrap();
    let status = wait(compile, Duration::from_secs(120)).ok_or("compiler did not finish")?;
    let rendered = fs::read_to_string(&diag).unwrap_or_default();
    let compiled = status.success() && exe.is_file();
    if let Expect::Refuse(codes, says) = &case.expect {
        if compiled {
            return Err("compiled, a refusal was expected".into());
        }
        for code in codes {
            if !rendered.contains(&format!("error[{code}]")) {
                return Err(format!("missing error[{code}] in:\n{rendered}"));
            }
        }
        for line in says {
            match line.strip_prefix('!') {
                Some(absent) if rendered.contains(absent) => {
                    return Err(format!("present {absent:?} in:\n{rendered}"));
                }
                Some(_) => {}
                None if !rendered.contains(line.as_str()) => {
                    return Err(format!("missing {line:?} in:\n{rendered}"));
                }
                None => {}
            }
        }
        if rendered.contains("E0901") || rendered.contains("E0432") {
            return Err(format!("internal diagnostic:\n{rendered}"));
        }
        return Ok(());
    }
    if !compiled {
        return Err(format!("refused:\n{rendered}"));
    }
    let out = dir.path().join("out");
    let err = dir.path().join("err");
    let mut run = Command::new(&exe);
    run.current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(fs::File::create(&out).unwrap())
        .stderr(fs::File::create(&err).unwrap());
    let want = match &case.expect {
        Expect::Balanced(w) => {
            run.env("AELYS_RC_STATS", "1").env("AELYS_ALLOC", "malloc");
            w
        }
        Expect::Run(w) => w,
        Expect::Refuse(..) => unreachable!(),
    };
    let status = wait(run.spawn().unwrap(), Duration::from_secs(60)).ok_or("did not terminate")?;
    let stdout = fs::read_to_string(&out).unwrap_or_default();
    let stderr = fs::read_to_string(&err).unwrap_or_default();
    let code = common::exit_code(&status);
    let got = if code == 0 {
        stdout.clone()
    } else {
        format!("{stdout}exit {code}\n")
    };
    if &got != want {
        return Err(format!("stdout mismatch\nwant {want:?}\ngot  {got:?}\nstderr {stderr}"));
    }
    if let Expect::Balanced(_) = case.expect {
        let line = stderr
            .lines()
            .find(|l| l.starts_with("[rc] allocs="))
            .ok_or_else(|| format!("no [rc] line in {stderr}"))?;
        let n: Vec<&str> = line
            .split_whitespace()
            .filter_map(|w| w.split_once('=').map(|(_, n)| n))
            .collect();
        if n.len() < 2 || n[0] != n[1] {
            return Err(format!("unbalanced {line}"));
        }
    }
    Ok(())
}

fn run_track(track: &str) {
    let cases = cases_of(track);
    if cases.is_empty() {
        eprintln!("v02 {track}: no cases");
        return;
    }
    let bin = cli_bin();
    let jobs: usize = std::env::var("V02_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(4);
    let work: Mutex<Vec<(usize, &str)>> = Mutex::new(
        (0..cases.len())
            .flat_map(|i| LEVELS.iter().map(move |l| (i, *l)))
            .collect(),
    );
    let failures = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                while let Some((i, level)) = work.lock().unwrap().pop() {
                    if let Err(e) = leg(&bin, &cases[i], level) {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{track}/{} {level}: {e}", cases[i].name));
                    }
                }
            });
        }
    });
    let mut failures = failures.into_inner().unwrap();
    failures.sort();
    eprintln!(
        "v02 {track}: {} cases x {} levels, {} failing legs",
        cases.len(),
        LEVELS.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn track_stage0() {
    run_track("stage0");
}

#[test]
fn track_a1_generic_structs() {
    run_track("a1");
}

#[test]
fn track_a2_methods() {
    run_track("a2");
}

#[test]
fn track_a3_traits() {
    run_track("a3");
}

#[test]
fn track_b_patterns() {
    run_track("b");
}

#[test]
fn track_c_tuples_const_alias() {
    run_track("c");
}

#[test]
fn track_d_collections() {
    run_track("d");
}

#[test]
fn track_e_first_hour() {
    run_track("e");
}

#[test]
fn track_showcase() {
    run_track("showcase");
}

#[test]
fn track_cross() {
    run_track("cross");
}

#[test]
fn track_std() {
    run_track("std");
}

#[test]
fn track_final_modcyc() {
    run_track("final_modcyc");
}
