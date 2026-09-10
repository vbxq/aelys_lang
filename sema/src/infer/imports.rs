use super::TypeInference;
use crate::constraint::{TypeError, TypeErrorKind};
use crate::modules::{ItemKind, Lookup, ModuleExports, ModuleImports, ModuleTypeDef, ModuleValue};
use crate::types::InferType;
use aelys_syntax::Span;

impl TypeInference {
    pub(crate) fn install_imports(&mut self, imports: ModuleImports) {
        for exports in imports.namespaces.values() {
            for def in exports.reachable.values() {
                self.register_module_type_body(def);
            }
            self.register_module_types(exports);
            for item in exports.values.values() {
                self.define_imported_value(item);
            }
        }
        for (local, item) in &imports.values {
            self.define_imported_value(item);
            self.import_aliases
                .insert(local.clone(), item.qualified.clone());
        }
        for (local, ty) in &imports.types {
            self.register_module_type_def(ty);
            self.import_aliases
                .insert(local.clone(), ty.qualified.clone());
        }
        self.module_imports = imports;
    }

    fn register_module_types(&mut self, exports: &ModuleExports) {
        for ty in exports.types.values() {
            self.register_module_type_def(ty);
        }
    }

    fn register_module_type_def(&mut self, ty: &crate::modules::ModuleType) {
        self.register_module_type_body(&ty.def);
    }

    fn register_module_type_body(&mut self, def: &ModuleTypeDef) {
        match def {
            ModuleTypeDef::Struct(def) => {
                self.imported_types.insert(def.name.clone());
                self.type_table.register_struct(def.clone());
            }
            ModuleTypeDef::Enum(def) => {
                self.imported_types.insert(def.name.clone());
                self.type_table.register_enum(def.clone());
            }
        }
    }

    pub(crate) fn field_is_reachable(&self, struct_name: &str, field: &str) -> bool {
        if !self.imported_types.contains(struct_name) {
            return true;
        }
        match self
            .type_table
            .get_struct(struct_name)
            .and_then(|def| def.fields.iter().find(|f| f.name == field))
        {
            Some(f) => f.is_pub,
            None => true,
        }
    }

    pub(crate) fn reject_private_field(&mut self, struct_name: &str, field: &str, span: Span) {
        if self.field_is_reachable(struct_name, field) {
            return;
        }
        let ty = crate::modules::strip_type_head(struct_name).to_string();
        self.errors.push(module_item_error(
            TypeErrorKind::FieldNotPublic {
                field: field.to_string(),
                ty,
            },
            span,
        ));
    }

    fn define_imported_value(&mut self, item: &ModuleValue) {
        match item.kind {
            ItemKind::Function => self
                .env
                .define_function_owned(item.qualified.clone(), item.ty.clone()),
            ItemKind::Global => {
                self.env
                    .define_local(item.qualified.clone(), item.ty.clone());
                self.imported_globals.insert(item.qualified.clone());
            }
        }
    }

    pub(crate) fn resolve_import_alias(&self, name: &str) -> Option<(String, InferType)> {
        let qualified = self.import_aliases.get(name)?;
        let ty = self
            .env
            .lookup(qualified)
            .or_else(|| self.env.lookup_function_ref(qualified))
            .cloned()?;
        Some((qualified.clone(), ty))
    }

    pub(crate) fn imported_type_name(&self, name: &str) -> Option<String> {
        let qualified = self.import_aliases.get(name)?;
        (self.type_table.has_struct(qualified) || self.type_table.has_enum(qualified))
            .then(|| qualified.clone())
    }

    pub(crate) fn air_type_name(&mut self, name: &str, span: Span) -> String {
        if let Some(qualified) = self.imported_type_name(name) {
            return qualified;
        }
        if name.contains('.')
            && let Some(qualified) = self.resolve_module_type_name(name, span)
        {
            return qualified;
        }
        name.to_string()
    }

    pub(crate) fn module_bound_elsewhere(&self, name: &str) -> Option<(String, String)> {
        let mut hits: Vec<(String, String)> = self
            .module_imports
            .namespaces
            .iter()
            .filter(|(binding, exports)| {
                binding.as_str() != name && exports.path.split('.').any(|seg| seg == name)
            })
            .map(|(binding, exports)| (exports.path.clone(), binding.clone()))
            .collect();
        hits.sort();
        hits.into_iter().next()
    }

    pub(crate) fn is_module_namespace(&self, name: &str) -> bool {
        self.module_imports.namespaces.contains_key(name)
            && self.env.lookup(name).is_none()
            && self.env.lookup_function_ref(name).is_none()
    }

    pub(crate) fn resolve_module_member(
        &mut self,
        namespace: &str,
        member: &str,
        span: Span,
    ) -> (String, InferType) {
        let Some(exports) = self.module_imports.namespaces.get(namespace).cloned() else {
            return (member.to_string(), InferType::Dynamic);
        };
        match exports.value(member) {
            Lookup::Found(item) => return (item.qualified.clone(), item.ty.clone()),
            Lookup::NotPublic => {
                self.errors.push(module_item_error(
                    TypeErrorKind::ModuleItemNotPublic {
                        module: exports.path.clone(),
                        item: member.to_string(),
                    },
                    span,
                ));
                return (member.to_string(), InferType::Dynamic);
            }
            Lookup::Missing => {}
        }
        self.errors.push(module_item_error(
            TypeErrorKind::ModuleItemNotFound {
                module: exports.path.clone(),
                item: member.to_string(),
            },
            span,
        ));
        (member.to_string(), InferType::Dynamic)
    }

    pub(crate) fn resolve_module_type_name(&mut self, name: &str, span: Span) -> Option<String> {
        let (namespace, member) = name.split_once('.')?;
        if member.contains('.') {
            return None;
        }
        let exports = self.module_imports.namespaces.get(namespace).cloned()?;
        match exports.module_type(member) {
            Lookup::Found(ty) => Some(ty.qualified.clone()),
            Lookup::NotPublic => {
                self.errors.push(module_item_error(
                    TypeErrorKind::ModuleItemNotPublic {
                        module: exports.path.clone(),
                        item: member.to_string(),
                    },
                    span,
                ));
                None
            }
            Lookup::Missing => {
                self.errors.push(module_item_error(
                    TypeErrorKind::ModuleItemNotFound {
                        module: exports.path.clone(),
                        item: member.to_string(),
                    },
                    span,
                ));
                None
            }
        }
    }
}

fn module_item_error(kind: TypeErrorKind, span: Span) -> TypeError {
    TypeError {
        kind,
        span,
        reason: crate::constraint::ConstraintReason::Other(String::new()),
        secondary_spans: Vec::new(),
        help: None,
        suggestion: None,
    }
}

#[cfg(test)]
mod tests {
    use super::TypeInference;
    use aelys_syntax::Span;

    #[test]
    fn a_qualified_type_name_passes_through_air_type_name_unchanged() {
        let mut infer = TypeInference::new();
        let span = Span::new(0, 0, 1, 1);
        for name in [
            "__q.res.Result",
            "__q.res.Option",
            "__q.a.b.Result",
            "__q.deeply.nested.path.Carrier",
        ] {
            assert_eq!(infer.air_type_name(name, span), name);
            assert!(
                infer.errors.is_empty(),
                "air_type_name({name}) must push no error, got {:?}",
                infer.errors
            );
        }
    }
}
