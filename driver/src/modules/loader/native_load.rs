use super::super::checksum::compute_file_checksum;
use super::super::types::{ExportInfo, LoadedNativeInfo, ModuleInfo, ModuleLoader};
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_native::AelysExportKind;
use aelys_runtime::native::NativeLoader;
use aelys_runtime::{VM, Value};
use aelys_syntax::NeedsStmt;
use semver::{Version, VersionReq};
use std::path::Path;

// FIXME: the transmute for native functions is scary - maybe use typed wrappers?

impl ModuleLoader {
    pub(crate) fn load_native_module(
        &mut self,
        file_path: &Path,
        module_path_str: &str,
        needs: &NeedsStmt,
        vm: &mut VM,
    ) -> Result<()> {
        let module_name = needs
            .path
            .last()
            .cloned()
            .expect("needs.path validated as non-empty");
        if let Some(policy) = self.manifest.as_ref().and_then(|m| m.module(&module_name)) {
            if !policy.capabilities.is_empty()
                && let Err(denied_cap) = vm.config().check_native_capabilities(&policy.capabilities)
            {
                return Err(AelysError::Compile(CompileError::new(
                    CompileErrorKind::NativeCapabilityDenied {
                        module: module_path_str.to_string(),
                        capability: denied_cap,
                        required: policy.capabilities.clone(),
                    },
                    needs.span,
                    self.source.clone(),
                )));
            }

            if let Some(expected_checksum) = &policy.checksum {
                let actual_checksum = compute_file_checksum(file_path).map_err(|e| {
                    self.native_error(
                        module_path_str,
                        aelys_runtime::native::NativeError::Io(e),
                        needs.span,
                    )
                })?;
                if &actual_checksum != expected_checksum {
                    return Err(AelysError::Compile(CompileError::new(
                        CompileErrorKind::NativeChecksumMismatch {
                            module: module_path_str.to_string(),
                            expected: expected_checksum.clone(),
                            actual: actual_checksum,
                        },
                        needs.span,
                        self.source.clone(),
                    )));
                }
            }
        }

        let loader = NativeLoader::new();
        let native_module = loader
            .load_dynamic(module_path_str, file_path)
            .map_err(|err| self.native_error(module_path_str, err, needs.span))?;

        if let Some(policy) = self.manifest.as_ref().and_then(|m| m.module(&module_name))
            && let Some(required_version) = &policy.required_version
        {
            let version_ok = match (&native_module.version, VersionReq::parse(required_version)) {
                (Some(module_ver), Ok(req)) => match Version::parse(module_ver) {
                    Ok(ver) => req.matches(&ver),
                    Err(_) => false,
                },
                (None, _) => false,
                (_, Err(_)) => false,
            };
            if !version_ok {
                return Err(AelysError::Compile(CompileError::new(
                    CompileErrorKind::NativeVersionMismatch {
                        module: module_path_str.to_string(),
                        required: required_version.clone(),
                        found: native_module.version.clone(),
                    },
                    needs.span,
                    self.source.clone(),
                )));
            }
        }

        // call the module's init function if present passing the VM API
        if !native_module.descriptor.is_null() {
            let descriptor = unsafe { &*native_module.descriptor };
            if let Some(init_fn) = descriptor.init {
                let api = aelys_runtime::build_native_vm_api();
                init_fn(&api);
            }
        }

        self.load_native_dependencies(&native_module, module_path_str, needs, vm)?;

        let native_exports = native_module.exports.clone();
        let mut exports = std::collections::HashMap::new();
        let mut mutability = std::collections::HashMap::new();
        let mut native_functions = Vec::new();
        let module_alias = self.get_module_alias(needs);

        for (name, export) in native_exports {
            let is_function = matches!(export.kind, AelysExportKind::Function);
            exports.insert(
                name.clone(),
                ExportInfo {
                    is_function,
                    is_mutable: false,
                },
            );
            mutability.insert(name.clone(), false);

            match export.kind {
                AelysExportKind::Function => {
                    native_functions.push(format!("{}::{}", module_alias, name));

                    if export.value.is_null() {
                        return Err(self.native_error(
                            module_path_str,
                            aelys_runtime::native::NativeError::InvalidDescriptor(
                                "null function pointer",
                            ),
                            needs.span,
                        ));
                    }

                    let func = unsafe {
                        std::mem::transmute::<*const std::ffi::c_void, aelys_native::AelysNativeFn>(
                            export.value,
                        )
                    };

                    let display_name = format!("{}::{}", module_alias, name);
                    let func_ref = vm
                        .alloc_foreign(&display_name, export.arity, func)
                        .map_err(AelysError::Runtime)?;
                    vm.set_global(name, Value::ptr(func_ref.index()));
                }
                AelysExportKind::Constant => {
                    if export.value.is_null() {
                        return Err(self.native_error(
                            module_path_str,
                            aelys_runtime::native::NativeError::InvalidDescriptor(
                                "null constant pointer",
                            ),
                            needs.span,
                        ));
                    }
                    let raw = unsafe { *(export.value as *const u64) };
                    vm.set_global(name, Value::from_raw(raw));
                }
                AelysExportKind::Type => {
                    vm.set_global(name, Value::null());
                }
            }
        }

        vm.update_global_mutability(mutability);

        self.register_exports(needs, &exports, vm)?;

        let module_name = needs
            .path
            .last()
            .cloned()
            .expect("needs.path validated as non-empty");
        let module_info = ModuleInfo {
            name: module_name,
            path: module_path_str.to_string(),
            file_path: file_path.to_path_buf(),
            version: native_module.version.clone(),
            exports,
            native_functions,
        };

        self.loaded_modules
            .insert(module_path_str.to_string(), module_info);
        vm.register_native_module(module_path_str.to_string(), native_module);
        if let Some(fingerprint) = aelys_modules::loader::FileFingerprint::from_path(file_path) {
            self.native_fingerprints
                .insert(module_path_str.to_string(), fingerprint);
        }

        self.loaded_native_modules.insert(
            module_path_str.to_string(),
            LoadedNativeInfo {
                file_path: file_path.to_path_buf(),
                name: module_path_str.to_string(),
            },
        );

        Ok(())
    }
}
