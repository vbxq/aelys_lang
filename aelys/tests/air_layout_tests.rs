use aelys_air::{
    AirEnumDef, AirEnumVariant, AirProgram, AirStructDef, AirStructField, AirType, CallingConv,
    layout::{compute_layouts, layout_of},
};

fn field(name: &str, ty: AirType) -> AirStructField {
    AirStructField {
        name: name.to_string(),
        ty,
        offset: None,
    }
}

fn sdef(name: &str, fields: Vec<AirStructField>) -> AirStructDef {
    AirStructDef {
        name: name.to_string(),
        type_params: vec![],
        fields,
        is_closure_env: false,
        span: None,
    }
}

fn program(structs: Vec<AirStructDef>) -> AirProgram {
    AirProgram {
        functions: vec![],
        structs,
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
    }
}

fn enum_def(name: &str, variants: Vec<AirEnumVariant>) -> AirEnumDef {
    AirEnumDef {
        name: name.to_string(),
        type_params: vec![],
        variants,
        span: None,
    }
}

fn variant(name: &str, tag: u32, payload: Vec<AirType>) -> AirEnumVariant {
    AirEnumVariant {
        name: name.to_string(),
        tag,
        payload,
    }
}

fn program_with_enums(structs: Vec<AirStructDef>, enums: Vec<AirEnumDef>) -> AirProgram {
    AirProgram {
        functions: vec![],
        structs,
        enums,
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
    }
}

#[test]
fn primitives() {
    assert_eq!(layout_of(&AirType::I8).size, 1);
    assert_eq!(layout_of(&AirType::I16).align, 2);
    assert_eq!(layout_of(&AirType::I32).size, 4);
    assert_eq!(layout_of(&AirType::F64).size, 8);
    assert_eq!(layout_of(&AirType::Bool).size, 1);
    assert_eq!(layout_of(&AirType::Str).size, 16);
    assert_eq!(layout_of(&AirType::Void).size, 0);
    assert_eq!(layout_of(&AirType::Void).align, 1);
    assert_eq!(layout_of(&AirType::Slice(Box::new(AirType::I32))).size, 16);
    assert_eq!(layout_of(&AirType::Slice(Box::new(AirType::I32))).align, 8);
    assert_eq!(layout_of(&AirType::Ptr(Box::new(AirType::I8))).size, 8);
    let fnptr = AirType::FnPtr {
        params: vec![AirType::I32],
        ret: Box::new(AirType::Void),
        conv: CallingConv::Aelys,
    };
    // Aelys closure values are fat pointers { fn_ptr, env_ptr } = 16 bytes.
    assert_eq!(layout_of(&fnptr).size, 16);
}

#[test]
fn array_layout() {
    let arr = AirType::Array(Box::new(AirType::I32), 10);
    let l = layout_of(&arr);
    assert_eq!(l.size, 40);
    assert_eq!(l.align, 4);
}

// struct Padded { a: i8, b: i32 }
// a@0(1) + 3 padding + b@4(4) = size 8, align 4
#[test]
fn padding_i8_i32() {
    let mut prog = program(vec![sdef(
        "Padded",
        vec![field("a", AirType::I8), field("b", AirType::I32)],
    )]);
    compute_layouts(&mut prog);
    assert_eq!(prog.structs[0].fields[0].offset, Some(0));
    assert_eq!(prog.structs[0].fields[1].offset, Some(4));
}

// struct Wide { x: i64, y: i8 }
// x@0(8) + y@8(1) + 7 trailing padding = size 16, align 8
#[test]
fn trailing_padding() {
    let mut prog = program(vec![sdef(
        "Wide",
        vec![field("x", AirType::I64), field("y", AirType::I8)],
    )]);
    compute_layouts(&mut prog);
    assert_eq!(prog.structs[0].fields[0].offset, Some(0));
    assert_eq!(prog.structs[0].fields[1].offset, Some(8));
}

// struct Mixed { a: i8, b: i16, c: i32, d: i64 }
// a@0, pad 1, b@2, c@4, d@8 → size 16, align 8
#[test]
fn mixed_alignment() {
    let mut prog = program(vec![sdef(
        "Mixed",
        vec![
            field("a", AirType::I8),
            field("b", AirType::I16),
            field("c", AirType::I32),
            field("d", AirType::I64),
        ],
    )]);
    compute_layouts(&mut prog);
    let s = &prog.structs[0];
    assert_eq!(s.fields[0].offset, Some(0));
    assert_eq!(s.fields[1].offset, Some(2));
    assert_eq!(s.fields[2].offset, Some(4));
    assert_eq!(s.fields[3].offset, Some(8));
}

// Inner { x: i32, y: i32 } → size 8, align 4
// Outer { tag: i8, inner: Inner } → tag@0, pad 3, inner@4
#[test]
fn nested_struct() {
    let mut prog = program(vec![
        sdef(
            "Inner",
            vec![field("x", AirType::I32), field("y", AirType::I32)],
        ),
        sdef(
            "Outer",
            vec![
                field("tag", AirType::I8),
                field("inner", AirType::Struct("Inner".into())),
            ],
        ),
    ]);
    compute_layouts(&mut prog);

    assert_eq!(prog.structs[0].fields[0].offset, Some(0));
    assert_eq!(prog.structs[0].fields[1].offset, Some(4));

    assert_eq!(prog.structs[1].fields[0].offset, Some(0));
    assert_eq!(prog.structs[1].fields[1].offset, Some(4));
}

// Outer declared before Inner → topo sort resolves order
#[test]
fn reverse_declaration_order() {
    let mut prog = program(vec![
        sdef(
            "Outer",
            vec![
                field("tag", AirType::I8),
                field("inner", AirType::Struct("Inner".into())),
            ],
        ),
        sdef(
            "Inner",
            vec![field("x", AirType::I32), field("y", AirType::I32)],
        ),
    ]);
    compute_layouts(&mut prog);

    assert_eq!(prog.structs[0].fields[0].offset, Some(0));
    assert_eq!(prog.structs[0].fields[1].offset, Some(4));
    assert_eq!(prog.structs[1].fields[0].offset, Some(0));
    assert_eq!(prog.structs[1].fields[1].offset, Some(4));
}

// Ptr(Self) is valid (linked list)
#[test]
fn self_ptr_is_valid() {
    let mut prog = program(vec![sdef(
        "Node",
        vec![
            field("value", AirType::I64),
            field(
                "next",
                AirType::Ptr(Box::new(AirType::Struct("Node".into()))),
            ),
        ],
    )]);
    compute_layouts(&mut prog);
    assert_eq!(prog.structs[0].fields[0].offset, Some(0));
    assert_eq!(prog.structs[0].fields[1].offset, Some(8));
}

#[test]
fn empty_struct() {
    let mut prog = program(vec![sdef("Empty", vec![])]);
    compute_layouts(&mut prog);
}

// Vec2 { f32, f32 } = size 8, align 4
// Mesh { id: i32, vertices: [Vec2; 4] } → id@0, vertices@4(32)
#[test]
fn array_of_struct_field() {
    let mut prog = program(vec![
        sdef(
            "Vec2",
            vec![field("x", AirType::F32), field("y", AirType::F32)],
        ),
        sdef(
            "Mesh",
            vec![
                field("id", AirType::I32),
                field(
                    "vertices",
                    AirType::Array(Box::new(AirType::Struct("Vec2".into())), 4),
                ),
            ],
        ),
    ]);
    compute_layouts(&mut prog);
    assert_eq!(prog.structs[1].fields[0].offset, Some(0));
    assert_eq!(prog.structs[1].fields[1].offset, Some(4));
}

#[test]
fn enum_struct_dependency_chain_resolves() {
    let mut prog = program_with_enums(
        vec![
            sdef("Payload", vec![field("x", AirType::I64)]),
            sdef(
                "Wrapper",
                vec![
                    field("e", AirType::Enum("Message".into())),
                    field("flag", AirType::I8),
                ],
            ),
        ],
        vec![enum_def(
            "Message",
            vec![
                variant("None", 0, vec![]),
                variant("Some", 1, vec![AirType::Struct("Payload".into())]),
            ],
        )],
    );
    compute_layouts(&mut prog);
    assert_eq!(prog.structs[0].fields[0].offset, Some(0));
    assert_eq!(prog.structs[1].fields[0].offset, Some(0));
    assert_eq!(prog.structs[1].fields[1].offset, Some(16));
}

// Closure env with mixed captures: i64@0, bool@8, str@16
#[test]
fn closure_env() {
    let mut prog = program(vec![AirStructDef {
        name: "__closure_env_foo".to_string(),
        type_params: vec![],
        fields: vec![
            field("captured_x", AirType::I64),
            field("captured_flag", AirType::Bool),
            field("captured_name", AirType::Str),
        ],
        is_closure_env: true,
        span: None,
    }]);
    compute_layouts(&mut prog);
    assert_eq!(prog.structs[0].fields[0].offset, Some(0));
    assert_eq!(prog.structs[0].fields[1].offset, Some(8));
    assert_eq!(prog.structs[0].fields[2].offset, Some(16));
}

#[test]
fn self_reference_reports_error() {
    let mut prog = program(vec![sdef(
        "Bad",
        vec![field("inner", AirType::Struct("Bad".into()))],
    )]);
    let errors = compute_layouts(&mut prog);
    assert!(!errors.is_empty(), "expected error for self-referencing struct");
    assert!(errors[0].contains("infinite size"));
}

#[test]
fn mutual_cycle_reports_error() {
    let mut prog = program(vec![
        sdef("A", vec![field("b", AirType::Struct("B".into()))]),
        sdef("B", vec![field("a", AirType::Struct("A".into()))]),
    ]);
    let errors = compute_layouts(&mut prog);
    assert!(!errors.is_empty(), "expected error for mutual cycle");
}

#[test]
fn struct_enum_cycle_reports_error() {
    let mut prog = program_with_enums(
        vec![sdef(
            "Node",
            vec![field("next", AirType::Enum("OptionNode".into()))],
        )],
        vec![enum_def(
            "OptionNode",
            vec![
                variant("Some", 0, vec![AirType::Struct("Node".into())]),
                variant("None", 1, vec![]),
            ],
        )],
    );
    let errors = compute_layouts(&mut prog);
    assert!(!errors.is_empty(), "expected error for struct-enum cycle");
}
