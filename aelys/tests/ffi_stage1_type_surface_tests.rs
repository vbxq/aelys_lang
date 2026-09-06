use aelys_opt::OptimizationLevel;
use aelys_sema::{InferType, TypeVarId};
use aelys_air::symbols::{ForeignTypePosition, foreign_type_rejection};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const MAIN: &str = "\n\nfn main() -> i64 {\n    return 0\n}\n";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn lower(source: &str) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write row");
    match aelys_driver::lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

fn admitted(id: &str, ty: &str) {
    let source = format!("unsafe extern fn probe(x: {ty}) -> {ty}{MAIN}");
    if let Err(rendered) = lower(&source) {
        panic!("{id}: `{ty}` MUST be inside the external type surface, rejected with:\n{rendered}");
    }
}

fn rejected(id: &str, decl: &str) -> String {
    let source = format!("{decl}{MAIN}");
    match lower(&source) {
        Ok(()) => panic!("{id}: `{decl}` MUST be rejected by the external type surface"),
        Err(rendered) => rendered,
    }
}

fn rejected_e0615(id: &str, decl: &str) -> String {
    let rendered = rejected(id, decl);
    assert!(
        rendered.contains("E0615"),
        "{id}: the verdict MUST be E0615, found:\n{rendered}"
    );
    rendered
}

fn twin_is_accepted(id: &str, decl: &str) {
    let source = format!("{decl}{MAIN}");
    if let Err(rendered) = lower(&source) {
        panic!("{id}: the `i64` twin MUST be accepted, rejected with:\n{rendered}");
    }
}

const PARAM_ROWS: [(&str, &str, &str); 9] = [
    (
        "F2-1",
        "unsafe extern fn probe(x: string)",
        "unsafe extern fn probe(x: i64)",
    ),
    (
        "F2-2",
        "unsafe extern fn probe(x: void)",
        "unsafe extern fn probe(x: i64)",
    ),
    (
        "F2-3",
        "unsafe extern fn probe(x: Rc<i64>)",
        "unsafe extern fn probe(x: i64)",
    ),
    (
        "F2-4",
        "unsafe extern fn probe(x: fn(i64) -> i64)",
        "unsafe extern fn probe(x: i64)",
    ),
    (
        "F2-5",
        "unsafe extern fn probe(x: [i64; 3])",
        "unsafe extern fn probe(x: i64)",
    ),
    (
        "F2-6",
        "unsafe extern fn probe(x: vec<i64>)",
        "unsafe extern fn probe(x: i64)",
    ),
    (
        "F2-7",
        "unsafe extern fn probe(x: &[i64])",
        "unsafe extern fn probe(x: i64)",
    ),
    (
        "F2-8",
        "struct S {\n    a: i64,\n}\n\nunsafe extern fn probe(x: S)",
        "struct S {\n    a: i64,\n}\n\nunsafe extern fn probe(x: i64)",
    ),
    (
        "F2-9",
        "enum E {\n    A,\n}\n\nunsafe extern fn probe(x: E)",
        "enum E {\n    A,\n}\n\nunsafe extern fn probe(x: i64)",
    ),
];

#[test]
fn the_nine_rejected_variants_are_e0615_and_each_has_an_accepted_twin() {
    for (id, rejected_decl, twin) in PARAM_ROWS {
        rejected_e0615(id, rejected_decl);
        twin_is_accepted(id, twin);
    }
}

#[test]
fn the_eleven_scalars_pass_in_both_positions_and_a_borrow_only_as_a_parameter() {
    for ty in [
        "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool",
    ] {
        admitted("F2-10", ty);
    }
    let source = format!("unsafe extern fn probe(x: &i64) -> i64{MAIN}");
    if let Err(rendered) = lower(&source) {
        panic!(
            "F2-10: `&i64` MUST be inside the surface as a parameter, rejected with:\n{rendered}"
        );
    }
    rejected_e0615("F2-10", "unsafe extern fn probe(x: &i64) -> &i64");
}

// an `rc<t>` and a `&t` both lower to `*t`, so only a verdict taken on the typed ast can tell them apart
#[test]
fn a_reference_counted_pointer_is_rejected_where_a_borrow_is_not() {
    rejected_e0615("F2-3", "unsafe extern fn probe(x: Rc<i64>)");
    twin_is_accepted("F2-3", "unsafe extern fn probe(x: &i64)");
}

#[test]
fn the_str_alias_is_rejected_exactly_like_string() {
    rejected_e0615("F2-11", "unsafe extern fn probe(x: str)");
}

#[test]
fn void_is_rejected_as_a_parameter_and_admitted_as_a_return() {
    rejected_e0615("F2-2", "unsafe extern fn probe(x: void)");
    let source = format!("unsafe extern fn probe(x: i64) -> void{MAIN}");
    if let Err(rendered) = lower(&source) {
        panic!("F2-12: `void` MUST be admitted as a return type, rejected with:\n{rendered}");
    }
}

// the accepted spellings are open, so a table of names would be walked around by one capital letter
#[test]
fn an_odd_capitalisation_of_string_is_e0615_and_not_an_unknown_type_name() {
    let rendered = rejected_e0615("F2-13", "unsafe extern fn probe(x: sTRING)");
    assert!(
        !rendered.contains("E0301"),
        "F2-13: `sTRING` reaches `InferType::String`, so the unknown-name verdict must not fire:\n{rendered}"
    );
}

#[test]
fn a_capitalised_type_name_is_e0615_and_not_the_downstream_vec_surface() {
    let rendered = rejected_e0615("F2-14", "unsafe extern fn probe(x: String)");
    assert!(
        !rendered.contains("E0412"),
        "F2-14: `String` becomes a struct type, and E0615 must precede the vec surface:\n{rendered}"
    );
}

#[test]
fn an_unannotated_parameter_is_e0615_and_not_the_downstream_vec_surface() {
    let rendered = rejected_e0615("F2-15", "unsafe extern fn probe(x)");
    assert!(
        !rendered.contains("E0412"),
        "F2-15: the parameter annotation is optional in the grammar, so the surface has to hold \
         the unwritten type itself:\n{rendered}"
    );
    assert!(
        rendered.contains("'dynamic'"),
        "F2-15: an unwritten parameter annotation reaches the surface as `dynamic`:\n{rendered}"
    );
    let rendered = rejected_e0615("F2-16", "unsafe extern fn probe(x) -> i64");
    assert!(
        !rendered.contains("E0412"),
        "F2-16: an annotated return does not rescue an unwritten parameter:\n{rendered}"
    );
}

#[test]
fn the_bare_form_and_a_nogc_claim_stay_outside_the_surface_verdict() {
    for decl in [
        "unsafe extern fn probe()",
        "unsafe extern nogc fn probe(x: i64) -> i64",
    ] {
        let source = format!("{decl}{MAIN}");
        if let Err(rendered) = lower(&source) {
            panic!("F2-17: `{decl}` MUST stay accepted, rejected with:\n{rendered}");
        }
    }
}

#[test]
fn an_ordinary_aelys_body_is_untouched_by_the_surface() {
    let source = format!("fn probe(x: string) -> string {{\n    return x\n}}{MAIN}");
    if let Err(rendered) = lower(&source) {
        panic!("F2-18: the surface only binds a foreign declaration, rejected with:\n{rendered}");
    }
}

#[test]
fn the_variants_no_annotation_can_reach_are_rejected_like_every_unlisted_one() {
    for (name, ty) in [
        ("range", InferType::Range),
        ("dynamic", InferType::Dynamic),
        ("var", InferType::Var(TypeVarId(0))),
    ] {
        assert!(
            foreign_type_rejection(&ty, ForeignTypePosition::Parameter).is_some(),
            "F2-19: `{name}` is not admitted by name, so the default verdict MUST be a rejection"
        );
    }
}

fn one_representative_per_variant() -> Vec<(&'static str, InferType)> {
    vec![
        ("i8", InferType::I8),
        ("i16", InferType::I16),
        ("i32", InferType::I32),
        ("i64", InferType::I64),
        ("u8", InferType::U8),
        ("u16", InferType::U16),
        ("u32", InferType::U32),
        ("u64", InferType::U64),
        ("f32", InferType::F32),
        ("f64", InferType::F64),
        ("bool", InferType::Bool),
        ("&i64", InferType::Ref {
            referent: Box::new(InferType::I64),
            mutable: false,
        }),
        ("void", InferType::Null),
        ("string", InferType::String),
        ("never", InferType::Never),
        ("fn", InferType::Function {
            params: vec![InferType::I64],
            ret: Box::new(InferType::I64),
            nogc: false,
        }),
        ("array", InferType::Array(Box::new(InferType::I64), Some(3))),
        ("vec", InferType::Vec(Box::new(InferType::I64))),
        ("rc", InferType::Rc(Box::new(InferType::I64))),
        ("slice", InferType::Slice {
            elem: Box::new(InferType::I64),
            mutable: false,
        }),
        ("tuple", InferType::Tuple(vec![InferType::I64])),
        ("range", InferType::Range),
        ("struct", InferType::Struct("S".to_string())),
        ("enum", InferType::Enum("E".to_string(), Vec::new())),
        ("var", InferType::Var(TypeVarId(0))),
        ("dynamic", InferType::Dynamic),
    ]
}

fn admitted_variants(position: ForeignTypePosition) -> Vec<&'static str> {
    one_representative_per_variant()
        .into_iter()
        .filter(|(_, ty)| foreign_type_rejection(ty, position).is_none())
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn twelve_variants_are_admitted_as_a_parameter_and_twelve_as_a_return() {
    assert_eq!(
        one_representative_per_variant().len(),
        26,
        "F2-20: one representative per variant, so a new variant owes a new row here"
    );
    assert_eq!(
        admitted_variants(ForeignTypePosition::Parameter),
        vec![
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool", "&i64",
        ],
        "F2-20: the admitted parameter set is closed and enumerated"
    );
    assert_eq!(
        admitted_variants(ForeignTypePosition::Return),
        vec![
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool", "void",
        ],
        "F2-20: `void` joins the set in the return position and `&i64` leaves it"
    );
}

const BORROW_ROWS: [(&str, &str); 5] = [
    ("F2-21", "unsafe extern fn probe(x: &string)"),
    ("F2-22", "unsafe extern fn probe(x: &Rc<i64>)"),
    ("F2-23", "unsafe extern fn probe(x: &vec<i64>)"),
    (
        "F2-24",
        "struct S {\n    a: i64,\n}\n\nunsafe extern fn probe(x: &S)",
    ),
    ("F2-25", "unsafe extern fn probe(x: &fn(i64) -> i64)"),
];

// a borrow of a rejected type reaches c as the address of a managed header, and reads arbitrary bytes
#[test]
fn a_borrow_of_a_rejected_type_is_rejected_and_the_i64_borrows_are_not() {
    for (id, decl) in [
        ("F2-26", "unsafe extern fn probe(x: &i64)"),
        ("F2-27", "unsafe extern fn probe(x: &mut i64)"),
    ] {
        twin_is_accepted(id, decl);
    }
    rejected_e0615("F2-29", "unsafe extern fn probe(x: i64) -> &i64");
    // every row is walked before the verdict, so an ablation names all five and not just the first
    let mut admitted = Vec::new();
    for (id, decl) in BORROW_ROWS {
        let source = format!("{decl}{MAIN}");
        match lower(&source) {
            Ok(()) => admitted.push(format!("{id}: `{decl}`")),
            Err(rendered) => assert!(
                rendered.contains("E0615"),
                "{id}: the verdict MUST be E0615, found:\n{rendered}"
            ),
        }
    }
    assert!(
        admitted.is_empty(),
        "a borrow of a rejected type MUST stay outside the surface, admitted:\n{}",
        admitted.join("\n")
    );
    rejected_e0615("F2-28", "unsafe extern fn probe(x: &void)");
    rejected_e0615("F2-7", "unsafe extern fn probe(x: &[i64])");
}

#[test]
fn the_return_position_admits_what_the_parameter_position_refuses() {
    assert!(
        foreign_type_rejection(&InferType::Null, ForeignTypePosition::Parameter).is_some(),
        "`void` in a parameter promises an opaque pointer and delivers only `null`"
    );
    assert_eq!(
        foreign_type_rejection(&InferType::Null, ForeignTypePosition::Return),
        None,
        "`void` in the return position is c's `void`, exactly"
    );
}

#[test]
fn the_infer_type_enumeration_still_counts_twenty_six_variants() {
    let text = fs::read_to_string(repo_root().join("sema/src/types/infer_type.rs"))
        .expect("read the type enumeration");
    let body = text
        .split_once("pub enum InferType {")
        .expect("the enumeration is named")
        .1
        .split_once("\n}")
        .expect("the enumeration closes")
        .0;
    let variants = body
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            line.starts_with("    ")
                && !line.starts_with("     ")
                && trimmed
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                && trimmed
                    .trim_end()
                    .ends_with([',', '{', '('])
        })
        .count();
    assert_eq!(
        variants, 26,
        "the surface is a verdict per variant, so a new variant is a new decision"
    );
}
