use aelys_sema::{InferType, TypeTable};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Copy,
    Affine,
    Managed,
}

pub const AFFINE_TEST_TYPE: &str = "Resource";

pub fn category(ty: &InferType, tt: &TypeTable) -> Category {
    if is_affine(ty) {
        return Category::Affine;
    }
    if tt.contains_rc_nominal(ty) || tt.contains_vec_by_value(ty) {
        return Category::Managed;
    }
    Category::Copy
}

pub fn is_affine(ty: &InferType) -> bool {
    matches!(ty, InferType::Struct(name) if name == AFFINE_TEST_TYPE)
}
