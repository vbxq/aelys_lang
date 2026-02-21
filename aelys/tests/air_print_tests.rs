use aelys_air::print::{fmt_const, fmt_type, print_block, print_function, print_program};
use aelys_air::*;

fn empty_program() -> AirProgram {
    AirProgram {
        functions: vec![],
        structs: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    }
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
fn struct_with_offsets() {
    let mut p = empty_program();
    p.structs.push(AirStructDef {
        name: "Point".into(),
        type_params: vec![],
        fields: vec![
            AirStructField {
                name: "x".into(),
                ty: AirType::I64,
                offset: Some(0),
            },
            AirStructField {
                name: "y".into(),
                ty: AirType::F64,
                offset: Some(8),
            },
        ],
        is_closure_env: false,
        span: None,
    });
    let out = print_program(&p);
    assert_eq!(
        out,
        "struct Point\n  x: i64  @ 0\n  y: f64  @ 8\n  [size=16, align=8]\n"
    );
}

#[test]
fn struct_closure_env() {
    let mut p = empty_program();
    p.structs.push(AirStructDef {
        name: "__closure_env_foo".into(),
        type_params: vec![],
        fields: vec![AirStructField {
            name: "x".into(),
            ty: AirType::I64,
            offset: Some(0),
        }],
        is_closure_env: true,
        span: None,
    });
    let out = print_program(&p);
    assert!(out.contains("struct __closure_env_foo  [closure_env]\n"));
    assert!(out.contains("[size=8, align=8]"));
}

#[test]
fn struct_no_offsets() {
    let mut p = empty_program();
    p.structs.push(AirStructDef {
        name: "Foo".into(),
        type_params: vec![],
        fields: vec![AirStructField {
            name: "a".into(),
            ty: AirType::I32,
            offset: None,
        }],
        is_closure_env: false,
        span: None,
    });
    let out = print_program(&p);
    assert!(out.contains("  a: i32  @ ?\n"));
}

#[test]
fn function_with_locals_and_blocks() {
    let mut p = empty_program();
    p.functions.push(AirFunction {
        id: FunctionId(0),
        name: "add".into(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![
            AirParam {
                id: LocalId(0),
                ty: AirType::I64,
                name: "a".into(),
                span: None,
            },
            AirParam {
                id: LocalId(1),
                ty: AirType::I64,
                name: "b".into(),
                span: None,
            },
        ],
        ret_ty: AirType::I64,
        locals: vec![
            AirLocal {
                id: LocalId(2),
                ty: AirType::I64,
                name: None,
                is_mut: false,
                span: None,
            },
            AirLocal {
                id: LocalId(3),
                ty: AirType::Bool,
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
                        place: Place::Local(LocalId(2)),
                        rvalue: Rvalue::BinaryOp(
                            BinOp::Add,
                            Operand::Copy(LocalId(0)),
                            Operand::Copy(LocalId(1)),
                        ),
                    },
                    span: None,
                },
                AirStmt {
                    kind: AirStmtKind::Assign {
                        place: Place::Local(LocalId(3)),
                        rvalue: Rvalue::BinaryOp(
                            BinOp::Lt,
                            Operand::Copy(LocalId(2)),
                            Operand::Const(AirConst::Int(10, AirIntSize::I64)),
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
    });
    let out = print_function(&p.functions[0], &p);
    let expected = "\
fn add(a: i64, b: i64) -> i64  [managed, aelys]
  locals:
    %2: i64
    %3: bool
  block0:
    %2: i64 = binop add %0: i64, %1: i64
    %3: bool = binop lt %2: i64, 10i64
    return %2: i64\n";
    assert_eq!(out, expected);
}

#[test]
fn extern_function() {
    let mut p = empty_program();
    p.functions.push(AirFunction {
        id: FunctionId(0),
        name: "puts".into(),
        gc_mode: GcMode::Manual,
        type_params: vec![],
        params: vec![AirParam {
            id: LocalId(0),
            ty: AirType::Str,
            name: "s".into(),
            span: None,
        }],
        ret_ty: AirType::Void,
        locals: vec![],
        blocks: vec![],
        is_extern: true,
        calling_conv: CallingConv::C,
        attributes: default_attribs(),
        span: None,
    });
    let out = print_function(&p.functions[0], &p);
    assert_eq!(out, "fn puts(s: str) -> void  [extern]\n");
}

#[test]
fn generic_function() {
    let mut p = empty_program();
    p.functions.push(AirFunction {
        id: FunctionId(0),
        name: "identity".into(),
        gc_mode: GcMode::Managed,
        type_params: vec![TypeParamId(0)],
        params: vec![AirParam {
            id: LocalId(0),
            ty: AirType::Param(TypeParamId(0)),
            name: "x".into(),
            span: None,
        }],
        ret_ty: AirType::Param(TypeParamId(0)),
        locals: vec![],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![],
            terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(0)))),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    });
    let out = print_function(&p.functions[0], &p);
    assert!(out.starts_with("fn identity<T0>(x: T0) -> T0  [managed, aelys]\n"));
}

#[test]
fn mono_instances() {
    let mut p = empty_program();
    p.functions.push(AirFunction {
        id: FunctionId(0),
        name: "identity".into(),
        gc_mode: GcMode::Managed,
        type_params: vec![TypeParamId(0)],
        params: vec![AirParam {
            id: LocalId(0),
            ty: AirType::Param(TypeParamId(0)),
            name: "x".into(),
            span: None,
        }],
        ret_ty: AirType::Param(TypeParamId(0)),
        locals: vec![],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![],
            terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(0)))),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    });
    p.functions.push(AirFunction {
        id: FunctionId(1),
        name: "__mono_identity_i64".into(),
        gc_mode: GcMode::Manual,
        type_params: vec![],
        params: vec![AirParam {
            id: LocalId(0),
            ty: AirType::I64,
            name: "x".into(),
            span: None,
        }],
        ret_ty: AirType::I64,
        locals: vec![],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![AirStmt {
                kind: AirStmtKind::Assign {
                    place: Place::Local(LocalId(0)),
                    rvalue: Rvalue::Use(Operand::Const(AirConst::Int(42, AirIntSize::I64))),
                },
                span: None,
            }],
            terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(0)))),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    });
    p.mono_instances.push(MonoInstance {
        original: FunctionId(0),
        type_args: vec![AirType::I64],
        result: FunctionId(1),
    });
    let out = print_program(&p);
    assert!(out.contains("# mono instances\n"));
    assert!(out.contains("identity[i64] -> __mono_identity_i64\n"));
}

#[test]
fn control_flow_terminators() {
    let mut p = empty_program();
    p.functions.push(AirFunction {
        id: FunctionId(0),
        name: "do_work".into(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![AirParam {
            id: LocalId(0),
            ty: AirType::Bool,
            name: "cond".into(),
            span: None,
        }],
        ret_ty: AirType::Void,
        locals: vec![],
        blocks: vec![
            AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Goto(BlockId(1)),
            },
            AirBlock {
                id: BlockId(1),
                stmts: vec![],
                terminator: AirTerminator::Branch {
                    cond: Operand::Copy(LocalId(0)),
                    then_block: BlockId(2),
                    else_block: BlockId(3),
                },
            },
            AirBlock {
                id: BlockId(2),
                stmts: vec![],
                terminator: AirTerminator::Goto(BlockId(1)),
            },
            AirBlock {
                id: BlockId(3),
                stmts: vec![],
                terminator: AirTerminator::Return(None),
            },
        ],
        is_extern: false,
        calling_conv: CallingConv::C,
        attributes: default_attribs(),
        span: None,
    });
    let out = print_function(&p.functions[0], &p);
    assert!(out.contains("    goto block1\n"));
    assert!(out.contains("    branch %0: bool -> block2 else block3\n"));
    assert!(out.contains("    return void\n"));
}

#[test]
fn void_call_and_fence() {
    let mut p = empty_program();
    p.functions.push(AirFunction {
        id: FunctionId(0),
        name: "test".into(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![AirParam {
            id: LocalId(0),
            ty: AirType::I64,
            name: "a".into(),
            span: None,
        }],
        ret_ty: AirType::Void,
        locals: vec![],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![
                AirStmt {
                    kind: AirStmtKind::CallVoid {
                        func: Callee::Named("foo".into()),
                        args: vec![Operand::Copy(LocalId(0))],
                    },
                    span: None,
                },
                AirStmt {
                    kind: AirStmtKind::MemoryFence(Ordering::SeqCst),
                    span: None,
                },
            ],
            terminator: AirTerminator::Return(None),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    });
    let out = print_function(&p.functions[0], &p);
    assert!(out.contains("    call void foo(%0: i64)\n"));
    assert!(out.contains("    fence seq_cst\n"));
}

#[test]
fn print_block_standalone() {
    let mut p = empty_program();
    p.functions.push(AirFunction {
        id: FunctionId(0),
        name: "f".into(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![AirParam {
            id: LocalId(0),
            ty: AirType::I64,
            name: "x".into(),
            span: None,
        }],
        ret_ty: AirType::I64,
        locals: vec![],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![],
            terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(0)))),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    });
    let out = print_block(&p.functions[0].blocks[0], &p);
    assert_eq!(out, "  block0:\n    return %0: i64\n");
}

#[test]
fn rvalue_formats() {
    assert_eq!(fmt_const(&AirConst::Int(42, AirIntSize::I64)), "42i64");
    assert_eq!(
        fmt_const(&AirConst::Float(std::f64::consts::PI, AirFloatSize::F64)),
        format!("{}f64", std::f64::consts::PI)
    );
    assert_eq!(
        fmt_const(&AirConst::Float(1.0, AirFloatSize::F32)),
        "1.0f32"
    );
    assert_eq!(fmt_const(&AirConst::Bool(true)), "true");
    assert_eq!(fmt_const(&AirConst::Null), "null");
    assert_eq!(fmt_const(&AirConst::Undef(AirType::I32)), "undef i32");
    assert_eq!(
        fmt_const(&AirConst::ZeroInit(AirType::Struct("Point".into()))),
        "zeroinit Point"
    );
}

#[test]
fn type_formats() {
    assert_eq!(fmt_type(&AirType::Ptr(Box::new(AirType::I64))), "*i64");
    assert_eq!(
        fmt_type(&AirType::Array(Box::new(AirType::F32), 4)),
        "[f32; 4]"
    );
    assert_eq!(fmt_type(&AirType::Slice(Box::new(AirType::U8))), "[u8]");
    assert_eq!(
        fmt_type(&AirType::FnPtr {
            params: vec![AirType::I64],
            ret: Box::new(AirType::Bool),
            conv: CallingConv::Aelys,
        }),
        "fn(i64) -> bool"
    );
    assert_eq!(fmt_type(&AirType::Param(TypeParamId(0))), "T0");
}
