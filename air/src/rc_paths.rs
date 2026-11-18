use crate::{AirEnumDef, AirEnumVariant, AirStructDef, AirType, EnumRef};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcPathStep {
    Field(String),
    EnumPayload {
        enum_ref: EnumRef,
        tag: u32,
        field_index: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcLeafPath {
    pub steps: Vec<RcPathStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcScan {
    None,
    Paths(Vec<RcLeafPath>),
    Undecidable(String),
    RejectedMultiVariant(String),
}

enum ScanReject {
    Undecidable(String),
    MultiVariant(String),
}

#[derive(Clone, Copy)]
pub struct Defs<'a> {
    pub structs: &'a [AirStructDef],
    pub imported_structs: &'a [AirStructDef],
    pub enums: &'a [AirEnumDef],
    pub imported_enums: &'a [AirEnumDef],
}

impl<'a> Defs<'a> {
    pub fn local(structs: &'a [AirStructDef], enums: &'a [AirEnumDef]) -> Self {
        Defs {
            structs,
            imported_structs: &[],
            enums,
            imported_enums: &[],
        }
    }

    pub fn struct_fields(&self, r: &crate::StructRef) -> Option<Vec<(String, AirType)>> {
        crate::struct_fields_in(self.structs.iter().chain(self.imported_structs.iter()), r)
    }

    pub fn find_enum(&self, name: &str) -> Option<&'a AirEnumDef> {
        self.enums
            .iter()
            .chain(self.imported_enums.iter())
            .find(|e| e.name == name)
    }
}

pub fn rc_field_paths(ty: &AirType, structs: &[AirStructDef], enums: &[AirEnumDef]) -> RcScan {
    rc_field_paths_in(ty, Defs::local(structs, enums))
}

pub fn rc_field_paths_in(ty: &AirType, defs: Defs<'_>) -> RcScan {
    let mut paths = Vec::new();
    let mut visited = HashSet::new();
    match scan(ty, defs, &mut Vec::new(), &mut paths, &mut visited) {
        Ok(()) => {
            if paths.is_empty() {
                RcScan::None
            } else {
                RcScan::Paths(paths)
            }
        }
        Err(ScanReject::Undecidable(why)) => RcScan::Undecidable(why),
        Err(ScanReject::MultiVariant(why)) => RcScan::RejectedMultiVariant(why),
    }
}

pub fn air_type_has_rc(ty: &AirType, structs: &[AirStructDef], enums: &[AirEnumDef]) -> bool {
    air_type_has_rc_in(ty, Defs::local(structs, enums))
}

pub fn air_type_has_rc_in(ty: &AirType, defs: Defs<'_>) -> bool {
    !matches!(rc_field_paths_in(ty, defs), RcScan::None)
}

fn scan(
    ty: &AirType,
    defs: Defs<'_>,
    prefix: &mut Vec<RcPathStep>,
    out: &mut Vec<RcLeafPath>,
    visited: &mut HashSet<String>,
) -> Result<(), ScanReject> {
    match ty {
        AirType::Rc(_) => {
            out.push(RcLeafPath {
                steps: prefix.clone(),
            });
            Ok(())
        }
        AirType::Struct(r) => {
            let name = r.symbol();
            let Some(fields) = defs.struct_fields(r) else {
                return Err(ScanReject::Undecidable(format!(
                    "`{name}` (unresolved/generic struct instantiation; its Rc-ness is undecidable at AIR level)"
                )));
            };
            if fields.iter().any(|(_, ty)| field_is_erased_generic(ty)) {
                return Err(ScanReject::Undecidable(format!(
                    "`{name}` is a generic struct instantiated with erased type arguments; \
                     a generic carrier of `Rc<T>` is undecidable at AIR level"
                )));
            }
            if !visited.insert(name.clone()) {
                return Ok(());
            }
            for (field, field_ty) in &fields {
                prefix.push(RcPathStep::Field(field.clone()));
                let r = scan(field_ty, defs, prefix, out, visited);
                prefix.pop();
                r?;
            }
            visited.remove(&name);
            Ok(())
        }
        AirType::Enum(r) => {
            let name = r.symbol();
            let Some(def) = enum_instance(r, defs) else {
                return Ok(());
            };
            let def = &def;
            if def
                .variants
                .iter()
                .flat_map(|v| v.payload.iter())
                .any(field_is_erased_generic)
            {
                return Err(ScanReject::Undecidable(format!(
                    "`{name}` is a generic enum instantiated with erased type arguments; \
                     a generic carrier of `Rc<T>` is undecidable at AIR level"
                )));
            }
            if !visited.insert(name.clone()) {
                return Ok(());
            }
            if def.variants.len() > 1 {
                for v in &def.variants {
                    for pty in &v.payload {
                        if matches!(pty, AirType::Rc(_)) {
                            continue;
                        }
                        let mut probe = Vec::new();
                        scan(pty, defs, &mut Vec::new(), &mut probe, visited)?;
                        if !probe.is_empty() || holds_guarded_rc(pty, defs, &mut HashSet::new()) {
                            visited.remove(&name);
                            return Err(ScanReject::MultiVariant(format!(
                                "`{name}` is a multi-variant enum that carries an `Rc<T>` nested \
                                 inside a variant payload; a multi-variant enum carries an `Rc` \
                                 only as a direct payload, and two multi-variant enums on one \
                                 `Rc` path are not supported yet"
                            )));
                        }
                    }
                }
                visited.remove(&name);
                return Ok(());
            }
            for v in &def.variants {
                for (field_index, pty) in v.payload.iter().enumerate() {
                    prefix.push(RcPathStep::EnumPayload {
                        enum_ref: r.clone(),
                        tag: v.tag,
                        field_index: field_index as u32,
                    });
                    let res = scan(pty, defs, prefix, out, visited);
                    prefix.pop();
                    res?;
                }
            }
            visited.remove(&name);
            Ok(())
        }
        AirType::Array(inner, _) | AirType::Slice(inner) | AirType::Vec(inner) => {
            match rc_field_paths_in(inner, defs) {
                RcScan::None if !holds_guarded_rc(inner, defs, &mut HashSet::new()) => Ok(()),
                _ => Err(ScanReject::Undecidable(
                    "an array/slice field whose element carries an `Rc<T>` is not supported yet"
                        .to_string(),
                )),
            }
        }
        AirType::Param(_) => Err(ScanReject::Undecidable(
            "a field of erased generic type parameter; its `Rc<T>`-ness is undecidable at AIR level"
                .to_string(),
        )),
        AirType::I8
        | AirType::I16
        | AirType::I32
        | AirType::I64
        | AirType::U8
        | AirType::U16
        | AirType::U32
        | AirType::U64
        | AirType::F32
        | AirType::F64
        | AirType::Bool
        | AirType::Char
        | AirType::Str
        | AirType::Ptr(_)
        | AirType::FnPtr { .. }
        | AirType::Opaque
        | AirType::Void => Ok(()),
    }
}

// a multi-variant enum holds an rc only as a direct payload, so one tag guards each leaf
pub fn holds_guarded_rc(ty: &AirType, defs: Defs<'_>, visited: &mut HashSet<String>) -> bool {
    match ty {
        AirType::Struct(r) => {
            let name = r.symbol();
            let Some(fields) = defs.struct_fields(r) else {
                return false;
            };
            if !visited.insert(name.clone()) {
                return false;
            }
            let found = fields
                .iter()
                .any(|(_, ty)| holds_guarded_rc(ty, defs, visited));
            visited.remove(&name);
            found
        }
        AirType::Enum(r) => {
            let Some(def) = enum_instance(r, defs) else {
                return false;
            };
            let payloads = || def.variants.iter().flat_map(|v| v.payload.iter());
            if def.variants.len() > 1 && payloads().any(|p| matches!(p, AirType::Rc(_))) {
                return true;
            }
            if !visited.insert(def.name.clone()) {
                return false;
            }
            let found = payloads().any(|p| holds_guarded_rc(p, defs, visited));
            visited.remove(&def.name);
            found
        }
        AirType::Array(inner, _) | AirType::Slice(inner) | AirType::Vec(inner) => {
            holds_guarded_rc(inner, defs, visited)
        }
        AirType::I8
        | AirType::I16
        | AirType::I32
        | AirType::I64
        | AirType::U8
        | AirType::U16
        | AirType::U32
        | AirType::U64
        | AirType::F32
        | AirType::F64
        | AirType::Bool
        | AirType::Char
        | AirType::Str
        | AirType::Ptr(_)
        | AirType::Rc(_)
        | AirType::FnPtr { .. }
        | AirType::Param(_)
        | AirType::Opaque
        | AirType::Void => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracedLeaf {
    pub path: RcLeafPath,
    pub guard: Option<usize>,
}

// the collector's view: every rc leaf, a leaf under a multi-variant enum named with its guarding step
pub fn rc_traced_leaves(ty: &AirType, defs: Defs<'_>) -> Result<Vec<TracedLeaf>, String> {
    let mut out = Vec::new();
    trace(ty, defs, &mut Vec::new(), None, &mut out, &mut HashSet::new())?;
    Ok(out)
}

fn trace(
    ty: &AirType,
    defs: Defs<'_>,
    prefix: &mut Vec<RcPathStep>,
    guard: Option<usize>,
    out: &mut Vec<TracedLeaf>,
    visited: &mut HashSet<String>,
) -> Result<(), String> {
    match ty {
        AirType::Rc(_) => {
            out.push(TracedLeaf {
                path: RcLeafPath {
                    steps: prefix.clone(),
                },
                guard,
            });
            Ok(())
        }
        AirType::Struct(r) => {
            let name = r.symbol();
            let Some(fields) = defs.struct_fields(r) else {
                return Ok(());
            };
            if !visited.insert(name.clone()) {
                return Ok(());
            }
            for (field, field_ty) in &fields {
                prefix.push(RcPathStep::Field(field.clone()));
                let res = trace(field_ty, defs, prefix, guard, out, visited);
                prefix.pop();
                res?;
            }
            visited.remove(&name);
            Ok(())
        }
        AirType::Enum(r) => {
            let Some(def) = enum_instance(r, defs) else {
                return Ok(());
            };
            if !visited.insert(def.name.clone()) {
                return Ok(());
            }
            let multi = def.variants.len() > 1;
            for v in &def.variants {
                for (field_index, pty) in v.payload.iter().enumerate() {
                    let step_guard = if multi && matches!(pty, AirType::Rc(_)) {
                        if guard.is_some() {
                            return Err(format!(
                                "`{}` nests a multi-variant enum carrying an `Rc` under another one",
                                def.name
                            ));
                        }
                        Some(prefix.len())
                    } else {
                        guard
                    };
                    prefix.push(RcPathStep::EnumPayload {
                        enum_ref: r.clone(),
                        tag: v.tag,
                        field_index: field_index as u32,
                    });
                    let res = trace(pty, defs, prefix, step_guard, out, visited);
                    prefix.pop();
                    res?;
                }
            }
            visited.remove(&def.name);
            Ok(())
        }
        AirType::I8
        | AirType::I16
        | AirType::I32
        | AirType::I64
        | AirType::U8
        | AirType::U16
        | AirType::U32
        | AirType::U64
        | AirType::F32
        | AirType::F64
        | AirType::Bool
        | AirType::Char
        | AirType::Str
        | AirType::Ptr(_)
        | AirType::Array(..)
        | AirType::Slice(_)
        | AirType::Vec(_)
        | AirType::FnPtr { .. }
        | AirType::Param(_)
        | AirType::Opaque
        | AirType::Void => Ok(()),
    }
}

pub fn enum_instance(r: &EnumRef, defs: Defs<'_>) -> Option<AirEnumDef> {
    if let Some(def) = defs.find_enum(&r.symbol()) {
        return Some(def.clone());
    }
    let template = defs.find_enum(&r.name)?;
    if r.args.is_empty()
        || template.type_params.len() != r.args.len()
        || r.args.iter().any(crate::counts::mentions_param)
    {
        return None;
    }
    let variants = template
        .variants
        .iter()
        .map(|v| AirEnumVariant {
            name: v.name.clone(),
            tag: v.tag,
            payload: v
                .payload
                .iter()
                .map(|ty| crate::mono::substitute_type_params(ty, &template.type_params, &r.args))
                .collect(),
        })
        .collect();
    Some(AirEnumDef {
        name: r.symbol(),
        type_params: Vec::new(),
        variants,
        span: template.span,
    })
}

fn field_is_erased_generic(ty: &AirType) -> bool {
    match ty {
        AirType::Param(_) => true,
        AirType::Array(inner, _) | AirType::Slice(inner) | AirType::Vec(inner) => {
            field_is_erased_generic(inner)
        }
        AirType::I8
        | AirType::I16
        | AirType::I32
        | AirType::I64
        | AirType::U8
        | AirType::U16
        | AirType::U32
        | AirType::U64
        | AirType::F32
        | AirType::F64
        | AirType::Bool
        | AirType::Char
        | AirType::Str
        | AirType::Ptr(_)
        | AirType::Rc(_)
        | AirType::Struct(_)
        | AirType::Enum(_)
        | AirType::FnPtr { .. }
        | AirType::Opaque
        | AirType::Void => false,
    }
}
