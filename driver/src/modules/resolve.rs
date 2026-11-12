use super::graph::{ModuleGraph, ModuleUnit, ResolvedImport};
use crate::SourceOptions;
use crate::embedded_std::{EMBEDDED_STD, embedded_std_path};
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_syntax::{ImportKind, NeedsStmt, NeedsTarget, Source, Span, Stmt, StmtKind};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct ModuleError {
    pub(crate) error: AelysError,
}

pub(crate) const MODULE_EXTENSION: &str = "aelys";

struct Discovery {
    roots: Vec<PathBuf>,
    std_source: StdSource,
    units: Vec<ModuleUnit>,
    by_path: HashMap<String, usize>,
    by_file: HashMap<PathBuf, usize>,
}

enum StdSource {
    Root(PathBuf),
    Embedded,
}

enum ModuleFile {
    Disk(PathBuf),
    Embedded {
        name: &'static str,
        content: &'static str,
    },
}

enum Located {
    Found(ModuleFile),
    Missing(Vec<String>),
    Ambiguous(Vec<String>),
}

pub(crate) fn discover(
    root_file: &Path,
    options: &SourceOptions,
) -> Result<ModuleGraph, ModuleError> {
    let root_dir = root_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let roots = ordered_roots(root_dir, &options.include);
    let std_source = std_source(&roots);
    let mut discovery = Discovery {
        roots,
        std_source,
        units: Vec::new(),
        by_path: HashMap::new(),
        by_file: HashMap::new(),
    };

    let mut prelude = None;
    if let Some(dotted) = &options.prelude {
        let segments: Vec<String> = dotted.split('.').map(str::to_string).collect();
        match discovery.locate(&segments) {
            // a prelude that does not resolve is not a prelude, and not an error
            Located::Missing(_) => {}
            Located::Found(file) => {
                let mut stack = Vec::new();
                prelude = Some(discovery.visit(&file, dotted.clone(), &mut stack)?);
            }
            Located::Ambiguous(roots) => {
                return Err(ModuleError {
                    error: off_source_failure(
                        root_file,
                        CompileErrorKind::AmbiguousModule {
                            module_path: dotted.clone(),
                            roots,
                        },
                    ),
                });
            }
        }
    }

    let mut stack = Vec::new();
    let root = discovery.visit(
        &ModuleFile::Disk(root_file.to_path_buf()),
        String::new(),
        &mut stack,
    )?;
    let mut reached = vec![false; discovery.units.len()];
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if std::mem::replace(&mut reached[index], true) {
            continue;
        }
        for import in &discovery.units[index].imports {
            pending.push(match import {
                ResolvedImport::Namespace { target, .. }
                | ResolvedImport::Symbols { target, .. } => *target,
            });
        }
    }
    for (unit, reached) in discovery.units.iter_mut().zip(reached) {
        unit.prelude_only = !reached;
    }

    Ok(ModuleGraph {
        units: discovery.units,
        prelude,
    })
}

fn std_source(roots: &[PathBuf]) -> StdSource {
    roots
        .iter()
        .find(|root| root.join("std").join("prelude.aelys").is_file())
        .map(|root| StdSource::Root(root.clone()))
        .unwrap_or(StdSource::Embedded)
}

fn ordered_roots(root_dir: PathBuf, include: &[PathBuf]) -> Vec<PathBuf> {
    let key = |dir: &Path| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut seen: HashSet<PathBuf> = HashSet::new();
    seen.insert(key(&root_dir));
    let mut roots = vec![root_dir];
    for dir in include {
        if seen.insert(key(dir)) {
            roots.push(dir.clone());
        }
    }
    roots
}

impl Discovery {
    fn visit(
        &mut self,
        file: &ModuleFile,
        dotted: String,
        stack: &mut Vec<String>,
    ) -> Result<usize, ModuleError> {
        if let Some(index) = self.by_path.get(&dotted) {
            return Ok(*index);
        }
        let identity = match file {
            ModuleFile::Disk(path) => {
                std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
            }
            ModuleFile::Embedded { name, .. } => PathBuf::from(embedded_std_path(name)),
        };
        if let Some(index) = self.by_file.get(&identity).copied() {
            self.by_path.insert(dotted, index);
            return Ok(index);
        }

        let source = match file {
            ModuleFile::Disk(path) => {
                let content = std::fs::read_to_string(path).map_err(|err| ModuleError {
                    error: read_failure(path, &err),
                })?;
                Source::new(path.display().to_string(), content)
            }
            ModuleFile::Embedded { name, content } => {
                Source::new(embedded_std_path(name), *content)
            }
        };
        let tokens = Lexer::with_source(source.clone())
            .scan()
            .map_err(|error| ModuleError { error })?;
        let stmts = Parser::new(tokens, source.clone())
            .parse()
            .map_err(|error| ModuleError { error })?;

        let mut bound: HashMap<String, TopLevelBinding> = HashMap::new();
        for binding in top_level_bindings(&stmts) {
            if let Some(previous) = bound.get(&binding.name) {
                return Err(self.fail(
                    &source,
                    binding.span,
                    CompileErrorKind::DuplicateDefinition {
                        name: binding.name.clone(),
                        form: binding.form,
                        previous_form: previous.form,
                        previous: previous.span,
                    },
                ));
            }
            bound.insert(binding.name.clone(), binding);
        }

        stack.push(dotted.clone());
        let mut imports = Vec::new();

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

            let target_file = match self.locate(path) {
                Located::Found(file) => file,
                Located::Missing(searched_paths) => {
                    return Err(self.fail(
                        &source,
                        needs.span,
                        CompileErrorKind::ModuleNotFound {
                            ignored: self.ignored_std_copies(path, &dotted_target),
                            module_path: dotted_target,
                            searched_paths,
                        },
                    ));
                }
                Located::Ambiguous(roots) => {
                    return Err(self.fail(
                        &source,
                        needs.span,
                        CompileErrorKind::AmbiguousModule {
                            module_path: dotted_target,
                            roots,
                        },
                    ));
                }
            };

            let target = self.visit(&target_file, dotted_target.clone(), stack)?;
            let names = binding_names(needs, path, kind);
            for name in &names {
                let binding = TopLevelBinding {
                    name: name.clone(),
                    form: "an import",
                    span: needs.span,
                };
                if bound.insert(name.clone(), binding).is_some() {
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
            bound_types: bound
                .values()
                .filter(|binding| binding.is_type())
                .map(|binding| binding.name.clone())
                .collect(),
            bound_values: bound
                .values()
                .filter(|binding| !binding.is_type())
                .map(|binding| binding.name.clone())
                .collect(),
            prelude_only: false,
        });
        self.by_path.insert(dotted, index);
        self.by_file.insert(identity, index);
        Ok(index)
    }

    fn locate(&self, path: &[String]) -> Located {
        let mut relative = PathBuf::new();
        for segment in path {
            relative.push(segment);
        }
        relative.set_extension(MODULE_EXTENSION);

        if path.len() >= 2 && path[0] == "std" {
            return self.locate_std(path, &relative);
        }

        let local = self.roots[0].join(&relative);
        if local.is_file() {
            return Located::Found(ModuleFile::Disk(local));
        }

        let mut hits: Vec<(PathBuf, PathBuf)> = Vec::new();
        for root in &self.roots[1..] {
            let candidate = root.join(&relative);
            if candidate.is_file() {
                hits.push((root.clone(), candidate));
            }
        }
        match hits.len() {
            0 => Located::Missing(
                self.roots
                    .iter()
                    .map(|root| root.join(&relative).display().to_string())
                    .collect(),
            ),
            1 => Located::Found(ModuleFile::Disk(hits.remove(0).1)),
            _ => Located::Ambiguous(
                hits.into_iter()
                    .map(|(root, _)| root.display().to_string())
                    .collect(),
            ),
        }
    }

    fn ignored_std_copies(&self, path: &[String], dotted: &str) -> Vec<String> {
        if path.len() < 2 || path[0] != "std" {
            return Vec::new();
        }
        let mut relative: PathBuf = path.iter().collect();
        relative.set_extension(MODULE_EXTENSION);
        let source = match &self.std_source {
            StdSource::Root(root) => format!("{} holds std/prelude.aelys", root.display()),
            StdSource::Embedded => "no search root holds std/prelude.aelys".to_string(),
        };
        self.roots
            .iter()
            .map(|root| root.join(&relative))
            .filter(|candidate| candidate.is_file())
            .map(|candidate| {
                format!(
                    "{dotted} comes from one source: {source}, so {} is ignored",
                    candidate.display()
                )
            })
            .collect()
    }

    fn locate_std(&self, path: &[String], relative: &Path) -> Located {
        match &self.std_source {
            StdSource::Root(root) => {
                let candidate = root.join(relative);
                if candidate.is_file() {
                    Located::Found(ModuleFile::Disk(candidate))
                } else {
                    Located::Missing(vec![candidate.display().to_string()])
                }
            }
            StdSource::Embedded => {
                let hit = (path.len() == 2)
                    .then(|| EMBEDDED_STD.iter().find(|(name, _)| *name == path[1]))
                    .flatten();
                match hit {
                    Some((name, content)) => Located::Found(ModuleFile::Embedded { name, content }),
                    None => Located::Missing(vec![embedded_std_path(&path[1..].join("/"))]),
                }
            }
        }
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
    off_source_failure(
        file,
        CompileErrorKind::SourceUnreadable {
            path: file.display().to_string(),
            io: err.to_string(),
        },
    )
}

// the command line has no span, so the fault is pinned on the root file with an empty body
fn off_source_failure(file: &Path, kind: CompileErrorKind) -> AelysError {
    let source = Source::new(file.display().to_string(), "");
    AelysError::Compile(CompileError::new(kind, Span::new(0, 0, 1, 1), source))
}

struct TopLevelBinding {
    name: String,
    form: &'static str,
    span: Span,
}

impl TopLevelBinding {
    fn is_type(&self) -> bool {
        matches!(self.form, "a struct" | "an enum")
    }
}

fn top_level_bindings(stmts: &[Stmt]) -> Vec<TopLevelBinding> {
    stmts
        .iter()
        .filter_map(|stmt| {
            let (name, form) = match &stmt.kind {
                StmtKind::Function(func) => (
                    func.name.clone(),
                    if func.foreign.is_some() {
                        "an external declaration"
                    } else {
                        "a function"
                    },
                ),
                StmtKind::Let { name, .. } => (name.clone(), "a global"),
                StmtKind::StructDecl { name, .. } => (name.clone(), "a struct"),
                StmtKind::EnumDecl { name, .. } => (name.clone(), "an enum"),
                _ => return None,
            };
            Some(TopLevelBinding {
                name,
                form,
                span: stmt.span,
            })
        })
        .collect()
}
