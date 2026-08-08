use super::{InferType, TypeTable};
use std::collections::HashSet;

impl TypeTable {
/// stricter than "not managed": an owned `string` and a general `fn` are also rejected. the walk
/// is fail-closed, so any type it cannot resolve is not a nogc value.
    pub fn is_nogc_value(&self, ty: &InferType) -> bool {
        let mut visited = HashSet::new();
        self.scan_nogc_value(ty, &[], &mut visited)
    }

    fn scan_nogc_value(
        &self,
        ty: &InferType,
        checked_params: &[String],
        visited: &mut HashSet<String>,
    ) -> bool {
        match ty {
            InferType::I8
            | InferType::I16
            | InferType::I32
            | InferType::I64
            | InferType::U8
            | InferType::U16
            | InferType::U32
            | InferType::U64
            | InferType::F32
            | InferType::F64
            | InferType::Bool
            | InferType::Null
            | InferType::Never => true,
            InferType::String => false,
// a reference never owns its referent
            InferType::Ref { .. } | InferType::Slice { .. } => true,
            InferType::Function { nogc, .. } => *nogc,
            InferType::Vec(_) | InferType::Rc(_) => false,
            InferType::Array(inner, Some(_)) => {
                self.scan_nogc_value(inner, checked_params, visited)
            }
            InferType::Tuple(elems) => elems
                .iter()
                .all(|e| self.scan_nogc_value(e, checked_params, visited)),
            InferType::Struct(name) => {
                if !self.has_struct(name) {
                    return checked_params.iter().any(|p| p == name);
                }
                let def = match self.get_struct(name) {
                    Some(def) => def,
                    None => return false,
                };
// a struct carries no type args, so a generic one has nothing to recurse on
                if !def.type_params.is_empty() {
                    return false;
                }
                if !visited.insert(name.clone()) {
                    return true;
                }
                let ok = def
                    .fields
                    .iter()
                    .all(|f| self.scan_nogc_value(&f.ty, &[], visited));
                visited.remove(name);
                ok
            }
            InferType::Enum(name, args) => {
                if !args
                    .iter()
                    .all(|a| self.scan_nogc_value(a, checked_params, visited))
                {
                    return false;
                }
                let def = match self.get_enum(name) {
                    Some(def) => def,
                    None => return false,
                };
                if def.type_params.len() != args.len() {
                    return false;
                }
                if !visited.insert(name.clone()) {
                    return true;
                }
                let ok = def
                    .variants
                    .iter()
                    .flat_map(|v| v.data.iter())
                    .all(|d| self.scan_nogc_value(d, &def.type_params, visited));
                visited.remove(name);
                ok
            }
            _ => false,
        }
    }
}

