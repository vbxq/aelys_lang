use aelys_air::{
    AirBlock, AirConst, AirFunction, AirIntSize, AirLocal, AirParam, AirProgram, AirStmt,
    AirStmtKind, AirTerminator, AirType, BlockId, Callee, CallingConv, FunctionAttribs, FunctionId,
    GcMode, InlineHint, LocalId, Operand, Place, Rvalue,
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
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
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
        // should NOT return %__aelys_string directly
        assert!(
            !ir.contains("declare %__aelys_string @get_name()"),
            "sret extern should NOT return struct directly on Windows: {ir}"
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
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
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
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    };

    let ir = compile_air_to_verified_ir(&program);

    // fastcc should return struct directly, never sret
    assert!(
        ir.contains("define fastcc %__aelys_string @internal_fn()"),
        "fastcc function should return struct directly: {ir}"
    );
    // sret should NOT appear anywhere for internal functions
    let sret_on_internal = ir
        .lines()
        .any(|l| l.contains("internal_fn") && l.contains("sret"));
    assert!(!sret_on_internal, "fastcc function must not use sret: {ir}");
}
