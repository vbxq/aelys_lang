use crate::types::{EnumDef, InferType, StructDef};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    Global,
}

#[derive(Debug, Clone)]
pub struct ModuleValue {
    pub kind: ItemKind,
    pub ty: InferType,
    pub qualified: String,
    pub is_pub: bool,
}

#[derive(Debug, Clone)]
pub enum ModuleTypeDef {
    Struct(StructDef),
    Enum(EnumDef),
}

#[derive(Debug, Clone)]
pub struct ModuleType {
    pub def: ModuleTypeDef,
    // the air type name, carrying the `__q.` head so mono cannot strip a module segment
    pub qualified: String,
    pub is_pub: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ModuleExports {
    pub path: String,
    pub values: HashMap<String, ModuleValue>,
    pub types: HashMap<String, ModuleType>,
    pub reachable: HashMap<String, ModuleTypeDef>,
}

pub enum Lookup<'a, T> {
    Found(&'a T),
    NotPublic,
    Missing,
}

impl ModuleExports {
    pub fn value(&self, name: &str) -> Lookup<'_, ModuleValue> {
        match self.values.get(name) {
            Some(item) if item.is_pub => Lookup::Found(item),
            Some(_) => Lookup::NotPublic,
            None => Lookup::Missing,
        }
    }

    pub fn module_type(&self, name: &str) -> Lookup<'_, ModuleType> {
        match self.types.get(name) {
            Some(item) if item.is_pub => Lookup::Found(item),
            Some(_) => Lookup::NotPublic,
            None => Lookup::Missing,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModuleImports {
    pub is_importable: bool,
    pub namespaces: HashMap<String, Arc<ModuleExports>>,
    pub values: HashMap<String, ModuleValue>,
    pub types: HashMap<String, ModuleType>,
}

impl ModuleImports {
    pub fn is_empty(&self) -> bool {
        self.namespaces.is_empty() && self.values.is_empty() && self.types.is_empty()
    }
}

use crate::typed_ast::{TypedProgram, TypedStmtKind};
use crate::types::{EnumVariant, StructField};
use std::collections::HashSet;

pub fn qualify_value(path: &str, name: &str) -> String {
    if path.is_empty() {
        return name.to_string();
    }
    if let Some(rest) = name.strip_prefix("__lambda_") {
        return format!("__lambda_{}.{}", path, rest);
    }
    if name.starts_with("__") {
        let cut = name.rfind('_').map(|i| i + 1).unwrap_or(name.len());
        return format!("{}{}.{}", &name[..cut], path, &name[cut..]);
    }
    format!("{}.{}", path, name)
}

pub const TYPE_HEAD: &str = "__q.";

pub fn qualify_type_name(path: &str, name: &str) -> String {
    if path.is_empty() {
        return name.to_string();
    }
    // a generated type keeps its own head, or `starts_with` guards downstream stop biting
    if name.starts_with("__") {
        return qualify_value(path, name);
    }
    format!("{}{}.{}", TYPE_HEAD, path, name)
}

pub fn strip_type_head(name: &str) -> &str {
    name.strip_prefix(TYPE_HEAD).unwrap_or(name)
}

pub fn source_type_name(name: &str) -> &str {
    let bare = strip_type_head(name);
    bare.rsplit('.').next().unwrap_or(bare)
}

pub fn qualify_infer_type(ty: &InferType, path: &str, owned: &HashSet<String>) -> InferType {
    if path.is_empty() {
        return ty.clone();
    }
    let go = |t: &InferType| qualify_infer_type(t, path, owned);
    match ty {
        InferType::Struct(name) if owned.contains(name) => {
            InferType::Struct(qualify_type_name(path, name))
        }
        InferType::Enum(name, args) => {
            let name = if owned.contains(name) {
                qualify_type_name(path, name)
            } else {
                name.clone()
            };
            InferType::Enum(name, args.iter().map(go).collect())
        }
        InferType::Function { params, ret, nogc } => InferType::Function {
            params: params.iter().map(go).collect(),
            ret: Box::new(go(ret)),
            nogc: *nogc,
        },
        InferType::Array(inner, size) => InferType::Array(Box::new(go(inner)), *size),
        InferType::Vec(inner) => InferType::Vec(Box::new(go(inner))),
        InferType::Rc(inner) => InferType::Rc(Box::new(go(inner))),
        InferType::Ref { referent, mutable } => InferType::Ref {
            referent: Box::new(go(referent)),
            mutable: *mutable,
        },
        InferType::Slice { elem, mutable } => InferType::Slice {
            elem: Box::new(go(elem)),
            mutable: *mutable,
        },
        InferType::Tuple(items) => InferType::Tuple(items.iter().map(go).collect()),
        other => other.clone(),
    }
}

fn reach(
    ty: &InferType,
    table: &crate::types::TypeTable,
    out: &mut HashMap<String, ModuleTypeDef>,
) {
    let named = match ty {
        InferType::Struct(name) | InferType::Enum(name, _) => Some(name.clone()),
        _ => None,
    };
    if let Some(name) = named {
        if !out.contains_key(&name) {
            if let Some(def) = table.get_struct(&name) {
                out.insert(name.clone(), ModuleTypeDef::Struct(def.clone()));
                for field in &def.fields.clone() {
                    reach(&field.ty, table, out);
                }
            } else if let Some(def) = table.get_enum(&name) {
                out.insert(name.clone(), ModuleTypeDef::Enum(def.clone()));
                for variant in &def.variants.clone() {
                    for payload in &variant.data {
                        reach(payload, table, out);
                    }
                }
            }
        }
    }
    match ty {
        InferType::Enum(_, args) | InferType::Tuple(args) => {
            for arg in args {
                reach(arg, table, out);
            }
        }
        InferType::Function { params, ret, .. } => {
            for param in params {
                reach(param, table, out);
            }
            reach(ret, table, out);
        }
        InferType::Array(inner, _)
        | InferType::Vec(inner)
        | InferType::Rc(inner)
        | InferType::Ref {
            referent: inner, ..
        }
        | InferType::Slice { elem: inner, .. } => reach(inner, table, out),
        _ => {}
    }
}

fn field_is_pub(table: &crate::types::TypeTable, struct_name: &str, field: &str) -> bool {
    table
        .get_struct(struct_name)
        .and_then(|def| def.fields.iter().find(|f| f.name == field))
        .is_some_and(|f| f.is_pub)
}

pub fn collect_exports(path: &str, program: &TypedProgram) -> ModuleExports {
    let mut owned: HashSet<String> = HashSet::new();
    for stmt in &program.stmts {
        match &stmt.kind {
            TypedStmtKind::StructDecl { name, .. } | TypedStmtKind::EnumDecl { name, .. } => {
                owned.insert(name.clone());
            }
            _ => {}
        }
    }

    let mut exports = ModuleExports {
        path: path.to_string(),
        ..Default::default()
    };

    for stmt in &program.stmts {
        match &stmt.kind {
            TypedStmtKind::Function(func) => {
                let ty = qualify_infer_type(
                    &InferType::Function {
                        params: func.params.iter().map(|p| p.ty.clone()).collect(),
                        ret: Box::new(func.return_type.clone()),
                        nogc: func.declared_nogc,
                    },
                    path,
                    &owned,
                );
                exports.values.insert(
                    func.name.clone(),
                    ModuleValue {
                        kind: ItemKind::Function,
                        ty,
                        qualified: qualify_value(path, &func.name),
                        is_pub: func.is_pub,
                    },
                );
            }
            TypedStmtKind::Let {
                name,
                var_type,
                is_pub,
                ..
            } => {
                exports.values.insert(
                    name.clone(),
                    ModuleValue {
                        kind: ItemKind::Global,
                        ty: qualify_infer_type(var_type, path, &owned),
                        qualified: qualify_value(path, name),
                        is_pub: *is_pub,
                    },
                );
            }
            TypedStmtKind::StructDecl {
                name,
                type_params,
                fields,
                is_pub,
            } => {
                let qualified = qualify_type_name(path, name);
                let def = StructDef {
                    name: qualified.clone(),
                    type_params: type_params.clone(),
                    fields: fields
                        .iter()
                        .map(|(field, ty)| StructField {
                            name: field.clone(),
                            ty: qualify_infer_type(ty, path, &owned),
                            is_pub: field_is_pub(&program.type_table, name, field),
                        })
                        .collect(),
                };
                exports.types.insert(
                    name.clone(),
                    ModuleType {
                        def: ModuleTypeDef::Struct(def),
                        qualified,
                        is_pub: *is_pub,
                    },
                );
            }
            TypedStmtKind::EnumDecl {
                name,
                type_params,
                variants,
                is_pub,
            } => {
                let qualified = qualify_type_name(path, name);
                let def = EnumDef {
                    name: qualified.clone(),
                    type_params: type_params.clone(),
                    variants: variants
                        .iter()
                        .map(|(variant, tag, data)| EnumVariant {
                            name: variant.clone(),
                            tag: *tag,
                            data: data
                                .iter()
                                .map(|ty| qualify_infer_type(ty, path, &owned))
                                .collect(),
                        })
                        .collect(),
                };
                exports.types.insert(
                    name.clone(),
                    ModuleType {
                        def: ModuleTypeDef::Enum(def),
                        qualified,
                        is_pub: *is_pub,
                    },
                );
            }
            _ => {}
        }
    }

    let mut reachable = HashMap::new();
    for item in exports.values.values() {
        reach(&item.ty, &program.type_table, &mut reachable);
    }
    for ty in exports.types.values() {
        match &ty.def {
            ModuleTypeDef::Struct(def) => {
                for field in &def.fields {
                    reach(&field.ty, &program.type_table, &mut reachable);
                }
            }
            ModuleTypeDef::Enum(def) => {
                for variant in &def.variants {
                    for payload in &variant.data {
                        reach(payload, &program.type_table, &mut reachable);
                    }
                }
            }
        }
    }
    exports.reachable = reachable;

    exports
}
