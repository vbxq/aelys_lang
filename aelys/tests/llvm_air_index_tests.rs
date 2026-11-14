use aelys_air::{
    AirBlock, AirConst, AirFunction, AirLocal, AirParam, AirProgram, AirStmt, AirStmtKind,
    AirTerminator, AirType, BlockId, CallingConv, FunctionAttribs, FunctionId, GcMode, InlineHint,
    LocalId, Operand, Place, Rvalue,
};
use aelys_codegen::CodegenContext;
use std::fs;
use tempfile::tempdir;

fn compile_air_error(program: &AirProgram) -> String {
    let mut codegen = CodegenContext::new("index_tests");
    match codegen.compile(program) {
        Ok(()) => panic!("codegen must refuse this program"),
        Err(e) => e.to_string(),
    }
}

fn compile_air_to_verified_ir(program: &AirProgram) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let ll_path = dir.path().join("module.ll");
    let ll_path_str = ll_path.to_string_lossy().to_string();

    let mut codegen = CodegenContext::new("index_tests");
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

/// array read: rvalue::index on array(i64, 4), verify gep + bounds check + load
#[test]
fn array_index_read_generates_gep_and_bounds_check() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![AirParam {
                id: LocalId(0),
                ty: AirType::I64,
                name: "idx".to_string(),
                span: None,
            }],
            ret_ty: AirType::I64,
            locals: vec![
                AirLocal {
                    id: LocalId(1),
                    ty: AirType::Array(Box::new(AirType::I64), 4),
                    name: Some("arr".to_string()),
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
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(1)),
                            rvalue: Rvalue::Use(Operand::Const(AirConst::ZeroInit(
                                AirType::Array(Box::new(AirType::I64), 4),
                            ))),
                        },
                        span: None,
                    },
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(2)),
                            rvalue: Rvalue::Index {
                                base: Operand::Copy(LocalId(1)),
                                index: Operand::Copy(LocalId(0)),
                            },
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
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let ir = compile_air_to_verified_ir(&program);

    assert!(
        ir.contains("getelementptr inbounds"),
        "array index should generate GEP:\n{ir}"
    );
    // should have bounds check
    assert!(
        ir.contains("icmp uge"),
        "array index should generate unsigned bounds check:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "array index should call __aelys_panic on OOB:\n{ir}"
    );
    assert!(ir.contains("idx_oob:"), "should have idx_oob block:\n{ir}");
    assert!(ir.contains("idx_ok:"), "should have idx_ok block:\n{ir}");
    // should have unreachable after panic
    assert!(
        ir.contains("unreachable"),
        "should have unreachable after panic:\n{ir}"
    );
    assert!(
        ir.contains("load i64"),
        "array index should load element:\n{ir}"
    );
}

#[test]
fn string_index_below_sema_is_refused_by_codegen() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![
                AirParam {
                    id: LocalId(0),
                    ty: AirType::Str,
                    name: "s".to_string(),
                    span: None,
                },
                AirParam {
                    id: LocalId(1),
                    ty: AirType::I64,
                    name: "idx".to_string(),
                    span: None,
                },
            ],
            ret_ty: AirType::Str,
            locals: vec![AirLocal {
                id: LocalId(2),
                ty: AirType::Str,
                name: None,
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![AirStmt {
                    kind: AirStmtKind::Assign {
                        place: Place::Local(LocalId(2)),
                        rvalue: Rvalue::Index {
                            base: Operand::Copy(LocalId(0)),
                            index: Operand::Copy(LocalId(1)),
                        },
                    },
                    span: None,
                }],
                terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(2)))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: default_attribs(),
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let err = compile_air_error(&program);

    assert!(
        err.contains("index of a string reached codegen"),
        "the refusal must name the construct:\n{err}"
    );
    assert!(
        err.contains("sema must reject it (E0304)"),
        "the refusal must name the rule that owns it:\n{err}"
    );
}

/// array write: place::index on array(i64, 4), verify gep + store + bounds check
#[test]
fn array_index_write_generates_gep_and_store() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![
                AirParam {
                    id: LocalId(0),
                    ty: AirType::I64,
                    name: "idx".to_string(),
                    span: None,
                },
                AirParam {
                    id: LocalId(1),
                    ty: AirType::I64,
                    name: "val".to_string(),
                    span: None,
                },
            ],
            ret_ty: AirType::Void,
            locals: vec![AirLocal {
                id: LocalId(2),
                ty: AirType::Array(Box::new(AirType::I64), 4),
                name: Some("arr".to_string()),
                is_mut: true,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(2)),
                            rvalue: Rvalue::Use(Operand::Const(AirConst::ZeroInit(
                                AirType::Array(Box::new(AirType::I64), 4),
                            ))),
                        },
                        span: None,
                    },
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Index(LocalId(2), Operand::Copy(LocalId(0))),
                            rvalue: Rvalue::Use(Operand::Copy(LocalId(1))),
                        },
                        span: None,
                    },
                ],
                terminator: AirTerminator::Return(None),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: default_attribs(),
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let ir = compile_air_to_verified_ir(&program);

    assert!(
        ir.contains("getelementptr inbounds"),
        "array write should generate GEP:\n{ir}"
    );
    assert!(
        ir.contains("store i64"),
        "array write should generate store:\n{ir}"
    );
    // should have bounds check
    assert!(
        ir.contains("icmp uge"),
        "array write should have bounds check:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "array write should call panic on OOB:\n{ir}"
    );
}

/// bounds check structure: verify idx_oob has unreachable after __aelys_panic
#[test]
fn bounds_check_structure_has_unreachable_after_panic() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![AirParam {
                id: LocalId(0),
                ty: AirType::I64,
                name: "idx".to_string(),
                span: None,
            }],
            ret_ty: AirType::I64,
            locals: vec![
                AirLocal {
                    id: LocalId(1),
                    ty: AirType::Array(Box::new(AirType::I64), 4),
                    name: Some("arr".to_string()),
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
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(1)),
                            rvalue: Rvalue::Use(Operand::Const(AirConst::ZeroInit(
                                AirType::Array(Box::new(AirType::I64), 4),
                            ))),
                        },
                        span: None,
                    },
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(2)),
                            rvalue: Rvalue::Index {
                                base: Operand::Copy(LocalId(1)),
                                index: Operand::Copy(LocalId(0)),
                            },
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
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let ir = compile_air_to_verified_ir(&program);

    // find the idx_oob block and verify it ends with unreachable
    let oob_block_start = ir
        .find("idx_oob:")
        .expect("should have idx_oob block in IR");
    let after_oob = &ir[oob_block_start..];
    // the block should contain the panic call and then unreachable
    let next_label = after_oob.find("\n\n").or_else(|| {
        after_oob[9..].find(':').map(|pos| pos + 9)
    });
    let oob_block_text = match next_label {
        Some(end) => &after_oob[..end],
        None => after_oob,
    };
    assert!(
        oob_block_text.contains("@__aelys_panic"),
        "idx_oob block should call __aelys_panic:\n{oob_block_text}"
    );
    assert!(
        oob_block_text.contains("unreachable"),
        "idx_oob block should end with unreachable:\n{oob_block_text}"
    );
}
