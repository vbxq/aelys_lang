use super::TypeInference;
use crate::types::InferType;
use aelys_syntax::{ImportKind, NeedsStmt};

impl TypeInference {
    /// Handle needs/import statement for type information
    pub(super) fn handle_needs_stmt(&mut self, needs: &NeedsStmt) {
        match &needs.kind {
            ImportKind::Symbols(names) => {
                for name in names {
                    self.env.define_local(name.clone(), InferType::Dynamic);
                }
            }
            ImportKind::Module { alias } => {
                let _module_name = alias
                    .clone()
                    .unwrap_or_else(|| needs.path.last().cloned().unwrap_or_default());
            }
            ImportKind::Wildcard => {}
        }
    }
}
