use crate::modules::loader::{LoadResult, ModuleImports, ModuleLoader};
use aelys_common::Result;
use aelys_runtime::VM;
use aelys_syntax::Source;
use aelys_syntax::{Stmt, StmtKind};
use std::path::Path;
use std::sync::Arc;

// process needs statements and load all required modules
pub fn load_modules_for_program(
    stmts: &[Stmt],
    entry_file: &Path,
    source: Arc<Source>,
    vm: &mut VM,
) -> Result<ModuleImports> {
    let mut loader = ModuleLoader::new(entry_file, source);
    let mut module_aliases = std::collections::HashSet::new();
    let mut known_globals = std::collections::HashSet::new();
    let mut known_native_globals = std::collections::HashSet::new();

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
        if let Some(module_info) = loader.get_module(&module_path) {
            for native_name in &module_info.native_functions {
                known_native_globals.insert(native_name.clone());
            }

            if matches!(needs.kind, aelys_syntax::ImportKind::Wildcard) {
                for name in module_info.exports.keys() {
                    known_globals.insert(name.clone());
                }
            }
        }
    }

    Ok(ModuleImports {
        module_aliases,
        known_globals,
        known_native_globals,
        next_call_site_slot: loader.next_call_site_slot,
    })
}

// same as above but returns the loader too (needed for .avbc bundling)
pub fn load_modules_with_loader(
    stmts: &[Stmt],
    entry_file: &Path,
    source: Arc<Source>,
    vm: &mut VM,
) -> Result<(ModuleImports, ModuleLoader)> {
    let mut loader = ModuleLoader::new(entry_file, source);
    let mut module_aliases = std::collections::HashSet::new();
    let mut known_globals = std::collections::HashSet::new();
    let mut known_native_globals = std::collections::HashSet::new();

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
        if let Some(module_info) = loader.get_module(&module_path) {
            for native_name in &module_info.native_functions {
                known_native_globals.insert(native_name.clone());
            }

            if matches!(needs.kind, aelys_syntax::ImportKind::Wildcard) {
                for name in module_info.exports.keys() {
                    known_globals.insert(name.clone());
                }
            }
        }
    }

    Ok((
        ModuleImports {
            module_aliases,
            known_globals,
            known_native_globals,
            next_call_site_slot: loader.next_call_site_slot,
        },
        loader,
    ))
}
