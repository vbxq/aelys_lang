use crate::CodegenContext;
use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::functions::{function_symbol_name, llvm_calling_convention};
use crate::lowering::stmts::RC_HEADER_SIZE;
use crate::types::{air_basic_type_to_llvm, closure_fat_ptr_type};
use aelys_air::objects::{DYN_CALL, dyn_trait, vtable_index_symbol, vtable_symbol};
use aelys_air::{AirProgram, AirType, CallingConv};
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, PointerValue};

// mirrors the type_id word of the rc header in aelys_rc.h
const TYPE_ID_OFFSET: u64 = 8;

fn llvm_err(e: impl ToString) -> CodegenError {
    CodegenError::LlvmError(e.to_string())
}

impl CodegenContext {
    pub(crate) fn emit_vtables(&self, program: &AirProgram) -> Result<(), CodegenError> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        for vt in &program.dyn_tables.vtables {
            let mut slots = Vec::with_capacity(vt.fns.len());
            for name in &vt.fns {
                let symbol = program
                    .functions
                    .iter()
                    .find(|f| f.name == *name)
                    .map(function_symbol_name)
                    .ok_or_else(|| llvm_err(format!("vtable slot `{name}` has no function")))?;
                let f = self
                    .module
                    .get_function(&symbol)
                    .ok_or_else(|| llvm_err(format!("vtable slot `{symbol}` was not declared")))?;
                slots.push(f.as_global_value().as_pointer_value());
            }
            let table = ptr_ty.array_type(slots.len() as u32);
            let global = self.module.add_global(
                table,
                None,
                &vtable_symbol(&vt.trait_q, &vt.self_ty),
            );
            global.set_linkage(Linkage::Internal);
            global.set_constant(true);
            global.set_initializer(&ptr_ty.const_array(&slots));
        }

        let entries = &program.rc_type_table.entries;
        for (q, _) in &program.dyn_tables.slots {
            let mut row = vec![ptr_ty.const_null()];
            for entry in entries {
                let vt = program
                    .dyn_tables
                    .vtables
                    .iter()
                    .find(|v| v.trait_q == *q && v.self_ty == entry.ty)
                    .and_then(|v| self.module.get_global(&vtable_symbol(q, &v.self_ty)));
                row.push(vt.map_or(ptr_ty.const_null(), |g| g.as_pointer_value()));
            }
            let table = ptr_ty.array_type(row.len() as u32);
            let global = self.module.add_global(table, None, &vtable_index_symbol(q));
            global.set_linkage(Linkage::Internal);
            global.set_constant(true);
            global.set_initializer(&ptr_ty.const_array(&row));
        }
        Ok(())
    }
}

impl<'a> FunctionCodegen<'a> {
    fn dyn_global(&self, symbol: &str) -> Result<PointerValue<'static>, CodegenError> {
        self.module
            .get_global(symbol)
            .map(|g| g.as_pointer_value())
            .ok_or_else(|| llvm_err(format!("trait object table `{symbol}` was not emitted")))
    }

    fn fat_pointer(
        &self,
        data: PointerValue<'static>,
        vtable: PointerValue<'static>,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let fat = closure_fat_ptr_type(self.context).get_undef();
        let fat = self
            .builder
            .build_insert_value(fat, data, 0, "dyn_data")
            .map_err(llvm_err)?;
        let fat = self
            .builder
            .build_insert_value(fat, vtable, 1, "dyn_vtable")
            .map_err(llvm_err)?;
        Ok(fat.into_struct_value().into())
    }

    pub(crate) fn generate_dyn_cast(
        &mut self,
        value: BasicValueEnum<'static>,
        from: &AirType,
        to: &AirType,
        q: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        match (from, to) {
            (AirType::Rc(_), AirType::Rc(_)) => Ok(value),
            (AirType::Ptr(inner), AirType::Ptr(_)) if dyn_trait(inner).is_none() => {
                let vtable = self.dyn_global(&vtable_symbol(q, inner))?;
                self.fat_pointer(value.into_pointer_value(), vtable)
            }
            (AirType::Rc(inner), AirType::Ptr(_)) if dyn_trait(inner).is_some() => {
                let data = value.into_pointer_value();
                let i8_ty = self.context.i8_type();
                let i32_ty = self.context.i32_type();
                let back = self
                    .context
                    .i64_type()
                    .const_int(RC_HEADER_SIZE - TYPE_ID_OFFSET, false)
                    .const_neg();
                let id_ptr = unsafe {
                    self.builder
                        .build_gep(i8_ty, data, &[back], "dyn_type_id_ptr")
                        .map_err(llvm_err)?
                };
                let id = self
                    .builder
                    .build_load(i32_ty, id_ptr, "dyn_type_id")
                    .map_err(llvm_err)?
                    .into_int_value();
                let id = self
                    .builder
                    .build_int_z_extend(id, self.context.i64_type(), "dyn_type_index")
                    .map_err(llvm_err)?;
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                let index = self.dyn_global(&vtable_index_symbol(q))?;
                let slot = unsafe {
                    self.builder
                        .build_gep(ptr_ty, index, &[id], "dyn_vtable_slot")
                        .map_err(llvm_err)?
                };
                let vtable = self
                    .builder
                    .build_load(ptr_ty, slot, "dyn_vtable")
                    .map_err(llvm_err)?
                    .into_pointer_value();
                self.fat_pointer(data, vtable)
            }
            _ => Err(CodegenError::UnsupportedType(format!(
                "no trait object conversion from {from:?} to {to:?}"
            ))),
        }
    }

    pub(crate) fn generate_dyn_call(
        &mut self,
        name: &str,
        arg_values: &[BasicValueEnum<'static>],
        expected_ret: Option<&AirType>,
    ) -> Result<Option<BasicValueEnum<'static>>, CodegenError> {
        let rest = name.strip_prefix(DYN_CALL).unwrap_or(name);
        let (q, method) = rest
            .rsplit_once('$')
            .ok_or_else(|| llvm_err(format!("malformed trait object call `{name}`")))?;
        let slot = self
            .program
            .dyn_tables
            .slots_of(q)
            .and_then(|ms| ms.iter().position(|m| m == method))
            .ok_or_else(|| llvm_err(format!("`{method}` has no slot in the vtable of `{q}`")))?;
        let (recv, args) = arg_values
            .split_first()
            .ok_or_else(|| llvm_err(format!("`{name}` has no receiver")))?;
        let recv = recv.into_struct_value();
        let data = self
            .builder
            .build_extract_value(recv, 0, "dyn_self")
            .map_err(llvm_err)?;
        let vtable = self
            .builder
            .build_extract_value(recv, 1, "dyn_vtable")
            .map_err(llvm_err)?
            .into_pointer_value();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let index = self.context.i64_type().const_int(slot as u64, false);
        let slot_ptr = unsafe {
            self.builder
                .build_gep(ptr_ty, vtable, &[index], "dyn_method_slot")
                .map_err(llvm_err)?
        };
        let method_ptr = self
            .builder
            .build_load(ptr_ty, slot_ptr, "dyn_method")
            .map_err(llvm_err)?
            .into_pointer_value();

        let mut param_types: Vec<BasicMetadataTypeEnum<'static>> = vec![ptr_ty.into(), ptr_ty.into()];
        param_types.extend(args.iter().map(|a| BasicMetadataTypeEnum::from(a.get_type())));
        let fn_ty = match expected_ret {
            None | Some(AirType::Void) => self.context.void_type().fn_type(&param_types, false),
            Some(ret) => air_basic_type_to_llvm(ret, self.context)?.fn_type(&param_types, false),
        };
        let mut call_args: Vec<BasicMetadataValueEnum<'static>> =
            vec![ptr_ty.const_null().into(), data.into()];
        call_args.extend(args.iter().map(|a| BasicMetadataValueEnum::from(*a)));
        let call = self
            .builder
            .build_indirect_call(fn_ty, method_ptr, &call_args, "dyn_call")
            .map_err(llvm_err)?;
        call.set_call_convention(llvm_calling_convention(CallingConv::Aelys));
        Ok(call.try_as_basic_value().basic())
    }
}
