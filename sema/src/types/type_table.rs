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
    pub type_params: Vec<String>,
    pub fields: Vec<StructField>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub tag: u32,
    pub data: Vec<InferType>, // empty = unit variant, non-empty = tuple variant
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone, Default)]
pub struct TypeTable {
    structs: HashMap<String, StructDef>,
    enums: HashMap<String, EnumDef>,
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

    pub fn register_enum(&mut self, def: EnumDef) {
        self.enums.insert(def.name.clone(), def);
    }

    pub fn get_enum(&self, name: &str) -> Option<&EnumDef> {
        self.enums.get(name)
    }

    pub fn has_enum(&self, name: &str) -> bool {
        self.enums.contains_key(name)
    }
}
