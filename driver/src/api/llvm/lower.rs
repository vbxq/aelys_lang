use aelys_common::error::{AelysError, Fault};
use aelys_opt::OptimizationLevel;
use aelys_syntax::Source;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::core_lib::resolve_aelys_core_lib;
use super::diagnostics::{
    backend_diagnostic_error, llvm_backend_error_to_diagnostic, program_anchor_span,
};
use super::link::{LinkRequirement, link_native_executable, linked_library_claims_runtime_symbol};
use super::runtime::RuntimeVariant;

pub(super) fn compile_air_with_llvm(
    path: &Path,
    air: &aelys_air::AirProgram,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    source: Arc<Source>,
    require: &[String],
) -> Result<(), AelysError> {
    compile_air_with_llvm_linked(
        path,
        air,
        opt_level,
        emit_llvm_ir,
        runtime,
        source,
        &LinkRequirement::default(),
        None,
        ObjectPath::Beside,
        None,
        require,
    )
    .map(|_| ())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompiledArtifact {
    Executable(PathBuf),
    Object(PathBuf),
    LlvmIr(PathBuf),
}

pub(super) enum ObjectPath<'a> {
    Beside,
    Private,
    At(&'a Path),
}

pub(super) fn compile_air_with_llvm_linked(
    path: &Path,
    air: &aelys_air::AirProgram,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    source: Arc<Source>,
    link: &LinkRequirement,
    output: Option<&Path>,
    object: ObjectPath<'_>,
    core_lib: Option<&Path>,
    require: &[String],
) -> Result<CompiledArtifact, AelysError> {
    let module_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aelys_module");
    let polled;
    let air = if runtime == RuntimeVariant::RcCycles {
        let mut with_polls = air.clone();
        aelys_air::passes::safepoints::insert_cycle_polls(&mut with_polls);
        polled = with_polls;
        &polled
    } else {
        air
    };
    let windows = aelys_air::passes::safepoints::release_store_windows(air).violations;
    if !windows.is_empty() {
        return Err(backend_diagnostic_error(
            source.clone(),
            program_anchor_span(air, source.as_ref()),
            "air-validation",
            windows.join("\n"),
            None,
            None,
            Fault::Compiler,
        ));
    }
    let mut codegen = aelys_codegen::CodegenContext::new(module_name);

    codegen
        .compile(air)
        .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;
    codegen
        .optimize(opt_level.llvm_pass_pipeline(), opt_level.numeric())
        .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;

    let has_main_entry = air
        .functions
        .iter()
        .any(|function| !function.is_extern && function.name == "main");

    let target = if emit_llvm_ir {
        requested(output, || ir_path_for(path))
    } else if has_main_entry {
        requested(output, || executable_path_for(path))
    } else {
        requested(output, || object_path_for(path))
    };
    if same_file(path, &target) {
        return Err(backend_diagnostic_error(
            source.clone(),
            program_anchor_span(air, source.as_ref()),
            "output",
            format!(
                "the output `{}` is the source file itself, and writing it would destroy the \
                 source",
                target.display()
            ),
            None,
            Some("name another output with `-o`".to_string()),
            Fault::Environment,
        ));
    }

    if emit_llvm_ir {
        let ir_path = target;
        let ir_path_str = ir_path.to_string_lossy().to_string();
        codegen
            .emit_ir(&ir_path_str)
            .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;
        return Ok(CompiledArtifact::LlvmIr(ir_path));
    }

    let exe_path = target;
    let keep_object = matches!(object, ObjectPath::Beside);
    let object_path = match (has_main_entry, object) {
        (false, _) => exe_path.clone(),
        (true, ObjectPath::Beside) => object_path_for(path),
        (true, ObjectPath::At(given)) => given.to_path_buf(),
        (true, ObjectPath::Private) => private_object_path().map_err(|message| {
            backend_diagnostic_error(
                source.clone(),
                program_anchor_span(air, source.as_ref()),
                "llvm-linker",
                message,
                None,
                None,
                Fault::Environment,
            )
        })?,
    };
    let object_path_str = object_path.to_string_lossy().to_string();
    codegen
        .emit_object(&object_path_str, opt_level.numeric())
        .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;

    if !has_main_entry {
        return Ok(CompiledArtifact::Object(object_path));
    }
    let linked = (|| {
        let anchor = program_anchor_span(air, source.as_ref());
        let core_lib = match core_lib {
            Some(given) => given.to_path_buf(),
            None => resolve_aelys_core_lib(runtime).map_err(|message| {
                backend_diagnostic_error(
                    source.clone(),
                    anchor,
                    "llvm-linker",
                    message,
                    None,
                    None,
                    Fault::Environment,
                )
            })?,
        };
        link_native_executable(&object_path, &exe_path, &core_lib, runtime, link, require).map_err(
            |message| {
                backend_diagnostic_error(
                    source.clone(),
                    anchor,
                    "llvm-linker",
                    message,
                    None,
                    foreign_declaration_help(air, source.as_ref()),
                    // linker stderr cannot separate a missing library from a broken object
                    Fault::Environment,
                )
            },
        )?;
        if let Some(symbol) = linked_library_claims_runtime_symbol(&exe_path, &core_lib, link) {
            // the executable is the evidence, so it must not survive the verdict
            let _ = std::fs::remove_file(&exe_path);
            return Err(claimed_runtime_symbol_error(
                source.clone(),
                anchor,
                symbol,
                link,
            ));
        }
        Ok(())
    })();
    if !keep_object {
        let _ = std::fs::remove_file(&object_path);
    }
    linked.map(|()| CompiledArtifact::Executable(exe_path))
}

// the object of a linked program never takes a name a user file could already hold
fn private_object_path() -> Result<PathBuf, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir();
    let extension = if cfg!(windows) { "obj" } else { "o" };
    for _ in 0..64 {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let candidate = dir.join(format!(
            "aelys-{}-{}-{}.{}",
            std::process::id(),
            nanos,
            n,
            extension
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!(
                    "cannot create a temporary object file in `{}`: {}",
                    dir.display(),
                    err
                ));
            }
        }
    }
    Err(format!(
        "cannot create a temporary object file in `{}`",
        dir.display()
    ))
}

fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(left), Ok(right)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
    }
}

fn claimed_runtime_symbol_error(
    source: Arc<Source>,
    span: aelys_syntax::Span,
    symbol: String,
    link: &LinkRequirement,
) -> AelysError {
    AelysError::Compile(aelys_common::error::CompileError::new(
        aelys_common::error::CompileErrorKind::LinkedLibraryClaimsRuntimeSymbol {
            symbol,
            libraries: link.libraries.clone(),
        },
        span,
        source,
    ))
}

fn foreign_declaration_help(air: &aelys_air::AirProgram, source: &Source) -> Option<String> {
    let mut sites = Vec::new();
    for function in air.functions.iter().filter(|function| function.is_extern) {
        let line = function
            .span
            .map(|span| source.line_col_at_offset(span.lo as usize).0)
            .unwrap_or(0);
        sites.push(format!("`{}` at line {}", function.name, line));
    }
    if sites.is_empty() {
        return None;
    }
    Some(format!(
        "an external function is resolved at link time; add `-L <dir> -l <name>` for the library \
         that defines it. this program declares {}",
        sites.join(", ")
    ))
}

fn requested(output: Option<&Path>, default: impl FnOnce() -> PathBuf) -> PathBuf {
    output.map(Path::to_path_buf).unwrap_or_else(default)
}

pub fn ir_path_for(path: &Path) -> PathBuf {
    let mut ir_path = PathBuf::from(path);
    ir_path.set_extension("ll");
    ir_path
}

pub fn object_path_for(path: &Path) -> PathBuf {
    let mut object = path.to_path_buf();
    object.set_extension(if cfg!(windows) { "obj" } else { "o" });
    object
}

pub fn executable_path_for(path: &Path) -> PathBuf {
    let mut output = path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}
