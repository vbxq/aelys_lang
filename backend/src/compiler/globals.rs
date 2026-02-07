use super::Compiler;

impl Compiler {
    /// Get or create global index, translating imported names to qualified names.
    /// Use for function calls to direct imports.
    pub fn get_or_create_global_index(&mut self, name: &str) -> u16 {
        let actual_name = self.resolve_global_name(name).to_string();
        self.get_or_create_global_index_raw(&actual_name)
    }

    /// Get or create global index without name translation.
    /// Use for variable declarations and assignments.
    pub fn get_or_create_global_index_raw(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.global_indices.get(name) {
            idx
        } else {
            let idx = self.next_global_index;
            self.global_indices.insert(name.to_string(), idx);
            self.next_global_index += 1;
            idx
        }
    }

    pub fn resolve_global_name<'a>(&'a self, name: &'a str) -> &'a str {
        // Only translate to qualified name if it's a stdlib import (contains ::)
        // Custom module entries contain module path (like "mod_a") not qualified names
        self.symbol_origins
            .get(name)
            .filter(|origin| origin.contains("::"))
            .map(String::as_str)
            .unwrap_or(name)
    }
}
