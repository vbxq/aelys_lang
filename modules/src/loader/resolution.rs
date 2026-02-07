use super::types::ModuleLoader;
use crate::resolution::{
    ExtensionPattern, ModuleKind, ModuleResolution, full_search_patterns,
    native_only_search_patterns,
};
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_syntax::Span;
use std::path::{Path, PathBuf};

impl ModuleLoader {
    // Module resolution is a bit fiddly - we support:
    //   - foo.aelys (single file)
    //   - foo/mod.aelys (directory module)
    //   - foo.so/foo.dll (native)
    //   - std.* (builtin, handled specially)
    // TODO: support package.toml style dependencies from a central repo
    pub fn resolve_path(&self, module_path: &[String]) -> Result<PathBuf> {
        match self.resolve_module_path(module_path)? {
            ModuleResolution {
                kind: ModuleKind::Script,
                path,
            } => Ok(path),
            ModuleResolution { .. } => Err(AelysError::Compile(CompileError::new(
                CompileErrorKind::ModuleNotFound {
                    module_path: module_path.join("."),
                    searched_paths: vec![],
                },
                Span::dummy(),
                self.source.clone(),
            ))),
        }
    }

    fn resolve_module_path(&self, module_path: &[String]) -> Result<ModuleResolution> {
        // Standard library modules are handled separately
        if is_std_module(module_path) {
            // Return a special error that signals this is a std module
            // The caller will handle this case
            return Err(AelysError::Compile(CompileError::new(
                CompileErrorKind::StdlibNotAvailable {
                    module: module_path.join("."),
                },
                Span::dummy(),
                self.source.clone(),
            )));
        }

        let module_name_str = module_path.join(".");

        // Check manifest for explicit path hint first
        if let Some(manifest) = &self.manifest
            && let Some(policy) = manifest.module(&module_name_str)
        {
            if let Some(explicit_path) = &policy.path {
                let resolved_path = self.base_dir.join(explicit_path);
                if let Some(canonical) = self.canonicalize_if_exists(&resolved_path, module_path)? {
                    // Determine kind from manifest or file extension
                    let kind = if policy.is_native() {
                        ModuleKind::Native
                    } else if policy.is_script() {
                        ModuleKind::Script
                    } else {
                        // Infer from extension
                        let ext = resolved_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("");
                        if ext == "so" || ext == "dll" || ext == "dylib" || ext == "aelys-lib" {
                            ModuleKind::Native
                        } else {
                            ModuleKind::Script
                        }
                    };
                    return Ok(ModuleResolution {
                        path: canonical,
                        kind,
                    });
                }
            }

            // If manifest specifies kind=native, only search for native modules
            if policy.is_native() {
                return self.resolve_native_only(module_path);
            }
        }

        // Use the shared helper with full search patterns
        self.search_with_patterns(module_path, full_search_patterns())
    }

    fn canonicalize_if_exists(
        &self,
        path: &Path,
        module_path: &[String],
    ) -> Result<Option<PathBuf>> {
        if !path.exists() {
            return Ok(None);
        }
        let canonical = path.canonicalize().map_err(|_| {
            AelysError::Compile(CompileError::new(
                CompileErrorKind::ModuleNotFound {
                    module_path: module_path.join("."),
                    searched_paths: vec![path.display().to_string()],
                },
                Span::dummy(),
                self.source.clone(),
            ))
        })?;
        if !canonical.starts_with(&self.base_root) {
            return Err(AelysError::Compile(CompileError::new(
                CompileErrorKind::ModuleNotFound {
                    module_path: module_path.join("."),
                    searched_paths: vec![path.display().to_string()],
                },
                Span::dummy(),
                self.source.clone(),
            )));
        }
        Ok(Some(canonical))
    }

    fn validate_and_build_path(&self, module_path: &[String]) -> Result<PathBuf> {
        // Empty module path is invalid - likely a parser bug or malformed AST
        if module_path.is_empty() {
            return Err(AelysError::Compile(CompileError::new(
                CompileErrorKind::ModuleNotFound {
                    module_path: "<empty>".to_string(),
                    searched_paths: vec![],
                },
                Span::dummy(),
                self.source.clone(),
            )));
        }
        let mut path = self.base_dir.clone();
        for segment in module_path {
            // Validate: no path traversal
            if segment == ".." || segment == "." || segment.contains('/') || segment.contains('\\')
            {
                return Err(AelysError::Compile(CompileError::new(
                    CompileErrorKind::ModuleNotFound {
                        module_path: module_path.join("."),
                        searched_paths: vec![],
                    },
                    Span::dummy(),
                    self.source.clone(),
                )));
            }
            path.push(segment);
        }
        Ok(path)
    }

    fn search_with_patterns(
        &self,
        module_path: &[String],
        patterns: &[ExtensionPattern],
    ) -> Result<ModuleResolution> {
        let base_path = self.validate_and_build_path(module_path)?;
        // SAFETY: validate_and_build_path ensures module_path is non-empty
        let module_name = module_path
            .last()
            .expect("module_path validated as non-empty");

        let mut searched_paths = Vec::new();

        for pattern in patterns {
            let candidate = pattern.to_path(&base_path, module_name);
            searched_paths.push(candidate.display().to_string());
            if let Some(canonical) = self.canonicalize_if_exists(&candidate, module_path)? {
                return Ok(ModuleResolution {
                    path: canonical,
                    kind: pattern.kind(),
                });
            }
        }

        Err(AelysError::Compile(CompileError::new(
            CompileErrorKind::ModuleNotFound {
                module_path: module_path.join("."),
                searched_paths,
            },
            Span::dummy(),
            self.source.clone(),
        )))
    }

    fn resolve_native_only(&self, module_path: &[String]) -> Result<ModuleResolution> {
        self.search_with_patterns(module_path, native_only_search_patterns())
    }
}

fn is_std_module(module_path: &[String]) -> bool {
    matches!(module_path.first().map(|s| s.as_str()), Some("std"))
}
