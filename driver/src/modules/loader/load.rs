use super::types::{LoadResult, ModuleLoader};
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_runtime::VM;
use aelys_runtime::stdlib;
use aelys_syntax::{ImportKind, NeedsStmt};

impl ModuleLoader {
    pub fn load_module(&mut self, needs: &NeedsStmt, vm: &mut VM) -> Result<LoadResult> {
        if needs.path.is_empty() {
            return Err(AelysError::Compile(CompileError::new(
                CompileErrorKind::ModuleNotFound {
                    module_path: "<empty>".to_string(),
                    searched_paths: vec![],
                },
                needs.span,
                self.source.clone(),
            )));
        }

        let module_path_str = needs.path.join(".");

        if stdlib::is_std_module(&needs.path) {
            return self.load_std_module(needs, vm);
        }

        if self.loading_stack.contains(&module_path_str) {
            let mut chain = self.loading_stack.clone();
            chain.push(module_path_str.clone());
            return Err(AelysError::Compile(CompileError::new(
                CompileErrorKind::CircularDependency { chain },
                needs.span,
                self.source.clone(),
            )));
        }

        if self.loaded_modules.contains_key(&module_path_str) {
            if let Some(prev) = self.native_fingerprints.get(&module_path_str).cloned() {
                let file_path = self
                    .loaded_modules
                    .get(&module_path_str)
                    .map(|info| info.file_path.clone())
                    .ok_or_else(|| {
                        AelysError::Compile(CompileError::new(
                            CompileErrorKind::ModuleNotFound {
                                module_path: module_path_str.clone(),
                                searched_paths: vec![],
                            },
                            needs.span,
                            self.source.clone(),
                        ))
                    })?;
                let current = self.current_native_fingerprint(&file_path);
                if current.as_ref() != Some(&prev) && !vm.config().allow_hot_reload {
                    return Err(AelysError::Compile(CompileError::new(
                        CompileErrorKind::InvalidNativeModule {
                            module: module_path_str.clone(),
                            reason: "hot reload disabled".to_string(),
                        },
                        needs.span,
                        self.source.clone(),
                    )));
                }
                if let Some(current) = current {
                    self.native_fingerprints
                        .insert(module_path_str.clone(), current);
                }
            }

            let module_info = self.loaded_modules.get(&module_path_str).ok_or_else(|| {
                AelysError::Compile(CompileError::new(
                    CompileErrorKind::ModuleNotFound {
                        module_path: module_path_str.clone(),
                        searched_paths: vec![],
                    },
                    needs.span,
                    self.source.clone(),
                ))
            })?;

            match &needs.kind {
                ImportKind::Symbols(symbols) => {
                    for symbol in symbols {
                        if !module_info.exports.contains_key(symbol) {
                            return Err(AelysError::Compile(CompileError::new(
                                CompileErrorKind::SymbolNotFound {
                                    symbol: symbol.clone(),
                                    module: module_path_str.clone(),
                                },
                                needs.span,
                                self.source.clone(),
                            )));
                        }

                        let value = vm.get_global(symbol).ok_or_else(|| {
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
                }
                ImportKind::Module { alias } => {
                    let module_alias = self.get_module_alias(needs);
                    for name in module_info.exports.keys() {
                        let value = vm.get_global(name).ok_or_else(|| {
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
                        vm.set_global(alias_name, value);
                        if alias.is_none() {
                            vm.set_global(name.clone(), value);
                        }
                    }
                }
                ImportKind::Wildcard => {
                    for name in module_info.exports.keys() {
                        let value = vm.get_global(name).ok_or_else(|| {
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
                }
            }
            return Ok(self.get_load_result(needs));
        }

        let (resolution, actual_path, symbol) =
            self.resolve_path_with_fallback(&needs.path, needs)?;
        let actual_path_str = actual_path.join(".");

        if let Some(sym) = symbol.as_ref()
            && self.loaded_modules.contains_key(&actual_path_str)
        {
            return Ok(LoadResult::Symbol(sym.clone()));
        }

        self.loading_stack.push(actual_path_str.clone());

        let effective_needs = if let Some(sym) = &symbol {
            NeedsStmt {
                path: actual_path.clone(),
                kind: ImportKind::Symbols(vec![sym.clone()]),
                span: needs.span,
            }
        } else {
            needs.clone()
        };

        let result = match resolution.kind {
            aelys_modules::resolution::ModuleKind::Script => {
                self.compile_module(&resolution.path, &actual_path_str, &effective_needs, vm)
            }
            aelys_modules::resolution::ModuleKind::Native => {
                self.load_native_module(&resolution.path, &actual_path_str, &effective_needs, vm)
            }
        };

        self.loading_stack.pop();

        result?;

        if let Some(sym) = symbol {
            Ok(LoadResult::Symbol(sym))
        } else {
            Ok(self.get_load_result(needs))
        }
    }
}
