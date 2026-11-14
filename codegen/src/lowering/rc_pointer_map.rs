use crate::CodegenContext;
use crate::CodegenError;
use crate::types::{air_basic_type_to_llvm, enum_struct_name};
use aelys_air::layout::payload_field_offsets;
use aelys_air::rc_paths::{RcLeafPath, RcPathStep, RcScan, rc_field_paths};
use aelys_air::{AirProgram, AirType};
use inkwell::targets::TargetData;
use inkwell::types::BasicTypeEnum;

impl CodegenContext {
    pub(crate) fn verify_rc_pointer_map(&self, program: &AirProgram) -> Result<(), CodegenError> {
        let repr = self
            .module
            .get_data_layout()
            .as_str()
            .to_str()
            .unwrap_or("")
            .to_string();
        if program.rc_type_table.entries.is_empty() {
            return Ok(());
        }
        if repr.is_empty() {
            return Err(CodegenError::LlvmError(
                "rc pointer map: the module has no data layout to verify it against".to_string(),
            ));
        }
        let td = TargetData::create(&repr);
        for entry in &program.rc_type_table.entries {
            let paths = match rc_field_paths(&entry.ty, &program.structs, &program.enums) {
                RcScan::Paths(paths) => paths,
                RcScan::None | RcScan::Undecidable(_) | RcScan::RejectedMultiVariant(_) => {
                    Vec::new()
                }
            };
            if paths.len() != entry.pointer_offsets.len() {
                return Err(map_error(
                    &entry.ty,
                    format!(
                        "{} offsets for {} rc leaves",
                        entry.pointer_offsets.len(),
                        paths.len()
                    ),
                ));
            }
            if paths.is_empty() {
                continue;
            }
            let data_size = td.get_abi_size(&air_basic_type_to_llvm(&entry.ty, self.context)?);
            for (path, &off) in paths.iter().zip(&entry.pointer_offsets) {
                let llvm_off = self.llvm_rc_leaf_offset(&td, &entry.ty, path, program)?;
                if u64::from(off) != llvm_off || off % 8 != 0 || u64::from(off) + 8 > data_size {
                    return Err(map_error(
                        &entry.ty,
                        format!(
                            "offset {off} but the llvm layout puts the rc leaf at {llvm_off} \
                         in {data_size} bytes"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    fn llvm_rc_leaf_offset(
        &self,
        td: &TargetData,
        data_ty: &AirType,
        path: &RcLeafPath,
        program: &AirProgram,
    ) -> Result<u64, CodegenError> {
        let mut acc: u64 = 0;
        let mut cur = data_ty.clone();
        for step in &path.steps {
            match step {
                RcPathStep::Field(field_name) => {
                    let AirType::Struct(struct_name) = &cur else {
                        return Err(map_error(data_ty, format!("field step on {cur:?}")));
                    };
                    let def = program
                        .structs
                        .iter()
                        .find(|s| &s.name == struct_name)
                        .ok_or_else(|| map_error(data_ty, format!("no struct {struct_name}")))?;
                    let index = def
                        .fields
                        .iter()
                        .position(|f| &f.name == field_name)
                        .ok_or_else(|| map_error(data_ty, format!("no field {field_name}")))?;
                    let st = self
                        .context
                        .get_struct_type(struct_name)
                        .ok_or_else(|| map_error(data_ty, format!("no llvm type {struct_name}")))?;
                    acc += td
                        .offset_of_element(&st, index as u32)
                        .ok_or_else(|| map_error(data_ty, format!("{struct_name}.{field_name}")))?;
                    cur = def.fields[index].ty.clone();
                }
                RcPathStep::EnumPayload {
                    enum_ref,
                    tag,
                    field_index,
                } => {
                    let name = enum_ref.symbol();
                    let def = program
                        .enums
                        .iter()
                        .find(|e| e.name == name)
                        .ok_or_else(|| map_error(data_ty, format!("no enum {name}")))?;
                    let variant = def
                        .variants
                        .iter()
                        .find(|v| v.tag == *tag)
                        .ok_or_else(|| map_error(data_ty, format!("no tag {tag} on {name}")))?;
                    let idx = *field_index as usize;
                    let target = variant
                        .payload
                        .get(idx)
                        .ok_or_else(|| map_error(data_ty, format!("no payload field {idx}")))?;
                    let st = self
                        .context
                        .get_struct_type(&enum_struct_name(&name))
                        .ok_or_else(|| map_error(data_ty, format!("no llvm type for {name}")))?;
                    acc += td
                        .offset_of_element(&st, 1)
                        .ok_or_else(|| map_error(data_ty, format!("{name} payload cell")))?;
                    acc += u64::from(
                        payload_field_offsets(&variant.payload, &program.struct_sizes)[idx],
                    );
                    cur = target.clone();
                }
            }
        }
        if !matches!(cur, AirType::Rc(_))
            || !matches!(
                air_basic_type_to_llvm(&cur, self.context)?,
                BasicTypeEnum::PointerType(_)
            )
        {
            return Err(map_error(data_ty, format!("path ends on {cur:?}")));
        }
        Ok(acc)
    }
}

fn map_error(ty: &AirType, what: String) -> CodegenError {
    CodegenError::LlvmError(format!("rc pointer map of {ty:?}: {what}"))
}
