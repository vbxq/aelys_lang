use super::types::{LoadResult, ModuleLoader};
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_modules::resolution::ModuleResolution;
use aelys_syntax::{ImportKind, NeedsStmt};

impl ModuleLoader {
    pub(crate) fn get_load_result(&self, needs: &NeedsStmt) -> LoadResult {
        match &needs.kind {
            ImportKind::Module { alias } => {
                let alias_name = alias.clone().unwrap_or_else(|| {
                    needs
                        .path
                        .last()
                        .cloned()
                        .expect("needs.path validated as non-empty")
                });
                LoadResult::Module(alias_name)
            }
            ImportKind::Symbols(symbols) => LoadResult::Symbol(
                symbols
                    .first()
                    .cloned()
                    .expect("symbols validated as non-empty"),
            ),
            ImportKind::Wildcard => LoadResult::Module(
                needs
                    .path
                    .last()
                    .cloned()
                    .expect("needs.path validated as non-empty"),
            ),
        }
    }

    pub(crate) fn resolve_path_with_fallback(
        &self,
        module_path: &[String],
        needs: &NeedsStmt,
    ) -> Result<(ModuleResolution, Vec<String>, Option<String>)> {
        if let Ok(resolution) = self.resolve_module_path(module_path) {
            return Ok((resolution, module_path.to_vec(), None));
        }

        if module_path.len() > 1 {
            let parent_path: Vec<String> = module_path[..module_path.len() - 1].to_vec();
            let symbol = module_path.last().cloned().ok_or_else(|| {
                AelysError::Compile(CompileError::new(
                    CompileErrorKind::ModuleNotFound {
                        module_path: module_path.join("."),
                        searched_paths: vec![],
                    },
                    needs.span,
                    self.source.clone(),
                ))
            })?;

            if let Ok(resolution) = self.resolve_module_path(&parent_path) {
                return Ok((resolution, parent_path, Some(symbol)));
            }
        }

        Err(AelysError::Compile(CompileError::new(
            CompileErrorKind::ModuleNotFound {
                module_path: module_path.join("."),
                searched_paths: self.search_paths_for(module_path),
            },
            needs.span,
            self.source.clone(),
        )))
    }

    pub(crate) fn get_module_alias(&self, needs: &NeedsStmt) -> String {
        match &needs.kind {
            ImportKind::Module { alias } => alias.clone().unwrap_or_else(|| {
                needs
                    .path
                    .last()
                    .cloned()
                    .expect("needs.path validated as non-empty")
            }),
            ImportKind::Symbols(_) | ImportKind::Wildcard => needs
                .path
                .last()
                .cloned()
                .expect("needs.path validated as non-empty"),
        }
    }
}
