use super::super::{Compiler, Upvalue};
use aelys_common::Result;
use aelys_sema::{ResolvedType, TypedFunction};

pub(super) struct TypedFunctionSetup {
    pub(super) func_var_reg: u8,
    pub(super) has_no_gc: bool,
    pub(super) nested_compiler: Compiler,
}

pub(super) fn setup_typed_function(
    parent: &mut Compiler,
    func: &TypedFunction,
) -> Result<TypedFunctionSetup> {
    let has_no_gc = func.decorators.iter().any(|d| d.name == "no_gc");
    let func_var_reg = parent.alloc_register()?;

    if parent.scope_depth == 0 {
        parent.globals.insert(func.name.clone(), false);
        if !parent.global_indices.contains_key(&func.name) {
            let idx = parent.next_global_index;
            parent.global_indices.insert(func.name.clone(), idx);
            parent.next_global_index += 1;
        }
    } else {
        let params_resolved: Vec<_> = func
            .params
            .iter()
            .map(|p| ResolvedType::from_infer_type(&p.ty))
            .collect();
        let ret_resolved = ResolvedType::from_infer_type(&func.return_type);

        parent.add_local(
            func.name.clone(),
            false,
            func_var_reg,
            ResolvedType::Function {
                params: params_resolved,
                ret: Box::new(ret_resolved),
            },
        );
    }

    let mut nested_compiler = Compiler::for_nested_function(
        Some(func.name.clone()),
        parent.source.clone(),
        parent.heap.clone(),
        parent.globals.clone(),
        parent.global_indices.clone(),
        parent.next_global_index,
        parent.locals.clone(),
        parent.upvalues.clone(),
        parent.all_enclosing_locals.clone(),
        parent.module_aliases.clone(),
        parent.known_globals.clone(),
        parent.known_native_globals.clone(),
        parent.symbol_origins.clone(),
        parent.next_call_site_slot,
    );
    nested_compiler.current.arity = func.params.len() as u8;
    nested_compiler.has_no_gc = has_no_gc;

    for (capture_name, _capture_ty) in &func.captures {
        if let Some((local_reg, mutable, _resolved_ty)) =
            parent.resolve_variable_typed(capture_name)
        {
            if !nested_compiler
                .upvalues
                .iter()
                .any(|uv| uv.name == *capture_name)
            {
                parent.mark_local_captured(local_reg);
                nested_compiler.upvalues.push(Upvalue {
                    is_local: true,
                    index: local_reg,
                    name: capture_name.clone(),
                    mutable,
                });
            }
            continue;
        }

        let _ = parent.resolve_upvalue(capture_name);

        if let Some((upvalue_idx, _)) = parent
            .upvalues
            .iter()
            .enumerate()
            .find(|(_, uv)| uv.name == *capture_name)
        {
            if !nested_compiler
                .upvalues
                .iter()
                .any(|uv| uv.name == *capture_name)
            {
                nested_compiler.upvalues.push(Upvalue {
                    is_local: false,
                    index: upvalue_idx as u8,
                    name: capture_name.clone(),
                    mutable: parent.upvalues[upvalue_idx].mutable,
                });
            }
        }
    }

    Ok(TypedFunctionSetup {
        func_var_reg,
        has_no_gc,
        nested_compiler,
    })
}
