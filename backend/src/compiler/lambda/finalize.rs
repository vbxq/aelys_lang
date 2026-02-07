use super::super::Compiler;
use aelys_bytecode::{OpCode, UpvalueDescriptor};
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;

pub(super) fn finalize_lambda(
    parent: &mut Compiler,
    mut lambda_compiler: Compiler,
    dest: u8,
    span: Span,
) -> Result<()> {
    let global_layout = if lambda_compiler.accessed_globals.is_empty() {
        aelys_bytecode::GlobalLayout::empty()
    } else {
        let mut names = vec![String::new(); lambda_compiler.next_global_index as usize];
        for (name, &idx) in &lambda_compiler.global_indices {
            names[idx as usize] = name.clone();
        }
        aelys_bytecode::GlobalLayout::new(names)
    };
    lambda_compiler.current.global_layout = global_layout;
    lambda_compiler.current.compute_global_layout_hash();
    lambda_compiler.current.finalize_bytecode();

    parent.mark_captures_from_nested(&lambda_compiler);
    parent.fix_transitive_captures(&mut lambda_compiler.upvalues);

    for upvalue in &lambda_compiler.upvalues {
        lambda_compiler
            .current
            .upvalue_descriptors
            .push(UpvalueDescriptor {
                is_local: upvalue.is_local,
                index: upvalue.index,
            });
    }

    let compiled_func = lambda_compiler.current;
    let upvalue_count = lambda_compiler.upvalues.len();
    if upvalue_count > 255 {
        return Err(CompileError::new(
            CompileErrorKind::TooManyUpvalues,
            span,
            parent.source.clone(),
        )
        .into());
    }

    parent.heap = lambda_compiler.heap;

    for (name, &idx) in &lambda_compiler.global_indices {
        if !parent.global_indices.contains_key(name) {
            parent.global_indices.insert(name.clone(), idx);
        }
    }
    if lambda_compiler.next_global_index > parent.next_global_index {
        parent.next_global_index = lambda_compiler.next_global_index;
    }

    if lambda_compiler.next_call_site_slot > parent.next_call_site_slot {
        parent.next_call_site_slot = lambda_compiler.next_call_site_slot;
    }

    let const_idx = parent.current.add_constant_function(compiled_func);
    if const_idx > u8::MAX as u16 {
        return Err(CompileError::new(
            CompileErrorKind::TooManyConstants,
            span,
            parent.source.clone(),
        )
        .into());
    }

    if upvalue_count > 0 {
        parent.emit_a(
            OpCode::MakeClosure,
            dest,
            const_idx as u8,
            upvalue_count as u8,
            span,
        );
    } else {
        parent.emit_b(OpCode::LoadK, dest, const_idx as i16, span);
    }

    Ok(())
}
