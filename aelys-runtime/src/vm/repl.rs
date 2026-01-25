use super::VM;
use std::collections::HashSet;

impl VM {
    pub fn add_repl_module_aliases(&mut self, aliases: &HashSet<String>) {
        self.repl_module_aliases.extend(aliases.iter().cloned());
    }

    pub fn add_repl_known_globals(&mut self, globals: &HashSet<String>) {
        self.repl_known_globals.extend(globals.iter().cloned());
    }

    pub fn add_repl_known_native_globals(&mut self, globals: &HashSet<String>) {
        self.repl_known_native_globals
            .extend(globals.iter().cloned());
    }

    /// Get the current REPL module aliases.
    pub fn repl_module_aliases(&self) -> &HashSet<String> {
        &self.repl_module_aliases
    }

    /// Get the current REPL known globals.
    pub fn repl_known_globals(&self) -> &HashSet<String> {
        &self.repl_known_globals
    }

    /// Get the current REPL known native globals.
    pub fn repl_known_native_globals(&self) -> &HashSet<String> {
        &self.repl_known_native_globals
    }
}
