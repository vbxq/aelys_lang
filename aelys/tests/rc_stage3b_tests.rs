use aelys_air::layout::compute_layouts;
use aelys_air::rc_paths::{RcLeafPath, RcPathStep};
use aelys_air::rc_types::{collect_rc_types, compute_offset_for_path};
use aelys_air::{AirEnumDef, AirEnumVariant, AirProgram, AirStructDef, AirStructField, AirType};
use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn rc_ir(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect("llvm backend compilation should succeed");
    fs::read_to_string(source_path.with_extension("ll")).expect("ir file")
}

fn table_words(ir: &str) -> Vec<u32> {
    let line = ir
        .lines()
        .find(|l| l.contains("@__aelys_rc_type_table"))
        .unwrap_or_else(|| panic!("no __aelys_rc_type_table in IR:\n{ir}"));
    let ty_open = line.find('[').expect("table type bracket");
    let ty_close = line[ty_open..].find(']').expect("table type close") + ty_open;
    let k: usize = line[ty_open + 1..ty_close]
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("bad array length in {line:?}"));

    let after_ty = &line[ty_close + 1..];
    if after_ty.contains("zeroinitializer") {
        return vec![0; k];
    }
    let open = line.rfind('[').expect("table initializer bracket");
    let close = line.rfind(']').expect("table initializer close");
    let words: Vec<u32> = line[open + 1..close]
        .split(',')
        .map(|tok| {
            tok.trim()
                .trim_start_matches("i32")
                .trim()
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("bad table word {tok:?} in {line:?}"))
        })
        .collect();
    assert_eq!(words.len(), k, "table word count mismatch in {line:?}");
    words
}

fn decode_table(words: &[u32]) -> Vec<(u32, Vec<u32>)> {
    let n = words[0] as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let count = words[1 + 2 * i];
        let off_idx = words[1 + 2 * i + 1] as usize;
        let offsets = words[off_idx..off_idx + count as usize].to_vec();
        out.push((count, offsets));
    }
    out
}

fn struct_def(name: &str, fields: &[(&str, AirType)]) -> AirStructDef {
    AirStructDef {
        name: name.to_string(),
        type_params: vec![],
        fields: fields
            .iter()
            .map(|(n, ty)| AirStructField {
                name: n.to_string(),
                ty: ty.clone(),
                offset: None,
            })
            .collect(),
        is_closure_env: false,
        span: None,
    }
}

fn rc_i64() -> AirType {
    AirType::Ptr(Box::new(AirType::I64))
}

fn program_with(structs: Vec<AirStructDef>, enums: Vec<AirEnumDef>) -> AirProgram {
    let mut p = AirProgram {
        functions: vec![],
        structs,
        enums,
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: Default::default(),
    };
    let errs = compute_layouts(&mut p);
    assert!(errs.is_empty(), "layout errors: {errs:?}");
    p
}

#[test]
fn unit_struct_offsets_match_field_offsets() {
    let p = program_with(
        vec![struct_def("Pair", &[("a", rc_i64()), ("b", rc_i64())])],
        vec![],
    );
    let off_a = compute_offset_for_path(
        &AirType::Struct("Pair".into()),
        &RcLeafPath {
            steps: vec![RcPathStep::Field("a".into())],
        },
        &p,
    )
    .expect("offset a");
    let off_b = compute_offset_for_path(
        &AirType::Struct("Pair".into()),
        &RcLeafPath {
            steps: vec![RcPathStep::Field("b".into())],
        },
        &p,
    )
    .expect("offset b");
    assert_eq!((off_a, off_b), (0, 8), "Pair leaves must be at 0 and 8");
}

#[test]
fn unit_enum_payload_head_is_four_not_eight() {
    let holder = AirEnumDef {
        name: "Holder".into(),
        type_params: vec![],
        variants: vec![AirEnumVariant {
            name: "Cell".into(),
            tag: 0,
            payload: vec![rc_i64()],
        }],
        span: None,
    };
    let p = program_with(vec![], vec![holder]);
    let off = compute_offset_for_path(
        &AirType::Enum("Holder".into()),
        &RcLeafPath {
            steps: vec![RcPathStep::EnumPayload {
                enum_name: "Holder".into(),
                tag: 0,
                field_index: 0,
            }],
        },
        &p,
    )
    .expect("enum offset");
    assert_eq!(
        off, 4,
        "enum payload Rc leaf MUST be at offset 4, not 8 (R-K4)"
    );
}

#[test]
fn unit_mixed_carrier_composes_struct_then_enum() {
    let holder = AirEnumDef {
        name: "Holder".into(),
        type_params: vec![],
        variants: vec![AirEnumVariant {
            name: "Cell".into(),
            tag: 0,
            payload: vec![rc_i64()],
        }],
        span: None,
    };
    let wrap = struct_def(
        "Wrap",
        &[("tag", AirType::I64), ("h", AirType::Enum("Holder".into()))],
    );
    let p = program_with(vec![wrap], vec![holder]);
    let off = compute_offset_for_path(
        &AirType::Struct("Wrap".into()),
        &RcLeafPath {
            steps: vec![
                RcPathStep::Field("h".into()),
                RcPathStep::EnumPayload {
                    enum_name: "Holder".into(),
                    tag: 0,
                    field_index: 0,
                },
            ],
        },
        &p,
    )
    .expect("mixed offset");
    assert_eq!(off, 12, "composed offset must be off(.h)=8 + enum_head=4");
}

#[test]
fn unit_missing_field_offset_is_a_hard_error() {
    let mut p = AirProgram {
        functions: vec![],
        structs: vec![struct_def("Pair", &[("a", rc_i64())])],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: Default::default(),
    };
    p.struct_sizes.clear();
    let err = compute_offset_for_path(
        &AirType::Struct("Pair".into()),
        &RcLeafPath {
            steps: vec![RcPathStep::Field("a".into())],
        },
        &p,
    );
    assert!(
        err.is_err(),
        "missing field offset must be a hard error, got {err:?}"
    );
}

const PAIR_CARRIER: &str = r#"
struct Pair { a: Rc<i64>, b: Rc<i64> }
fn use_pair(p: Rc<Pair>) -> i64 { return 0 }
fn mk() -> i64 {
    let x: Rc<i64> = Rc::new(1)
    let y: Rc<i64> = Rc::new(2)
    let p: Rc<Pair> = Rc::new(Pair { a: x, b: y })
    return use_pair(p)
}
fn main() -> i64 { return mk() }
"#;

#[test]
fn b1_carrier_writes_distinct_nonzero_type_id() {
    let ir = rc_ir(PAIR_CARRIER);
    assert!(
        ir.contains("store i32 2, ptr %rc_hdr_ptr"),
        "the Pair carrier must stamp the distinct 1-based type_id 2 @8; got:\n{ir}"
    );
    let words = table_words(&ir);
    assert_eq!(
        words[0], 3,
        "expected reserved + 2 real reference types (n_entries=3); got {words:?}"
    );
}

#[test]
fn b4_scalar_count_zero_and_value_type_absent() {
    let ir = rc_ir(
        r#"
struct Point { x: i64, y: i64 }
fn use_point(p: Point) -> i64 { return p.x }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(7)
    let pt: Point = Point { x: 1, y: 2 }
    return Rc::get(r) + use_point(pt)
}
"#,
    );
    let entries = decode_table(&table_words(&ir));
    assert_eq!(
        entries.len(),
        2,
        "reserved sentinel + Rc<i64> should be in the table; got {entries:?}"
    );
    assert_eq!(entries[0], (0, vec![]), "reserved sentinel slot (count 0)");
    assert_eq!(
        entries[1],
        (0, vec![]),
        "Rc<i64> must be count 0 (no Ptr leaf)"
    );
}

#[test]
fn b4_pure_value_program_has_empty_table_and_no_rc_calls() {
    let ir = rc_ir(
        r#"
struct Point { x: i64, y: i64 }
fn use_point(p: Point) -> i64 { return p.x + p.y }
fn main() -> i64 {
    let pt: Point = Point { x: 1, y: 2 }
    return use_point(pt)
}
"#,
    );
    let words = table_words(&ir);
    assert_eq!(
        words,
        vec![1, 0, 3],
        "a no-Rc program emits only the reserved sentinel entry \
         [n_entries=1, {{count:0, offset_idx:3}}]"
    );
    assert!(
        !ir.contains("__aelys_rc_retain") && !ir.contains("__aelys_rc_release"),
        "a pure value program must emit no rc_* calls; got:\n{ir}"
    );
}

#[test]
fn b5_table_is_deterministic_across_builds() {
    let a = table_words(&rc_ir(PAIR_CARRIER));
    let b = table_words(&rc_ir(PAIR_CARRIER));
    assert_eq!(a, b, "the table blob must be deterministic across builds");
    let entries = decode_table(&a);
    assert!(
        entries.contains(&(2, vec![0, 8])),
        "Pair entry must be (2, [0,8]); table {a:?} decoded {entries:?}"
    );
}

#[test]
fn b2_table_symbol_is_external_in_object() {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, PAIR_CARRIER).expect("write source");

    let _ = compile_file_with_llvm(&source_path, OptimizationLevel::None, false);
    let object = source_path.with_extension(if cfg!(windows) { "obj" } else { "o" });
    if !object.is_file() {
        eprintln!("object not produced (linker unavailable?); skipping nm assertion");
        return;
    }
    let Ok(out) = Command::new("nm").arg(&object).output() else {
        eprintln!("nm unavailable; skipping symbol assertion");
        return;
    };
    let syms = String::from_utf8_lossy(&out.stdout);
    let defined_external = syms.lines().any(|l| {
        l.contains("__aelys_rc_type_table") && l.split_whitespace().any(|c| c == "R" || c == "D")
    });
    assert!(
        defined_external,
        "__aelys_rc_type_table must be a defined external symbol; nm:\n{syms}"
    );
}

const ENUM_CARRIER: &str = r#"
enum Holder { Cell(Rc<i64>) }
fn use_h(h: Rc<Holder>) -> i64 { return 0 }
fn mk() -> i64 {
    let x: Rc<i64> = Rc::new(7)
    let h: Rc<Holder> = Rc::new(Holder::Cell(x))
    return use_h(h)
}
fn main() -> i64 { return mk() }
"#;

const RUNTIME_PROBE_C: &str = r#"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern const uint32_t __aelys_rc_type_table[];

typedef struct { int32_t tag; uint8_t payload[8]; } HolderData;

int main(void) {
    const uint32_t *t = __aelys_rc_type_table;
    uint32_t n_types = t[0];
    if (n_types < 3) { fprintf(stderr, "FAIL n_types=%u\n", n_types); return 1; }
    uint32_t type_id = 2; /* Holder is the 2nd real type (i64 is 1); 0 is the reserved sentinel. */
    uint32_t count = t[1 + 2 * type_id + 0];
    uint32_t offset_idx = t[1 + 2 * type_id + 1];
    if (count != 1) { fprintf(stderr, "FAIL count=%u\n", count); return 1; }
    uint32_t offset = t[offset_idx];
    if (offset != 4) { fprintf(stderr, "FAIL offset=%u (must be 4)\n", offset); return 1; }

    int64_t child_value = 0x1234567890ABCDEFLL;
    void *child_ptr = &child_value;
    HolderData *data = calloc(1, sizeof(HolderData));
    data->tag = 0;
    memcpy((uint8_t *)data + 4, &child_ptr, sizeof(void *));

    void *recovered = *(void **)((uint8_t *)data + offset);
    if (recovered != child_ptr) { fprintf(stderr, "FAIL deref mismatch\n"); free(data); return 1; }
    if (*(int64_t *)recovered != child_value) { fprintf(stderr, "FAIL value mismatch\n"); free(data); return 1; }
    free(data);
    printf("PASS offset=%u dereferenced to the real Rc<i64> field\n", offset);
    return 0;
}
"#;

#[test]
fn b3_enum_offset_four_dereferences_to_the_real_field() {
    let enum_ir = rc_ir(ENUM_CARRIER);
    let enum_entries = decode_table(&table_words(&enum_ir));
    assert!(
        enum_entries.contains(&(1, vec![4])),
        "enum carrier must record a single leaf at offset 4 (R-K4); got {enum_entries:?}"
    );
    let mixed_ir = rc_ir(
        r#"
enum Holder { Cell(Rc<i64>) }
struct Wrap { tag: i64, h: Holder }
fn use_w(w: Rc<Wrap>) -> i64 { return 0 }
fn mk() -> i64 {
    let x: Rc<i64> = Rc::new(7)
    let w: Rc<Wrap> = Rc::new(Wrap { tag: 99, h: Holder::Cell(x) })
    return use_w(w)
}
fn main() -> i64 { return mk() }
"#,
    );
    let mixed_entries = decode_table(&table_words(&mixed_ir));
    assert!(
        mixed_entries.contains(&(1, vec![12])),
        "mixed carrier must record off(.h)=8 + enum_head=4 = 12; got {mixed_entries:?}"
    );

    let dir = tempdir().expect("tempdir");
    let table_ll = dir.path().join("table_only.ll");
    let table_line = enum_ir
        .lines()
        .find(|l| l.contains("@__aelys_rc_type_table"))
        .expect("table line in IR");
    fs::write(&table_ll, format!("{table_line}\n")).expect("write table .ll");

    let probe_c = dir.path().join("probe.c");
    fs::write(&probe_c, RUNTIME_PROBE_C).expect("write probe.c");

    let table_o = dir.path().join("table_only.o");
    let cc = Command::new("clang")
        .arg("-O0")
        .arg("-c")
        .arg(&table_ll)
        .arg("-o")
        .arg(&table_o)
        .output();
    match cc {
        Ok(out) if out.status.success() => {}
        Ok(_) | Err(_) => {
            eprintln!(
                "clang unavailable / table compile failed; skipping C exec proof (IR offset=4 already asserted)"
            );
            return;
        }
    }

    let bin = dir.path().join("probe_bin");
    let link = Command::new("clang")
        .arg("-O0")
        .arg(&probe_c)
        .arg(&table_o)
        .arg("-o")
        .arg(&bin)
        .output();
    match link {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!(
                "C probe link failed; skipping exec proof:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            return;
        }
        Err(_) => {
            eprintln!("clang unavailable for link; skipping exec proof");
            return;
        }
    }

    let run = Command::new(&bin).output().expect("run C probe");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        run.status.success(),
        "B3 mini-runtime must prove offset=4 dereferences the real field;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS offset=4"),
        "expected the C probe to confirm offset=4; stdout:\n{stdout}"
    );
}

#[test]
fn b_extra_collect_dedups_and_orders_by_appearance() {
    use aelys_air::{
        AirBlock, AirFunction, AirStmt, AirStmtKind, BlockId, CallingConv, FunctionAttribs,
        FunctionId, GcMode, InlineHint, LocalId,
    };
    let rc_alloc = |id: u32, ty: AirType| AirStmt {
        kind: AirStmtKind::RcAlloc {
            local: LocalId(id),
            ty,
        },
        span: None,
    };
    let mut p = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "f".into(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Void,
            locals: vec![],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![
                    rc_alloc(0, AirType::I64),
                    rc_alloc(1, AirType::I64),
                    rc_alloc(2, AirType::Struct("Pair".into())),
                ],
                terminator: aelys_air::AirTerminator::Return(None),
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
        structs: vec![struct_def("Pair", &[("a", rc_i64()), ("b", rc_i64())])],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: Default::default(),
    };
    let errs = compute_layouts(&mut p);
    assert!(errs.is_empty(), "layout: {errs:?}");
    let table = collect_rc_types(&p).expect("collect");
    assert_eq!(table.entries.len(), 2, "i64 (dedup) + Pair");
    assert_eq!(table.entries[0].type_id, 1);
    assert_eq!(table.entries[0].pointer_offsets, Vec::<u32>::new());
    assert_eq!(table.entries[1].type_id, 2);
    assert_eq!(table.entries[1].pointer_offsets, vec![0, 8]);
    assert_eq!(table.lookup_id(&AirType::I64), Some(1));
    assert_eq!(table.lookup_id(&AirType::Struct("Pair".into())), Some(2));
    assert_eq!(table.lookup_id(&AirType::F64), None);
}

const NODE_CARRIER: &str = r#"
struct Node { next: Rc<Node> }
fn use_node(n: Rc<Node>) -> i64 { return 0 }
fn mk(leaf: Rc<Node>) -> i64 {
    let n: Rc<Node> = Rc::new(Node { next: leaf })
    return use_node(n)
}
fn main() -> i64 { return 0 }
"#;

#[test]
fn opta_no_real_type_is_zero() {
    use aelys_air::{
        AirBlock, AirFunction, AirStmt, AirStmtKind, BlockId, CallingConv, FunctionAttribs,
        FunctionId, GcMode, InlineHint, LocalId,
    };
    let rc_alloc = |id: u32, ty: AirType| AirStmt {
        kind: AirStmtKind::RcAlloc {
            local: LocalId(id),
            ty,
        },
        span: None,
    };
    let mut p = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "f".into(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Void,
            locals: vec![],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![
                    rc_alloc(0, AirType::I64),
                    rc_alloc(1, AirType::Struct("Pair".into())),
                ],
                terminator: aelys_air::AirTerminator::Return(None),
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
        structs: vec![struct_def("Pair", &[("a", rc_i64()), ("b", rc_i64())])],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: Default::default(),
    };
    let errs = compute_layouts(&mut p);
    assert!(errs.is_empty(), "layout: {errs:?}");
    let table = collect_rc_types(&p).expect("collect");
    assert!(!table.entries.is_empty(), "the program has >=1 RcAlloc");
    assert!(
        table.entries.iter().all(|e| e.type_id != 0),
        "no real reference type may be assigned the reserved sentinel id 0; got {:?}",
        table.entries.iter().map(|e| e.type_id).collect::<Vec<_>>()
    );
    assert_eq!(
        table.entries[0].type_id, 1,
        "first real type must be 1-based (id 1), not the reserved sentinel 0"
    );
}

#[test]
fn opta_reserved_index0_is_count_zero() {
    let w = table_words(&rc_ir(NODE_CARRIER));
    assert_eq!(
        w[1 + 2 * 0],
        0,
        "table-index 0 (the type_id-0 sentinel) must have count 0 so a stray \
         type_id==0 object traces zero children, safe even without NO_TRACE; got {w:?}"
    );
    assert_eq!(
        w[0], 2,
        "n_entries = 1 real (Node) + 1 reserved sentinel; got {w:?}"
    );
    assert!(
        w[1 + 2 * 1] > 0,
        "first real type (Node) must have a non-zero pointer-leaf count at index 1; got {w:?}"
    );
}
