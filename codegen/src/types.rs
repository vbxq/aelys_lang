use aelys_air::AirType;

pub fn air_type_to_llvm<'ctx>(
    ty: &AirType,
    context: &'ctx inkwell::context::Context,
) -> inkwell::types::BasicTypeEnum<'ctx> {
    let _ = (ty, context);
    todo!()
}
