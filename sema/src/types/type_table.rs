use super::InferType;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub ty: InferType,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<StructField>,
}

#[derive(Debug, Clone, Default)]
pub struct TypeTable {
    structs: HashMap<String, StructDef>,
}

impl TypeTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_struct(&mut self, def: StructDef) {
        self.structs.insert(def.name.clone(), def);
    }

    pub fn get_struct(&self, name: &str) -> Option<&StructDef> {
        self.structs.get(name)
    }

    pub fn has_struct(&self, name: &str) -> bool {
        self.structs.contains_key(name)
    }
}
