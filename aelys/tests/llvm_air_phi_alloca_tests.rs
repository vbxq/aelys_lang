use aelys_air::{
    AirBlock, AirConst, AirFunction, AirIntSize, AirLocal, AirParam, AirProgram, AirStmt,
    AirStmtKind, AirTerminator, AirType, BinOp, BlockId, CallingConv, FunctionAttribs, FunctionId,
    GcMode, InlineHint, LocalId, Operand, Place, Rvalue,
};
use aelys_codegen::CodegenContext;
use std::fs;
use tempfile::tempdir;

fn compile_air_to_verified_ir(program: &AirProgram) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let ll_path = dir.path().join("module.ll");
    let ll_path_str = ll_path.to_string_lossy().to_string();

    let mut codegen = CodegenContext::new("phi_tests");
    codegen
        .compile(program)
        .expect("codegen compilation should succeed");
    codegen
        .emit_ir(&ll_path_str)
        .expect("llvm ir should be emitted");

    let ir = fs::read_to_string(&ll_path).expect("llvm ir file should be generated");

    let context = inkwell::context::Context::create();
    let buffer =
        inkwell::memory_buffer::MemoryBuffer::create_from_file(&ll_path).expect("ir readable");
    let module = context
        .create_module_from_ir(buffer)
        .expect("llvm ir should parse into a module");
    module
        .verify()
        .expect("module.verify() should succeed for generated ir");

    ir
}

fn default_attribs() -> FunctionAttribs {
    FunctionAttribs {
        inline: InlineHint::Default,
        no_gc: false,
        no_unwind: false,
        cold: false,
    }
}

#[test]
fn multi_block_assign_gets_alloca_and_verifies() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![AirParam {
                id: LocalId(0),
                ty: AirType::Bool,
                name: "cond".to_string(),
                span: None,
            }],
            ret_ty: AirType::I64,
            locals: vec![AirLocal {
                id: LocalId(1),
                ty: AirType::I64,
                // unnamed + immutable would normally skip alloca
                name: None,
                is_mut: false,
                span: None,
            }],
            blocks: vec![
                AirBlock {
                    id: BlockId(0),
                    stmts: vec![],
                    terminator: AirTerminator::Branch {
                        cond: Operand::Copy(LocalId(0)),
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                    },
                },
                AirBlock {
                    id: BlockId(1),
                    stmts: vec![AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(1)),
                            rvalue: Rvalue::Use(Operand::Const(AirConst::Int(10, AirIntSize::I64))),
                        },
                        span: None,
                    }],
                    terminator: AirTerminator::Goto(BlockId(3)),
                },
                AirBlock {
                    id: BlockId(2),
                    stmts: vec![AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(1)),
                            rvalue: Rvalue::Use(Operand::Const(AirConst::Int(20, AirIntSize::I64))),
                        },
                        span: None,
                    }],
                    terminator: AirTerminator::Goto(BlockId(3)),
                },
                AirBlock {
                    id: BlockId(3),
                    stmts: vec![],
                    terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(1)))),
                },
            ],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: default_attribs(),
            span: None,
        }],
        structs: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    };

    let ir = compile_air_to_verified_ir(&program);

    // The local must use alloca since it's assigned in bb1 and bb2
    assert!(
        ir.contains("alloca i64"),
        "multi-block assigned local must use alloca:\n{ir}"
    );
    // Must have store in both branches
    let store_count = ir.matches("store i64").count();
    assert!(
        store_count >= 2,
        "expected at least 2 stores (one per branch), got {store_count}:\n{ir}"
    );
    // Must load in the merge block
    assert!(
        ir.contains("load i64"),
        "merge block must load the local:\n{ir}"
    );
}

/// Same-block reassignment should not force alloca (value_map handles it fine)
#[test]
fn same_block_reassign_stays_ssa() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![AirParam {
                id: LocalId(0),
                ty: AirType::I64,
                name: "x".to_string(),
                span: None,
            }],
            ret_ty: AirType::I64,
            locals: vec![
                AirLocal {
                    id: LocalId(1),
                    ty: AirType::I64,
                    name: None,
                    is_mut: false,
                    span: None,
                },
                AirLocal {
                    id: LocalId(2),
                    ty: AirType::I64,
                    name: None,
                    is_mut: false,
                    span: None,
                },
            ],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![
                    // _1 = x + 1
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(1)),
                            rvalue: Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::Copy(LocalId(0)),
                                Operand::Const(AirConst::Int(1, AirIntSize::I64)),
                            ),
                        },
                        span: None,
                    },
                    // _2 = _1 + 1 (uses _1 from same block, no phi needed)
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(2)),
                            rvalue: Rvalue::BinaryOp(
                                BinOp::Add,
                                Operand::Copy(LocalId(1)),
                                Operand::Const(AirConst::Int(1, AirIntSize::I64)),
                            ),
                        },
                        span: None,
                    },
                ],
                terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(2)))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: default_attribs(),
            span: None,
        }],
        structs: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    };

    let ir = compile_air_to_verified_ir(&program);

    // Single-block unnamed temps should not use alloca
    assert!(
        !ir.contains("alloca"),
        "same-block SSA temps must not use alloca:\n{ir}"
    );
}
