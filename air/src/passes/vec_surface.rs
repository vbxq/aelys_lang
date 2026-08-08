use crate::{AirProgram, AirStmtKind, AirType, Span};
use std::collections::{HashMap, HashSet};

pub const MARKER: &str = "[vec-surface]";

pub struct VecSurfaceError {
    pub message: String,
    pub span: Option<Span>,
}

pub fn check_vec_surface(program: &AirProgram) -> Result<(), Vec<VecSurfaceError>> {
    let scan = Scan {
        structs: program
            .structs
            .iter()
            .map(|s| (s.name.as_str(), s))
            .collect(),
        enums: program.enums.iter().map(|e| (e.name.as_str(), e)).collect(),
    };
    let mut errors = Vec::new();

    for func in &program.functions {
        for param in &func.params {
            scan.slot(
                &param.ty,
                &format!("parameter `{}` of `{}`", param.name, func.name),
                param.span.or(func.span),
                &mut errors,
            );
        }
        for local in &func.locals {
            let what = match &local.name {
                Some(name) => format!("local `{}` of `{}`", name, func.name),
                None => format!("a temporary of `{}`", func.name),
            };
            scan.slot(&local.ty, &what, local.span.or(func.span), &mut errors);
        }
        for block in &func.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::RcAlloc { ty, .. } = &stmt.kind {
                    scan.held(
                        ty,
                        &format!("the `Rc` payload allocated in `{}`", func.name),
                        stmt.span.or(func.span),
                        &mut errors,
                    );
                }
            }
        }
    }

    for global in &program.globals {
        scan.slot(
            &global.ty,
            &format!("global `{}`", global.name),
            global.span,
            &mut errors,
        );
    }

// e2b already rejected every concrete-vec instantiation at the mono call site, so no surviving
    for def in &program.structs {
        if !def.type_params.is_empty() {
            continue;
        }
        for field in &def.fields {
            scan.slot(
                &field.ty,
                &format!("field `{}` of struct `{}`", field.name, def.name),
                def.span,
                &mut errors,
            );
        }
    }

    for def in &program.enums {
        if !def.type_params.is_empty() {
            continue;
        }
        for variant in &def.variants {
            for payload in &variant.payload {
                scan.held(
                    payload,
                    &format!("the payload of `{}::{}`", def.name, variant.name),
                    def.span,
                    &mut errors,
                );
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn type_args_reject(program: &AirProgram, ty: &AirType) -> Option<String> {
    let scan = Scan {
        structs: program
            .structs
            .iter()
            .map(|s| (s.name.as_str(), s))
            .collect(),
        enums: program.enums.iter().map(|e| (e.name.as_str(), e)).collect(),
    };
    let mut visited = HashSet::new();
    match scan.holds_vec(ty, &mut visited) {
        Ok(true) => Some(format!("`{}` holds a `Vec<T>` by value", type_name(ty))),
        Ok(false) => None,
        Err(reason) => Some(reason),
    }
}

struct Scan<'a> {
    structs: HashMap<&'a str, &'a crate::AirStructDef>,
    enums: HashMap<&'a str, &'a crate::AirEnumDef>,
}

impl Scan<'_> {
    fn slot(&self, ty: &AirType, what: &str, span: Option<Span>, out: &mut Vec<VecSurfaceError>) {
        let mut visited = HashSet::new();
        match self.offend_slot(ty, &mut visited) {
            Ok(false) => {}
            Ok(true) => out.push(VecSurfaceError {
                message: format!(
                    "{MARKER} {what} has type `{}`, which holds a `Vec<T>` by value inside \
                     another container; a Vec inside a Vec/array/struct/enum is not supported \
                     yet (the buffer would be shared without a retain, the transitive Vec \
                     retain/release is not implemented)",
                    type_name(ty)
                ),
                span,
            }),
            Err(reason) => out.push(VecSurfaceError {
                message: format!(
                    "{MARKER} {what} has type `{}`, which the Vec surface check cannot decide: \
                     {reason}",
                    type_name(ty)
                ),
                span,
            }),
        }
    }

    fn held(&self, ty: &AirType, what: &str, span: Option<Span>, out: &mut Vec<VecSurfaceError>) {
        let mut visited = HashSet::new();
        match self.holds_vec(ty, &mut visited) {
            Ok(false) => {}
            Ok(true) => out.push(VecSurfaceError {
                message: format!(
                    "{MARKER} {what} has type `{}`, which holds a `Vec<T>` by value; a Vec \
                     inside a Vec/array/struct/enum/Rc is not supported yet (the buffer would \
                     be shared without a retain, the transitive Vec retain/release is not \
                     implemented)",
                    type_name(ty)
                ),
                span,
            }),
            Err(reason) => out.push(VecSurfaceError {
                message: format!(
                    "{MARKER} {what} has type `{}`, which the Vec surface check cannot decide: \
                     {reason}",
                    type_name(ty)
                ),
                span,
            }),
        }
    }

    fn holds_vec(&self, ty: &AirType, visited: &mut HashSet<String>) -> Result<bool, String> {
        match ty {
            AirType::Vec(_) => Ok(true),
            AirType::Array(inner, _) => self.holds_vec(inner, visited),
            AirType::Struct(name) => {
                let Some(def) = self.structs.get(name.as_str()) else {
                    return Err(format!("struct `{name}` has no definition in the program"));
                };
                if !visited.insert(name.clone()) {
                    return Ok(false);
                }
                let mut found = false;
                for field in &def.fields {
                    found |= self.holds_vec(&field.ty, visited)?;
                }
                visited.remove(name);
                Ok(found)
            }
            AirType::Enum(name) => {
                let Some(def) = self.enums.get(name.as_str()) else {
                    return Err(format!("enum `{name}` has no definition in the program"));
                };
                if !visited.insert(name.clone()) {
                    return Ok(false);
                }
                let mut found = false;
                for variant in &def.variants {
                    for payload in &variant.payload {
                        found |= self.holds_vec(payload, visited)?;
                    }
                }
                visited.remove(name);
                Ok(found)
            }
            AirType::Param(id) => Err(format!(
                "type parameter {} survived monomorphization",
                id.0
            )),
            AirType::Opaque => Err("an unresolved `Dynamic` type survived lowering".to_string()),
// a pointer, a slice and a fn pointer refer to a buffer, they never carry one
            AirType::Ptr(_) | AirType::Slice(_) | AirType::FnPtr { .. } => Ok(false),
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
            | AirType::Void => Ok(false),
        }
    }

    fn offend_slot(&self, ty: &AirType, visited: &mut HashSet<String>) -> Result<bool, String> {
        match ty {
            AirType::Vec(inner) | AirType::Array(inner, _) => self.holds_vec(inner, visited),
            AirType::Struct(name) => {
                let Some(def) = self.structs.get(name.as_str()) else {
                    return Err(format!("struct `{name}` has no definition in the program"));
                };
                if !visited.insert(name.clone()) {
                    return Ok(false);
                }
                let mut found = false;
                for field in &def.fields {
                    found |= if def.is_closure_env {
                        self.offend_slot(&field.ty, visited)?
                    } else {
                        self.holds_vec(&field.ty, visited)?
                    };
                }
                visited.remove(name);
                Ok(found)
            }
            AirType::Enum(name) => {
                let Some(def) = self.enums.get(name.as_str()) else {
                    return Err(format!("enum `{name}` has no definition in the program"));
                };
                if !visited.insert(name.clone()) {
                    return Ok(false);
                }
                let mut found = false;
                for variant in &def.variants {
                    for payload in &variant.payload {
                        found |= self.holds_vec(payload, visited)?;
                    }
                }
                visited.remove(name);
                Ok(found)
            }
            AirType::Param(id) => Err(format!(
                "type parameter {} survived monomorphization",
                id.0
            )),
            AirType::Opaque => Err("an unresolved `Dynamic` type survived lowering".to_string()),
            AirType::Ptr(_) | AirType::Slice(_) | AirType::FnPtr { .. } => Ok(false),
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
            | AirType::Void => Ok(false),
        }
    }
}

fn type_name(ty: &AirType) -> String {
    crate::mono::substitute::type_to_string(ty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TypeParamId;

    fn empty_program() -> AirProgram {
        AirProgram {
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            globals: Vec::new(),
            source_files: Vec::new(),
            mono_instances: Vec::new(),
            struct_sizes: std::collections::HashMap::new(),
            rc_type_table: crate::rc_types::RcTypeTable::default(),
        }
    }

// m-b: a type the scan cannot decide must reject, never silently pass. a `_ => false`
    #[test]
    fn param_type_arg_rejects_not_passes() {
        let program = empty_program();
        assert!(type_args_reject(&program, &AirType::Param(TypeParamId(0))).is_some());
    }

    #[test]
    fn opaque_type_arg_rejects_not_passes() {
        let program = empty_program();
        assert!(type_args_reject(&program, &AirType::Opaque).is_some());
    }

    #[test]
    fn unknown_struct_type_arg_rejects_not_passes() {
        let program = empty_program();
        assert!(type_args_reject(&program, &AirType::Struct("Ghost".to_string())).is_some());
    }

    #[test]
    fn primitive_type_arg_is_accepted() {
        let program = empty_program();
        assert!(type_args_reject(&program, &AirType::I64).is_none());
    }

    #[test]
    fn a_vec_held_inside_the_type_arg_is_rejected() {
        let program = empty_program();
        let arr_of_vec = AirType::Array(Box::new(AirType::Vec(Box::new(AirType::I64))), 2);
        assert!(type_args_reject(&program, &arr_of_vec).is_some());
    }

    fn opt_enum(name: &str, type_params: Vec<TypeParamId>, payload: AirType) -> crate::AirEnumDef {
        crate::AirEnumDef {
            name: name.to_string(),
            type_params,
            variants: vec![crate::AirEnumVariant {
                name: "Some".to_string(),
                tag: 0,
                payload: vec![payload],
            }],
            span: None,
        }
    }

    #[test]
    fn generic_enum_template_with_param_payload_is_accepted() {
        let mut program = empty_program();
        program
            .enums
            .push(opt_enum("Opt", vec![TypeParamId(0)], AirType::Param(TypeParamId(0))));
        assert!(check_vec_surface(&program).is_ok());
    }

    #[test]
    fn generic_enum_instantiated_at_vec_is_rejected() {
        let mut program = empty_program();
        program.enums.push(opt_enum(
            "__mono_Opt_Vec$i64",
            Vec::new(),
            AirType::Vec(Box::new(AirType::I64)),
        ));
        assert!(check_vec_surface(&program).is_err());
    }
}

