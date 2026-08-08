// silently rerouted to the builtin `::` lowering) -> e0419. item 4 rejects an indirect

use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use tempfile::tempdir;

const REJECT_LEVELS: &[OptimizationLevel] = &[OptimizationLevel::None, OptimizationLevel::Standard];

fn lower_err(src: &str, level: OptimizationLevel) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    lower_file_to_air(&source_path, level)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn assert_rejects_with(src: &str, code: &str) {
    for level in REJECT_LEVELS {
        match lower_err(src, *level) {
            Ok(()) => panic!("expected reject with {code} at {level:?}, but it compiled"),
            Err(err) => assert!(
                err.contains(&format!("[{code}]")),
                "at {level:?} the reject must carry {code}: {err}"
            ),
        }
    }
}

fn assert_compiles(src: &str) {
    if let Err(err) = lower_err(src, OptimizationLevel::None) {
        panic!("expected the twin to compile, got: {err}");
    }
}

const USER_ENUM_VEC: &str = r#"
enum Vec { Empty, One(i64) }
fn main() -> i64 { return 0 }
"#;

const USER_STRUCT_RC: &str = r#"
struct Rc { count: i64 }
fn main() -> i64 { return 0 }
"#;

const USER_STRUCT_VEC: &str = r#"
struct Vec { len: i64 }
fn main() -> i64 { return 0 }
"#;

const USER_ENUM_RC: &str = r#"
enum Rc { A, B }
fn main() -> i64 { return 0 }
"#;

#[test]
fn user_enum_named_vec_rejects_e0419() {
    assert_rejects_with(USER_ENUM_VEC, "E0419");
}

#[test]
fn user_struct_named_rc_rejects_e0419() {
    assert_rejects_with(USER_STRUCT_RC, "E0419");
}

#[test]
fn user_struct_named_vec_rejects_e0419() {
    assert_rejects_with(USER_STRUCT_VEC, "E0419");
}

#[test]
fn user_enum_named_rc_rejects_e0419() {
    assert_rejects_with(USER_ENUM_RC, "E0419");
}

const RENAMED_TYPES: &str = r#"
enum MyList { Empty, One(i64) }
struct RefCount { count: i64 }
fn main() -> i64 { return 0 }
"#;

#[test]
fn twin_renamed_user_types_compile() {
    assert_compiles(RENAMED_TYPES);
}

const BUILTIN_VEC_RC_USAGE: &str = r#"
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 10)
    let r: Rc<i64> = Rc::new(42)
    return v[0] + Rc::get(r)
}
"#;

#[test]
fn twin_builtin_vec_rc_usage_compiles() {
    assert_compiles(BUILTIN_VEC_RC_USAGE);
}

const INDIRECT_IF: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = if 1 == 1 { b } else { b }
    return 0
}
"#;

const INDIRECT_GROUPING: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = (b)
    return 0
}
"#;

#[test]
fn indirect_if_rc_field_assign_rejects_e0420() {
    assert_rejects_with(INDIRECT_IF, "E0420");
}

#[test]
fn indirect_grouping_rc_field_assign_rejects_e0420() {
    assert_rejects_with(INDIRECT_GROUPING, "E0420");
}

const DIRECT_IDENT: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    return 0
}
"#;

const DIRECT_RC_NEW: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    a.next = Rc::new(Node { val: 2, next: Rc::null() })
    return 0
}
"#;

const DIRECT_MEMBER: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b.next
    return 0
}
"#;

#[test]
fn twin_direct_ident_rc_field_assign_compiles() {
    assert_compiles(DIRECT_IDENT);
}

#[test]
fn twin_direct_rc_new_field_assign_compiles() {
    assert_compiles(DIRECT_RC_NEW);
}

#[test]
fn twin_direct_member_rc_field_assign_compiles() {
    assert_compiles(DIRECT_MEMBER);
}

