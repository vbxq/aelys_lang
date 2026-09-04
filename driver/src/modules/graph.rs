use aelys_syntax::{Source, Stmt};
use std::sync::Arc;

pub(crate) enum ResolvedImport {
    // `needs a.b` and `needs a.b as k` bind one name to a whole module
    Namespace {
        local: String,
        target: usize,
    },
    // `needs x, y from a.b` binds each name to one exported item
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
}

pub(crate) struct ModuleGraph {
    // dependency order: every unit appears after everything it imports, the root last
    pub(crate) units: Vec<ModuleUnit>,
}

impl ModuleGraph {
    pub(crate) fn root(&self) -> &ModuleUnit {
        self.units.last().expect("a graph always holds its root")
    }
}
