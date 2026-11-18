#[path = "level_legs/mod.rs"]
mod level_legs;

use level_legs::{LEVELS, LevelLeg, legs_of, level_disagreements};
use tempfile::tempdir;

enum Expect {
    Refused {
        code: &'static str,
        at: (u32, u32),
        says: &'static str,
        alone: bool,
    },
    Runs(&'static str),
    Balanced(&'static str),
}

struct Case {
    name: &'static str,
    src: &'static str,
    module: Option<(&'static str, &'static str)>,
    expect: Expect,
}

fn rc_balanced(stderr: &str) -> bool {
    let Some(line) = stderr.lines().find(|l| l.starts_with("[rc] allocs=")) else {
        return false;
    };
    let mut counts = line
        .split_whitespace()
        .filter_map(|w| w.split_once('=').map(|(_, n)| n.to_string()));
    matches!((counts.next(), counts.next()), (Some(a), Some(f)) if a == f)
}

fn check(row: &str, cases: &[Case]) {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    for case in cases {
        let env: &[(&str, &str)] = match case.expect {
            Expect::Balanced(_) => &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "malloc")],
            _ => &[],
        };
        let legs = legs_of(dir.path(), case.name, case.src, case.module, env);
        assert_eq!(legs.len(), LEVELS.len(), "{row}: legs of {}", case.name);
        if let Expect::Balanced(out) = case.expect {
            for (tag, leg) in &legs {
                let met = matches!(leg, LevelLeg::Ran { stdout, stderr, exit }
                    if stdout == out && *exit == 0 && rc_balanced(stderr));
                if !met {
                    failures.push(format!("{} at {tag}: {}", case.name, leg.render()));
                }
            }
            continue;
        }
        failures.extend(level_disagreements(case.name, &legs));
        let leg = &legs[0].1;
        let met = match (&case.expect, leg) {
            (
                Expect::Refused {
                    code,
                    at,
                    says,
                    alone,
                },
                LevelLeg::Refused(rendered),
            ) => {
                (!alone || rendered.matches("error[").count() == 1)
                    && rendered.contains(&format!("error[{code}]"))
                    && rendered.contains(&format!("root.aelys:{}:{}", at.0, at.1))
                    && rendered.contains(says)
                    && !rendered.contains("E0901")
            }
            (
                Expect::Runs(out),
                LevelLeg::Ran {
                    stdout,
                    stderr,
                    exit,
                },
            ) => stdout == out && stderr.is_empty() && *exit == 0,
            _ => false,
        };
        if !met {
            failures.push(format!("{} at -O0: {}", case.name, leg.render()));
        }
    }
    assert!(failures.is_empty(), "{row}:\n{}", failures.join("\n"));
}

#[test]
fn release01_62_an_inference_variable_left_after_solving_asks_for_an_annotation() {
    check(
        "release01_62",
        &[
            Case {
                name: "rc_null_let",
                src: "fn main() {\n    let x = Rc::null();\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 5),
                    says: "type annotations needed: the type of `x` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "rc_null_eq",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    if Rc::null() == Rc::null() { return ExitCode::from(1); }\n    return ExitCode::SUCCESS;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 8),
                    says: "type annotations needed: the type of this expression is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "rc_null_stmt",
                src: "fn main() {\n    Rc::null();\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 5),
                    says: "type annotations needed: the type of this expression is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "rc_is_null_of_null",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    if Rc::is_null(Rc::null()) { return ExitCode::from(1); }\n    return ExitCode::SUCCESS;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 20),
                    says: "type annotations needed",
                    alone: false,
                },
            },
            Case {
                name: "rc_new_of_null",
                src: "fn main() {\n    let r = Rc::new(Rc::null());\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 5),
                    says: "type annotations needed: the type of `r` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "empty_vec_literal",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    let v = vec![];\n    return ExitCode::from(v.len() as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 5),
                    says: "type annotations needed: the type of `v` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "empty_vec_read_in_dead_branch",
                src: "fn main() { let v = vec![];\n    if false { println!(\"{}\", v.len()); }\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 20),
                    says: "type annotations needed: the type of `v` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "lambda_parameter_never_used",
                src: "fn main() { let g = move |x| { 0 };\n    println!(\"ok\");\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 31),
                    says: "type annotations needed: the type of the parameter `x` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "generic_vec_return_never_fixed",
                src: "fn mk<T>() -> Vec<T> { let v: Vec<T> = Vec::new();\n    v }\nfn main() { let v = mk();\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (3, 20),
                    says: "type annotations needed: the type of `v` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "lambda_returning_rc_null",
                src: "fn main() { let g = move || { Rc::null() };\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 20),
                    says: "type annotations needed: the type of `g` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "function_parameter_never_used",
                src: "fn f(x) -> i64 { 0 }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 6),
                    says: "type annotations needed: the type of the parameter `x` is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "unused_vec_new_in_nogc",
                src: "use std::process::ExitCode; nogc fn f() -> i64 {\n    let mut v = Vec::new();\n    0\n}\nfn main() -> ExitCode { return ExitCode::from(f() as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 5),
                    says: "type annotations needed: the type of `v` is never inferred",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_63_a_type_resolved_after_its_first_use_is_still_accepted() {
    check(
        "release01_63",
        &[
            Case {
                name: "rc_null_assigned_later",
                src: "struct S { n: i64 }\nfn main() { let mut r = Rc::null();\n    r = Rc::new(S { n: 4 });\n    println!(\"{}\", r.n);\n }\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Case {
                name: "rc_null_compared_to_typed",
                src: "use std::process::ExitCode; struct S { n: i64 }\nfn main() -> ExitCode { let a: Rc<S> = Rc::new(S { n: 4 });\n    if a == Rc::null() { return ExitCode::from(1); }\n    println!(\"ok\");\n    return ExitCode::SUCCESS; }\n",
                module: None,
                expect: Expect::Runs("ok\n"),
            },
            Case {
                name: "lambda_parameter_fixed_by_call",
                src: "fn main() { let g = move |x| { x + 1 };\n    println!(\"{}\", g(2));\n }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "lambda_parameter_fixed_by_unused_call",
                src: "fn main() { let g = move |x| { 7 };\n    println!(\"{}\", g(1));\n }\n",
                module: None,
                expect: Expect::Runs("7\n"),
            },
            Case {
                name: "empty_vec_assigned_later",
                src: "fn main() { let mut v = vec![];\n    v = vec![1, 2, 3];\n    println!(\"{}\", v.len());\n }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "rc_null_returned",
                src: "struct S { n: i64 }\nfn f(c: bool) -> Rc<S> { let mut r = Rc::null();\n    if c { r = Rc::new(S { n: 4 }); }\n    r }\nfn main() { let x = f(true);\n    println!(\"{}\", x.n);\n }\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Case {
                name: "rc_null_in_struct_literal",
                src: "struct S { n: i64 }\nstruct H { r: Rc<S> }\nfn main() { let h = H { r: Rc::null() };\n    if Rc::is_null(h.r) { println!(\"null\"); }\n }\n",
                module: None,
                expect: Expect::Runs("null\n"),
            },
            Case {
                name: "lambda_through_generic",
                src: "fn ap<T>(f: fn(T) -> T, x: T) -> T { f(x) }\nfn main() { println!(\"{}\", ap(move |y| { return y }, 5));\n }\n",
                module: None,
                expect: Expect::Runs("5\n"),
            },
            Case {
                name: "vec_new_passed",
                src: "fn total(v: Vec<i64>) -> i64 { v.len() }\nfn main() { let v = Vec::new();\n    println!(\"{}\", total(v));\n }\n",
                module: None,
                expect: Expect::Runs("0\n"),
            },
            Case {
                name: "rc_null_passed",
                src: "struct S { n: i64 }\nfn g(r: Rc<S>) -> i64 { 1 }\nfn main() { let r = Rc::null();\n    println!(\"{}\", g(r));\n }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Case {
                name: "match_statement_every_arm_returns",
                src: "enum Op { Inc, Dec }\nfn apply(op: Op, x: i64) -> i64 {\n    match op {\n        Op::Inc => return x + 1,\n        Op::Dec => return x - 1,\n    }\n    x\n}\nfn main() {\n    println!(\"{}\", apply(Op::Inc, 41));\n}\n",
                module: None,
                expect: Expect::Runs("42\n"),
            },
            Case {
                name: "nested_match_statement_returns",
                src: "enum A { X, Y }\nfn f(a: A, b: A) -> i64 {\n    match a {\n        A::X => {\n            match b {\n                A::X => return 1,\n                A::Y => return 2,\n            }\n            return 0;\n        }\n        A::Y => return 9,\n    }\n    -1\n}\nfn main() {\n    println!(\"{}\", f(A::X, A::Y));\n}\n",
                module: None,
                expect: Expect::Runs("2\n"),
            },
        ],
    );
}

#[test]
fn release01_64_a_projection_that_never_resolves_asks_for_an_annotation() {
    check(
        "release01_64",
        &[
            Case {
                name: "field_of_unconstrained_parameter",
                src: "fn main() {\n    let f = move |p| -> i64 { p.a };\n    println!(\"ok\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 35),
                    says: "type annotations needed: the type of this value is never inferred, so its field `a` cannot be resolved",
                    alone: false,
                },
            },
            Case {
                name: "slice_of_unconstrained_parameter",
                src: "fn main() {\n    let s = move |t| -> i64 { let x = t[0..1];\n 0 };\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 36),
                    says: "type annotations needed: the type of this value is never inferred, so it cannot be sliced",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_65_a_generic_function_used_as_a_value_is_refused_by_name() {
    check(
        "release01_65",
        &[
            Case {
                name: "generic_fn_in_annotated_let",
                src: "use std::process::ExitCode; fn id<T>(x: T) -> T { x }\nfn main() -> ExitCode {\n    let f: fn(i64) -> i64 = id;\n    return ExitCode::from(f(3) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 29),
                    says: "a generic function cannot be used as a value yet; wrap it in a closure or a non-generic function",
                    alone: false,
                },
            },
            Case {
                name: "generic_fn_in_global",
                src: "use std::process::ExitCode; fn id<T>(x: T) -> T { x }\nstatic G: fn(i64) -> i64 = id;\nfn main() -> ExitCode { return ExitCode::from(G(2) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 25),
                    says: "a generic function cannot be used as a value yet",
                    alone: false,
                },
            },
            Case {
                name: "generic_fn_typed_by_vec",
                src: "use std::process::ExitCode; fn sink<U>(u: U) -> i64 { 0 }\nfn main() -> ExitCode { let v = vec![1, 2]; let f: fn(vec<i64>) -> i64 = sink; return ExitCode::from(f(v) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 68),
                    says: "a generic function cannot be used as a value yet",
                    alone: false,
                },
            },
            Case {
                name: "local_closure_shadowing_a_generic_name",
                src: "fn id<T>(x: T) -> T { x }\nfn main() { let id = move |x: i64| -> i64 { x + 1 };\n    let f: fn(i64) -> i64 = id;\n    println!(\"{}\", f(3));\n }\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
        ],
    );
}

#[test]
fn release01_66_a_slice_of_a_generic_parameter_is_refused_like_its_index() {
    check(
        "release01_66",
        &[
            Case {
                name: "slice_of_t",
                src: "use std::process::ExitCode; fn firstof<T>(x: T) -> i64 {\n    let s = x[0..1];\n    5\n}\nfn main() -> ExitCode {\n    let a: [i64; 3] = [1, 2, 3];\n    return ExitCode::from(firstof(a) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 13),
                    says: "a value of the generic type parameter `T` cannot be sliced",
                    alone: false,
                },
            },
            Case {
                name: "index_of_t",
                src: "use std::process::ExitCode; fn firstof<T>(x: T) -> i64 {\n    let s = x[0];\n    5\n}\nfn main() -> ExitCode {\n    let a: [i64; 3] = [1, 2, 3];\n    return ExitCode::from(firstof(a) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 13),
                    says: "a value of the generic type parameter `T` cannot be indexed",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_67_an_enum_type_with_the_wrong_number_of_type_arguments_is_refused() {
    check(
        "release01_67",
        &[
            Case {
                name: "bare_generic_enum_parameter",
                src: "enum Opt<T> { Some(T), Nil }\nfn f(o: Opt) -> i64 { 0 }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 9),
                    says: "type annotations needed: the enum `Opt` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "bare_generic_enum_let",
                src: "use std::process::ExitCode; enum Opt<T> { Some(T), Nil }\nfn main() -> ExitCode { let o: Opt = Opt::Some(5);\n match o { Opt::Some(v) => { return ExitCode::from(v as u8); } Opt::Nil => { return ExitCode::SUCCESS; } } }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 27),
                    says: "the enum `Opt` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "bare_generic_enum_field",
                src: "enum Opt<T> { Some(T), Nil }\nstruct H { o: Opt }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 15),
                    says: "the enum `Opt` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "bare_generic_enum_return",
                src: "enum Opt<T> { Some(T), Nil }\nfn f() -> Opt { Opt::Some(5) }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 11),
                    says: "the enum `Opt` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "too_many_type_arguments",
                src: "enum Opt<T> { Some(T), Nil }\nfn f(o: Opt<i64, i64>) -> i64 { 0 }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 9),
                    says: "the enum `Opt` takes 1 type argument but 2 were given",
                    alone: false,
                },
            },
            Case {
                name: "type_arguments_on_a_plain_enum",
                src: "enum E { A, B }\nfn f(o: E<i64>) -> i64 { 0 }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 9),
                    says: "the enum `E` takes 0 type arguments but 1 was given",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_68_an_rc_of_the_unit_type_is_refused_by_name() {
    check(
        "release01_68",
        &[
            Case {
                name: "rc_new_of_unit_call",
                src: "fn u() { }\nfn main() {\n    let r = Rc::new(u());\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 13),
                    says: "an `Rc` of the unit type is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "rc_of_unit_through_a_generic",
                src: "fn u() { }\nfn mk<T>(x: T) -> Rc<T> { Rc::new(x) }\nfn main() {\n    let r = mk(u());\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (4, 13),
                    says: "an `Rc` of the unit type is not supported yet",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_69_a_member_read_through_a_borrowed_rc_is_refused_in_a_nogc_fn_too() {
    check(
        "release01_69",
        &[
            Case {
                name: "nogc_member_through_deref",
                src: "struct S { n: i64 }\nnogc fn f(x: &Rc<S>) -> i64 { (*x).n }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0410",
                    at: (2, 39),
                    says: "`*` here reads an `Rc<S>` out of a reference: reading an `Rc` through a reference is not supported yet, pass the `Rc` itself",
                    alone: false,
                },
            },
            Case {
                name: "plain_member_through_deref",
                src: "struct S { n: i64 }\nfn f(x: &Rc<S>) -> i64 { (*x).n }\nfn main() {\n    let p: Rc<S> = Rc::new(S { n: 4 });\n    println!(\"{}\", f(&p));\n}\n",
                module: None,
                expect: Expect::Balanced("4\n"),
            },
        ],
    );
}

#[test]
fn release01_70_an_err_built_for_a_never_error_type_is_a_type_mismatch() {
    check(
        "release01_70",
        &[
            Case {
                name: "err_of_never_result",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { Result::Err(1) }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (2, 32),
                    says: "a `Result<T, Never>` can only be built with `Result::Ok`",
                    alone: false,
                },
            },
            Case {
                name: "ok_of_never_result",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { Result::Ok(7) }\nfn main() {\n    let v = f().into_ok();\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("7\n"),
            },
        ],
    );
}

#[test]
fn release01_71_a_backend_only_refusal_is_named_in_sema_at_the_expression() {
    check(
        "release01_71",
        &[
            Case {
                name: "field_assign_through_rc_of_scalar",
                src: "fn main() {\n    let r: Rc<i64> = Rc::new(5);\n    r.k = 3;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (3, 5),
                    says: "field assignment through `Rc<i64>` requires a struct payload",
                    alone: false,
                },
            },
            Case {
                name: "struct_in_format",
                src: "struct S { a: i64 }\nfn main() {\n    let s = S { a: 1 };\n    println!(\"{s}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0732",
                    at: (4, 13),
                    says: "`S` does not implement `Display`; write `impl Display for S` to print it",
                    alone: false,
                },
            },
            Case {
                name: "struct_printed_directly",
                src: "struct S { a: i64 }\nfn main() {\n    let s = S { a: 1 };\n    println!(\"{}\", s);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0732",
                    at: (4, 13),
                    says: "`S` does not implement `Display`; write `impl Display for S` to print it",
                    alone: false,
                },
            },
            Case {
                name: "enum_in_format",
                src: "enum E { A, B(i64) }\nfn main() {\n    let e = E::B(3);\n    println!(\"{e}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (4, 13),
                    says: "formatting an enum inside a string is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "array_in_format",
                src: "fn main() {\n    let a: [i64; 2] = [1, 2];\n    println!(\"{a}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 13),
                    says: "printing an array is not supported yet; print its elements",
                    alone: false,
                },
            },
            Case {
                name: "vec_in_format",
                src: "fn main() {\n    let v: Vec<i64> = vec![1, 2];\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 13),
                    says: "printing a `Vec` is not supported yet; print its elements",
                    alone: false,
                },
            },
            Case {
                name: "closure_in_format",
                src: "fn main() {\n    let k: i64 = 2;\n    let f = move |x: i64| -> i64 { x + k };\n    println!(\"{f}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (4, 13),
                    says: "printing a function or closure is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "rc_in_format",
                src: "fn main() {\n    let r: Rc<i64> = Rc::new(3);\n    println!(\"{r}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 13),
                    says: "printing an `Rc` is not supported yet; print the value it points to",
                    alone: false,
                },
            },
            Case {
                name: "reference_in_format",
                src: "fn main() {\n    let x: i64 = 3;\n    let r = &x;\n    println!(\"{r}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (4, 13),
                    says: "printing a reference is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "slice_in_format",
                src: "fn main() {\n    let a: [i64; 2] = [1, 2];\n    let s = a[0..1];\n    println!(\"{s}\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (4, 13),
                    says: "printing a slice is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "unit_printed_directly",
                src: "fn u() { }\nfn main() {\n    println!(\"{}\", u());\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 13),
                    says: "printing the unit value is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "unit_compared",
                src: "use std::process::ExitCode; fn f() { }\nfn main() -> ExitCode {\n    if f() == f() { return ExitCode::from(1); }\n    return ExitCode::SUCCESS;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (3, 8),
                    says: "comparison (`==`) is not supported for the unit value",
                    alone: false,
                },
            },
            Case {
                name: "enum_printed_directly",
                src: "enum E { A, B(i64) }\nfn main() {\n    let e = E::B(3);\n    println!(\"{}\", e);\n}\n",
                module: None,
                expect: Expect::Runs("E::B\n"),
            },
        ],
    );
}

#[test]
fn release01_72_an_imported_generic_function_used_as_a_value_is_refused_by_name() {
    check(
        "release01_72",
        &[
            Case {
                name: "imported_by_name",
                src: "mod lib; use lib::id;\nfn main() { let f = id;\n }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { x }\npub fn show<T>(x: T) { println!(\"{}\", x); }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 28),
                    says: "a generic function cannot be used as a value yet",
                    alone: true,
                },
            },
            Case {
                name: "imported_by_name_called",
                src: "use std::process::ExitCode; mod lib; use lib::id;\nfn main() -> ExitCode { let f = id;\n    return ExitCode::from(f(3) as u8); }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { x }\npub fn show<T>(x: T) { println!(\"{}\", x); }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 28),
                    says: "a generic function cannot be used as a value yet",
                    alone: true,
                },
            },
            Case {
                name: "imported_through_namespace",
                src: "mod lib;\nfn main() { let f = lib::id;\n }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { x }\npub fn show<T>(x: T) { println!(\"{}\", x); }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 28),
                    says: "a generic function cannot be used as a value yet",
                    alone: true,
                },
            },
            Case {
                name: "namespace_value_after_a_call",
                src: "mod lib;\nfn main() { lib::show(3);\n    let p = lib::id;\n }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { x }\npub fn show<T>(x: T) { println!(\"{}\", x); }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 13),
                    says: "a generic function cannot be used as a value yet",
                    alone: true,
                },
            },
            Case {
                name: "unannotated_local_value",
                src: "fn id<T>(x: T) -> T { x }\nfn main() {\n    let f = id;\n    println!(\"{}\", f(3));\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 13),
                    says: "a generic function cannot be used as a value yet",
                    alone: true,
                },
            },
            Case {
                name: "returned_as_a_value",
                src: "use std::process::ExitCode; fn id<T>(x: T) -> T { x }\nfn get() -> fn(i64) -> i64 { id }\nfn main() -> ExitCode { return ExitCode::from(get()(2) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 37),
                    says: "a generic function cannot be used as a value yet",
                    alone: true,
                },
            },
            Case {
                name: "passed_where_a_closure_is_expected",
                src: "use std::process::ExitCode; fn id<T>(x: T) -> T { x }\nfn ap(f: fn(i64) -> i64, x: i64) -> i64 { f(x) }\nfn main() -> ExitCode { return ExitCode::from(ap(id, 3) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (3, 30),
                    says: "a generic function cannot be used as a value yet",
                    alone: true,
                },
            },
        ],
    );
}

#[test]
fn release01_73_print_used_as_a_value_is_refused_by_name() {
    check(
        "release01_73",
        &[
            Case {
                name: "println_bound_to_a_let",
                src: "fn main() { let p = println;\n    p(\"x\");\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (1, 28),
                    says: "`println` cannot be used as a value yet; call it directly",
                    alone: false,
                },
            },
            Case {
                name: "println_passed_as_an_argument",
                src: "fn ap(f: fn(String), s: String) { f(s); }\nfn main() { ap(println, \"hi\");\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 23),
                    says: "`println` cannot be used as a value yet; call it directly",
                    alone: false,
                },
            },
            Case {
                name: "print_bound_and_unused",
                src: "fn main() { let p = print;\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (1, 28),
                    says: "`print` cannot be used as a value yet; call it directly",
                    alone: false,
                },
            },
            Case {
                name: "println_in_a_struct_field",
                src: "struct H { f: fn(String) }\nfn main() { let h = H { f: println };\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (2, 35),
                    says: "`println` cannot be used as a value yet",
                    alone: false,
                },
            },
            Case {
                name: "println_returned",
                src: "fn g() -> fn(String) { println }\nfn main() { g()(\"x\");\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (1, 31),
                    says: "`println` cannot be used as a value yet",
                    alone: false,
                },
            },
            Case {
                name: "println_called_through_a_grouping",
                src: "fn main() { (println)(\"x\");\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (1, 21),
                    says: "`println` cannot be used as a value yet",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_74_the_err_arm_of_a_never_result_is_unreachable_and_compiles() {
    check(
        "release01_74",
        &[
            Case {
                name: "question_into_a_never_result",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { Result::Ok(1) }\nfn g() -> Result<i64, Never> { let x = f()?;\n    Result::Ok(x + 1) }\nfn main() { match g() { Result::Ok(x) => { println!(\"{x}\"); } Result::Err(_) => { println!(\"err\"); } }\n }\n",
                module: None,
                expect: Expect::Runs("2\n"),
            },
            Case {
                name: "question_into_an_i64_result",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { Result::Ok(1) }\nfn g() -> Result<i64, i64> { let x = f()?;\n    Result::Ok(x) }\nfn main() { match g() { Result::Ok(x) => { println!(\"{x}\"); } Result::Err(_) => { println!(\"err\"); } }\n }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Case {
                name: "err_arm_binding_never_used_as_i64",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { Result::Ok(1) }\nfn main() { let v = match f() { Result::Ok(x) => x, Result::Err(e) => e };\n    println!(\"{v}\");\n }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Case {
                name: "question_in_a_generic_instantiated_with_never",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f<E>(r: Result<i64, E>) -> Result<i64, E> { let x = r?;\n    Result::Ok(x) }\nfn main() { let r: Result<i64, Never> = Result::Ok(1);\n    match f(r) { Result::Ok(x) => { println!(\"{x}\"); } Result::Err(_) => { println!(\"err\"); } }\n }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
        ],
    );
}

#[test]
fn release01_75_a_builtin_or_struct_type_with_the_wrong_number_of_type_arguments_is_refused() {
    check(
        "release01_75",
        &[
            Case {
                name: "bare_vec_let",
                src: "fn main() { let v: Vec = Vec::new();\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 27),
                    says: "the type `Vec` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "bare_rc_let",
                src: "struct S { n: i64 }\nfn main() { let r: Rc = Rc::new(S { n: 1 });\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 27),
                    says: "the type `Rc` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "vec_with_two_arguments",
                src: "fn main() { let v: Vec<i64, i64> = Vec::new();\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 27),
                    says: "the type `Vec` takes 1 type argument but 2 were given",
                    alone: false,
                },
            },
            Case {
                name: "plain_struct_with_an_argument",
                src: "use std::process::ExitCode; struct S { n: i64 }\nfn main() -> ExitCode { let s: S<i64> = S { n: 1 };\n    return ExitCode::from(s.n as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 27),
                    says: "the struct `S` takes 0 type arguments but 1 was given",
                    alone: false,
                },
            },
            Case {
                name: "bare_rc_parameter",
                src: "use std::process::ExitCode; struct S { n: i64 }\nfn f(r: Rc) -> i64 { 0 }\nfn main() -> ExitCode { return ExitCode::from(f(Rc::new(S { n: 1 })) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 9),
                    says: "the type `Rc` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "bare_rc_field",
                src: "struct S { n: i64 }\nstruct H { r: Rc }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 15),
                    says: "the type `Rc` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "bare_vec_return",
                src: "fn f() -> Vec { let v: Vec<i64> = Vec::new();\n    v }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 11),
                    says: "the type `Vec` takes 1 type argument but 0 were given",
                    alone: false,
                },
            },
            Case {
                name: "rc_with_two_arguments",
                src: "struct S { n: i64 }\nfn main() { let r: Rc<S, S> = Rc::new(S { n: 1 });\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 27),
                    says: "the type `Rc` takes 1 type argument but 2 were given",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_76_printing_through_a_type_parameter_is_decided_at_the_instance() {
    check(
        "release01_76",
        &[
            Case {
                name: "string_formatted_by_a_generic",
                src: "fn show<T>(x: T) { println!(\"v={x}\"); }\nfn main() { show(\"s\");\n }\n",
                module: None,
                expect: Expect::Runs("v=s\n"),
            },
            Case {
                name: "string_formatted_through_two_generics",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { pass(\"ab\");\n    pass(3);\n }\n",
                module: None,
                expect: Expect::Runs("[ab]\n[3]\n"),
            },
            Case {
                name: "enum_printed_directly_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { shown(E::B(2));\n }\n",
                module: None,
                expect: Expect::Runs("E::B\n"),
            },
            Case {
                name: "struct_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { show(S { n: 1 });\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (7, 20),
                    says: "printing a struct is not supported yet; print its fields",
                    alone: false,
                },
            },
            Case {
                name: "struct_printed_directly_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { shown(S { n: 1 });\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (7, 20),
                    says: "printing a struct is not supported yet; print its fields",
                    alone: false,
                },
            },
            Case {
                name: "struct_formatted_through_two_generics",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { pass(S { n: 1 });\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (7, 20),
                    says: "printing a struct is not supported yet; print its fields",
                    alone: false,
                },
            },
            Case {
                name: "enum_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { show(E::A);\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (7, 20),
                    says: "formatting an enum inside a string is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "array_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { show([1, 2]);\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (7, 20),
                    says: "printing an array is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "rc_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { show(Rc::new(S { n: 1 }));\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (7, 20),
                    says: "printing an `Rc` is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "reference_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { let y: i64 = 3;\n    show(&y);\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (8, 5),
                    says: "printing a reference is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "unit_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn main() { show(u());\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (7, 20),
                    says: "printing the unit value is not supported yet",
                    alone: false,
                },
            },
            Case {
                name: "function_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println!(\"[{x}]\"); }\nfn shown<T>(x: T) { println!(\"{}\", x); }\nfn pass<U>(x: U) { show(x); }\nfn g1() -> i64 { 1 }\nfn main() { show(g1);\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (8, 20),
                    says: "printing a function or closure is not supported yet",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_77_an_enum_prints_under_its_source_name() {
    check(
        "release01_77",
        &[
            Case {
                name: "imported_generic_enum",
                src: "mod lib; use lib::W;\nfn main() { let w: W<i64> = W::A(3);\n    println!(\"{}\", w);\n }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { x }\npub fn show<T>(x: T) { println!(\"{}\", x); }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Runs("W::A\n"),
            },
            Case {
                name: "imported_enum",
                src: "mod lib; use lib::K;\nfn main() { let k = K::A;\n    println!(\"{}\", k);\n }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { x }\npub fn show<T>(x: T) { println!(\"{}\", x); }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Runs("K::A\n"),
            },
            Case {
                name: "local_generic_enum",
                src: "enum Opt<T> { Some(T), Nil }\nfn main() { let o: Opt<i64> = Opt::Some(1);\n    println!(\"{}\", o);\n }\n",
                module: None,
                expect: Expect::Runs("Opt::Some\n"),
            },
        ],
    );
}

#[test]
fn release01_78_a_field_through_an_rc_of_a_type_parameter_is_refused_at_the_definition() {
    check(
        "release01_78",
        &[
            Case {
                name: "field_assign_instantiated_by_a_struct",
                src: "use std::process::ExitCode; struct S { n: i64 }\nfn set<T>(r: Rc<T>) { r.n = 3; }\nfn main() -> ExitCode { let r = Rc::new(S { n: 1 });\n    set(r);\n    return ExitCode::from(r.n as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 23),
                    says: "field assignment on unconstrained generic type parameter 'T'",
                    alone: false,
                },
            },
            Case {
                name: "field_assign_instantiated_by_i64",
                src: "fn set<T>(r: Rc<T>) { r.n = 3; }\nfn main() { let r = Rc::new(5);\n    set(r);\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 23),
                    says: "field assignment on unconstrained generic type parameter 'T'",
                    alone: false,
                },
            },
            Case {
                name: "field_read_instantiated_by_i64",
                src: "use std::process::ExitCode; fn get<T>(r: Rc<T>) -> i64 { r.n }\nfn main() -> ExitCode { let r = Rc::new(5);\n    return ExitCode::from(get(r) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 37),
                    says: "field access on unconstrained generic type parameter 'T'",
                    alone: false,
                },
            },
            Case {
                name: "field_read_on_a_bare_parameter",
                src: "use std::process::ExitCode; fn get<T>(x: T) -> i64 { x.n }\nfn main() -> ExitCode { return ExitCode::from(get(3) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 33),
                    says: "field access on unconstrained generic type parameter 'T'",
                    alone: false,
                },
            },
        ],
    );
}

#[test]
fn release01_79_an_unresolved_type_diagnostic_names_the_right_site_once() {
    check(
        "release01_79",
        &[
            Case {
                name: "help_spells_the_source_type",
                src: "use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }\nfn main() -> ExitCode { let r = Result::Ok(3);\n    return ExitCode::from((match r { Result::Ok(x) => x, Result::Err(_) => 0 }) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 20),
                    says: "annotate the binding: `let r: Result<i64, T> = ..`",
                    alone: false,
                },
            },
            Case {
                name: "wildcard_pattern_is_not_the_site",
                src: "use std::process::ExitCode; enum Opt<T> { Some(T), Nil }\nfn mk<T>() -> Opt<T> { Opt::Nil }\nfn main() -> ExitCode { return ExitCode::from((match mk() { Opt::Some(_) => 1, Opt::Nil => 0 }) as u8); }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (3, 33),
                    says: "type annotations needed: the type of this expression is never inferred",
                    alone: false,
                },
            },
            Case {
                name: "never_is_spelled_never",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn e<T>(x: T) -> Result<i64, T> { Result::Err(x) }\nfn main() { let r: Result<i64, Never> = e(3);\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (3, 20),
                    says: "found `Never`",
                    alone: false,
                },
            },
            Case {
                name: "unresolved_value_printed",
                src: "fn main() { let x = Vec::new();\n    println!(\"{}\", x);\n }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (1, 20),
                    says: "type annotations needed: the type of `x` is never inferred",
                    alone: true,
                },
            },
            Case {
                name: "let_of_a_match_whose_arms_all_return",
                src: "enum Opt<T> { Some(T), Nil }\nfn f(o: Opt<i64>) -> i64 { let x = match o { Opt::Some(a) => return a, Opt::Nil => return 0 };\n    9 }\nfn main() { println!(\"{}\", f(Opt::Some(4)));\n }\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Case {
                name: "let_of_an_if_whose_branches_all_return",
                src: "fn f(c: bool) -> i64 { let x = if c { return 1; } else { return 2; };\n    9 }\nfn main() { println!(\"{}\", f(true));\n }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
        ],
    );
}

#[test]
fn release01_80_an_instance_only_a_cut_never_arm_reached_is_not_emitted() {
    check(
        "release01_80",
        &[
            Case {
                name: "instance_reached_from_two_instantiations",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn id<T>(x: T) -> T { x }\nfn f<E>(r: Result<i64, E>) -> i64 { match r { Result::Ok(x) => x, Result::Err(e) => { let z = id(e);\n 0 } } }\nfn main() { let a: Result<i64, i64> = Result::Ok(1);\n    let b: Result<i64, Never> = Result::Ok(2);\n    println!(\"{}\", f(a) + f(b));\n }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "payload_wrapped_by_a_generic",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nenum Opt<T> { Some(T), Nil }\nfn mk<T>(x: T) -> Opt<T> { Opt::Some(x) }\nfn f<E>(r: Result<i64, E>) -> i64 { match r { Result::Ok(x) => x, Result::Err(e) => { let o = mk(e);\n 1 } } }\nfn main() { let r: Result<i64, Never> = Result::Ok(3);\n    println!(\"{}\", f(r));\n }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "payload_passed_to_a_generic",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn id<T>(x: T) -> T { x }\nfn f<E>(r: Result<i64, E>) -> i64 { match r { Result::Ok(x) => x, Result::Err(e) => { let z = id(e);\n 0 } } }\nfn main() { let r: Result<i64, Never> = Result::Ok(3);\n    println!(\"{}\", f(r));\n }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
        ],
    );
}

#[test]
fn release01_81_a_string_formatted_through_a_type_parameter_is_freed_exactly_once() {
    check(
        "release01_81",
        &[
            Case {
                name: "lit",
                src: "fn show<T>(x: T) { println!(\"[{x}]\"); }\nfn main() { show(\"lit\");\n }\n",
                module: None,
                expect: Expect::Balanced("[lit]\n"),
            },
            Case {
                name: "owned",
                src: "fn show<T>(x: T) { println!(\"[{x}]\"); }\nfn main() { let s = \"a\" + \"b\";\n show(s);\n show(s);\n println!(\"{}\", s);\n }\n",
                module: None,
                expect: Expect::Balanced("[ab]\n[ab]\nab\n"),
            },
            Case {
                name: "loop_owned",
                src: "fn show<T>(x: T) { println!(\"{x}{x}\"); }\nfn main() { for i in 0..3 { let s = \"z\" + \"w\";\n show(s); }\n }\n",
                module: None,
                expect: Expect::Balanced("zwzw\nzwzw\nzwzw\n"),
            },
            Case {
                name: "struct_field",
                src: "struct P { s: String }\nfn show<T>(x: T) { println!(\"<{x}>\"); }\nfn main() { let p = P { s: \"f\" + \"g\" };\n show(p.s);\n println!(\"{}\", p.s);\n }\n",
                module: None,
                expect: Expect::Balanced("<fg>\nfg\n"),
            },
            Case {
                name: "temp_arg",
                src: "fn show<T>(x: T) { println!(\"<{x}>\"); }\nfn mk() -> String { \"m\" + \"k\" }\nfn main() { show(mk());\n show(\"x\" + \"y\");\n }\n",
                module: None,
                expect: Expect::Balanced("<mk>\n<xy>\n"),
            },
            Case {
                name: "nested_generic",
                src: "fn show<T>(x: T) { println!(\"<{x}>\"); }\nfn pass<U>(x: U) { show(x);\n show(x); }\nfn main() { let s = \"n\" + \"g\";\n pass(s);\n pass(s);\n }\n",
                module: None,
                expect: Expect::Balanced("<ng>\n<ng>\n<ng>\n<ng>\n"),
            },
            Case {
                name: "ret_fmt",
                src: "fn fmt1<T>(x: T) -> String { format!(\"v={x}\") }\nfn main() { let s = \"r\" + \"s\";\n let a = fmt1(s);\n let b = fmt1(3);\n println!(\"{}\", a);\n println!(\"{}\", b);\n println!(\"{}\", s);\n }\n",
                module: None,
                expect: Expect::Balanced("v=rs\nv=3\nrs\n"),
            },
            Case {
                name: "two_params",
                src: "fn two<T, U>(a: T, b: U) { println!(\"{a}-{b}-{a}\"); }\nfn main() { let s = \"q\" + \"\";\n two(s, 1);\n two(2, s);\n two(s, s);\n }\n",
                module: None,
                expect: Expect::Balanced("q-1-q\n2-q-2\nq-q-q\n"),
            },
            Case {
                name: "fmt_only_str",
                src: "fn show<T>(x: T) { println!(\"{x}\"); }\nfn main() { let s = \"only\" + \"\";\n show(s);\n }\n",
                module: None,
                expect: Expect::Balanced("only\n"),
            },
            Case {
                name: "recursive_generic_fmt",
                src: "fn rec<T>(x: T, n: i64) { if n > 0 { println!(\"{x}\");\n rec(x, n - 1); } }\nfn main() { let s = \"rr\" + \"\";\n rec(s, 3);\n }\n",
                module: None,
                expect: Expect::Balanced("rr\nrr\nrr\n"),
            },
            Case {
                name: "nongeneric_only",
                src: "fn main() { let s = \"only\" + \"\";\n println!(\"{s}\");\n }\n",
                module: None,
                expect: Expect::Balanced("only\n"),
            },
            Case {
                name: "only_lit",
                src: "fn show<T>(x: T) { println!(\"{x}\"); }\nfn main() { show(\"lit\");\n }\n",
                module: None,
                expect: Expect::Balanced("lit\n"),
            },
            Case {
                name: "only_bound",
                src: "fn show<T>(x: T) { let t = format!(\"{x}\");\n println!(\"{}\", t); }\nfn main() { let s = \"o\" + \"\";\n show(s);\n }\n",
                module: None,
                expect: Expect::Balanced("o\n"),
            },
            Case {
                name: "only_int",
                src: "fn show<T>(x: T) { println!(\"{x}\"); }\nfn main() { show(3);\n }\n",
                module: None,
                expect: Expect::Balanced("3\n"),
            },
            Case {
                name: "print_only",
                src: "fn show<T>(x: T) { print!(\"{x}\"); }\nfn main() { show(\"p\");\n }\n",
                module: None,
                expect: Expect::Balanced("p"),
            },
            Case {
                name: "return_only",
                src: "fn fm<T>(x: T) -> String { format!(\"{x}\") }\nfn main() { let s = fm(\"r\");\n println!(\"{}\", s);\n }\n",
                module: None,
                expect: Expect::Balanced("r\n"),
            },
            Case {
                name: "arg_only",
                src: "use std::process::ExitCode; fn sink(s: String) -> i64 { 0 }\nfn fm<T>(x: T) -> i64 { sink(format!(\"{x}\")) }\nfn main() -> ExitCode { return ExitCode::from(fm(\"a\") as u8);}\n",
                module: None,
                expect: Expect::Balanced(""),
            },
            Case {
                name: "cmp_only",
                src: "fn fm<T>(x: T) -> i64 { let n = format!(\"{x}\") == \"a\";\n if n { return 1; }\n 0 }\nfn main() { println!(\"{}\", fm(\"a\"));\n }\n",
                module: None,
                expect: Expect::Balanced("1\n"),
            },
            Case {
                name: "str_option_payload",
                src: "enum Opt<T> { Some(T), Nil }\nfn show<T>(x: T) { println!(\"<{x}>\"); }\nfn main() { let o: Opt<String> = Opt::Some(\"o\" + \"p\");\n match o { Opt::Some(s) => show(s), Opt::Nil => show(\"none\") }\n }\n",
                module: None,
                expect: Expect::Balanced("<op>\n"),
            },
            Case {
                name: "echo_one_part",
                src: "fn show<T>(x: T) { println!(\"{x}\"); }\nfn showp<T>(x: T) { print!(\"{x}\"); }\nfn mk() -> String { \"m\" + \"k\" }\nstruct P { s: String }\nfn echo<T>(x: T) -> T { println!(\"{x}\");\n x }\nfn main() { let s = echo(\"e\" + \"c\");\n println!(\"{}\", s);\n let t = echo(mk());\n println!(\"{}\", t);\n }\n",
                module: None,
                expect: Expect::Balanced("ec\nec\nmk\nmk\n"),
            },
            Case {
                name: "echo_two_parts",
                src: "fn echo<T>(x: T) -> T { println!(\"<{x}>\");\n x }\nfn main() { let s = echo(\"e\" + \"c\");\n println!(\"{}\", s);\n }\n",
                module: None,
                expect: Expect::Balanced("<ec>\nec\n"),
            },
            Case {
                name: "echo_bound_by_let",
                src: "fn echo<T>(x: T) -> T { let t = format!(\"<{x}>\");\n println!(\"{}\", t);\n x }\nfn main() { let s = echo(\"e\" + \"c\");\n println!(\"{}\", s);\n }\n",
                module: None,
                expect: Expect::Balanced("<ec>\nec\n"),
            },
            Case {
                name: "fmt_then_move_local",
                src: "use std::process::ExitCode; fn echo<T>(x: T) -> i64 { println!(\"<{x}>\");\n let y = x;\n 0 }\nfn main() -> ExitCode { return ExitCode::from(echo(\"e\" + \"c\") as u8); }\n",
                module: None,
                expect: Expect::Balanced("<ec>\n"),
            },
            Case {
                name: "fmt_then_wrap",
                src: "enum Opt<T> { Some(T), Nil }\nfn keep<T>(x: T) -> Opt<T> { println!(\"<{x}>\");\n Opt::Some(x) }\nfn main() { match keep(\"e\" + \"c\") { Opt::Some(s) => println!(\"{}\", s), Opt::Nil => println!(\"n\") }\n }\n",
                module: None,
                expect: Expect::Balanced("<ec>\nec\n"),
            },
        ],
    );
}
