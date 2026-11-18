use crate::types::{EnumDef, InferType, StructDef};
use aelys_syntax::TypeBounds;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    Global,
    Const,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleValue {
    pub kind: ItemKind,
    pub ty: InferType,
    pub qualified: String,
    pub is_pub: bool,
    pub type_params: Vec<String>,
    // the importer has to re-check the bounds, the exporting module never sees this call site
    pub bounds: Vec<TypeBounds>,
    pub literal: Option<crate::typed_ast::LiteralInit>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleTypeDef {
    Struct(StructDef),
    Enum(EnumDef),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleType {
    pub def: ModuleTypeDef,
    // the air type name, carrying the `__q.` head so mono cannot strip a module segment
    pub qualified: String,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AliasDef {
    pub params: Vec<String>,
    pub target: aelys_syntax::TypeAnnotation,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodEntry {
    pub name: String,
    pub receiver: aelys_syntax::Receiver,
    pub fn_name: String,
    pub is_pub: bool,
    pub module: String,
    pub value: Option<ModuleValue>,
    pub head: Option<(Vec<String>, InferType)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethodSig {
    pub name: String,
    pub receiver: aelys_syntax::Receiver,
    pub has_default: bool,
    pub nogc: bool,
    pub stub: String,
    pub ty: InferType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
    pub name: String,
    pub qualified: String,
    pub type_params: Vec<String>,
    pub module: String,
    pub is_pub: bool,
    pub methods: Vec<TraitMethodSig>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplDef {
    pub trait_q: String,
    pub trait_args: Vec<InferType>,
    pub type_params: Vec<String>,
    pub bounds: Vec<TypeBounds>,
    pub self_ty: InferType,
    pub methods: Vec<(String, String)>,
    pub module: String,
    pub span: aelys_syntax::Span,
    pub values: Vec<ModuleValue>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleExports {
    pub path: String,
    pub values: HashMap<String, ModuleValue>,
    pub types: HashMap<String, ModuleType>,
    pub reachable: HashMap<String, ModuleTypeDef>,
    pub aliases: HashMap<String, AliasDef>,
    pub methods: HashMap<String, Vec<MethodEntry>>,
    pub traits: HashMap<String, TraitDef>,
    pub impls: Vec<ImplDef>,
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
    pub aliases: HashMap<String, AliasDef>,
    pub methods: HashMap<String, Vec<MethodEntry>>,
    pub module_path: String,
    pub std_privilege: bool,
    pub traits: HashMap<String, TraitDef>,
    pub all_traits: HashMap<String, TraitDef>,
    pub impls: Vec<ImplDef>,
}

impl ModuleImports {
    pub fn is_empty(&self) -> bool {
        self.namespaces.is_empty()
            && self.values.is_empty()
            && self.types.is_empty()
            && self.aliases.is_empty()
    }
}

use crate::typed_ast::{TypedProgram, TypedStmtKind};
use crate::types::{EnumVariant, StructField};
use std::collections::HashSet;

pub fn qualify_value(path: &str, name: &str) -> String {
    if path.is_empty() || name.starts_with("__impl$") || name.starts_with("__trait$") {
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

pub(crate) fn qualify_alias_body(
    ann: &mut aelys_syntax::TypeAnnotation,
    path: &str,
    owned: &HashSet<String>,
    params: &[String],
    namespaces: Option<&HashMap<String, Arc<ModuleExports>>>,
) {
    let bare = ann.type_param.is_none() && ann.type_params.is_empty();
    if !(bare && params.contains(&ann.name)) {
        if owned.contains(&ann.name) {
            ann.name = qualify_type_name(path, &ann.name);
        } else if let Some(namespaces) = namespaces
            && let Some((ns, member)) = ann.name.split_once('.')
            && let Some(ex) = namespaces.get(ns)
            && let Some(ty) = ex.types.get(member)
        {
            ann.name = ty.qualified.clone();
        }
    }
    let go = |a: &mut aelys_syntax::TypeAnnotation| {
        qualify_alias_body(a, path, owned, params, namespaces)
    };
    if let Some(p) = &mut ann.type_param {
        go(p);
    }
    for p in &mut ann.type_params {
        go(p);
    }
    if let Some(ps) = &mut ann.fn_params {
        for p in ps {
            go(p);
        }
    }
    if let Some(r) = &mut ann.fn_ret {
        go(r);
    }
    if let Some(es) = &mut ann.tuple_elems {
        for e in es {
            go(e);
        }
    }
}

pub fn module_of_type(qualified: &str) -> Option<&str> {
    let bare = qualified.strip_prefix(TYPE_HEAD)?;
    bare.rsplit_once('.').map(|(module, _)| module)
}

pub fn is_std_module(path: &str) -> bool {
    path == "std" || path.starts_with("std.")
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
        InferType::Struct(name, args) => {
            let name = if owned.contains(name) {
                qualify_type_name(path, name)
            } else {
                name.clone()
            };
            InferType::Struct(name, args.iter().map(go).collect())
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
        InferType::Struct(name, _) | InferType::Enum(name, _) => Some(name.clone()),
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
        InferType::Enum(_, args) | InferType::Struct(_, args) | InferType::Tuple(args) => {
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

fn qualify_def(def: ModuleTypeDef, path: &str, owned: &HashSet<String>) -> ModuleTypeDef {
    match def {
        ModuleTypeDef::Struct(mut def) => {
            for field in &mut def.fields {
                field.ty = qualify_infer_type(&field.ty, path, owned);
            }
            ModuleTypeDef::Struct(def)
        }
        ModuleTypeDef::Enum(mut def) => {
            for variant in &mut def.variants {
                for payload in &mut variant.data {
                    *payload = qualify_infer_type(payload, path, owned);
                }
            }
            ModuleTypeDef::Enum(def)
        }
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
                let bounds = func
                    .type_params
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let mut set = func.bounds.get(i).cloned().unwrap_or_default();
                        set.nogc |= func.declared_nogc;
                        set
                    })
                    .collect();
                let value = ModuleValue {
                    kind: ItemKind::Function,
                    ty,
                    qualified: qualify_value(path, &func.name),
                    is_pub: func.is_pub,
                    type_params: func.type_params.clone(),
                    bounds,
                    literal: None,
                };
                let foreign = program
                    .decls
                    .foreign_methods
                    .iter()
                    .find(|(fn_name, _)| *fn_name == func.name)
                    .map(|(_, key)| key.clone());
                let method_key = match (func.name.split_once("::"), foreign) {
                    (Some((_, mname)), Some(key)) => Some((key, mname)),
                    (Some((tname, mname)), None) if owned.contains(tname) => {
                        Some((qualify_type_name(path, tname), mname))
                    }
                    (Some((tname, mname)), None) if path.starts_with("std.") => {
                        crate::infer::builtin_key_of_prefix(tname).map(|k| (k.to_string(), mname))
                    }
                    _ => None,
                };
                if let Some((key, mname)) = method_key {
                    let receiver = match func.params.first() {
                        Some(p) if p.name == "self" => match &p.ty {
                            InferType::Ref { mutable: true, .. }
                            | InferType::Slice { mutable: true, .. } => {
                                aelys_syntax::Receiver::RefMut
                            }
                            InferType::Ref { .. } | InferType::Slice { .. } => {
                                aelys_syntax::Receiver::Ref
                            }
                            _ => aelys_syntax::Receiver::Value,
                        },
                        _ => aelys_syntax::Receiver::None,
                    };
                    exports.methods.entry(key).or_default().push(MethodEntry {
                        name: mname.to_string(),
                        receiver,
                        fn_name: value.qualified.clone(),
                        is_pub: func.is_pub,
                        module: path.to_string(),
                        value: Some(value.clone()),
                        head: None,
                    });
                }
                exports.values.insert(func.name.clone(), value);
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
                        type_params: Vec::new(),
                        bounds: Vec::new(),
                        literal: program.literal_globals.get(name).cloned(),
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
    // a neighbour's type seen from inside a cycle names this module's types bare
    exports.reachable = reachable
        .into_iter()
        .filter(|(name, _)| !owned.contains(name))
        .map(|(name, def)| (name, qualify_def(def, path, &owned)))
        .collect();

    for (name, is_pub, lit) in &program.decls.consts {
        exports.values.insert(
            name.clone(),
            ModuleValue {
                kind: ItemKind::Const,
                ty: lit.ty.clone(),
                qualified: qualify_value(path, name),
                is_pub: *is_pub,
                type_params: Vec::new(),
                bounds: Vec::new(),
                literal: Some(crate::typed_ast::LiteralInit::Const(lit.clone())),
            },
        );
    }
    for (name, def) in &program.decls.aliases {
        let mut def = def.clone();
        qualify_alias_body(&mut def.target, path, &owned, &def.params, None);
        exports.aliases.insert(name.clone(), def);
    }

    for def in &program.traits {
        let mut def = def.clone();
        for m in def.methods.iter_mut() {
            m.ty = qualify_infer_type(&m.ty, path, &owned);
        }
        exports.traits.insert(def.name.clone(), def);
    }
    for imp in &program.impls {
        let mut imp = imp.clone();
        imp.self_ty = qualify_infer_type(&imp.self_ty, path, &owned);
        imp.trait_args = imp
            .trait_args
            .iter()
            .map(|t| qualify_infer_type(t, path, &owned))
            .collect();
        imp.values = imp
            .methods
            .iter()
            .filter_map(|(_, sym)| exports.values.get(sym).cloned())
            .collect();
        exports.impls.push(imp);
    }
    exports
}
