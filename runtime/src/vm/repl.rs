use super::VM;
use std::collections::{HashMap, HashSet};

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

    pub fn add_repl_symbol_origins(&mut self, origins: &HashMap<String, String>) {
        self.repl_symbol_origins
            .extend(origins.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    pub fn repl_module_aliases(&self) -> &HashSet<String> {
        &self.repl_module_aliases
    }

    pub fn repl_known_globals(&self) -> &HashSet<String> {
        &self.repl_known_globals
    }

    pub fn repl_known_native_globals(&self) -> &HashSet<String> {
        &self.repl_known_native_globals
    }

    pub fn repl_symbol_origins(&self) -> &HashMap<String, String> {
        &self.repl_symbol_origins
    }
}
