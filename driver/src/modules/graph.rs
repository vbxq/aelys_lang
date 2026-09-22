use aelys_syntax::{Source, Stmt};
use std::sync::Arc;

pub(crate) enum ResolvedImport {
    Namespace {
        local: String,
        target: usize,
    },
    Symbols {
        names: Vec<String>,
        target: usize,
        span: aelys_syntax::Span,
    },
}

pub(crate) struct ModuleUnit {
    pub(crate) dotted: String,
    pub(crate) source: Arc<Source>,
    pub(crate) stmts: Vec<Stmt>,
    pub(crate) imports: Vec<ResolvedImport>,
    pub(crate) bound: std::collections::HashSet<String>,
}

pub(crate) struct ModuleGraph {
    pub(crate) units: Vec<ModuleUnit>,
    // the prelude is visited first, so units at or below it are its own subtree and get no prelude
    pub(crate) prelude: Option<usize>,
}

impl ModuleGraph {
    pub(crate) fn root(&self) -> &ModuleUnit {
        self.units.last().expect("a graph always holds its root")
    }
}
