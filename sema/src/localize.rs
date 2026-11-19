use crate::modules::{
    AliasDef, ImplDef, MethodEntry, ModuleExports, ModuleImports, ModuleTypeDef, ModuleValue,
    TYPE_HEAD, TraitDef,
};
use crate::types::InferType;
use aelys_syntax::TypeAnnotation;
use std::collections::HashMap;
use std::sync::Arc;

// a module names its own types bare, so `__q.<path>.T` coming back from a neighbour means `T`
pub fn localize_imports(imports: &mut ModuleImports) {
    if imports.module_path.is_empty() {
        return;
    }
    let own = Own {
        head: format!("{TYPE_HEAD}{}.", imports.module_path),
    };
    for exports in imports.namespaces.values_mut() {
        own.exports(Arc::make_mut(exports));
    }
    for value in imports.values.values_mut() {
        own.value(value);
    }
    for ty in imports.types.values_mut() {
        own.def(&mut ty.def);
    }
    for alias in imports.aliases.values_mut() {
        own.alias(alias);
    }
    imports.methods = own.methods(std::mem::take(&mut imports.methods));
    for def in imports
        .traits
        .values_mut()
        .chain(imports.all_traits.values_mut())
    {
        own.trait_def(def);
    }
    for imp in &mut imports.impls {
        own.impl_def(imp);
    }
}

pub fn erase_type_vars(exports: &mut ModuleExports) {
    for value in exports.values.values_mut() {
        erase(&mut value.ty);
    }
    for entries in exports.methods.values_mut() {
        for entry in entries {
            if let Some(value) = &mut entry.value {
                erase(&mut value.ty);
            }
        }
    }
    for imp in &mut exports.impls {
        for value in &mut imp.values {
            erase(&mut value.ty);
        }
    }
}

fn erase(ty: &mut InferType) {
    if matches!(ty, InferType::Var(_)) {
        *ty = InferType::Dynamic;
        return;
    }
    for_each_child(ty, erase);
}

fn for_each_child(ty: &mut InferType, mut f: impl FnMut(&mut InferType)) {
    match ty {
        InferType::Struct(_, args) | InferType::Enum(_, args) | InferType::Tuple(args) => {
            args.iter_mut().for_each(f)
        }
        InferType::Function { params, ret, .. } => {
            params.iter_mut().for_each(&mut f);
            f(ret);
        }
        InferType::Array(inner, _)
        | InferType::Vec(inner)
        | InferType::Rc(inner)
        | InferType::Ref {
            referent: inner, ..
        }
        | InferType::Slice { elem: inner, .. } => f(inner),
        _ => {}
    }
}

struct Own {
    head: String,
}

impl Own {
    fn is_own(&self, name: &str) -> bool {
        name.strip_prefix(&self.head)
            .is_some_and(|rest| !rest.contains('.'))
    }

    fn name(&self, name: &mut String) {
        if self.is_own(name) {
            name.drain(..self.head.len());
        }
    }

    fn ty(&self, ty: &mut InferType) {
        if let InferType::Struct(name, _) | InferType::Enum(name, _) = ty {
            self.name(name);
        }
        for_each_child(ty, |child| self.ty(child));
    }

    fn value(&self, value: &mut ModuleValue) {
        self.ty(&mut value.ty);
    }

    fn def(&self, def: &mut ModuleTypeDef) {
        match def {
            ModuleTypeDef::Struct(def) => {
                self.name(&mut def.name);
                for field in &mut def.fields {
                    self.ty(&mut field.ty);
                }
            }
            ModuleTypeDef::Enum(def) => {
                self.name(&mut def.name);
                for variant in &mut def.variants {
                    variant.data.iter_mut().for_each(|ty| self.ty(ty));
                }
            }
        }
    }

    fn annotation(&self, ann: &mut TypeAnnotation) {
        self.name(&mut ann.name);
        if let Some(inner) = &mut ann.type_param {
            self.annotation(inner);
        }
        for inner in ann
            .type_params
            .iter_mut()
            .chain(ann.fn_params.iter_mut().flatten())
            .chain(ann.tuple_elems.iter_mut().flatten())
        {
            self.annotation(inner);
        }
        if let Some(ret) = &mut ann.fn_ret {
            self.annotation(ret);
        }
    }

    fn alias(&self, alias: &mut AliasDef) {
        self.annotation(&mut alias.target);
    }

    fn methods(
        &self,
        methods: HashMap<String, Vec<MethodEntry>>,
    ) -> HashMap<String, Vec<MethodEntry>> {
        let mut out: HashMap<String, Vec<MethodEntry>> = HashMap::new();
        for (mut key, mut entries) in methods {
            self.name(&mut key);
            for entry in &mut entries {
                if let Some(value) = &mut entry.value {
                    self.value(value);
                }
                if let Some((_, head)) = &mut entry.head {
                    self.ty(head);
                }
            }
            out.entry(key).or_default().extend(entries);
        }
        out
    }

    fn trait_def(&self, def: &mut TraitDef) {
        for method in &mut def.methods {
            self.ty(&mut method.ty);
        }
    }

    fn impl_def(&self, imp: &mut ImplDef) {
        self.ty(&mut imp.self_ty);
        imp.trait_args.iter_mut().for_each(|ty| self.ty(ty));
        imp.values.iter_mut().for_each(|value| self.value(value));
    }

    fn exports(&self, exports: &mut ModuleExports) {
        for value in exports.values.values_mut() {
            self.value(value);
        }
        for ty in exports.types.values_mut() {
            self.def(&mut ty.def);
        }
        exports.reachable.retain(|name, _| !self.is_own(name));
        for def in exports.reachable.values_mut() {
            self.def(def);
        }
        for alias in exports.aliases.values_mut() {
            self.alias(alias);
        }
        exports.methods = self.methods(std::mem::take(&mut exports.methods));
        for def in exports.traits.values_mut() {
            self.trait_def(def);
        }
        for imp in &mut exports.impls {
            self.impl_def(imp);
        }
    }
}
