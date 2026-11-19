use super::graph::{ModuleGraph, ModuleUnit, ResolvedImport};
use crate::SourceOptions;
use crate::embedded_std::{EMBEDDED_STD, embedded_std_path};
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::modules::is_std_module;
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
    imported_as: HashMap<usize, String>,
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
        imported_as: HashMap::new(),
    };

    let mut prelude = None;
    if let Some(dotted) = &options.prelude {
        let segments: Vec<String> = dotted.split('.').map(str::to_string).collect();
        match discovery.locate(&segments) {
            // a prelude that does not resolve is not a prelude, and not an error
            Located::Missing(_) => {}
            Located::Found(file) => {
                prelude = Some(discovery.load(&file, dotted.clone())?);
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

    let root = discovery.load(&ModuleFile::Disk(root_file.to_path_buf()), String::new())?;
    let mut reached = vec![false; discovery.units.len()];
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if std::mem::replace(&mut reached[index], true) {
            continue;
        }
        pending.extend(
            discovery.units[index]
                .imports
                .iter()
                .map(ResolvedImport::target),
        );
    }
    for (unit, reached) in discovery.units.iter_mut().zip(reached) {
        unit.prelude_only = !reached;
    }

    let mut order = Order::of(&discovery.units, prelude.into_iter().chain([root]));
    if order.cyclic.contains(&root)
        && let Some(name) = discovery.imported_as.get(&root)
    {
        discovery.units[root].dotted = name.clone();
    }
    let grouped = !order.cyclic.is_empty() || discovery.units.iter().any(impls_a_foreign_type);
    if grouped {
        // std never imports a user module, so typing all of std first keeps every import ahead
        let (std, user): (Vec<usize>, Vec<usize>) = order
            .sequence
            .iter()
            .partition(|&&unit| is_std_module(&discovery.units[unit].dotted));
        order.sequence = std.into_iter().chain(user).collect();
    }
    Ok(order.apply(discovery.units, prelude, grouped))
}

fn impls_a_foreign_type(unit: &ModuleUnit) -> bool {
    if is_std_module(&unit.dotted) {
        return false;
    }
    let imported = |name: &str| {
        unit.imports.iter().any(|import| match import {
            ResolvedImport::Symbols { names, .. } => names.iter().any(|n| n == name),
            ResolvedImport::Namespace { .. } => false,
        })
    };
    unit.stmts.iter().any(|stmt| match &stmt.kind {
        StmtKind::Impl(decl) => decl.self_ty.name.contains('.') || imported(&decl.self_ty.name),
        _ => false,
    })
}

// imports in source order, so an acyclic graph keeps the post order units were always typed in
struct Order {
    sequence: Vec<usize>,
    cyclic: HashSet<usize>,
}

impl Order {
    fn of(units: &[ModuleUnit], starts: impl Iterator<Item = usize>) -> Self {
        let mut tarjan = Tarjan {
            units,
            index: vec![None; units.len()],
            low: vec![0; units.len()],
            on_stack: vec![false; units.len()],
            stack: Vec::new(),
            next: 0,
            finished: vec![0; units.len()],
            finish_clock: 0,
            components: Vec::new(),
        };
        for start in starts {
            if tarjan.index[start].is_none() {
                tarjan.visit(start);
            }
        }
        let mut order = Order {
            sequence: Vec::new(),
            cyclic: HashSet::new(),
        };
        for mut component in std::mem::take(&mut tarjan.components) {
            component.sort_by_key(|&unit| tarjan.finished[unit]);
            if component.len() > 1 {
                order.cyclic.extend(component.iter().copied());
            }
            order.sequence.extend(component);
        }
        order
    }

    fn apply(self, units: Vec<ModuleUnit>, prelude: Option<usize>, grouped: bool) -> ModuleGraph {
        let mut position = vec![usize::MAX; units.len()];
        for (at, &unit) in self.sequence.iter().enumerate() {
            position[unit] = at;
        }
        let mut slots: Vec<Option<ModuleUnit>> = units.into_iter().map(Some).collect();
        let units = self
            .sequence
            .iter()
            .map(|&old| {
                let mut unit = slots[old].take().expect("each unit is placed once");
                for import in &mut unit.imports {
                    let target = import.target_mut();
                    *target = position[*target];
                }
                unit
            })
            .collect();
        ModuleGraph {
            units,
            prelude: prelude.map(|old| position[old]),
            grouped,
        }
    }
}

struct Tarjan<'a> {
    units: &'a [ModuleUnit],
    index: Vec<Option<usize>>,
    low: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    next: usize,
    finished: Vec<usize>,
    finish_clock: usize,
    components: Vec<Vec<usize>>,
}

impl Tarjan<'_> {
    fn visit(&mut self, unit: usize) {
        self.index[unit] = Some(self.next);
        self.low[unit] = self.next;
        self.next += 1;
        self.stack.push(unit);
        self.on_stack[unit] = true;
        for target in self.units[unit].imports.iter().map(ResolvedImport::target) {
            match self.index[target] {
                None => {
                    self.visit(target);
                    self.low[unit] = self.low[unit].min(self.low[target]);
                }
                Some(seen) if self.on_stack[target] => {
                    self.low[unit] = self.low[unit].min(seen);
                }
                Some(_) => {}
            }
        }
        self.finished[unit] = self.finish_clock;
        self.finish_clock += 1;
        if Some(self.low[unit]) == self.index[unit] {
            let mut component = Vec::new();
            loop {
                let member = self.stack.pop().expect("the unit is on the stack");
                self.on_stack[member] = false;
                component.push(member);
                if member == unit {
                    break;
                }
            }
            self.components.push(component);
        }
    }
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
    fn load(&mut self, file: &ModuleFile, dotted: String) -> Result<usize, ModuleError> {
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
            self.imported_as
                .entry(index)
                .or_insert_with(|| dotted.clone());
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

        let index = self.units.len();
        self.units.push(ModuleUnit {
            dotted: dotted.clone(),
            source: source.clone(),
            file: match file {
                ModuleFile::Disk(path) => Some(path.clone()),
                ModuleFile::Embedded { .. } => None,
            },
            stmts: Vec::new(),
            imports: Vec::new(),
            bound_types: HashSet::new(),
            bound_values: HashSet::new(),
            prelude_only: false,
        });
        self.by_path.insert(dotted, index);
        self.by_file.insert(identity, index);
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

            let target = self.load(&target_file, dotted_target.clone())?;
            if target == index {
                return Err(self.fail(
                    &source,
                    needs.span,
                    CompileErrorKind::ModuleImportsItself {
                        module_path: dotted_target,
                    },
                ));
            }
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
        let unit = &mut self.units[index];
        unit.stmts = stmts;
        unit.imports = imports;
        unit.bound_types = bound
            .values()
            .filter(|binding| binding.is_type())
            .map(|binding| binding.name.clone())
            .collect();
        unit.bound_values = bound
            .values()
            .filter(|binding| !binding.is_type())
            .map(|binding| binding.name.clone())
            .collect();
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
        matches!(self.form, "a struct" | "an enum" | "a type alias")
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
                StmtKind::Const { name, .. } => (name.clone(), "a const"),
                StmtKind::TypeAlias { name, .. } => (name.clone(), "a type alias"),
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
