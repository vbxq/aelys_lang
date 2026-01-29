use crate::modules::loader::{LoadResult, ModuleImports, ModuleLoader};
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_runtime::VM;
use aelys_syntax::Source;
use aelys_syntax::{ImportKind, Stmt, StmtKind};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub fn load_modules_for_program(
    stmts: &[Stmt],
    entry_file: &Path,
    source: Arc<Source>,
    vm: &mut VM,
) -> Result<ModuleImports> {
    let mut loader = ModuleLoader::new(entry_file, source.clone());
    let mut module_aliases = std::collections::HashSet::new();
    let mut known_globals = std::collections::HashSet::new();
    let mut known_native_globals = std::collections::HashSet::new();
    let mut symbol_origins: HashMap<String, String> = HashMap::new();

    let needs_stmts: Vec<&aelys_syntax::NeedsStmt> = stmts
        .iter()
        .filter_map(|s| {
            if let StmtKind::Needs(needs) = &s.kind {
                Some(needs)
            } else {
                None
            }
        })
        .collect();

    for needs in &needs_stmts {
        let result = loader.load_module(needs, vm)?;

        match result {
            LoadResult::Module(alias) => {
                module_aliases.insert(alias.clone());
            }
            LoadResult::Symbol(symbol) => {
                known_globals.insert(symbol);
            }
        }

        let module_path = needs.path.join(".");
        let is_stdlib = module_path.starts_with("std.");
        if let Some(module_info) = loader.get_module(&module_path) {
            for native_name in &module_info.native_functions {
                known_native_globals.insert(native_name.clone());
            }

            let module_alias = loader.get_module_alias(needs);
            match &needs.kind {
                ImportKind::Module { alias: None } => {
                    for name in module_info.exports.keys() {
                        let qualified = format!("{}::{}", module_alias, name);
                        if let Some(existing) = symbol_origins.get(name) {
                            return Err(AelysError::Compile(CompileError::new(
                                CompileErrorKind::SymbolConflict {
                                    symbol: name.clone(),
                                    modules: vec![existing.clone(), module_path.clone()],
                                },
                                needs.span,
                                source.clone(),
                            )));
                        }
                        // Store module path for conflict detection
                        // For stdlib: also store qualified name for bytecode translation
                        symbol_origins.insert(name.clone(), if is_stdlib { qualified.clone() } else { module_path.clone() });
                        known_globals.insert(name.clone());
                        if module_info.native_functions.contains(&qualified) {
                            known_native_globals.insert(name.clone());
                        }
                    }
                }
                ImportKind::Symbols(symbols) => {
                    for sym in symbols {
                        let qualified = format!("{}::{}", module_alias, sym);
                        symbol_origins.insert(sym.clone(), if is_stdlib { qualified.clone() } else { module_path.clone() });
                        known_globals.insert(sym.clone());
                        if module_info.native_functions.contains(&qualified) {
                            known_native_globals.insert(sym.clone());
                        }
                    }
                }
                ImportKind::Wildcard => {
                    for name in module_info.exports.keys() {
                        let qualified = format!("{}::{}", module_alias, name);
                        symbol_origins.insert(name.clone(), if is_stdlib { qualified.clone() } else { module_path.clone() });
                        known_globals.insert(name.clone());
                        if module_info.native_functions.contains(&qualified) {
                            known_native_globals.insert(name.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(ModuleImports {
        module_aliases,
        known_globals,
        known_native_globals,
        symbol_origins,
        next_call_site_slot: loader.next_call_site_slot,
    })
}

pub fn load_modules_with_loader(
    stmts: &[Stmt],
    entry_file: &Path,
    source: Arc<Source>,
    vm: &mut VM,
) -> Result<(ModuleImports, ModuleLoader)> {
    let mut loader = ModuleLoader::new(entry_file, source.clone());
    let mut module_aliases = std::collections::HashSet::new();
    let mut known_globals = std::collections::HashSet::new();
    let mut known_native_globals = std::collections::HashSet::new();
    let mut symbol_origins: HashMap<String, String> = HashMap::new();

    let needs_stmts: Vec<&aelys_syntax::NeedsStmt> = stmts
        .iter()
        .filter_map(|s| {
            if let StmtKind::Needs(needs) = &s.kind {
                Some(needs)
            } else {
                None
            }
        })
        .collect();

    for needs in &needs_stmts {
        let result = loader.load_module(needs, vm)?;

        match result {
            LoadResult::Module(alias) => {
                module_aliases.insert(alias.clone());
            }
            LoadResult::Symbol(symbol) => {
                known_globals.insert(symbol);
            }
        }

        let module_path = needs.path.join(".");
        let is_stdlib = module_path.starts_with("std.");
        if let Some(module_info) = loader.get_module(&module_path) {
            for native_name in &module_info.native_functions {
                known_native_globals.insert(native_name.clone());
            }

            let module_alias = loader.get_module_alias(needs);
            match &needs.kind {
                ImportKind::Module { alias: None } => {
                    for name in module_info.exports.keys() {
                        let qualified = format!("{}::{}", module_alias, name);
                        if let Some(existing) = symbol_origins.get(name) {
                            return Err(AelysError::Compile(CompileError::new(
                                CompileErrorKind::SymbolConflict {
                                    symbol: name.clone(),
                                    modules: vec![existing.clone(), module_path.clone()],
                                },
                                needs.span,
                                source.clone(),
                            )));
                        }
                        // Store module path for conflict detection
                        // For stdlib: also store qualified name for bytecode translation
                        symbol_origins.insert(name.clone(), if is_stdlib { qualified.clone() } else { module_path.clone() });
                        known_globals.insert(name.clone());
                        if module_info.native_functions.contains(&qualified) {
                            known_native_globals.insert(name.clone());
                        }
                    }
                }
                ImportKind::Symbols(symbols) => {
                    for sym in symbols {
                        let qualified = format!("{}::{}", module_alias, sym);
                        symbol_origins.insert(sym.clone(), if is_stdlib { qualified.clone() } else { module_path.clone() });
                        known_globals.insert(sym.clone());
                        if module_info.native_functions.contains(&qualified) {
                            known_native_globals.insert(sym.clone());
                        }
                    }
                }
                ImportKind::Wildcard => {
                    for name in module_info.exports.keys() {
                        let qualified = format!("{}::{}", module_alias, name);
                        symbol_origins.insert(name.clone(), if is_stdlib { qualified.clone() } else { module_path.clone() });
                        known_globals.insert(name.clone());
                        if module_info.native_functions.contains(&qualified) {
                            known_native_globals.insert(name.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok((
        ModuleImports {
            module_aliases,
            known_globals,
            known_native_globals,
            symbol_origins,
            next_call_site_slot: loader.next_call_site_slot,
        },
        loader,
    ))
}
