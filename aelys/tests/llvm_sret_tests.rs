use aelys_air::{
    AirBlock, AirConst, AirFunction, AirLocal, AirProgram, AirStmt, AirStmtKind, AirTerminator,
    AirType, BlockId, Callee, CallingConv, FunctionAttribs, FunctionId, GcMode, InlineHint,
    LocalId, Operand, Place, Rvalue,
};
use aelys_codegen::CodegenContext;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use std::fs;
use tempfile::tempdir;

fn compile_air_to_verified_ir(program: &AirProgram) -> String {
    let dir = tempdir().expect("tempdir");
    let ll_path = dir.path().join("module.ll");
    let ll_str = ll_path.to_string_lossy().to_string();

    let mut codegen = CodegenContext::new("sret_test");
    codegen.compile(program).expect("codegen should succeed");
    codegen.emit_ir(&ll_str).expect("emit_ir should succeed");

    let ir = fs::read_to_string(&ll_path).expect("ir file should exist");

    let context = Context::create();
    let buffer = MemoryBuffer::create_from_file(&ll_path).expect("ir should be readable");
    let module = context
        .create_module_from_ir(buffer)
        .expect("ir should parse");
    module.verify().expect("module should verify");

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
fn extern_c_struct_return_compiles_and_verifies() {
    let program = AirProgram {
        functions: vec![
            // extern "C" fn get_name() -> Str
            AirFunction {
                id: FunctionId(0),
                name: "get_name".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::Str,
                locals: vec![],
                blocks: vec![],
                is_extern: true,
                calling_conv: CallingConv::C,
                attributes: default_attribs(),
                span: None,
            },
            // fn caller() -> i64 { let s = get_name(); s.len }
            AirFunction {
                id: FunctionId(1),
                name: "caller".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::I64,
                locals: vec![
                    AirLocal {
                        id: LocalId(0),
                        ty: AirType::Str,
                        name: None,
                        is_mut: false,
                        span: None,
                    },
                    AirLocal {
                        id: LocalId(1),
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
                                place: Place::Local(LocalId(0)),
                                rvalue: Rvalue::Call {
                                    func: Callee::Extern("get_name".to_string(), CallingConv::C),
                                    args: vec![],
                                },
                            },
                            span: None,
                        },
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(1)),
                                rvalue: Rvalue::FieldAccess {
                                    base: Operand::Copy(LocalId(0)),
                                    field: "len".to_string(),
                                },
                            },
                            span: None,
                        },
                    ],
                    terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(1)))),
                }],
                is_extern: false,
                calling_conv: CallingConv::Aelys,
                attributes: default_attribs(),
                span: None,
            },
        ],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
    };

    let ir = compile_air_to_verified_ir(&program);

    if cfg!(target_os = "windows") {
        // declaration should be void with sret ptr as first param
        assert!(
            ir.contains("declare void @get_name(ptr"),
            "sret extern should be declared as void(ptr sret(...)): {ir}"
        );
        assert!(
            ir.contains("sret"),
            "sret attribute should be present on Windows: {ir}"
        );
        // should not return %__aelys_string directly
        assert!(
            !ir.contains("declare %__aelys_string @get_name()"),
            "sret extern should not return struct directly on Windows: {ir}"
        );
    } else {
        // on non-Windows, struct is returned directly
        assert!(
            ir.contains("declare %__aelys_string @get_name()"),
            "non-sret extern should return struct directly: {ir}"
        );
    }
}

/// An Aelys function with C calling convention returning a struct should usen, sret on Windows: the return is stored via the sret pointer and the function, returns void at the LLVM level.
#[test]
fn c_convention_aelys_fn_returning_struct_uses_sret() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "make_greeting".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Str,
            locals: vec![AirLocal {
                id: LocalId(0),
                ty: AirType::Str,
                name: None,
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![AirStmt {
                    kind: AirStmtKind::Assign {
                        place: Place::Local(LocalId(0)),
                        rvalue: Rvalue::Use(Operand::Const(AirConst::Str("hello".to_string()))),
                    },
                    span: None,
                }],
                terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(0)))),
            }],
            is_extern: false,
            calling_conv: CallingConv::C,
            attributes: default_attribs(),
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
    };

    let ir = compile_air_to_verified_ir(&program);

    if cfg!(target_os = "windows") {
        // definition should be void with sret param
        assert!(
            ir.contains("define void @make_greeting(ptr"),
            "sret function should be defined as void(ptr sret(...)): {ir}"
        );
        assert!(
            ir.contains("sret"),
            "sret attribute should be present on Windows: {ir}"
        );
        // the return should be `ret void`, not `ret %__aelys_string ...`
        assert!(
            ir.contains("ret void"),
            "sret function should return void: {ir}"
        );
    } else {
        assert!(
            ir.contains("define %__aelys_string @make_greeting()"),
            "non-sret function should return struct directly: {ir}"
        );
    }
}

/// fastcc (Aelys-internal) functions returning structs should not use sret
/// LLVM handles the ABI internally for fastcc within the same module.
#[test]
fn fastcc_struct_return_does_not_use_sret() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "internal_fn".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Str,
            locals: vec![AirLocal {
                id: LocalId(0),
                ty: AirType::Str,
                name: None,
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![AirStmt {
                    kind: AirStmtKind::Assign {
                        place: Place::Local(LocalId(0)),
                        rvalue: Rvalue::Use(Operand::Const(AirConst::Str("hello".to_string()))),
                    },
                    span: None,
                }],
                terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(0)))),
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
    };

    let ir = compile_air_to_verified_ir(&program);

    // fastcc should return struct directly, never sret
    assert!(
        ir.contains("define fastcc %__aelys_string @internal_fn()"),
        "fastcc function should return struct directly: {ir}"
    );
    // sret should not appear anywhere for internal functions
    let sret_on_internal = ir
        .lines()
        .any(|l| l.contains("internal_fn") && l.contains("sret"));
    assert!(!sret_on_internal, "fastcc function must not use sret: {ir}");
}

#[test]
fn extern_c_data_enum_return_uses_sret() {
    let program = AirProgram {
        functions: vec![
            AirFunction {
                id: FunctionId(0),
                name: "get_opt".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::Enum("Opt".to_string()),
                locals: vec![],
                blocks: vec![],
                is_extern: true,
                calling_conv: CallingConv::C,
                attributes: default_attribs(),
                span: None,
            },
            AirFunction {
                id: FunctionId(1),
                name: "caller".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::I64,
                locals: vec![
                    AirLocal {
                        id: LocalId(0),
                        ty: AirType::Enum("Opt".to_string()),
                        name: None,
                        is_mut: false,
                        span: None,
                    },
                    AirLocal {
                        id: LocalId(1),
                        ty: AirType::I32,
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
                                place: Place::Local(LocalId(0)),
                                rvalue: Rvalue::Call {
                                    func: Callee::Extern("get_opt".to_string(), CallingConv::C),
                                    args: vec![],
                                },
                            },
                            span: None,
                        },
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(1)),
                                rvalue: Rvalue::EnumTag {
                                    enum_name: "Opt".to_string(),
                                    operand: Operand::Copy(LocalId(0)),
                                },
                            },
                            span: None,
                        },
                    ],
                    terminator: AirTerminator::Return(Some(Operand::Const(AirConst::Int(
                        0,
                        aelys_air::AirIntSize::I64,
                    )))),
                }],
                is_extern: false,
                calling_conv: CallingConv::Aelys,
                attributes: default_attribs(),
                span: None,
            },
        ],
        structs: vec![],
        enums: vec![aelys_air::AirEnumDef {
            name: "Opt".to_string(),
            type_params: vec![],
            variants: vec![
                aelys_air::AirEnumVariant {
                    name: "Some".to_string(),
                    payload: vec![AirType::I64],
                    tag: 0,
                },
                aelys_air::AirEnumVariant {
                    name: "None".to_string(),
                    payload: vec![],
                    tag: 1,
                },
            ],
            span: None,
        }],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::from([(
            "Opt".to_string(),
            aelys_air::layout::TypeLayout { size: 16, align: 8 },
        )]),
    };

    let ir = compile_air_to_verified_ir(&program);

    if cfg!(target_os = "windows") {
        assert!(
            ir.contains("declare void @get_opt(ptr"),
            "data enum extern should use sret on Windows: {ir}"
        );
        assert!(
            ir.contains("sret"),
            "data enum extern should carry sret attribute on Windows: {ir}"
        );
        assert!(
            !ir.contains("declare %__aelys_enum_Opt @get_opt()"),
            "data enum extern must not return aggregate directly on Windows: {ir}"
        );
    }
}

#[test]
fn extern_c_data_enum_param_is_rejected() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "consume_opt".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![aelys_air::AirParam {
                id: LocalId(0),
                ty: AirType::Enum("Opt".to_string()),
                name: "opt".to_string(),
                span: None,
            }],
            ret_ty: AirType::Void,
            locals: vec![],
            blocks: vec![],
            is_extern: true,
            calling_conv: CallingConv::C,
            attributes: default_attribs(),
            span: None,
        }],
        structs: vec![],
        enums: vec![aelys_air::AirEnumDef {
            name: "Opt".to_string(),
            type_params: vec![],
            variants: vec![
                aelys_air::AirEnumVariant {
                    name: "Some".to_string(),
                    payload: vec![AirType::I64],
                    tag: 0,
                },
                aelys_air::AirEnumVariant {
                    name: "None".to_string(),
                    payload: vec![],
                    tag: 1,
                },
            ],
            span: None,
        }],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::from([(
            "Opt".to_string(),
            aelys_air::layout::TypeLayout { size: 16, align: 8 },
        )]),
    };

    let mut codegen = CodegenContext::new("enum_param_reject");
    let err = codegen
        .compile(&program)
        .expect_err("extern C data enum param should be rejected");
    let rendered = err.to_string();
    assert!(
        rendered.contains("enum parameter"),
        "unexpected error for extern C data enum param: {rendered}"
    );
}

#[test]
fn indirect_c_fnptr_data_enum_return_uses_sret() {
    let program = AirProgram {
        functions: vec![
            AirFunction {
                id: FunctionId(0),
                name: "get_opt".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::Enum("Opt".to_string()),
                locals: vec![],
                blocks: vec![],
                is_extern: true,
                calling_conv: CallingConv::C,
                attributes: default_attribs(),
                span: None,
            },
            AirFunction {
                id: FunctionId(1),
                name: "caller".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::I64,
                locals: vec![
                    AirLocal {
                        id: LocalId(0),
                        ty: AirType::FnPtr {
                            params: vec![],
                            ret: Box::new(AirType::Enum("Opt".to_string())),
                            conv: CallingConv::C,
                        },
                        name: None,
                        is_mut: false,
                        span: None,
                    },
                    AirLocal {
                        id: LocalId(1),
                        ty: AirType::Enum("Opt".to_string()),
                        name: None,
                        is_mut: false,
                        span: None,
                    },
                    AirLocal {
                        id: LocalId(2),
                        ty: AirType::I32,
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
                                place: Place::Local(LocalId(0)),
                                rvalue: Rvalue::Use(Operand::Const(AirConst::FnRef(
                                    "get_opt".to_string(),
                                ))),
                            },
                            span: None,
                        },
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(1)),
                                rvalue: Rvalue::Call {
                                    func: Callee::FnPtr(LocalId(0)),
                                    args: vec![],
                                },
                            },
                            span: None,
                        },
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(2)),
                                rvalue: Rvalue::EnumTag {
                                    enum_name: "Opt".to_string(),
                                    operand: Operand::Copy(LocalId(1)),
                                },
                            },
                            span: None,
                        },
                    ],
                    terminator: AirTerminator::Return(Some(Operand::Const(AirConst::Int(
                        0,
                        aelys_air::AirIntSize::I64,
                    )))),
                }],
                is_extern: false,
                calling_conv: CallingConv::Aelys,
                attributes: default_attribs(),
                span: None,
            },
        ],
        structs: vec![],
        enums: vec![aelys_air::AirEnumDef {
            name: "Opt".to_string(),
            type_params: vec![],
            variants: vec![
                aelys_air::AirEnumVariant {
                    name: "Some".to_string(),
                    payload: vec![AirType::I64],
                    tag: 0,
                },
                aelys_air::AirEnumVariant {
                    name: "None".to_string(),
                    payload: vec![],
                    tag: 1,
                },
            ],
            span: None,
        }],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::from([(
            "Opt".to_string(),
            aelys_air::layout::TypeLayout { size: 16, align: 8 },
        )]),
    };

    let ir = compile_air_to_verified_ir(&program);

    if cfg!(target_os = "windows") {
        let indirect_call_line = ir
            .lines()
            .find(|line| line.contains("call") && line.contains("sret_slot"))
            .expect("expected indirect call using the hidden sret slot");
        assert!(
            indirect_call_line.contains("call void @get_opt(")
                || indirect_call_line.contains("call void %"),
            "indirect c fnptr should lower through a call instruction: {indirect_call_line}\n{ir}"
        );
        assert!(
            indirect_call_line.contains("sret("),
            "indirect c fnptr callsite must carry the sret attribute: {indirect_call_line}\n{ir}"
        );
        assert!(ir.contains("sret_slot"), "{ir}");
        assert!(
            !ir.contains("call %__aelys_enum_Opt @get_opt()"),
            "indirect c fnptr must not return the aggregate directly on Windows: {ir}"
        );
    }
}
