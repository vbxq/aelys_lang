use aelys_air::bir::build::build_program;
use aelys_air::bir::{effect_summaries, Effect, EffectSet};
use aelys_driver::{compile_file_with_llvm, compile_to_typed_ast, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::tempdir;

// the index is a runtime argument, so the bounds check survives constant folding.
const SLICE_INDEX_SRC: &str = r#"
nogc fn get(a: &[i64], i: i64) -> i64 { return a[i] }
fn main() -> i64 { let arr = [10, 20, 30]; return get(arr[..], INDEX) }
"#;

const UNWRAP_SRC: &str = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }
nogc fn take(r: Result<i64, E>) -> i64 { return r.unwrap() }
fn main() -> i64 { return take(PAYLOAD) }
"#;

fn slice_index_src(index: i64) -> String {
    SLICE_INDEX_SRC.replace("INDEX", &index.to_string())
}

fn unwrap_src(payload: &str) -> String {
    UNWRAP_SRC.replace("PAYLOAD", payload)
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found")
        || error.contains("failed to run")
        || error.contains("failed with status Some(-1073741819)")
}

fn exe_path_for(source_path: &Path) -> PathBuf {
    let mut output = source_path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

fn skip(reason: &str) {
    if std::env::var_os("AELYS_REQUIRE_RUNTIME").is_some() {
        panic!("[nogc-panic] SKIP-AS-FAILURE (AELYS_REQUIRE_RUNTIME=1): {reason}");
    }
    eprintln!("[nogc-panic] SKIPPED: {reason}");
}

fn accepts(label: &str, src: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .unwrap_or_else(|err| panic!("[{label}] a nogc fn that can panic must compile: {err}"));
}

fn emit_ir(src: &str, level: OptimizationLevel) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, level, true).expect("llvm ir emission should succeed");
    fs::read_to_string(source_path.with_extension("ll")).expect("read emitted ir")
}

fn run(src: &str, level: OptimizationLevel) -> Option<Output> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm(&source_path, level, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                skip("native linker unavailable");
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        skip("executable not produced");
        return None;
    }
    Some(Command::new(&exe).output().expect("run compiled exe"))
}

#[cfg(unix)]
fn abort_signal(out: &Output) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    out.status.signal()
}

#[cfg(not(unix))]
fn abort_signal(_out: &Output) -> Option<i32> {
    None
}

fn assert_aborted(label: &str, out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    if cfg!(unix) {
        assert_eq!(
            abort_signal(out),
            Some(6),
            "[{label}] the panic must abort by SIGABRT, got code {:?}; stderr:\n{stderr}",
            out.status.code()
        );
    } else {
        assert_eq!(
            out.status.code(),
            Some(134),
            "[{label}] the panic must abort (134); stderr:\n{stderr}"
        );
    }
}

fn summaries_of(src: &str) -> std::collections::HashMap<String, EffectSet> {
    let program = compile_to_typed_ast(src).expect("source should type-check");
    let bir = build_program(&program);
    effect_summaries(&bir)
}

// bounds-checked indexing carries panic only, so a nogc fn may index a slice or a fixed array.
#[test]
fn nogc_fn_with_bounds_checked_index_compiles() {
    accepts("slice param", &slice_index_src(1));
    accepts(
        "fixed array local",
        r#"
nogc fn pick(i: i64) -> i64 { let arr = [10, 20, 30]; return arr[i] }
fn main() -> i64 { return pick(1) }
"#,
    );
    accepts(
        "index assign",
        r#"
nogc fn poke(a: &mut [i64], i: i64) { a[i] = 7 }
fn main() -> i64 { let mut arr = [1, 2, 3]; poke(arr[..], 0); return arr[0] }
"#,
    );
}

#[test]
fn nogc_fn_with_result_assert_compiles() {
    accepts("unwrap", &unwrap_src("Result::Ok(7)"));
    accepts(
        "expect",
        r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }
nogc fn take(r: Result<i64, E>) -> i64 { return r.expect("boom") }
fn main() -> i64 { return take(Result::Ok(7)) }
"#,
    );
}

#[test]
fn panicking_nogc_effects_carry_panic_without_managed() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }
nogc fn get(a: &[i64], i: i64) -> i64 { return a[i] }
nogc fn take(r: Result<i64, E>) -> i64 { return r.unwrap() }
fn managed_witness() -> Rc<i64> { return Rc::new(1) }
fn main() -> i64 { let arr = [1, 2]; return get(arr[..], 0) }
"#;
    let summaries = summaries_of(src);
    for name in ["get", "take"] {
        let eff = *summaries
            .get(name)
            .unwrap_or_else(|| panic!("no effect summary for `{name}`"));
        assert!(
            eff.contains(Effect::Panic),
            "`{name}` must carry Panic (it can panic)"
        );
        assert!(
            !eff.contains(Effect::Managed),
            "`{name}` must not carry Managed: the panic path never touches managed memory"
        );
        assert!(
            !eff.contains(Effect::Alloc),
            "`{name}` must not carry Alloc: the panic path allocates nothing"
        );
    }

    let witness = *summaries
        .get("managed_witness")
        .expect("no effect summary for `managed_witness`");
    assert!(
        witness.contains(Effect::Managed) && witness.contains(Effect::Alloc),
        "the harness must be able to observe Managed/Alloc, else the assertions above are vacuous"
    );
}

const MANAGED_SYMBOLS: &[&str] = &[
    "__aelys_alloc",
    "aelys_immix_alloc",
    "__aelys_rc_retain",
    "__aelys_rc_release",
    "__aelys_arc_retain",
    "__aelys_arc_release",
    "__aelys_vec_",
    "@malloc",
    "@free",
];

fn function_bodies(ir: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut name: Option<String> = None;
    let mut body = String::new();
    for line in ir.lines() {
        if line.starts_with("define") {
            let sym = line
                .split('@')
                .nth(1)
                .and_then(|rest| rest.split('(').next())
                .unwrap_or_default()
                .to_string();
            name = Some(sym);
            body.clear();
            continue;
        }
        if name.is_some() {
            if line == "}" {
                out.push((name.take().expect("open define"), std::mem::take(&mut body)));
            } else {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    out
}

fn panic_blocks(body: &str) -> Vec<Vec<String>> {
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in body.lines() {
        if !line.starts_with(char::is_whitespace) && line.contains(':') {
            blocks.push(std::mem::take(&mut current));
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        current.push(trimmed.to_string());
    }
    blocks.push(current);
    blocks
        .into_iter()
        .filter(|b| b.iter().any(|l| l.contains("__aelys_panic")))
        .collect()
}

// assert every panic block in `ir` is the panic call plus unreachable, and return how many there were.
fn assert_panic_blocks_are_bare(label: &str, ir: &str, bodies: &[(String, String)]) -> usize {
    let mut seen = 0;
    for (name, body) in bodies {
        for block in panic_blocks(body) {
            seen += 1;
            assert_eq!(
                block.len(),
                2,
                "[{label}] @{name}: the panic block must be exactly the panic call and unreachable, got {block:?}"
            );
            let call = block[0].strip_prefix("tail ").unwrap_or(&block[0]);
            assert!(
                call.starts_with("call void @__aelys_panic(ptr "),
                "[{label}] @{name}: the panic block must call only __aelys_panic, got {:?}",
                block[0]
            );
            assert_eq!(
                block[1], "unreachable",
                "[{label}] @{name}: the panic call must be terminal (abort, no unwinding), got {:?}",
                block[1]
            );

            let global = block[0]
                .split_whitespace()
                .find(|t| t.starts_with("@str"))
                .map(|t| t.trim_end_matches(','))
                .unwrap_or_else(|| {
                    panic!("[{label}] @{name}: no message global in {:?}", block[0])
                });
            assert!(
                ir.contains(&format!("{global} = private constant")),
                "[{label}] @{name}: the panic message {global} must be a private constant \
                 (static metadata, no dynamic formatting)"
            );
        }
    }
    seen
}

fn assert_no_managed_symbol(label: &str, ir: &str) {
    for symbol in MANAGED_SYMBOLS {
        assert!(
            !ir.contains(symbol),
            "[{label}] the module must not reference `{symbol}`: nothing managed is on the panic path"
        );
    }
    assert!(
        ir.contains("declare void @__aelys_panic(ptr, i64)"),
        "[{label}] the panic runtime entry must be the static (ptr, len) form"
    );
}

#[test]
fn emitted_panic_path_is_only_aelys_panic_on_a_static_literal() {
    for (label, src, fn_name) in [
        ("oob index", slice_index_src(7), "get"),
        ("unwrap on err", unwrap_src("Result::Err(E::X)"), "take"),
    ] {
        let ir = emit_ir(&src, OptimizationLevel::None);
        let bodies = function_bodies(&ir);
        let nogc_fn = bodies
            .iter()
            .find(|(name, _)| name == fn_name)
            .unwrap_or_else(|| panic!("[{label}] no @{fn_name} in the emitted ir:\n{ir}"));
        assert!(
            !panic_blocks(&nogc_fn.1).is_empty(),
            "[{label}] @{fn_name} must contain a panic block, else this test is vacuous:\n{}",
            nogc_fn.1
        );
        assert!(assert_panic_blocks_are_bare(label, &ir, &bodies) > 0);
        assert_no_managed_symbol(label, &ir);
    }
}

// the optimizer must not put anything managed on the panic path either.
#[test]
fn optimized_panic_path_stays_bare() {
    for (label, src) in [
        ("oob index O2", slice_index_src(7)),
        ("unwrap on err O2", unwrap_src("Result::Err(E::X)")),
    ] {
        let ir = emit_ir(&src, OptimizationLevel::Standard);
        let bodies = function_bodies(&ir);
        assert!(
            assert_panic_blocks_are_bare(label, &ir, &bodies) > 0,
            "[{label}] the optimized module must still contain a panic block"
        );
        assert_no_managed_symbol(label, &ir);
    }
}

// an out-of-bounds index inside a nogc fn aborts, at every optimization level.
#[test]
fn nogc_out_of_bounds_index_aborts() {
    for level in [OptimizationLevel::None, OptimizationLevel::Standard] {
        let Some(out) = run(&slice_index_src(7), level) else {
            return;
        };
        assert_aborted(&format!("oob {level:?}"), &out);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("index out of bounds"),
            "[oob {level:?}] the static bounds-check literal must reach stderr, got: {stderr}"
        );
    }
}

// the twin: the same program in bounds returns its value, so the abort above is the index.
#[test]
fn nogc_in_bounds_index_returns_the_value() {
    for level in [OptimizationLevel::None, OptimizationLevel::Standard] {
        let Some(out) = run(&slice_index_src(1), level) else {
            return;
        };
        assert_eq!(
            out.status.code(),
            Some(20),
            "[in bounds {level:?}] get(arr[..], 1) must return 20; stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn nogc_unwrap_on_err_aborts() {
    let Some(out) = run(&unwrap_src("Result::Err(E::X)"), OptimizationLevel::None) else {
        return;
    };
    assert_aborted("unwrap err", &out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("called .unwrap() on an Err value"),
        "the static unwrap literal must reach stderr, got: {stderr}"
    );
}

#[test]
fn nogc_unwrap_on_ok_returns_the_payload() {
    let Some(out) = run(&unwrap_src("Result::Ok(7)"), OptimizationLevel::None) else {
        return;
    };
    assert_eq!(
        out.status.code(),
        Some(7),
        "take(Result::Ok(7)) must return 7; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn find_core_archive(file: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, file: &str) -> Option<PathBuf> {
        for entry in fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, file) {
                    return Some(found);
                }
            } else if path.file_name().and_then(|s| s.to_str()) == Some(file) {
                return Some(path);
            }
        }
        None
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap_or(manifest);
    walk(&root.join("target"), file)
}

fn asan_run(src: &str) -> Option<Output> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                skip("native linker unavailable (asan)");
                return None;
            }
            panic!("compilation should succeed: {err}");
        }
    }

    let object = source_path.with_extension(if cfg!(windows) { "obj" } else { "o" });
    if !object.is_file() {
        skip("object not produced (asan)");
        return None;
    }
    let Some(archive) = find_core_archive("libaelys-core.a") else {
        skip("core archive not found (asan)");
        return None;
    };
    let lib_dir = archive.parent().expect("archive has a parent");

    let asan_exe = dir.path().join("module_asan");
    let link = Command::new("clang")
        .arg("-fsanitize=address")
        .arg("-g")
        .arg(&object)
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-laelys-core")
        .arg("-o")
        .arg(&asan_exe)
        .output();
    let link = match link {
        Ok(out) => out,
        Err(_) => {
            skip("clang unavailable (asan)");
            return None;
        }
    };
    if !link.status.success() {
        skip(&format!(
            "asan link failed:\n{}",
            String::from_utf8_lossy(&link.stderr)
        ));
        return None;
    }

    Some(
        Command::new(&asan_exe)
            .env("ASAN_OPTIONS", "detect_leaks=1")
            .output()
            .expect("run asan-instrumented exe"),
    )
}

fn assert_asan_clean(label: &str, out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "[{label}] asan must report nothing; stderr:\n{stderr}"
    );
}

// the bounds check fires before the out-of-bounds read, so asan sees the abort and no bad access.
#[test]
fn nogc_panic_path_is_asan_clean() {
    if let Some(out) = asan_run(&slice_index_src(7)) {
        assert_aborted("asan oob", &out);
        assert_asan_clean("asan oob", &out);
    }
    if let Some(out) = asan_run(&slice_index_src(1)) {
        assert_eq!(
            out.status.code(),
            Some(20),
            "[asan in bounds] must still return 20; stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_asan_clean("asan in bounds", &out);
    }
}

