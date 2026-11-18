use crate::*;

pub const TUPLE_PREFIX: &str = "__tuple~";

pub fn is_tuple_name(name: &str) -> bool {
    name.starts_with(TUPLE_PREFIX) || name.starts_with(&format!("__mono_{TUPLE_PREFIX}"))
}

pub fn tuple_def_name(arity: usize) -> String {
    format!("{TUPLE_PREFIX}{arity}")
}

pub fn tuple_ref(elems: Vec<AirType>) -> StructRef {
    NominalRef::new(tuple_def_name(elems.len()), elems)
}

pub fn tuple_def(arity: usize) -> AirStructDef {
    AirStructDef {
        name: tuple_def_name(arity),
        type_params: (0..arity as u32).map(TypeParamId).collect(),
        fields: (0..arity)
            .map(|i| AirStructField {
                name: i.to_string(),
                ty: AirType::Param(TypeParamId(i as u32)),
                offset: None,
            })
            .collect(),
        is_closure_env: false,
        span: None,
    }
}
