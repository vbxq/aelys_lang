use super::types::{ExportInfo, LoadResult, ModuleInfo, ModuleLoader};
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_runtime::VM;
use aelys_runtime::stdlib;
use aelys_syntax::{ImportKind, NeedsStmt};
use std::path::PathBuf;

impl ModuleLoader {
    pub(crate) fn load_new_std_module(
        &mut self,
        needs: &NeedsStmt,
        vm: &mut VM,
    ) -> Result<LoadResult> {
        let module_path_str = needs.path.join(".");
        let module_name = stdlib::get_std_module_name(&needs.path).ok_or_else(|| {
            AelysError::Compile(CompileError::new(
                CompileErrorKind::ModuleNotFound {
                    module_path: module_path_str.clone(),
                    searched_paths: vec![],
                },
                needs.span,
                self.source.clone(),
            ))
        })?;

        let std_exports =
            stdlib::register_std_module(vm, module_name).map_err(|e| AelysError::Runtime(e))?;

        let mut exports_map = std::collections::HashMap::new();
        for name in &std_exports.all_exports {
            exports_map.insert(
                name.clone(),
                ExportInfo {
                    is_function: true,
                    is_mutable: false,
                },
            );
        }

        let module_info = ModuleInfo {
            name: module_name.to_string(),
            path: module_path_str.clone(),
            file_path: PathBuf::from(format!("<std/{}>", module_name)),
            version: None,
            exports: exports_map,
            native_functions: std_exports.native_functions,
        };
        self.loaded_modules
            .insert(module_path_str.clone(), module_info);

        let module_alias = self.get_module_alias(needs);

        match &needs.kind {
            ImportKind::Module { alias } => {
                for name in &std_exports.all_exports {
                    let qualified_name = format!("{}::{}", module_name, name);
                    let value = vm.get_global(&qualified_name).ok_or_else(|| {
                        AelysError::Compile(CompileError::new(
                            CompileErrorKind::SymbolNotFound {
                                symbol: name.clone(),
                                module: module_path_str.clone(),
                            },
                            needs.span,
                            self.source.clone(),
                        ))
                    })?;
                    let alias_name = format!("{}::{}", module_alias, name);
                    vm.set_global(alias_name, value.clone());

                    if alias.is_none() {
                        vm.set_global(name.clone(), value);
                    }
                }
                Ok(LoadResult::Module(module_alias))
            }
            ImportKind::Symbols(symbols) => {
                if symbols.is_empty() {
                    return Err(AelysError::Compile(CompileError::new(
                        CompileErrorKind::SymbolNotFound {
                            symbol: "<empty>".to_string(),
                            module: module_path_str.clone(),
                        },
                        needs.span,
                        self.source.clone(),
                    )));
                }
                for symbol in symbols {
                    if !std_exports.all_exports.contains(symbol) {
                        return Err(AelysError::Compile(CompileError::new(
                            CompileErrorKind::SymbolNotFound {
                                symbol: symbol.clone(),
                                module: module_path_str.clone(),
                            },
                            needs.span,
                            self.source.clone(),
                        )));
                    }
                    let qualified_name = format!("{}::{}", module_name, symbol);
                    let value = vm.get_global(&qualified_name).ok_or_else(|| {
                        AelysError::Compile(CompileError::new(
                            CompileErrorKind::SymbolNotFound {
                                symbol: symbol.clone(),
                                module: module_path_str.clone(),
                            },
                            needs.span,
                            self.source.clone(),
                        ))
                    })?;
                    vm.set_global(symbol.clone(), value);
                }
                Ok(LoadResult::Symbol(
                    symbols
                        .first()
                        .cloned()
                        .expect("symbols validated as non-empty"),
                ))
            }
            ImportKind::Wildcard => {
                for name in &std_exports.all_exports {
                    let qualified_name = format!("{}::{}", module_name, name);
                    let value = vm.get_global(&qualified_name).ok_or_else(|| {
                        AelysError::Compile(CompileError::new(
                            CompileErrorKind::SymbolNotFound {
                                symbol: name.clone(),
                                module: module_path_str.clone(),
                            },
                            needs.span,
                            self.source.clone(),
                        ))
                    })?;
                    vm.set_global(name.clone(), value);
                }
                Ok(LoadResult::Module(module_alias))
            }
        }
    }
}
