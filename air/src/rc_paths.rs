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

    pub fn find_struct(&self, name: &str) -> Option<&'a AirStructDef> {
        self.structs
            .iter()
            .chain(self.imported_structs.iter())
            .find(|s| s.name == name)
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
        AirType::Struct(name) => {
            let Some(def) = defs.find_struct(name) else {
                return Err(ScanReject::Undecidable(format!(
                    "`{name}` (unresolved/generic struct instantiation; its Rc-ness is undecidable at AIR level)"
                )));
            };
            if def.fields.iter().any(|f| field_is_erased_generic(&f.ty)) {
                return Err(ScanReject::Undecidable(format!(
                    "`{name}` is a generic struct instantiated with erased type arguments; \
                     a generic carrier of `Rc<T>` is undecidable at AIR level"
                )));
            }
            if !visited.insert(name.clone()) {
                return Ok(());
            }
            for f in &def.fields {
                prefix.push(RcPathStep::Field(f.name.clone()));
                let r = scan(&f.ty, defs, prefix, out, visited);
                prefix.pop();
                r?;
            }
            visited.remove(name);
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
                let mut probe = Vec::new();
                for v in &def.variants {
                    for pty in &v.payload {
                        scan(pty, defs, &mut Vec::new(), &mut probe, visited)?;
                    }
                }
                visited.remove(&name);
                if !probe.is_empty() {
                    return Err(ScanReject::MultiVariant(format!(
                        "`{name}` is a multi-variant enum carrying an `Rc<T>` in a variant payload; \
                         materializing the payload of the wrong variant would release/retain a \
                         non-Rc slot (SEGV/UAF). Multi-variant Rc enums are not supported yet"
                    )));
                }
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
                RcScan::None => Ok(()),
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

// a multi variant instance is refused after mono, so only a single variant template is traced
pub fn enum_instance(r: &EnumRef, defs: Defs<'_>) -> Option<AirEnumDef> {
    if let Some(def) = defs.find_enum(&r.symbol()) {
        return Some(def.clone());
    }
    let template = defs.find_enum(&r.name)?;
    if r.args.is_empty()
        || template.variants.len() != 1
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
