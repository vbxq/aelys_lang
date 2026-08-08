// it is the only oracle in the tree that can exercise an air shape the surface language cannot yet
// the arm has no head
// hygiene: the owed value is 101 and every plausible wrong answer (7, 0, 2) is also < 256, so the

use aelys_air::{
    AirBlock, AirConst, AirFunction, AirLocal, AirParam, AirProgram, AirStmt, AirStmtKind,
    AirTerminator, AirType, BlockId, CallingConv, FunctionAttribs, FunctionId, GcMode, InlineHint,
    LocalId, Operand, Place, Rvalue,
};
use aelys_driver::{RuntimeVariant, compile_air_program_to_executable};
use aelys_opt::OptimizationLevel;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

fn attribs() -> FunctionAttribs {
    FunctionAttribs {
        inline: InlineHint::Default,
        no_gc: false,
        no_unwind: false,
        cold: false,
    }
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found") || error.contains("failed to run")
}

/// emit, link and run a hand-built air program at every optimization level. `none` means the
/// toolchain cannot link here, which is a skip and not a failure.
fn run_air_program(name: &str, program: &AirProgram) -> Option<Vec<(&'static str, i32)>> {
    let mut out: Vec<(&'static str, i32)> = Vec::new();
    let dir = tempdir().expect("tempdir");
    for (level, opt) in LEVELS {
        let path = dir.path().join(format!("{name}{}.aelys", level.replace('-', "_")));
        match compile_air_program_to_executable(&path, program, *opt, RuntimeVariant::Rc) {
            Ok(()) => {}
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    eprintln!("{name}: linker unavailable, skipping");
                    return None;
                }
                panic!("{name} at {level} must compile:\n{err}");
            }
        }
        let exe = exe_path_for(&path);
        if !exe.is_file() {
            eprintln!("{name}: no executable produced, skipping");
            return None;
        }
        let status = Command::new(&exe).status().expect("run exe");
        out.push((*level, status.code().unwrap_or(-1)));
    }
    Some(out)
}

fn program_with(locals: Vec<AirLocal>, stmts: Vec<AirStmt>, ret: Operand) -> AirProgram {
    AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "main".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: Vec::<AirParam>::new(),
            ret_ty: AirType::I64,
            locals,
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts,
                terminator: AirTerminator::Return(Some(ret)),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: attribs(),
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    }
}

fn local(id: u32, ty: AirType, name: Option<&str>, is_mut: bool) -> AirLocal {
    AirLocal {
        id: LocalId(id),
        ty,
        name: name.map(|s| s.to_string()),
        is_mut,
        span: None,
    }
}

fn assign(place: Place, rvalue: Rvalue) -> AirStmt {
    AirStmt {
        kind: AirStmtKind::Assign { place, rvalue },
        span: None,
    }
}

/// at the sentinel this fails codegen with `cannot index into ptr(array(i64, 3))`, raised by
#[test]
fn index_through_a_pointer_to_an_array_writes_the_owner() {
    let arr = AirType::Array(Box::new(AirType::I64), 3);
    let program = program_with(
        vec![
            local(0, arr.clone(), Some("a"), true),
            local(1, AirType::Ptr(Box::new(arr.clone())), None, false),
            local(2, AirType::I64, None, false),
        ],
        vec![
            assign(
                Place::Local(LocalId(0)),
                Rvalue::Use(Operand::Const(AirConst::ZeroInit(arr.clone()))),
            ),
            assign(
                Place::Local(LocalId(1)),
                Rvalue::AddressOf(Place::Local(LocalId(0))),
            ),
            assign(
                Place::Index(LocalId(1), Operand::Const(AirConst::Int(1, aelys_air::AirIntSize::I64))),
                Rvalue::Use(Operand::Const(AirConst::Int(101, aelys_air::AirIntSize::I64))),
            ),
// read back through the owner, never through the pointer that wrote
            assign(
                Place::Local(LocalId(2)),
                Rvalue::Index {
                    base: Operand::Copy(LocalId(0)),
                    index: Operand::Const(AirConst::Int(1, aelys_air::AirIntSize::I64)),
                },
            ),
        ],
        Operand::Copy(LocalId(2)),
    );

    let Some(results) = run_air_program("air_index_ptr", &program) else {
        return;
    };
    for (level, code) in results {
        assert_eq!(
            code, 101,
            "Place::Index(Ptr(Array)) must write the owner's element at {level}"
        );
    }
}

/// `place::field(ptr(struct))` on a stack-derived pointer. this is the shape the design's
#[test]
fn field_through_a_stack_derived_pointer_writes_the_owner() {
    let cell = AirType::Struct("Cell".to_string());
    let mut program = program_with(
        vec![
            local(0, cell.clone(), Some("c"), true),
            local(1, AirType::Ptr(Box::new(cell.clone())), None, false),
            local(2, AirType::I64, None, false),
        ],
        vec![
            assign(
                Place::Local(LocalId(0)),
                Rvalue::StructInit {
                    name: "Cell".to_string(),
                    fields: vec![
                        ("f".to_string(), Operand::Const(AirConst::Int(7, aelys_air::AirIntSize::I64))),
                        ("g".to_string(), Operand::Const(AirConst::Int(0, aelys_air::AirIntSize::I64))),
                    ],
                },
            ),
            assign(
                Place::Local(LocalId(1)),
                Rvalue::AddressOf(Place::Local(LocalId(0))),
            ),
            assign(
                Place::Field(LocalId(1), "f".to_string()),
                Rvalue::Use(Operand::Const(AirConst::Int(101, aelys_air::AirIntSize::I64))),
            ),
            assign(
                Place::Local(LocalId(2)),
                Rvalue::FieldAccess {
                    base: Operand::Copy(LocalId(0)),
                    field: "f".to_string(),
                },
            ),
        ],
        Operand::Copy(LocalId(2)),
    );
    program.structs = vec![aelys_air::AirStructDef {
        name: "Cell".to_string(),
        type_params: vec![],
        fields: vec![
            aelys_air::AirStructField {
                name: "f".to_string(),
                ty: AirType::I64,
                offset: None,
            },
            aelys_air::AirStructField {
                name: "g".to_string(),
                ty: AirType::I64,
                offset: None,
            },
        ],
        is_closure_env: false,
        span: None,
    }];

    let Some(results) = run_air_program("air_field_ptr", &program) else {
        return;
    };
    for (level, code) in results {
        assert_eq!(
            code, 101,
            "Place::Field(Ptr(Struct)) must write the owner's field at {level}"
        );
    }
}

/// `place`. every intermediate is a `ptr(t)` local, and the answer must still reach the owner.
#[test]
fn a_chained_address_of_a_field_writes_the_owner() {
    let inner = AirType::Struct("In".to_string());
    let outer = AirType::Struct("Out".to_string());
    let mut program = program_with(
        vec![
            local(0, outer.clone(), Some("o"), true),
            local(1, AirType::Ptr(Box::new(outer.clone())), None, false),
            local(2, AirType::Ptr(Box::new(inner.clone())), None, false),
            local(3, inner.clone(), None, false),
            local(4, AirType::I64, None, false),
        ],
        vec![
            assign(
                Place::Local(LocalId(0)),
                Rvalue::Use(Operand::Const(AirConst::ZeroInit(outer.clone()))),
            ),
            assign(
                Place::Local(LocalId(1)),
                Rvalue::AddressOf(Place::Local(LocalId(0))),
            ),
            assign(
                Place::Local(LocalId(2)),
                Rvalue::AddressOf(Place::Field(LocalId(1), "i".to_string())),
            ),
            assign(
                Place::Field(LocalId(2), "v".to_string()),
                Rvalue::Use(Operand::Const(AirConst::Int(101, aelys_air::AirIntSize::I64))),
            ),
            assign(
                Place::Local(LocalId(3)),
                Rvalue::FieldAccess {
                    base: Operand::Copy(LocalId(0)),
                    field: "i".to_string(),
                },
            ),
            assign(
                Place::Local(LocalId(4)),
                Rvalue::FieldAccess {
                    base: Operand::Copy(LocalId(3)),
                    field: "v".to_string(),
                },
            ),
        ],
        Operand::Copy(LocalId(4)),
    );
    program.structs = vec![
        aelys_air::AirStructDef {
            name: "In".to_string(),
            type_params: vec![],
            fields: vec![aelys_air::AirStructField {
                name: "v".to_string(),
                ty: AirType::I64,
                offset: None,
            }],
            is_closure_env: false,
            span: None,
        },
        aelys_air::AirStructDef {
            name: "Out".to_string(),
            type_params: vec![],
            fields: vec![aelys_air::AirStructField {
                name: "i".to_string(),
                ty: inner.clone(),
                offset: None,
            }],
            is_closure_env: false,
            span: None,
        },
    ];

    let Some(results) = run_air_program("air_chain", &program) else {
        return;
    };
    for (level, code) in results {
        assert_eq!(
            code, 101,
            "a chained AddressOf(Place::Field(..)) must write the owner at {level}"
        );
    }
}

