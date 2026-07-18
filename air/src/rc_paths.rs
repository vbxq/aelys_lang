use crate::{AirEnumDef, AirStructDef, AirType};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RcPathStep {
    Field(String),
    EnumPayload {
        enum_name: String,
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

pub fn rc_field_paths(ty: &AirType, structs: &[AirStructDef], enums: &[AirEnumDef]) -> RcScan {
    let mut paths = Vec::new();
    let mut visited = HashSet::new();
    match scan(ty, structs, enums, &mut Vec::new(), &mut paths, &mut visited) {
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
    !matches!(rc_field_paths(ty, structs, enums), RcScan::None)
}

fn scan(
    ty: &AirType,
    structs: &[AirStructDef],
    enums: &[AirEnumDef],
    prefix: &mut Vec<RcPathStep>,
    out: &mut Vec<RcLeafPath>,
    visited: &mut HashSet<String>,
) -> Result<(), ScanReject> {
    match ty {
        AirType::Ptr(_) => {
            out.push(RcLeafPath {
                steps: prefix.clone(),
            });
            Ok(())
        }
        AirType::Struct(name) => {
            let Some(def) = structs.iter().find(|s| &s.name == name) else {
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
                let r = scan(&f.ty, structs, enums, prefix, out, visited);
                prefix.pop();
                r?;
            }
            visited.remove(name);
            Ok(())
        }
        AirType::Enum(name) => {
            let Some(def) = enums.iter().find(|e| &e.name == name) else {
                return Ok(());
            };
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
                        scan(pty, structs, enums, &mut Vec::new(), &mut probe, visited)?;
                    }
                }
                visited.remove(name);
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
                        enum_name: name.clone(),
                        tag: v.tag,
                        field_index: field_index as u32,
                    });
                    let r = scan(pty, structs, enums, prefix, out, visited);
                    prefix.pop();
                    r?;
                }
            }
            visited.remove(name);
            Ok(())
        }
        AirType::Array(inner, _) | AirType::Slice(inner) | AirType::Vec(inner) => {
            match rc_field_paths(inner, structs, enums) {
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
        | AirType::Str
        | AirType::FnPtr { .. }
        | AirType::Opaque
        | AirType::Void => Ok(()),
    }
}

fn field_is_erased_generic(ty: &AirType) -> bool {
    match ty {
        AirType::Param(_) => true,
        AirType::Array(inner, _) | AirType::Slice(inner) | AirType::Vec(inner) => {
            field_is_erased_generic(inner)
        }
        _ => false,
    }
}
