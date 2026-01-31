use super::types::{ModuleInfo, ModuleLoader};
use aelys_backend::Compiler;
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::Optimizer;
use aelys_runtime::VM;
use aelys_sema::TypeInference;
use aelys_syntax::{Source, StmtKind};
use std::path::Path;
use std::sync::Arc;

// TODO: parallel compilation for independent modules would be nice

impl ModuleLoader {
    pub(crate) fn compile_module(
        &mut self,
        file_path: &Path,
        module_path_str: &str,
        needs: &aelys_syntax::NeedsStmt,
        vm: &mut VM,
    ) -> Result<()> {
        let content = std::fs::read_to_string(file_path).map_err(|_| {
            AelysError::Compile(CompileError::new(
                CompileErrorKind::ModuleNotFound {
                    module_path: module_path_str.to_string(),
                    searched_paths: vec![file_path.display().to_string()],
                },
                needs.span,
                self.source.clone(),
            ))
        })?;

        let module_source = Source::new(&file_path.display().to_string(), &content);
        let tokens = Lexer::with_source(module_source.clone()).scan()?;
        let stmts = Parser::new(tokens, module_source.clone()).parse()?;

        let exports = self.collect_exports(&stmts, module_path_str)?;

        let module_name = needs
            .path
            .last()
            .cloned()
            .expect("needs.path validated as non-empty");
        let module_info = ModuleInfo {
            name: module_name,
            path: module_path_str.to_string(),
            file_path: file_path.to_path_buf(),
            version: None,
            exports: exports.clone(),
            native_functions: Vec::new(),
        };
        self.loaded_modules
            .insert(module_path_str.to_string(), module_info);

        let original_base_dir = self.base_dir.clone();
        let original_base_root = self.base_root.clone();
        self.base_dir = file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(original_base_dir.clone());
        self.base_root = self
            .base_dir
            .canonicalize()
            .unwrap_or_else(|_| self.base_dir.clone());

        let mut module_aliases = std::collections::HashSet::new();
        let mut known_globals = std::collections::HashSet::new();
        let mut known_native_globals = std::collections::HashSet::new();

        for stmt in &stmts {
            if let StmtKind::Needs(nested_needs) = &stmt.kind {
                let result = self.load_module(nested_needs, vm)?;

                match result {
                    super::types::LoadResult::Module(alias) => {
                        module_aliases.insert(alias.clone());
                    }
                    super::types::LoadResult::Symbol(symbol) => {
                        known_globals.insert(symbol);
                    }
                }

                let nested_module_path = nested_needs.path.join(".");
                if let Some(module_info) = self.get_module(&nested_module_path) {
                    for native_name in &module_info.native_functions {
                        known_native_globals.insert(native_name.clone());
                    }

                    match &nested_needs.kind {
                        aelys_syntax::ImportKind::Module { alias: None }
                        | aelys_syntax::ImportKind::Wildcard => {
                            for name in module_info.exports.keys() {
                                known_globals.insert(name.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        self.base_dir = original_base_dir;
        self.base_root = original_base_root;

        let main_stmts: Vec<_> = stmts
            .into_iter()
            .filter(|s| !matches!(s.kind, StmtKind::Needs(_)))
            .collect();

        let typed_program = TypeInference::infer_program(main_stmts, module_source.clone())
            .map_err(|errors| {
                if let Some(err) = errors.first() {
                    AelysError::Compile(CompileError::new(
                        CompileErrorKind::TypeInferenceError(format!("{}", err)),
                        err.span,
                        module_source.clone(),
                    ))
                } else {
                    AelysError::Compile(CompileError::new(
                        CompileErrorKind::TypeInferenceError("Unknown type error".to_string()),
                        aelys_syntax::Span::dummy(),
                        module_source.clone(),
                    ))
                }
            })?;

        let mut optimizer = Optimizer::new(aelys_opt::OptimizationLevel::Standard);
        let typed_program = optimizer.optimize(typed_program);

        let mut compiler = Compiler::with_modules(
            Some(module_path_str.to_string()),
            module_source.clone(),
            module_aliases,
            known_globals,
            known_native_globals,
            std::collections::HashMap::new(),
        );
        compiler.next_call_site_slot = self.next_call_site_slot;
        let (mut function, mut compile_heap, _globals) = compiler.compile_typed(&typed_program)?;
        // update call_site_slot from the compiled function (it includes slots used during compilation)
        self.next_call_site_slot = function.call_site_count;

        let remap = vm.merge_heap(&mut compile_heap)?;
        function.remap_constants(&remap);

        let global_layout = Arc::clone(&function.global_layout);

        let func_ref = vm.alloc_function(function)?;
        vm.execute(func_ref)?;

        vm.sync_globals_to_hashmap(global_layout.names());

        self.register_exports(needs, &exports, vm)?;

        Ok(())
    }
}
