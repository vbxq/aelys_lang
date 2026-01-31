use super::types::ModuleLoader;
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_runtime::VM;
use aelys_runtime::stdlib;
use aelys_syntax::{ImportKind, NeedsStmt};

impl ModuleLoader {
    pub(crate) fn load_loaded_std_module(
        &self,
        needs: &NeedsStmt,
        vm: &mut VM,
    ) -> Result<super::types::LoadResult> {
        let module_path_str = needs.path.join(".");
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
            }
            ImportKind::Module { alias } => {
                let module_alias = self.get_module_alias(needs);
                for name in module_info.exports.keys() {
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
            }
            ImportKind::Wildcard => {
                for name in module_info.exports.keys() {
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
            }
        }

        Ok(self.get_load_result(needs))
    }
}
