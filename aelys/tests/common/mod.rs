#![allow(dead_code)]

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;
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

// every backend fault renders under E09xx, so a fifth code is caught here without editing a row
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
