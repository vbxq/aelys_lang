use super::graph::{ModuleGraph, ModuleUnit, ResolvedImport};
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_syntax::{ImportKind, NeedsStmt, NeedsTarget, Source, Span, Stmt, StmtKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct ModuleError {
    pub(crate) error: AelysError,
}

pub(crate) const MODULE_EXTENSION: &str = "aelys";

struct Discovery {
    root_dir: PathBuf,
    units: Vec<ModuleUnit>,
    by_path: HashMap<String, usize>,
}

pub(crate) fn discover(root_file: &Path) -> Result<ModuleGraph, ModuleError> {
    let root_dir = root_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut discovery = Discovery {
        root_dir,
        units: Vec::new(),
        by_path: HashMap::new(),
    };

    let mut stack = Vec::new();
    discovery.visit(root_file, String::new(), &mut stack)?;

    Ok(ModuleGraph {
        units: discovery.units,
    })
}

impl Discovery {
    // returns the index of the finished unit; every dependency is finished before it
    fn visit(
        &mut self,
        file: &Path,
        dotted: String,
        stack: &mut Vec<String>,
    ) -> Result<usize, ModuleError> {
        if let Some(index) = self.by_path.get(&dotted) {
            return Ok(*index);
        }

        let content = std::fs::read_to_string(file).map_err(|err| ModuleError {
            error: read_failure(file, &err),
        })?;
        let source = Source::new(file.display().to_string(), content);
        let tokens = Lexer::with_source(source.clone())
            .scan()
            .map_err(|error| ModuleError { error })?;
        let stmts = Parser::new(tokens, source.clone())
            .parse()
            .map_err(|error| ModuleError { error })?;

        stack.push(dotted.clone());
        let mut imports = Vec::new();
        let mut bound: HashMap<String, ()> = HashMap::new();
        for name in top_level_names(&stmts) {
            bound.insert(name, ());
        }

        for stmt in &stmts {
            let StmtKind::Needs(needs) = &stmt.kind else {
                continue;
            };
            let (path, kind) = match &needs.target {
                NeedsTarget::Foreign { header } => {
                    return Err(self.fail(
                        &source,
                        needs.span,
                        CompileErrorKind::ForeignHeaderImport {
                            header: header.clone(),
                        },
                    ));
                }
                NeedsTarget::Module { path, kind } => (path, kind),
            };

            let dotted_target = path.join(".");
            if let ImportKind::Wildcard = kind {
                return Err(self.fail(
                    &source,
                    needs.span,
                    CompileErrorKind::WildcardImport {
                        module_path: dotted_target,
                    },
                ));
            }
            if let Some(segment) = path.iter().find(|s| s.starts_with("__")) {
                return Err(self.fail(
                    &source,
                    needs.span,
                    CompileErrorKind::ReservedModuleSegment {
                        module_path: dotted_target,
                        segment: segment.clone(),
                    },
                ));
            }
            if stack.contains(&dotted_target) {
                let mut chain: Vec<String> =
                    stack.iter().map(|module| display_path(module)).collect();
                chain.push(display_path(&dotted_target));
                return Err(self.fail(
                    &source,
                    needs.span,
                    CompileErrorKind::CircularDependency { chain },
                ));
            }

            let target_file = self.file_for(path);
            if !target_file.is_file() {
                return Err(self.fail(
                    &source,
                    needs.span,
                    CompileErrorKind::ModuleNotFound {
                        module_path: dotted_target,
                        searched_paths: vec![target_file.display().to_string()],
                    },
                ));
            }

            let target = self.visit(&target_file, dotted_target.clone(), stack)?;
            let names = binding_names(needs, path, kind);
            for name in &names {
                if bound.insert(name.clone(), ()).is_some() {
                    return Err(self.fail(
                        &source,
                        needs.span,
                        CompileErrorKind::SymbolConflict {
                            symbol: name.clone(),
                            modules: vec![dotted_target.clone()],
                        },
                    ));
                }
            }
            imports.push(match kind {
                ImportKind::Symbols(_) => ResolvedImport::Symbols {
                    names,
                    target,
                    span: needs.span,
                },
                _ => ResolvedImport::Namespace {
                    local: names.into_iter().next().unwrap_or_default(),
                    target,
                },
            });
        }
        stack.pop();

        let index = self.units.len();
        self.units.push(ModuleUnit {
            dotted: dotted.clone(),
            source,
            stmts,
            imports,
        });
        self.by_path.insert(dotted, index);
        Ok(index)
    }

    fn file_for(&self, path: &[String]) -> PathBuf {
        let mut file = self.root_dir.clone();
        for segment in path {
            file.push(segment);
        }
        file.set_extension(MODULE_EXTENSION);
        file
    }

    fn fail(&self, source: &Arc<Source>, span: Span, kind: CompileErrorKind) -> ModuleError {
        ModuleError {
            error: AelysError::Compile(CompileError::new(kind, span, source.clone())),
        }
    }
}

fn display_path(dotted: &str) -> String {
    if dotted.is_empty() {
        "<root>".to_string()
    } else {
        dotted.to_string()
    }
}

fn binding_names(_needs: &NeedsStmt, path: &[String], kind: &ImportKind) -> Vec<String> {
    match kind {
        ImportKind::Symbols(names) => names.clone(),
        ImportKind::Module { alias } => vec![
            alias
                .clone()
                .or_else(|| path.last().cloned())
                .unwrap_or_default(),
        ],
        ImportKind::Wildcard => Vec::new(),
    }
}

fn read_failure(file: &Path, err: &std::io::Error) -> AelysError {
    let source = Source::new(file.display().to_string(), "");
    AelysError::Compile(CompileError::new(
        CompileErrorKind::BackendDiagnostic {
            backend: "driver".to_string(),
            message: format!("failed to read {}: {}", file.display(), err),
            note: None,
            help: None,
        },
        Span::new(0, 0, 1, 1),
        source,
    ))
}

fn top_level_names(stmts: &[Stmt]) -> Vec<String> {
    stmts
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Function(func) => Some(func.name.clone()),
            StmtKind::Let { name, .. } => Some(name.clone()),
            StmtKind::StructDecl { name, .. } | StmtKind::EnumDecl { name, .. } => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}
