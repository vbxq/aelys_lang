use super::Compiler;

impl Compiler {
    pub fn get_or_create_global_index(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.global_indices.get(name) {
            idx
        } else {
            let idx = self.next_global_index;
            self.global_indices.insert(name.to_string(), idx);
            self.next_global_index += 1;
            idx
        }
    }
}
