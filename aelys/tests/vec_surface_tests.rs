// the guaranteed vec value-semantics surface, rejected with e0412.

use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use tempfile::tempdir;

fn lower(src: &str) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn reject_e0412(src: &str) {
    match lower(src) {
        Ok(()) => panic!("expected E0412, but the program compiled"),
        Err(err) => assert!(
            err.contains("[E0412]") && err.contains("[vec-surface]"),
            "expected an E0412 vec-surface rejection, got: {err}"
        ),
    }
}

fn accepts(src: &str) {
    lower(src).unwrap_or_else(|err| panic!("this program must compile: {err}"));
}

#[test]
fn reject_form_grouping_initializer() {
    reject_e0412(
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let w = (v)
    return 0
}",
    );
}

#[test]
fn accept_form_identifier_initializer() {
    accepts(
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let w = v
    return 0
}",
    );
}

#[test]
fn reject_form_if_initializer() {
    reject_e0412(
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let c = 1
    let w = if c == 1 { v } else { v }
    return 0
}",
    );
}

#[test]
fn reject_form_block_initializer() {
    reject_e0412(
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let z = 1
    let w = { let q = z; v }
    return 0
}",
    );
}

#[test]
fn accept_form_call_initializer() {
    accepts(
        "fn mk(a: vec<i64>) -> vec<i64> { return a }
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let w = mk(v)
    return 0
}",
    );
}

#[test]
fn accept_form_vec_new_and_literal_initializers() {
    accepts(
        "fn main() -> i64 {
    let v: vec<i64> = Vec::new()
    let w: vec<i64> = vec[1, 2]
    return 0
}",
    );
}

// conditional a uaf, so the conditional is rejected and the identifier is accepted.
#[test]
fn reject_form_conditional_return() {
    reject_e0412(
        "fn pick(a: vec<i64>, c: i64) -> vec<i64> { return if c == 1 { a } else { a } }
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let w = pick(v, 1)
    return 0
}",
    );
}

#[test]
fn accept_form_identifier_return() {
    accepts(
        "fn pick(a: vec<i64>, c: i64) -> vec<i64> { return a }
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let w = pick(v, 1)
    return 0
}",
    );
}

#[test]
fn reject_form_grouping_deref_assign() {
    reject_e0412(
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let mut u = Vec::new()
    Vec::push(u, 2)
    let r = &mut u
    *r = (v)
    return 0
}",
    );
}

#[test]
fn accept_form_identifier_deref_assign() {
    accepts(
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let mut u = Vec::new()
    Vec::push(u, 2)
    let r = &mut u
    *r = v
    return 0
}",
    );
}

#[test]
fn reject_shape_nested_vec_push() {
    reject_e0412(
        "fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let mut vv = Vec::new()
    Vec::push(vv, inner)
    return 0
}",
    );
}

#[test]
fn accept_shape_primitive_push() {
    accepts(
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}",
    );
}

#[test]
fn reject_shape_vec_literal_of_vec() {
    reject_e0412(
        "fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let v = vec[inner]
    return 0
}",
    );
}

#[test]
fn accept_shape_vec_literal_of_primitive() {
    accepts(
        "fn main() -> i64 {
    let v = vec[1, 2, 3]
    return 0
}",
    );
}

#[test]
fn reject_shape_array_literal_of_vec() {
    reject_e0412(
        "fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let arr = [inner]
    return 0
}",
    );
}

#[test]
fn accept_shape_array_literal_of_primitive() {
    accepts(
        "fn main() -> i64 {
    let arr = [1, 2, 3]
    return 0
}",
    );
}

#[test]
fn reject_shape_sized_array_of_vec() {
    reject_e0412(
        "fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let arr = [inner; 2]
    return 0
}",
    );
}

#[test]
fn accept_shape_sized_array_of_primitive() {
    accepts(
        "fn main() -> i64 {
    let arr = [0; 2]
    return arr[0]
}",
    );
}

#[test]
fn reject_shape_generic_fn_with_vec_arg() {
    reject_e0412(
        "fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return keep(v) }",
    );
}

#[test]
fn accept_shape_generic_fn_with_primitive_arg() {
    accepts("fn keep<T>(x: T) -> i64 { let y = x; return 0 }\nfn main() -> i64 { return keep(7) }");
}

#[test]
fn reject_shape_generic_enum_with_vec_payload() {
    reject_e0412(
        "enum Opt<T> { Some(T), Nil }
fn main() -> i64 {
    let mut i1 = Vec::new()
    Vec::push(i1, 1)
    let a = Opt::Some(i1)
    return 0
}",
    );
}

#[test]
fn accept_shape_generic_enum_with_primitive_payload() {
    accepts(
        "enum Opt<T> { Some(T), Nil }
fn main() -> i64 {
    let a = Opt::Some(4)
    return 0
}",
    );
}

