use aelys_sema::{InferType, TypeTable};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Copy,
    Affine,
    Managed,
    AffineManaged,
}

impl Category {
    pub fn is_affine(self) -> bool {
        matches!(self, Category::Affine | Category::AffineManaged)
    }

    pub fn is_managed(self) -> bool {
        matches!(self, Category::Managed | Category::AffineManaged)
    }
}

pub const AFFINE_TEST_TYPE: &str = "Resource";

pub fn category(ty: &InferType, tt: &TypeTable) -> Category {
    let managed = tt.contains_rc_nominal(ty) || tt.contains_vec_by_value(ty);
    match (is_affine(ty), managed) {
        (true, true) => Category::AffineManaged,
        (true, false) => Category::Affine,
        (false, true) => Category::Managed,
        (false, false) => Category::Copy,
    }
}

pub fn is_affine(ty: &InferType) -> bool {
    matches!(ty, InferType::Struct(name) if source_type_name(name) == AFFINE_TEST_TYPE)
}

pub fn source_type_name(name: &str) -> &str {
    let bare = aelys_sema::modules::strip_type_head(name);
    bare.rsplit('.').next().unwrap_or(bare)
}
