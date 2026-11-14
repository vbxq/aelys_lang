#![allow(dead_code)]

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const RUN_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LevelLeg {
    Refused(String),
    Ran {
        stdout: String,
        stderr: String,
        exit: i32,
    },
    DidNotTerminate,
}

impl LevelLeg {
    pub fn render(&self) -> String {
        match self {
            LevelLeg::Refused(rendered) => format!("refused {rendered:?}"),
            LevelLeg::Ran {
                stdout,
                stderr,
                exit,
            } => format!("exit={exit} stdout={stdout:?} stderr={stderr:?}"),
            LevelLeg::DidNotTerminate => "did not terminate".to_string(),
        }
    }

    pub fn refusal(&self) -> Option<&str> {
        match self {
            LevelLeg::Refused(rendered) => Some(rendered),
            _ => None,
        }
    }
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
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

fn run_bounded(exe: &Path, env: &[(&str, &str)]) -> LevelLeg {
    let mut command = Command::new(exe);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn compiled exe");
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
        if let Some(status) = child.try_wait().expect("poll compiled exe") {
            return LevelLeg::Ran {
                stdout: out_reader.join().expect("stdout reader"),
                stderr: err_reader.join().expect("stderr reader"),
                exit: exit_code(&status),
            };
        }
        if start.elapsed() > RUN_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return LevelLeg::DidNotTerminate;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// each level compiles in its own directory, so the directory is cut out of a rendered span
pub fn leg_at(
    stage: &Path,
    src: &str,
    module: Option<(&str, &str)>,
    level: OptimizationLevel,
    env: &[(&str, &str)],
) -> LevelLeg {
    fs::create_dir_all(stage).expect("stage dir");
    let root = stage.join("root.aelys");
    fs::write(&root, src).expect("write root");
    if let Some((file, text)) = module {
        fs::write(stage.join(file), text).expect("write module");
    }
    match compile_file_with_llvm_variant(&root, level, false, RuntimeVariant::Rc) {
        Err(err) => {
            let rendered = err.to_string();
            let stage_text = stage.to_string_lossy();
            LevelLeg::Refused(without_toolchain_echo(
                &rendered.replace(stage_text.as_ref(), "<stage>"),
            ))
        }
        Ok(()) => run_bounded(&exe_path_for(&root), env),
    }
}

pub fn legs_of(
    dir: &Path,
    name: &str,
    src: &str,
    module: Option<(&str, &str)>,
    env: &[(&str, &str)],
) -> Vec<(&'static str, LevelLeg)> {
    LEVELS
        .iter()
        .map(|(tag, level)| {
            let stage = dir.join(format!("{name}_{}", &tag[1..]));
            (*tag, leg_at(&stage, src, module, *level, env))
        })
        .collect()
}

pub fn level_disagreements(name: &str, legs: &[(&str, LevelLeg)]) -> Vec<String> {
    let Some((first_tag, first)) = legs.first() else {
        return vec![format!("{name}: no leg at all")];
    };
    legs.iter()
        .skip(1)
        .filter(|(_, leg)| leg != first)
        .map(|(tag, leg)| {
            format!(
                "{name}: {first_tag} {} but {tag} {}",
                first.render(),
                leg.render()
            )
        })
        .collect()
}

// the toolchain echo lists one line per call site, and the optimizer inlines call sites away
fn without_toolchain_echo(rendered: &str) -> String {
    let mut out = Vec::new();
    let mut in_echo = false;
    for line in rendered.lines() {
        if line == "stderr:" {
            in_echo = true;
            out.push(line);
            continue;
        }
        if in_echo && line.trim_start().starts_with("-->") {
            in_echo = false;
        }
        if !in_echo
            || !(line.contains(": in function `") || line.contains("undefined reference to `"))
        {
            out.push(line);
        }
    }
    out.join("\n")
}
