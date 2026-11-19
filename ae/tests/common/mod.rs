#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub const HELLO: &str = "fn main() -> i64 {\n    println(\"Hello, world!\")\n    return 0\n}\n";

pub const SUM_TO_TEN: &str = "fn main() -> i64 {
    let mut total = 0
    let mut i = 1
    while i <= 10 {
        total = total + i
        i = i + 1
    }
    println(total)
    return 0
}
";

pub const PRINTS_ONE: &str = "fn main() -> i64 {\n    println(1)\n    return 0\n}\n";

pub const PRINTS_TWO: &str = "fn main() -> i64 {\n    println(2)\n    return 0\n}\n";

pub const EXITS_THREE: &str = "fn main() -> i64 {\n    return 3\n}\n";

pub const ABORTS: &str = "unsafe extern nogc fn abort()

fn main() -> i64 {
    unsafe { abort() }
    return 0
}
";

pub const TYPE_ERROR: &str = "fn main() -> i64 {\n    let n: i64 = \"three\"\n    return 0\n}\n";

pub const NO_MAIN: &str = "fn helper() -> i64 {\n    return 1\n}\n";

pub const HELPER_MODULE: &str = "pub fn helper() -> i64 {\n    return 7\n}\n";

pub const SQRT_OF_TWO: &str = "unsafe extern nogc fn sqrt(x: f64) -> f64

fn main() -> i64 {
    let r = unsafe { sqrt(2.0) }
    println(r)
    return 0
}
";

pub const CALLS_AEFOO: &str = "unsafe extern nogc fn aefoo_answer() -> i64

fn main() -> i64 {
    println(unsafe { aefoo_answer() })
    return 0
}
";

pub const CALLS_SDL: &str = "unsafe extern nogc fn SDL_GetRevisionNumber() -> i32

fn main() -> i64 {
    let r = unsafe { SDL_GetRevisionNumber() }
    println(r >= 0)
    return 0
}
";

pub const AEFOO_C: &str = "long long aefoo_answer(void) { return 42; }\n";

pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub fn capped(program: &str) -> Command {
    let mut command = Command::new("systemd-run");
    command.args([
        "--user",
        "--scope",
        "-q",
        "-p",
        "MemoryMax=2G",
        "-p",
        "MemorySwapMax=0",
        program,
    ]);
    command
}

pub fn ae_with(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let output = capped(env!("CARGO_BIN_EXE_ae"))
        .args(args)
        .current_dir(dir)
        .envs(envs.iter().copied())
        .env_remove("AELYS_CORE_LIB")
        .output()
        .expect("systemd-run starts");
    Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn ae(dir: &Path, args: &[&str]) -> Output {
    ae_with(dir, args, &[])
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ae").join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

pub fn project(name: &str, native: &str, main: &str) -> PathBuf {
    let dir = scratch(name);
    write(
        &dir.join("Ae.toml"),
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n{native}"),
    );
    write(&dir.join("src/main.aelys"), main);
    dir
}

#[track_caller]
pub fn assert_ok(output: &Output) {
    assert_eq!(output.code, Some(0), "stderr:\n{}", output.stderr);
}

#[track_caller]
pub fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "missing {needle:?} in:\n{haystack}"
    );
}
