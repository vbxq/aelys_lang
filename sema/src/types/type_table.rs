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

    pub fn rc_nominal_scan(&self, ty: &InferType) -> RcNominalScan {
        let mut visited = std::collections::HashSet::new();
        if self.scan_rc_nominal(ty, &mut visited) {
            RcNominalScan::HasRc
        } else {
            RcNominalScan::None
        }
    }
    
    pub fn contains_rc_nominal(&self, ty: &InferType) -> bool {
        let mut visited = std::collections::HashSet::new();
        self.scan_rc_nominal(ty, &mut visited)
    }

    pub fn contains_vec_by_value(&self, ty: &InferType) -> bool {
        let mut visited = std::collections::HashSet::new();
        self.scan_vec_by_value(ty, &mut visited)
    }

    fn scan_vec_by_value(
        &self,
        ty: &InferType,
        visited: &mut std::collections::HashSet<String>,
    ) -> bool {
        match ty {
            InferType::Vec(_) => true,
            InferType::Array(inner, _) => self.scan_vec_by_value(inner, visited),
            InferType::Tuple(elems) => elems.iter().any(|e| self.scan_vec_by_value(e, visited)),
            // behind an Rc the Vec is a pointer, not held by value
            InferType::Rc(_) => false,
            InferType::Function { .. } => false,
            InferType::Struct(name) => {
                let Some(def) = self.structs.get(name) else {
                    return false;
                };
                if !def.type_params.is_empty() {
                    return false;
                }
                if !visited.insert(name.clone()) {
                    return false;
                }
                let found = def
                    .fields
                    .iter()
                    .any(|f| self.scan_vec_by_value(&f.ty, visited));
                visited.remove(name);
                found
            }
            InferType::Enum(name, args) => {
                if args.iter().any(|a| self.scan_vec_by_value(a, visited)) {
                    return true;
                }
                if let Some(def) = self.enums.get(name) {
                    if def.type_params.is_empty() && visited.insert(name.clone()) {
                        let found = def
                            .variants
                            .iter()
                            .flat_map(|v| v.data.iter())
                            .any(|d| self.scan_vec_by_value(d, visited));
                        visited.remove(name);
                        return found;
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn scan_rc_nominal(
        &self,
        ty: &InferType,
        visited: &mut std::collections::HashSet<String>,
    ) -> bool {
        match ty {
            InferType::Rc(_) => true,
            InferType::Array(inner, _) | InferType::Vec(inner) => {
                self.scan_rc_nominal(inner, visited)
            }
            InferType::Tuple(elems) => elems.iter().any(|e| self.scan_rc_nominal(e, visited)),
            InferType::Function { .. } => false,
            InferType::Struct(name) => {
                let Some(def) = self.structs.get(name) else {
                    // unresolvable here, the construction site guards this instead
                    return false;
                };
                // fields are erased type params, so recursing would miss or false-positive
                if !def.type_params.is_empty() {
                    return false;
                }
                if !visited.insert(name.clone()) {
                    return false;
                }
                let found = def.fields.iter().any(|f| self.scan_rc_nominal(&f.ty, visited));
                visited.remove(name);
                found
            }
            InferType::Enum(name, args) => {
                // a generic enum keeps its concrete args, which already carry the Rc-ness
                if args.iter().any(|a| self.scan_rc_nominal(a, visited)) {
                    return true;
                }
                if let Some(def) = self.enums.get(name) {
                    if def.type_params.is_empty() && visited.insert(name.clone()) {
                        let found = def
                            .variants
                            .iter()
                            .flat_map(|v| v.data.iter())
                            .any(|d| self.scan_rc_nominal(d, visited));
                        visited.remove(name);
                        return found;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcNominalScan {
    None,
    HasRc,
}
