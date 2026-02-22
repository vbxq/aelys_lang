use crate::modules::load_modules_with_loader;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{VM, VmConfig};
use aelys_syntax::{Source, StmtKind};
use std::path::{Path, PathBuf};
#[cfg(feature = "llvm-backend")]
use std::process::Command;

const BUILTIN_NAMES: &[&str] = &["alloc", "free", "load", "store", "type"];

pub fn lower_file_to_air(
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<aelys_air::AirProgram, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;

    let name = path.display().to_string();
    let src = Source::new(&name, &content);

    let tokens = Lexer::with_source(src.clone())
        .scan()
        .map_err(|err| err.to_string())?;
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .map_err(|err| err.to_string())?;

    let mut vm = VM::with_config_and_args(src.clone(), VmConfig::default(), Vec::new())
        .map_err(|err| err.to_string())?;
    if let Ok(abs_path) = path.canonicalize() {
        vm.set_script_path(abs_path.display().to_string());
    } else {
        vm.set_script_path(path.display().to_string());
    }

    let (imports, _) = load_modules_with_loader(&stmts, path, src.clone(), &mut vm)
        .map_err(|err| err.to_string())?;

    let main_stmts: Vec<_> = stmts
        .into_iter()
        .filter(|stmt| !matches!(stmt.kind, StmtKind::Needs(_)))
        .collect();

    let mut all_known_globals = imports.known_globals.clone();
    for builtin in BUILTIN_NAMES {
        all_known_globals.insert(builtin.to_string());
    }

    let typed_program = aelys_sema::TypeInference::infer_program_with_imports(
        main_stmts,
        src,
        imports.module_aliases,
        all_known_globals,
    )
    .map_err(|errors| {
        errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Unknown type error".to_string())
    })?;

    let mut optimizer = Optimizer::new(opt_level);
    let typed_program = optimizer.optimize(typed_program);

    let mut air = aelys_air::lower::lower(&typed_program);
    aelys_air::layout::compute_layouts(&mut air);
    let mut air = aelys_air::mono::monomorphize(air);
    aelys_air::passes::copy_elim::eliminate_copies(&mut air);
    aelys_air::passes::dead_locals::eliminate_dead_locals(&mut air);
    Ok(air)
}

pub fn compile_file_with_llvm(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
) -> Result<(), String> {
    let air = lower_file_to_air(path, opt_level)?;
    compile_air_with_llvm(path, &air, emit_llvm_ir)
}

#[cfg(feature = "llvm-backend")]
fn compile_air_with_llvm(
    path: &Path,
    air: &aelys_air::AirProgram,
    emit_llvm_ir: bool,
) -> Result<(), String> {
    let module_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aelys_module");
    let mut codegen = aelys_codegen::CodegenContext::new(module_name);

    codegen.compile(air).map_err(|err| format!("{:?}", err))?;
    let object_path = object_path_for(path);
    let object_path_str = object_path.to_string_lossy().to_string();
    codegen
        .emit_object(&object_path_str)
        .map_err(|err| format!("{:?}", err))?;

    if emit_llvm_ir {
        let mut ir_path = PathBuf::from(path);
        ir_path.set_extension("ll");
        let ir_path_str = ir_path.to_string_lossy().to_string();
        codegen
            .emit_ir(&ir_path_str)
            .map_err(|err| format!("{:?}", err))?;
    }

    let has_main_entry = air
        .functions
        .iter()
        .any(|function| !function.is_extern && function.name == "main");
    if has_main_entry {
        let core_lib = resolve_aelys_core_lib()?;
        let exe_path = executable_path_for(path);
        link_native_executable(&object_path, &exe_path, &core_lib)?;
    }

    Ok(())
}

#[cfg(feature = "llvm-backend")]
fn object_path_for(path: &Path) -> PathBuf {
    let mut object = path.to_path_buf();
    object.set_extension(if cfg!(windows) { "obj" } else { "o" });
    object
}

#[cfg(feature = "llvm-backend")]
fn executable_path_for(path: &Path) -> PathBuf {
    let mut output = path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

#[cfg(feature = "llvm-backend")]
fn resolve_aelys_core_lib() -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("AELYS_CORE_LIB") {
        let path = PathBuf::from(&raw);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "AELYS_CORE_LIB points to a missing file: {}",
            path.display()
        ));
    }

    let roots = candidate_search_roots();

    for root in &roots {
        if let Some(path) = find_aelys_core_lib(root) {
            return Ok(path);
        }
    }

    for root in &roots {
        let _ = build_aelys_core(root);
    }

    for root in &roots {
        if let Some(path) = find_aelys_core_lib(root) {
            return Ok(path);
        }
    }

    let searched = roots
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "could not locate aelys-core static library. Set AELYS_CORE_LIB or build `aelys-core`.\nsearched:\n{}",
        searched
    ))
}

#[cfg(feature = "llvm-backend")]
fn find_aelys_core_lib(root: &Path) -> Option<PathBuf> {
    for profile_dir in core_profile_dirs(root) {
        if let Some(path) = find_core_lib_in_build_out(&profile_dir.join("build")) {
            return Some(path);
        }
    }

    for candidate in exact_core_lib_candidates(root) {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(feature = "llvm-backend")]
fn exact_core_lib_candidates(root: &Path) -> Vec<PathBuf> {
    const NAMES: &[&str] = &["aelys-core.lib", "libaelys-core.a"];

    let mut candidates = Vec::with_capacity(NAMES.len() * 2);
    for name in NAMES {
        candidates.push(root.join(name));
        candidates.push(root.join("deps").join(name));
    }
    candidates
}

#[cfg(feature = "llvm-backend")]
fn is_core_lib_name(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    matches!(
        file_name.to_ascii_lowercase().as_str(),
        "aelys-core.lib" | "libaelys-core.a"
    )
}

#[cfg(feature = "llvm-backend")]
fn find_core_lib_in_dir(dir: &Path) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_core_lib_name(&path) {
            return Some(path);
        }
    }
    None
}

#[cfg(feature = "llvm-backend")]
fn find_core_lib_in_build_out(build_dir: &Path) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(build_dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let out_dir = entry.path().join("out");
        if let Some(path) = find_core_lib_in_dir(&out_dir) {
            return Some(path);
        }
    }
    None
}

#[cfg(feature = "llvm-backend")]
fn core_profile_dirs(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("target").join("debug"),
        root.join("target").join("release"),
    ]
}

#[cfg(feature = "llvm-backend")]
fn candidate_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(raw) = std::env::var("CARGO_MANIFEST_DIR") {
        push_unique(&mut roots, PathBuf::from(raw));
    }

    if let Some(raw) = option_env!("CARGO_MANIFEST_DIR") {
        let manifest = PathBuf::from(raw);
        push_unique(&mut roots, manifest.clone());
        if let Some(parent) = manifest.parent() {
            push_unique(&mut roots, parent.to_path_buf());
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push_unique(&mut roots, dir.to_path_buf());
            if let Some(parent) = dir.parent() {
                push_unique(&mut roots, parent.to_path_buf());
                if let Some(grand_parent) = parent.parent() {
                    push_unique(&mut roots, grand_parent.to_path_buf());
                }
            }
        }
    }

    if let Ok(current) = std::env::current_dir() {
        push_unique(&mut roots, current);
    }

    roots
}

#[cfg(feature = "llvm-backend")]
fn push_unique(items: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !items.iter().any(|existing| existing == &candidate) {
        items.push(candidate);
    }
}

#[cfg(feature = "llvm-backend")]
fn build_aelys_core(root: &Path) -> Result<(), String> {
    if !root.join("Cargo.toml").is_file() {
        return Err(format!(
            "no Cargo.toml in candidate root {}",
            root.display()
        ));
    }

    let mut args = vec!["build".to_string(), "-p".to_string(), "aelys-core".to_string()];
    if running_release_binary() {
        args.push("--release".to_string());
    }

    run_process_in_dir("cargo", &args, Some(root))
}

#[cfg(feature = "llvm-backend")]
fn running_release_binary() -> bool {
    if let Ok(exe) = std::env::current_exe() {
        return exe
            .components()
            .any(|component| component.as_os_str() == "release");
    }
    false
}

#[cfg(feature = "llvm-backend")]
fn link_native_executable(
    object_path: &Path,
    exe_path: &Path,
    core_lib: &Path,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        link_windows(object_path, exe_path, core_lib)
    }
    #[cfg(not(windows))]
    {
        link_unix(object_path, exe_path, core_lib)
    }
}

#[cfg(feature = "llvm-backend")]
#[cfg(windows)]
fn link_windows(object_path: &Path, exe_path: &Path, core_lib: &Path) -> Result<(), String> {
    let obj = object_path.to_string_lossy().to_string();
    let exe = exe_path.to_string_lossy().to_string();
    let lib = core_lib.to_string_lossy().to_string();

    let link_args = vec![
        "/NOLOGO".to_string(),
        "/SUBSYSTEM:CONSOLE".to_string(),
        format!("/OUT:{}", exe),
        obj,
        lib,
        "msvcrt.lib".to_string(),
        "kernel32.lib".to_string(),
    ];

    let mut errors = Vec::new();
    for linker in windows_linkers() {
        match run_process(&linker, &link_args) {
            Ok(()) => return Ok(()),
            Err(err) => errors.push(err),
        }
    }

    Err(errors.join("\n"))
}

#[cfg(feature = "llvm-backend")]
#[cfg(not(windows))]
fn link_unix(object_path: &Path, exe_path: &Path, core_lib: &Path) -> Result<(), String> {
    let obj = object_path.to_string_lossy().to_string();
    let exe = exe_path.to_string_lossy().to_string();
    let lib_dir = core_lib
        .parent()
        .ok_or_else(|| format!("invalid aelys-core path: {}", core_lib.display()))?;

    let args = vec![
        "-o".to_string(),
        exe,
        obj,
        format!("-L{}", lib_dir.to_string_lossy()),
        "-laelys-core".to_string(),
    ];
    run_process("cc", &args)
}

#[cfg(feature = "llvm-backend")]
fn run_process(program: &str, args: &[String]) -> Result<(), String> {
    run_process_in_dir(program, args, None)
}

#[cfg(feature = "llvm-backend")]
fn run_process_in_dir(program: &str, args: &[String], dir: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(path) = dir {
        command.current_dir(path);
    }

    let output = command
        .output()
        .map_err(|err| format!("failed to run `{}`: {}", program, err))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "`{}` failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        program,
        output.status.code(),
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(feature = "llvm-backend")]
#[cfg(windows)]
fn windows_linkers() -> Vec<String> {
    let mut linkers = Vec::new();

    if let Ok(prefix) = std::env::var("LLVM_SYS_181_PREFIX") {
        let candidate = PathBuf::from(prefix).join("bin").join("lld-link.exe");
        if candidate.is_file() {
            linkers.push(candidate.to_string_lossy().to_string());
        }
    }

    let default_lld = PathBuf::from(r"C:\llvm\bin\lld-link.exe");
    if default_lld.is_file() {
        let lld = default_lld.to_string_lossy().to_string();
        if !linkers.iter().any(|existing| existing == &lld) {
            linkers.push(lld);
        }
    }

    linkers.push("lld-link".to_string());
    linkers.push("link".to_string());
    linkers
}

#[cfg(not(feature = "llvm-backend"))]
fn compile_air_with_llvm(
    _path: &Path,
    _air: &aelys_air::AirProgram,
    _emit_llvm_ir: bool,
) -> Result<(), String> {
    Err("LLVM backend is not enabled in this build of aelys-driver!".to_string())
}
