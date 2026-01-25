use super::super::types::ModuleLoader;
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_runtime::VM;
use aelys_runtime::native::NativeModule;
use aelys_syntax::{ImportKind, NeedsStmt};
use semver::{Version, VersionReq};

impl ModuleLoader {
    pub(crate) fn load_native_dependencies(
        &mut self,
        native_module: &NativeModule,
        module_path_str: &str,
        needs: &NeedsStmt,
        vm: &mut VM,
    ) -> Result<()> {
        for dep in &native_module.required_modules {
            let name = dep.name.trim();
            if name.is_empty() {
                return Err(AelysError::Compile(CompileError::new(
                    CompileErrorKind::InvalidNativeModule {
                        module: module_path_str.to_string(),
                        reason: "empty dependency name".to_string(),
                    },
                    needs.span,
                    self.source.clone(),
                )));
            }
            let path: Vec<String> = name.split('.').map(|s| s.to_string()).collect();
            let dep_needs = NeedsStmt {
                path: path.clone(),
                kind: ImportKind::Module { alias: None },
                span: needs.span,
            };
            self.load_module(&dep_needs, vm)?;

            if let Some(version_req) = &dep.version_req {
                let dep_path_str = path.join(".");
                let dep_info = self.loaded_modules.get(&dep_path_str).ok_or_else(|| {
                    AelysError::Compile(CompileError::new(
                        CompileErrorKind::InvalidNativeModule {
                            module: module_path_str.to_string(),
                            reason: format!("dependency '{}' not loaded", dep_path_str),
                        },
                        needs.span,
                        self.source.clone(),
                    ))
                })?;
                let dep_version = dep_info.version.as_ref().ok_or_else(|| {
                    AelysError::Compile(CompileError::new(
                        CompileErrorKind::InvalidNativeModule {
                            module: module_path_str.to_string(),
                            reason: format!(
                                "dependency '{}' has no version metadata",
                                dep_path_str
                            ),
                        },
                        needs.span,
                        self.source.clone(),
                    ))
                })?;
                let req = VersionReq::parse(version_req).map_err(|_| {
                    AelysError::Compile(CompileError::new(
                        CompileErrorKind::InvalidNativeModule {
                            module: module_path_str.to_string(),
                            reason: format!(
                                "invalid version requirement '{}' for '{}'",
                                version_req, dep_path_str
                            ),
                        },
                        needs.span,
                        self.source.clone(),
                    ))
                })?;
                let version = Version::parse(dep_version).map_err(|_| {
                    AelysError::Compile(CompileError::new(
                        CompileErrorKind::InvalidNativeModule {
                            module: module_path_str.to_string(),
                            reason: format!(
                                "invalid version '{}' for dependency '{}'",
                                dep_version, dep_path_str
                            ),
                        },
                        needs.span,
                        self.source.clone(),
                    ))
                })?;
                if !req.matches(&version) {
                    return Err(AelysError::Compile(CompileError::new(
                        CompileErrorKind::InvalidNativeModule {
                            module: module_path_str.to_string(),
                            reason: format!(
                                "version conflict: dependency '{}' version {} does not satisfy {}",
                                dep_path_str, version, req
                            ),
                        },
                        needs.span,
                        self.source.clone(),
                    )));
                }
            }
        }

        Ok(())
    }
}
