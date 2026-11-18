use crate::{AirType, StructRef};

pub use aelys_sema::DYN_CALL;

const DYN_OBJECT: &str = "__dynobj$";

pub fn dyn_type(q: &str) -> AirType {
    AirType::Struct(StructRef::plain(format!("{DYN_OBJECT}{q}")))
}

pub fn dyn_trait(ty: &AirType) -> Option<&str> {
    match ty {
        AirType::Struct(r) if r.args.is_empty() => r.name.strip_prefix(DYN_OBJECT),
        _ => None,
    }
}

pub fn dyn_trait_behind(ty: &AirType) -> Option<&str> {
    match ty {
        AirType::Ptr(inner) | AirType::Rc(inner) => dyn_trait(inner),
        _ => None,
    }
}

#[derive(Clone)]
pub struct AirVtable {
    pub trait_q: String,
    pub self_ty: AirType,
    pub fns: Vec<String>,
}

#[derive(Clone, Default)]
pub struct DynTables {
    pub slots: Vec<(String, Vec<String>)>,
    pub vtables: Vec<AirVtable>,
}

impl DynTables {
    pub fn slots_of(&self, q: &str) -> Option<&[String]> {
        self.slots
            .iter()
            .find(|(t, _)| t == q)
            .map(|(_, m)| m.as_slice())
    }

    pub fn absorb(&mut self, other: DynTables) {
        for (q, methods) in other.slots {
            if self.slots_of(&q).is_none() {
                self.slots.push((q, methods));
            }
        }
        self.vtables.extend(other.vtables);
    }
}

pub fn vtable_symbol(q: &str, self_ty: &AirType) -> String {
    format!(
        "__vt${q}${}",
        crate::mono::substitute::type_to_string(self_ty)
    )
}

pub fn vtable_index_symbol(q: &str) -> String {
    format!("__vtidx${q}")
}
