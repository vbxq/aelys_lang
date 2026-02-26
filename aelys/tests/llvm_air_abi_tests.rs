use aelys_air::layout::layout_of;
use aelys_air::{
    AirBlock, AirConst, AirFunction, AirIntSize, AirLocal, AirParam, AirProgram, AirStmt,
    AirStmtKind, AirTerminator, AirType, BlockId, CallingConv, FunctionAttribs, FunctionId, GcMode,
    InlineHint, LocalId, Operand, Place, Rvalue,
};
use aelys_codegen::CodegenContext;
use aelys_codegen::types::alignment_of;
use inkwell::OptimizationLevel;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::types::BasicTypeEnum;
use std::fs;
use tempfile::tempdir;

fn compile_air_to_verified_ir(program: &AirProgram) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let ll_path = dir.path().join("module.ll");
    let ll_path_str = ll_path.to_string_lossy().to_string();

    let mut codegen = CodegenContext::new("abi_hardening");
    codegen
        .compile(program)
        .expect("codegen compilation should succeed");
    codegen
        .emit_ir(&ll_path_str)
        .expect("llvm ir should be emitted");

    let ir = fs::read_to_string(&ll_path).expect("llvm ir file should be generated");

    let context = Context::create();
    let buffer = MemoryBuffer::create_from_file(&ll_path).expect("llvm ir should be readable");
    let module = context
        .create_module_from_ir(buffer)
        .expect("llvm ir should parse into a module");
    module
        .verify()
        .expect("module.verify() should succeed for generated ir");

    ir
}

#[test]
fn llvm_panic_uses_ptr_len_signature() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "panic_probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Void,
            locals: vec![],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Panic {
                    message: "X".to_string(),
                    span: None,
                },
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    };

    let ir = compile_air_to_verified_ir(&program);
    assert!(ir.contains("declare void @__aelys_panic(ptr, i64)"), "{ir}");
    assert!(ir.contains("call void @__aelys_panic(ptr"), "{ir}");
    assert!(!ir.contains("declare void @__aelys_panic(ptr)"), "{ir}");
    assert!(
        ir.contains("noreturn"),
        "panic declaration should have noreturn attribute: {ir}"
    );
}

#[test]
fn air_and_llvm_string_layout_match_x86_64_abi() {
    let air_layout = layout_of(&AirType::Str);
    assert_eq!(air_layout.size, 16);
    assert_eq!(air_layout.align, 8);

    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "sink".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![aelys_air::AirParam {
                id: aelys_air::LocalId(0),
                ty: AirType::Str,
                name: "s".to_string(),
                span: None,
            }],
            ret_ty: AirType::I64,
            locals: vec![],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Return(Some(Operand::Const(AirConst::Int(
                    0,
                    AirIntSize::I64,
                )))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    };

    let ir = compile_air_to_verified_ir(&program);
    assert!(ir.contains("%__aelys_string = type { ptr, i64 }"), "{ir}");
    assert!(
        ir.contains("define fastcc i64 @sink(%__aelys_string"),
        "{ir}"
    );
    assert!(!ir.contains("define fastcc i64 @sink(ptr"), "{ir}");

    let context = Context::create();
    let mut nul_terminated_ir = ir.into_bytes();
    nul_terminated_ir.push(0);
    let buffer = MemoryBuffer::create_from_memory_range_copy(&nul_terminated_ir, "module.ll");
    let module = context
        .create_module_from_ir(buffer)
        .expect("llvm ir should parse into a module");

    let str_ty = module
        .get_struct_type("__aelys_string")
        .expect("string struct should exist");
    assert!(!str_ty.is_packed(), "string struct must be non-packed");

    let fields = str_ty.get_field_types();
    assert_eq!(
        fields.len(),
        2,
        "string struct must have exactly two fields"
    );
    assert!(
        matches!(fields[0], BasicTypeEnum::PointerType(_)),
        "field #0 must be ptr"
    );
    match fields[1] {
        BasicTypeEnum::IntType(int_ty) => assert_eq!(int_ty.get_bit_width(), 64),
        _ => panic!("field #1 must be i64"),
    }

    Target::initialize_native(&InitializationConfig::default())
        .expect("native target initialization should succeed");
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).expect("target triple should be supported");
    let cpu = TargetMachine::get_host_cpu_name().to_string();
    let features = TargetMachine::get_host_cpu_features().to_string();
    let target_machine = target
        .create_target_machine(
            &triple,
            &cpu,
            &features,
            OptimizationLevel::None,
            RelocMode::Default,
            CodeModel::Default,
        )
        .expect("target machine should be created");
    let target_data = target_machine.get_target_data();

    let llvm_align = alignment_of(str_ty.into());
    let llvm_align_abi = target_data.get_abi_alignment(&str_ty);
    let llvm_size = target_data.get_abi_size(&str_ty);

    assert_eq!(llvm_align, 8);
    assert_eq!(llvm_align_abi, 8);
    assert_eq!(llvm_size, 16);
    assert_eq!(llvm_align, air_layout.align);
    assert_eq!(
        u32::try_from(llvm_size).expect("size should fit u32"),
        air_layout.size
    );
}

#[test]
fn ssa_params_use_llvm_args_directly() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "echo".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![AirParam {
                id: LocalId(0),
                ty: AirType::Str,
                name: "s".to_string(),
                span: None,
            }],
            ret_ty: AirType::I64,
            locals: vec![AirLocal {
                id: LocalId(1),
                ty: AirType::I64,
                name: None,
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![AirStmt {
                    kind: AirStmtKind::Assign {
                        place: Place::Local(LocalId(1)),
                        rvalue: Rvalue::FieldAccess {
                            base: Operand::Copy(LocalId(0)),
                            field: "len".to_string(),
                        },
                    },
                    span: None,
                }],
                terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(1)))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    };

    let ir = compile_air_to_verified_ir(&program);
    assert!(
        !ir.contains("alloca"),
        "SSA params/locals must not generate alloca:\n{ir}"
    );
    assert!(
        !ir.contains("store"),
        "SSA params/locals must not generate store:\n{ir}"
    );
    assert!(
        ir.contains("extractvalue"),
        "field access on SSA str should use extractvalue:\n{ir}"
    );
}

#[test]
fn mutable_locals_use_alloca_with_correct_alignment() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "mut_probe".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![
                AirParam {
                    id: LocalId(0),
                    ty: AirType::I64,
                    name: "x".to_string(),
                    span: None,
                },
                AirParam {
                    id: LocalId(1),
                    ty: AirType::I64,
                    name: "y".to_string(),
                    span: None,
                },
            ],
            ret_ty: AirType::I64,
            locals: vec![AirLocal {
                id: LocalId(2),
                ty: AirType::I64,
                name: Some("a".to_string()),
                is_mut: true,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(2)),
                            rvalue: Rvalue::Use(Operand::Copy(LocalId(0))),
                        },
                        span: None,
                    },
                    AirStmt {
                        kind: AirStmtKind::Assign {
                            place: Place::Local(LocalId(2)),
                            rvalue: Rvalue::Use(Operand::Copy(LocalId(1))),
                        },
                        span: None,
                    },
                ],
                terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(2)))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
    };

    let ir = compile_air_to_verified_ir(&program);
    assert!(
        ir.contains("alloca i64"),
        "mutable local must use alloca:\n{ir}"
    );
    for line in ir.lines().filter(|l| l.contains("alloca i64")) {
        assert!(
            line.contains("align 8"),
            "i64 alloca must have align 8: {line}"
        );
    }
    for line in ir.lines().filter(|l| l.contains("store i64 ")) {
        assert!(
            line.contains("align 8"),
            "i64 store must have align 8: {line}"
        );
    }
    for line in ir.lines().filter(|l| l.contains("load i64,")) {
        assert!(
            line.contains("align 8"),
            "i64 load must have align 8: {line}"
        );
    }
}
