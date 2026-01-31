use crate::cli::vm_config::parse_vm_args_or_error;
use aelys_common::{WarningConfig, format_warnings};
use aelys_driver::run_file_full;
use aelys_modules::manifest::Manifest;
use aelys_opt::OptimizationLevel;
use aelys_runtime::VM;
use aelys_runtime::native::NativeLoader;
use aelys_syntax::{ImportKind, NeedsStmt, Source, Span};
use semver::{Version, VersionReq};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn run_with_options(
    path: &str,
    program_args: Vec<String>,
    vm_args: Vec<String>,
    opt_level: OptimizationLevel,
    warn_config: WarningConfig,
) -> Result<i32, String> {
    let parsed = parse_vm_args_or_error(&vm_args)?;
    let config = parsed.config;

    let path_ref = Path::new(path);
    let value = match detect_format(path_ref)? {
        InputFormat::Assembly => run_aasm_file(path_ref, config, program_args)?,
        InputFormat::Bytecode => run_avbc_file(path_ref, config, program_args)?,
        InputFormat::Source => {
            ensure_utf8_source(path_ref)?;
            let result = run_file_full(path_ref, config, program_args, opt_level)
                .map_err(|err| err.to_string())?;

            let filtered: Vec<_> = result.warnings.iter()
                .filter(|w| warn_config.is_enabled(&w.kind))
                .collect();

            for w in &filtered {
                eprintln!("{}", format_warnings(std::slice::from_ref(*w)));
            }

            if warn_config.treat_as_error && !filtered.is_empty() {
                return Err(format!("{} warning(s) treated as errors", filtered.len()));
            }

            result.value
        }
    };

    if !value.is_null() {
        println!("{}", value.to_string());
    }
    Ok(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputFormat {
    Source,
    Assembly,
    Bytecode,
}

fn detect_format(path: &Path) -> Result<InputFormat, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "aasm" {
        return Ok(InputFormat::Assembly);
    }

    if is_bytecode(path)? {
        return Ok(InputFormat::Bytecode);
    }

    if ext == "avbc" {
        return Err(format!("{} is not valid bytecode (VBXQ)", path.display()));
    }

    Ok(InputFormat::Source)
}

fn is_bytecode(path: &Path) -> Result<bool, String> {
    // VBXQ magic bytes
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
    let mut magic = [0u8; 4];
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == aelys_bytecode::asm::binary::MAGIC),
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(err) => Err(format!("failed to read {}: {}", path.display(), err)),
    }
}

fn run_aasm_file(
    path: &Path,
    config: aelys_runtime::VmConfig,
    program_args: Vec<String>,
) -> Result<aelys_runtime::Value, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let (functions, mut heap) =
        aelys_bytecode::asm::assemble(&content).map_err(|err| err.to_string())?;
    if functions.is_empty() {
        return Err("no functions found in assembly file".to_string());
    }

    let mut function = reconstruct_function_hierarchy(functions);

    let src = Source::new(path.display().to_string(), "");
    let mut vm = VM::with_config_and_args(src.clone(), config, program_args)
        .map_err(|err| err.to_string())?;
    if let Ok(abs_path) = path.canonicalize() {
        vm.set_script_path(abs_path.display().to_string());
    } else {
        vm.set_script_path(path.display().to_string());
    }

    let required_modules = collect_required_modules(&function);
    load_required_modules(&mut vm, path, src, &required_modules, None, &HashMap::new())?;

    let remap = vm.merge_heap(&mut heap).map_err(|err| err.to_string())?;
    function.remap_constants(&remap);

    let func_ref = vm.alloc_function(function).map_err(|err| err.to_string())?;
    vm.execute(func_ref).map_err(|err| err.to_string())
}

fn run_avbc_file(
    path: &Path,
    config: aelys_runtime::VmConfig,
    program_args: Vec<String>,
) -> Result<aelys_runtime::Value, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let (mut function, mut heap, manifest_bytes, bundles) =
        aelys_bytecode::asm::deserialize_with_manifest(&bytes).map_err(|err| err.to_string())?;

    let manifest = match manifest_bytes.as_deref() {
        Some(bytes) => Some(Manifest::from_bytes(bytes).map_err(|err| err.to_string())?),
        None => None,
    };

    let bundled_modules: HashMap<String, aelys_bytecode::asm::NativeBundle> =
        bundles.into_iter().map(|b| (b.name.clone(), b)).collect();

    let src = Source::new(path.display().to_string(), "");
    let mut vm = VM::with_config_and_args(src.clone(), config, program_args)
        .map_err(|err| err.to_string())?;
    if let Ok(abs_path) = path.canonicalize() {
        vm.set_script_path(abs_path.display().to_string());
    } else {
        vm.set_script_path(path.display().to_string());
    }

    let required_modules = collect_required_modules(&function);
    load_required_modules(
        &mut vm,
        path,
        src,
        &required_modules,
        manifest.as_ref(),
        &bundled_modules,
    )?;

    let remap = vm.merge_heap(&mut heap).map_err(|err| err.to_string())?;
    function.remap_constants(&remap);

    let func_ref = vm.alloc_function(function).map_err(|err| err.to_string())?;
    vm.execute(func_ref).map_err(|err| err.to_string())
}

fn collect_required_modules(function: &aelys_bytecode::Function) -> HashSet<String> {
    let mut modules = HashSet::new();
    collect_required_modules_rec(function, &mut modules);
    modules
}

fn collect_required_modules_rec(
    function: &aelys_bytecode::Function,
    modules: &mut HashSet<String>,
) {
    for name in function.global_layout.names() {
        if let Some(module_name) = name.split("::").next() {
            if name.contains("::") {
                modules.insert(module_name.to_string());
            }
        }
    }
    for nested in &function.nested_functions {
        collect_required_modules_rec(nested, modules);
    }
}

fn load_required_modules(
    vm: &mut VM,
    entry_path: &Path,
    source: std::sync::Arc<Source>,
    modules: &HashSet<String>,
    manifest: Option<&Manifest>,
    bundled_modules: &HashMap<String, aelys_bytecode::asm::NativeBundle>,
) -> Result<(), String> {
    let mut loader = aelys_driver::modules::ModuleLoader::with_manifest(
        entry_path,
        source.clone(),
        manifest.cloned(),
    );

    for module_name in modules {
        if let Some(bundle) = bundled_modules.get(module_name) {
            load_bundled_module(vm, module_name, bundle, manifest)?;
            continue;
        }

        if try_load_std_module(vm, &mut loader, module_name).is_ok() {
            continue;
        }

        let needs = NeedsStmt {
            path: vec![module_name.clone()],
            kind: ImportKind::Module { alias: None },
            span: Span::dummy(),
        };
        loader
            .load_module(&needs, vm)
            .map_err(|err| err.to_string())?;
    }

    Ok(())
}

fn try_load_std_module(
    vm: &mut VM,
    loader: &mut aelys_driver::modules::ModuleLoader,
    module_name: &str,
) -> Result<(), String> {
    let needs = NeedsStmt {
        path: vec!["std".to_string(), module_name.to_string()],
        kind: ImportKind::Module { alias: None },
        span: Span::dummy(),
    };
    loader
        .load_module(&needs, vm)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn load_bundled_module(
    vm: &mut VM,
    module_name: &str,
    bundle: &aelys_bytecode::asm::NativeBundle,
    manifest: Option<&Manifest>,
) -> Result<(), String> {
    if let Some(policy) = manifest.and_then(|m| m.module(module_name)) {
        if !policy.capabilities.is_empty() {
            if let Err(denied) = vm.config().check_native_capabilities(&policy.capabilities) {
                return Err(format!(
                    "native capability denied for {}: {}",
                    module_name, denied
                ));
            }
        }

        if let Some(expected) = &policy.checksum {
            let actual = compute_simple_hash(&bundle.bytes);
            if &actual != expected {
                return Err(format!(
                    "native checksum mismatch for {} (expected {}, got {})",
                    module_name, expected, actual
                ));
            }
        }
    }

    let loader = NativeLoader::new();
    let native_module = loader
        .load_embedded(&bundle.name, &bundle.bytes)
        .map_err(|err| err.to_string())?;

    if let Some(policy) = manifest.and_then(|m| m.module(module_name)) {
        if let Some(required) = &policy.required_version {
            let ok = match (&native_module.version, VersionReq::parse(required)) {
                (Some(found), Ok(req)) => Version::parse(found)
                    .map(|v| req.matches(&v))
                    .unwrap_or(false),
                _ => false,
            };
            if !ok {
                return Err(format!(
                    "native version mismatch for {} (required {}, found {:?})",
                    module_name, required, native_module.version
                ));
            }
        }
    }

    register_native_module(&native_module, module_name, vm)?;
    vm.register_native_module(module_name.to_string(), native_module);
    Ok(())
}

fn register_native_module(
    native_module: &aelys_modules::native::NativeModule,
    module_alias: &str,
    vm: &mut VM,
) -> Result<(), String> {
    use aelys_native::AelysExportKind;

    for (name, export) in &native_module.exports {
        let qualified_name = format!("{}::{}", module_alias, name);
        match export.kind {
            AelysExportKind::Function => {
                if export.value.is_null() {
                    return Err(format!("null function pointer for {}", name));
                }
                let func = unsafe {
                    std::mem::transmute::<*const std::ffi::c_void, aelys_native::AelysNativeFn>(
                        export.value,
                    )
                };
                let func_ref = vm
                    .alloc_foreign(&qualified_name, export.arity, func)
                    .map_err(|err| err.to_string())?;
                vm.set_global(qualified_name, aelys_runtime::Value::ptr(func_ref.index()));
            }
            AelysExportKind::Constant => {
                if export.value.is_null() {
                    return Err(format!("null constant pointer for {}", name));
                }
                let raw = unsafe { *(export.value as *const u64) };
                vm.set_global(qualified_name, aelys_runtime::Value::from_raw(raw));
            }
            AelysExportKind::Type => {
                vm.set_global(qualified_name, aelys_runtime::Value::null());
            }
        }
    }
    Ok(())
}

// FNV-1a, good enough for integrity checks
fn compute_simple_hash(data: &[u8]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

fn ensure_utf8_source(path: &Path) -> Result<(), String> {
    match std::fs::read_to_string(path) {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => Err(format!(
            "{} is not UTF-8 text and not bytecode (VBXQ)",
            path.display()
        )),
        Err(_) => Ok(()),
    }
}

fn reconstruct_function_hierarchy(
    mut functions: Vec<aelys_bytecode::Function>,
) -> aelys_bytecode::Function {
    if functions.len() <= 1 {
        return functions
            .into_iter()
            .next()
            .unwrap_or_else(|| aelys_bytecode::Function::new(None, 0));
    }

    let mut main_func = functions.remove(0);
    main_func.nested_functions = functions;
    main_func
}
