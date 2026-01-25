use super::types::{ExportInfo, ModuleInfo, ModuleLoader};
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_runtime::VM;
use aelys_syntax::{ImportKind, Stmt, StmtKind};

impl ModuleLoader {
    pub(crate) fn collect_exports(
        &self,
        stmts: &[Stmt],
        _module_path: &str,
    ) -> Result<std::collections::HashMap<String, ExportInfo>> {
        let mut exports = std::collections::HashMap::new();

        for stmt in stmts {
            match &stmt.kind {
                StmtKind::Function(func) => {
                    if func.is_pub {
                        exports.insert(
                            func.name.clone(),
                            ExportInfo {
                                is_function: true,
                                is_mutable: false,
                            },
                        );
                    }
                }
                StmtKind::Let {
                    name,
                    mutable,
                    is_pub,
                    ..
                } => {
                    if *is_pub {
                        exports.insert(
                            name.clone(),
                            ExportInfo {
                                is_function: false,
                                is_mutable: *mutable,
                            },
                        );
                    }
                }
                StmtKind::Needs(_) => {}
                _ => {}
            }
        }

        Ok(exports)
    }

    pub(crate) fn register_exports(
        &self,
        needs: &aelys_syntax::NeedsStmt,
        exports: &std::collections::HashMap<String, ExportInfo>,
        vm: &mut VM,
    ) -> Result<()> {
        let module_alias = self.get_module_alias(needs);
        let module_path_str = needs.path.join(".");

        match &needs.kind {
            ImportKind::Module { .. } => {
                for (name, _info) in exports {
                    let qualified_name = format!("{}::{}", module_alias, name);
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
                    vm.set_global(qualified_name, value);
                }
            }
            ImportKind::Symbols(symbols) => {
                for symbol in symbols {
                    if !exports.contains_key(symbol) {
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
            ImportKind::Wildcard => {
                for (name, _info) in exports {
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

        Ok(())
    }

    pub fn is_symbol_public(&self, module_path: &str, symbol: &str) -> bool {
        self.loaded_modules
            .get(module_path)
            .map(|m| m.exports.contains_key(symbol))
            .unwrap_or(false)
    }

    pub fn get_module(&self, module_path: &str) -> Option<&ModuleInfo> {
        self.loaded_modules.get(module_path)
    }
}
