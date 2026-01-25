use super::super::Compiler;
use aelys_bytecode::{GlobalLayout, OpCode, UpvalueDescriptor, Value};
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;
use std::sync::Arc;

pub(super) fn finalize_untyped_function(
    parent: &mut Compiler,
    mut func_compiler: Compiler,
    func_name: &str,
    func_span: Span,
    func_var_reg: u8,
) -> Result<()> {
    func_compiler.current.global_layout = build_untyped_global_layout(&func_compiler);
    func_compiler.current.compute_global_layout_hash();
    func_compiler.current.finalize_bytecode();

    parent.mark_captures_from_nested(&func_compiler);
    parent.fix_transitive_captures(&mut func_compiler.upvalues);

    for upvalue in &func_compiler.upvalues {
        func_compiler
            .current
            .upvalue_descriptors
            .push(UpvalueDescriptor {
                is_local: upvalue.is_local,
                index: upvalue.index,
            });
    }

    let compiled_func = func_compiler.current;
    let upvalue_count = func_compiler.upvalues.len();
    if upvalue_count > 255 {
        return Err(CompileError::new(
            CompileErrorKind::TooManyUpvalues,
            func_span,
            parent.source.clone(),
        )
        .into());
    }

    parent.heap = func_compiler.heap;

    for (name, &idx) in &func_compiler.global_indices {
        if !parent.global_indices.contains_key(name) {
            parent.global_indices.insert(name.clone(), idx);
        }
    }
    if func_compiler.next_global_index > parent.next_global_index {
        parent.next_global_index = func_compiler.next_global_index;
    }
    if func_compiler.next_call_site_slot > parent.next_call_site_slot {
        parent.next_call_site_slot = func_compiler.next_call_site_slot;
    }

    let const_idx = parent.current.add_constant_function(compiled_func);
    if const_idx > u16::MAX {
        return Err(CompileError::new(
            CompileErrorKind::TooManyConstants,
            func_span,
            parent.source.clone(),
        )
        .into());
    }

    if upvalue_count > 0 {
        parent.emit_a(
            OpCode::MakeClosure,
            func_var_reg,
            const_idx as u8,
            upvalue_count as u8,
            func_span,
        );
    } else {
        parent.emit_b(OpCode::LoadK, func_var_reg, const_idx as i16, func_span);
    }

    if let Some(&idx) = parent.global_indices.get(func_name) {
        parent.accessed_globals.insert(func_name.to_string());
        parent.emit_b(OpCode::SetGlobalIdx, func_var_reg, idx as i16, func_span);
    } else {
        let name_ref = parent.heap.intern_string(func_name);
        let name_const_idx = parent.add_constant(Value::ptr(name_ref.index()), func_span)?;
        parent.emit_a(
            OpCode::SetGlobal,
            func_var_reg,
            name_const_idx as u8,
            0,
            func_span,
        );
    }

    Ok(())
}

fn build_untyped_global_layout(compiler: &Compiler) -> Arc<GlobalLayout> {
    if compiler.accessed_globals.is_empty() {
        GlobalLayout::empty()
    } else {
        let mut names = vec![String::new(); compiler.next_global_index as usize];
        for (name, &idx) in &compiler.global_indices {
            names[idx as usize] = name.clone();
        }
        GlobalLayout::new(names)
    }
}
