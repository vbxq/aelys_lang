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
                src: "fn main() -> i64 {\n    let x = Rc::null()\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    if Rc::null() == Rc::null() { return 1 }\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    Rc::null()\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    if Rc::is_null(Rc::null()) { return 1 }\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let r = Rc::new(Rc::null())\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let v = vec[]\n    return v.len\n}\n",
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
                src: "fn main() -> i64 { let v = vec[]\n    if false { println(\"{v.len}\") }\n    return 0 }\n",
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
                src: "fn main() -> i64 { let g = fn(x) { return 0 }\n    println(\"ok\")\n    return 0 }\n",
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
                src: "fn mk<T>() -> Vec<T> { let v: Vec<T> = Vec::new()\n    return v }\nfn main() -> i64 { let v = mk()\n    return 0 }\n",
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
                src: "fn main() -> i64 { let g = fn() { return Rc::null() }\n    return 0 }\n",
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
                src: "fn f(x) -> i64 { return 0 }\nfn main() -> i64 { return 0 }\n",
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
                src: "nogc fn f() -> i64 {\n    let mut v = Vec::new()\n    return 0\n}\nfn main() -> i64 { return f() }\n",
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
                src: "struct S { n: i64 }\nfn main() -> i64 { let mut r = Rc::null()\n    r = Rc::new(S { n: 4 })\n    println(\"{r.n}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Case {
                name: "rc_null_compared_to_typed",
                src: "struct S { n: i64 }\nfn main() -> i64 { let a: Rc<S> = Rc::new(S { n: 4 })\n    if a == Rc::null() { return 1 }\n    println(\"ok\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("ok\n"),
            },
            Case {
                name: "lambda_parameter_fixed_by_call",
                src: "fn main() -> i64 { let g = fn(x) { return x + 1 }\n    println(\"{g(2)}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "lambda_parameter_fixed_by_unused_call",
                src: "fn main() -> i64 { let g = fn(x) { return 7 }\n    println(\"{g(1)}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("7\n"),
            },
            Case {
                name: "empty_vec_assigned_later",
                src: "fn main() -> i64 { let mut v = vec[]\n    v = vec[1, 2, 3]\n    println(\"{v.len}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "rc_null_returned",
                src: "struct S { n: i64 }\nfn f(c: bool) -> Rc<S> { let mut r = Rc::null()\n    if c { r = Rc::new(S { n: 4 }) }\n    return r }\nfn main() -> i64 { let x = f(true)\n    println(\"{x.n}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Case {
                name: "rc_null_in_struct_literal",
                src: "struct S { n: i64 }\nstruct H { r: Rc<S> }\nfn main() -> i64 { let h = H { r: Rc::null() }\n    if Rc::is_null(h.r) { println(\"null\") }\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("null\n"),
            },
            Case {
                name: "lambda_through_generic",
                src: "fn ap<T>(f: fn(T) -> T, x: T) -> T { return f(x) }\nfn main() -> i64 { println(\"{ap(fn(y) { return y }, 5)}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("5\n"),
            },
            Case {
                name: "vec_new_passed",
                src: "fn total(v: Vec<i64>) -> i64 { return v.len }\nfn main() -> i64 { let v = Vec::new()\n    println(\"{total(v)}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("0\n"),
            },
            Case {
                name: "rc_null_passed",
                src: "struct S { n: i64 }\nfn g(r: Rc<S>) -> i64 { return 1 }\nfn main() -> i64 { let r = Rc::null()\n    println(\"{g(r)}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Case {
                name: "match_statement_every_arm_returns",
                src: "enum Op { Inc, Dec }\nfn apply(op: Op, x: i64) -> i64 {\n    match op {\n        Op::Inc => return x + 1,\n        Op::Dec => return x - 1,\n    }\n    return x\n}\nfn main() -> i64 {\n    println(\"{apply(Op::Inc, 41)}\")\n    return 0\n}\n",
                module: None,
                expect: Expect::Runs("42\n"),
            },
            Case {
                name: "nested_match_statement_returns",
                src: "enum A { X, Y }\nfn f(a: A, b: A) -> i64 {\n    match a {\n        A::X => {\n            match b {\n                A::X => return 1,\n                A::Y => return 2,\n            }\n            return 0\n        }\n        A::Y => return 9,\n    }\n    return -1\n}\nfn main() -> i64 {\n    println(\"{f(A::X, A::Y)}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let f = fn(p) -> i64 { return p.a }\n    println(\"ok\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let s = fn(t) -> i64 { let x = t[0..1]\n return 0 }\n    return 0\n}\n",
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
                src: "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n    let f: fn(i64) -> i64 = id\n    return f(3)\n}\n",
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
                src: "fn id<T>(x: T) -> T { return x }\nlet G: fn(i64) -> i64 = id\nfn main() -> i64 { return G(2) }\n",
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
                src: "fn sink<U>(u: U) -> i64 { return 0 }\nfn main() -> i64 { let v = vec[1, 2]; let f: fn(vec<i64>) -> i64 = sink; return f(v) }\n",
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
                src: "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 { let id = fn(x: i64) -> i64 { return x + 1 }\n    let f: fn(i64) -> i64 = id\n    println(\"{f(3)}\")\n    return 0 }\n",
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
                src: "fn firstof<T>(x: T) -> i64 {\n    let s = x[0..1]\n    return 5\n}\nfn main() -> i64 {\n    let a: [i64; 3] = [1, 2, 3]\n    return firstof(a)\n}\n",
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
                src: "fn firstof<T>(x: T) -> i64 {\n    let s = x[0]\n    return 5\n}\nfn main() -> i64 {\n    let a: [i64; 3] = [1, 2, 3]\n    return firstof(a)\n}\n",
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
                src: "enum Opt<T> { Some(T), Nil }\nfn f(o: Opt) -> i64 { return 0 }\nfn main() -> i64 { return 0 }\n",
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
                src: "enum Opt<T> { Some(T), Nil }\nfn main() -> i64 { let o: Opt = Opt::Some(5)\n match o { Opt::Some(v) => { return v } Opt::Nil => { return 0 } } }\n",
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
                src: "enum Opt<T> { Some(T), Nil }\nstruct H { o: Opt }\nfn main() -> i64 { return 0 }\n",
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
                src: "enum Opt<T> { Some(T), Nil }\nfn f() -> Opt { return Opt::Some(5) }\nfn main() -> i64 { return 0 }\n",
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
                src: "enum Opt<T> { Some(T), Nil }\nfn f(o: Opt<i64, i64>) -> i64 { return 0 }\nfn main() -> i64 { return 0 }\n",
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
                src: "enum E { A, B }\nfn f(o: E<i64>) -> i64 { return 0 }\nfn main() -> i64 { return 0 }\n",
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
                src: "fn u() { }\nfn main() -> i64 {\n    let r = Rc::new(u())\n    return 0\n}\n",
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
                src: "fn u() { }\nfn mk<T>(x: T) -> Rc<T> { return Rc::new(x) }\nfn main() -> i64 {\n    let r = mk(u())\n    return 0\n}\n",
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
                src: "struct S { n: i64 }\nnogc fn f(x: &Rc<S>) -> i64 { return (*x).n }\nfn main() -> i64 { return 0 }\n",
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
                src: "struct S { n: i64 }\nfn f(x: &Rc<S>) -> i64 { return (*x).n }\nfn main() -> i64 { return 0 }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0410",
                    at: (2, 34),
                    says: "`*` here reads an `Rc<S>` out of a reference: reading an `Rc` through a reference is not supported yet, pass the `Rc` itself",
                    alone: false,
                },
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
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { return Result::Err(1) }\nfn main() -> i64 { return 0 }\n",
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
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { return Result::Ok(7) }\nfn main() -> i64 {\n    let v = f().into_ok()\n    println(\"{v}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let r: Rc<i64> = Rc::new(5)\n    r.k = 3\n    return 0\n}\n",
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
                src: "struct S { a: i64 }\nfn main() -> i64 {\n    let s = S { a: 1 }\n    println(\"{s}\")\n    return 0\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (4, 13),
                    says: "printing a struct is not supported yet; print its fields",
                    alone: false,
                },
            },
            Case {
                name: "struct_printed_directly",
                src: "struct S { a: i64 }\nfn main() -> i64 {\n    let s = S { a: 1 }\n    println(s)\n    return 0\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0902",
                    at: (4, 13),
                    says: "printing a struct is not supported yet; print its fields",
                    alone: false,
                },
            },
            Case {
                name: "enum_in_format",
                src: "enum E { A, B(i64) }\nfn main() -> i64 {\n    let e = E::B(3)\n    println(\"{e}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let a: [i64; 2] = [1, 2]\n    println(\"{a}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let v: Vec<i64> = vec[1, 2]\n    println(\"{v}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let k: i64 = 2\n    let f = fn(x: i64) -> i64 { return x + k }\n    println(\"{f}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let r: Rc<i64> = Rc::new(3)\n    println(\"{r}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let x: i64 = 3\n    let r = &x\n    println(\"{r}\")\n    return 0\n}\n",
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
                src: "fn main() -> i64 {\n    let a: [i64; 2] = [1, 2]\n    let s = a[0..1]\n    println(\"{s}\")\n    return 0\n}\n",
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
                src: "fn u() { }\nfn main() -> i64 {\n    println(u())\n    return 0\n}\n",
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
                src: "fn f() { }\nfn main() -> i64 {\n    if f() == f() { return 1 }\n    return 0\n}\n",
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
                src: "enum E { A, B(i64) }\nfn main() -> i64 {\n    let e = E::B(3)\n    println(e)\n    return 0\n}\n",
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
                src: "needs id from lib\nfn main() -> i64 { let f = id\n    return 0 }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { return x }\npub fn show<T>(x: T) { println(x) }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
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
                src: "needs id from lib\nfn main() -> i64 { let f = id\n    return f(3) }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { return x }\npub fn show<T>(x: T) { println(x) }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
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
                src: "needs lib\nfn main() -> i64 { let f = lib.id\n    return 0 }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { return x }\npub fn show<T>(x: T) { println(x) }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
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
                src: "needs lib\nfn main() -> i64 { lib.show(3)\n    let p = lib.id\n    return 0 }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { return x }\npub fn show<T>(x: T) { println(x) }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
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
                src: "fn id<T>(x: T) -> T { return x }\nfn main() {\n    let f = id\n    println(f(3))\n}\n",
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
                src: "fn id<T>(x: T) -> T { return x }\nfn get() -> fn(i64) -> i64 { return id }\nfn main() -> i64 { return get()(2) }\n",
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
                src: "fn id<T>(x: T) -> T { return x }\nfn ap(f: fn(i64) -> i64, x: i64) -> i64 { return f(x) }\nfn main() -> i64 { return ap(id, 3) }\n",
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
                src: "fn main() -> i64 { let p = println\n    p(\"x\")\n    return 0 }\n",
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
                src: "fn ap(f: fn(string), s: string) { f(s) }\nfn main() -> i64 { ap(println, \"hi\")\n    return 0 }\n",
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
                src: "fn main() -> i64 { let p = print\n    return 0 }\n",
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
                src: "struct H { f: fn(string) }\nfn main() -> i64 { let h = H { f: println }\n    return 0 }\n",
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
                src: "fn g() -> fn(string) { return println }\nfn main() -> i64 { g()(\"x\")\n    return 0 }\n",
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
                src: "fn main() -> i64 { (println)(\"x\")\n    return 0 }\n",
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
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { return Result::Ok(1) }\nfn g() -> Result<i64, Never> { let x = f()?\n    return Result::Ok(x + 1) }\nfn main() -> i64 { match g() { Result::Ok(x) => { println(\"{x}\") } Result::Err(_) => { println(\"err\") } }\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("2\n"),
            },
            Case {
                name: "question_into_an_i64_result",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { return Result::Ok(1) }\nfn g() -> Result<i64, i64> { let x = f()?\n    return Result::Ok(x) }\nfn main() -> i64 { match g() { Result::Ok(x) => { println(\"{x}\") } Result::Err(_) => { println(\"err\") } }\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Case {
                name: "err_arm_binding_never_used_as_i64",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f() -> Result<i64, Never> { return Result::Ok(1) }\nfn main() -> i64 { let v = match f() { Result::Ok(x) => x, Result::Err(e) => e }\n    println(\"{v}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Case {
                name: "question_in_a_generic_instantiated_with_never",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn f<E>(r: Result<i64, E>) -> Result<i64, E> { let x = r?\n    return Result::Ok(x) }\nfn main() -> i64 { let r: Result<i64, Never> = Result::Ok(1)\n    match f(r) { Result::Ok(x) => { println(\"{x}\") } Result::Err(_) => { println(\"err\") } }\n    return 0 }\n",
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
                src: "fn main() -> i64 { let v: Vec = Vec::new()\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nfn main() -> i64 { let r: Rc = Rc::new(S { n: 1 })\n    return 0 }\n",
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
                src: "fn main() -> i64 { let v: Vec<i64, i64> = Vec::new()\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nfn main() -> i64 { let s: S<i64> = S { n: 1 }\n    return s.n }\n",
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
                src: "struct S { n: i64 }\nfn f(r: Rc) -> i64 { return 0 }\nfn main() -> i64 { return f(Rc::new(S { n: 1 })) }\n",
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
                src: "struct S { n: i64 }\nstruct H { r: Rc }\nfn main() -> i64 { return 0 }\n",
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
                src: "fn f() -> Vec { let v: Vec<i64> = Vec::new()\n    return v }\nfn main() -> i64 { return 0 }\n",
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
                src: "struct S { n: i64 }\nfn main() -> i64 { let r: Rc<S, S> = Rc::new(S { n: 1 })\n    return 0 }\n",
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
                src: "fn show<T>(x: T) { println(\"v={x}\") }\nfn main() -> i64 { show(\"s\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("v=s\n"),
            },
            Case {
                name: "string_formatted_through_two_generics",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { pass(\"ab\")\n    pass(3)\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("[ab]\n[3]\n"),
            },
            Case {
                name: "enum_printed_directly_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { shown(E::B(2))\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("E::B\n"),
            },
            Case {
                name: "struct_formatted_by_a_generic",
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { show(S { n: 1 })\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { shown(S { n: 1 })\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { pass(S { n: 1 })\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { show(E::A)\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { show([1, 2])\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { show(Rc::new(S { n: 1 }))\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { let y: i64 = 3\n    show(&y)\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn main() -> i64 { show(u())\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nenum E { A, B(i64) }\nfn u() { }\nfn show<T>(x: T) { println(\"[{x}]\") }\nfn shown<T>(x: T) { println(x) }\nfn pass<U>(x: U) { show(x) }\nfn g1() -> i64 { return 1 }\nfn main() -> i64 { show(g1)\n    return 0 }\n",
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
                src: "needs W from lib\nfn main() -> i64 { let w: W<i64> = W::A(3)\n    println(w)\n    return 0 }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { return x }\npub fn show<T>(x: T) { println(x) }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Runs("W::A\n"),
            },
            Case {
                name: "imported_enum",
                src: "needs K from lib\nfn main() -> i64 { let k = K::A\n    println(k)\n    return 0 }\n",
                module: Some((
                    "lib.aelys",
                    "pub fn id<T>(x: T) -> T { return x }\npub fn show<T>(x: T) { println(x) }\npub enum K { A }\npub enum W<T> { A(T), B }\n",
                )),
                expect: Expect::Runs("K::A\n"),
            },
            Case {
                name: "local_generic_enum",
                src: "enum Opt<T> { Some(T), Nil }\nfn main() -> i64 { let o: Opt<i64> = Opt::Some(1)\n    println(o)\n    return 0 }\n",
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
                src: "struct S { n: i64 }\nfn set<T>(r: Rc<T>) { r.n = 3 }\nfn main() -> i64 { let r = Rc::new(S { n: 1 })\n    set(r)\n    return r.n }\n",
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
                src: "fn set<T>(r: Rc<T>) { r.n = 3 }\nfn main() -> i64 { let r = Rc::new(5)\n    set(r)\n    return 0 }\n",
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
                src: "fn get<T>(r: Rc<T>) -> i64 { return r.n }\nfn main() -> i64 { let r = Rc::new(5)\n    return get(r) }\n",
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
                src: "fn get<T>(x: T) -> i64 { return x.n }\nfn main() -> i64 { return get(3) }\n",
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
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn main() -> i64 { let r = Result::Ok(3)\n    return match r { Result::Ok(x) => x, Result::Err(_) => 0 } }\n",
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
                src: "enum Opt<T> { Some(T), Nil }\nfn mk<T>() -> Opt<T> { return Opt::Nil }\nfn main() -> i64 { return match mk() { Opt::Some(_) => 1, Opt::Nil => 0 } }\n",
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
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn e<T>(x: T) -> Result<i64, T> { return Result::Err(x) }\nfn main() -> i64 { let r: Result<i64, Never> = e(3)\n    return 0 }\n",
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
                src: "fn main() -> i64 { let x = Vec::new()\n    println(x)\n    return 0 }\n",
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
                src: "enum Opt<T> { Some(T), Nil }\nfn f(o: Opt<i64>) -> i64 { let x = match o { Opt::Some(a) => return a, Opt::Nil => return 0 }\n    return 9 }\nfn main() -> i64 { println(\"{f(Opt::Some(4))}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Case {
                name: "let_of_an_if_whose_branches_all_return",
                src: "fn f(c: bool) -> i64 { let x = if c { return 1 } else { return 2 }\n    return 9 }\nfn main() -> i64 { println(\"{f(true)}\")\n    return 0 }\n",
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
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn id<T>(x: T) -> T { return x }\nfn f<E>(r: Result<i64, E>) -> i64 { return match r { Result::Ok(x) => x, Result::Err(e) => { let z = id(e)\n 0 } } }\nfn main() -> i64 { let a: Result<i64, i64> = Result::Ok(1)\n    let b: Result<i64, Never> = Result::Ok(2)\n    println(\"{f(a) + f(b)}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "payload_wrapped_by_a_generic",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nenum Opt<T> { Some(T), Nil }\nfn mk<T>(x: T) -> Opt<T> { return Opt::Some(x) }\nfn f<E>(r: Result<i64, E>) -> i64 { return match r { Result::Ok(x) => x, Result::Err(e) => { let o = mk(e)\n 1 } } }\nfn main() -> i64 { let r: Result<i64, Never> = Result::Ok(3)\n    println(\"{f(r)}\")\n    return 0 }\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "payload_passed_to_a_generic",
                src: "enum Result<T, E> { Ok(T), Err(E) }\nfn id<T>(x: T) -> T { return x }\nfn f<E>(r: Result<i64, E>) -> i64 { return match r { Result::Ok(x) => x, Result::Err(e) => { let z = id(e)\n 0 } } }\nfn main() -> i64 { let r: Result<i64, Never> = Result::Ok(3)\n    println(\"{f(r)}\")\n    return 0 }\n",
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
                src: "fn show<T>(x: T) { println(\"[{x}]\") }\nfn main() -> i64 { show(\"lit\")\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("[lit]\n"),
            },
            Case {
                name: "owned",
                src: "fn show<T>(x: T) { println(\"[{x}]\") }\nfn main() -> i64 { let s = \"a\" + \"b\"\n show(s)\n show(s)\n println(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("[ab]\n[ab]\nab\n"),
            },
            Case {
                name: "loop_owned",
                src: "fn show<T>(x: T) { println(\"{x}{x}\") }\nfn main() -> i64 { for i in 0..3 { let s = \"z\" + \"w\"\n show(s) }\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("zwzw\nzwzw\nzwzw\n"),
            },
            Case {
                name: "struct_field",
                src: "struct P { s: string }\nfn show<T>(x: T) { println(\"<{x}>\") }\nfn main() -> i64 { let p = P { s: \"f\" + \"g\" }\n show(p.s)\n println(p.s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("<fg>\nfg\n"),
            },
            Case {
                name: "temp_arg",
                src: "fn show<T>(x: T) { println(\"<{x}>\") }\nfn mk() -> string { return \"m\" + \"k\" }\nfn main() -> i64 { show(mk())\n show(\"x\" + \"y\")\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("<mk>\n<xy>\n"),
            },
            Case {
                name: "nested_generic",
                src: "fn show<T>(x: T) { println(\"<{x}>\") }\nfn pass<U>(x: U) { show(x)\n show(x) }\nfn main() -> i64 { let s = \"n\" + \"g\"\n pass(s)\n pass(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("<ng>\n<ng>\n<ng>\n<ng>\n"),
            },
            Case {
                name: "ret_fmt",
                src: "fn fmt1<T>(x: T) -> string { return \"v={x}\" }\nfn main() -> i64 { let s = \"r\" + \"s\"\n let a = fmt1(s)\n let b = fmt1(3)\n println(a)\n println(b)\n println(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("v=rs\nv=3\nrs\n"),
            },
            Case {
                name: "two_params",
                src: "fn two<T, U>(a: T, b: U) { println(\"{a}-{b}-{a}\") }\nfn main() -> i64 { let s = \"q\" + \"\"\n two(s, 1)\n two(2, s)\n two(s, s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("q-1-q\n2-q-2\nq-q-q\n"),
            },
            Case {
                name: "fmt_only_str",
                src: "fn show<T>(x: T) { println(\"{x}\") }\nfn main() -> i64 { let s = \"only\" + \"\"\n show(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("only\n"),
            },
            Case {
                name: "recursive_generic_fmt",
                src: "fn rec<T>(x: T, n: i64) { if n > 0 { println(\"{x}\")\n rec(x, n - 1) } }\nfn main() -> i64 { let s = \"rr\" + \"\"\n rec(s, 3)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("rr\nrr\nrr\n"),
            },
            Case {
                name: "nongeneric_only",
                src: "fn main() -> i64 { let s = \"only\" + \"\"\n println(\"{s}\")\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("only\n"),
            },
            Case {
                name: "only_lit",
                src: "fn show<T>(x: T) { println(\"{x}\") }\nfn main() -> i64 { show(\"lit\")\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("lit\n"),
            },
            Case {
                name: "only_bound",
                src: "fn show<T>(x: T) { let t = \"{x}\"\n println(t) }\nfn main() -> i64 { let s = \"o\" + \"\"\n show(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("o\n"),
            },
            Case {
                name: "only_int",
                src: "fn show<T>(x: T) { println(\"{x}\") }\nfn main() -> i64 { show(3)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("3\n"),
            },
            Case {
                name: "print_only",
                src: "fn show<T>(x: T) { print(\"{x}\") }\nfn main() -> i64 { show(\"p\")\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("p"),
            },
            Case {
                name: "return_only",
                src: "fn fm<T>(x: T) -> string { return \"{x}\" }\nfn main() -> i64 { let s = fm(\"r\")\n println(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("r\n"),
            },
            Case {
                name: "arg_only",
                src: "fn sink(s: string) -> i64 { return 0 }\nfn fm<T>(x: T) -> i64 { return sink(\"{x}\") }\nfn main() -> i64 { return fm(\"a\")}\n",
                module: None,
                expect: Expect::Balanced(""),
            },
            Case {
                name: "cmp_only",
                src: "fn fm<T>(x: T) -> i64 { let n = \"{x}\" == \"a\"\n if n { return 1 }\n return 0 }\nfn main() -> i64 { println(\"{fm(\"a\")}\")\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("1\n"),
            },
            Case {
                name: "str_option_payload",
                src: "enum Opt<T> { Some(T), Nil }\nfn show<T>(x: T) { println(\"<{x}>\") }\nfn main() -> i64 { let o: Opt<string> = Opt::Some(\"o\" + \"p\")\n match o { Opt::Some(s) => show(s), Opt::Nil => show(\"none\") }\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("<op>\n"),
            },
            Case {
                name: "echo_one_part",
                src: "fn show<T>(x: T) { println(\"{x}\") }\nfn showp<T>(x: T) { print(\"{x}\") }\nfn mk() -> string { return \"m\" + \"k\" }\nstruct P { s: string }\nfn echo<T>(x: T) -> T { println(\"{x}\")\n return x }\nfn main() -> i64 { let s = echo(\"e\" + \"c\")\n println(s)\n let t = echo(mk())\n println(t)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("ec\nec\nmk\nmk\n"),
            },
            Case {
                name: "echo_two_parts",
                src: "fn echo<T>(x: T) -> T { println(\"<{x}>\")\n return x }\nfn main() -> i64 { let s = echo(\"e\" + \"c\")\n println(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("<ec>\nec\n"),
            },
            Case {
                name: "echo_bound_by_let",
                src: "fn echo<T>(x: T) -> T { let t = \"<{x}>\"\n println(t)\n return x }\nfn main() -> i64 { let s = echo(\"e\" + \"c\")\n println(s)\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("<ec>\nec\n"),
            },
            Case {
                name: "fmt_then_move_local",
                src: "fn echo<T>(x: T) -> i64 { println(\"<{x}>\")\n let y = x\n return 0 }\nfn main() -> i64 { return echo(\"e\" + \"c\") }\n",
                module: None,
                expect: Expect::Balanced("<ec>\n"),
            },
            Case {
                name: "fmt_then_wrap",
                src: "enum Opt<T> { Some(T), Nil }\nfn keep<T>(x: T) -> Opt<T> { println(\"<{x}>\")\n return Opt::Some(x) }\nfn main() -> i64 { match keep(\"e\" + \"c\") { Opt::Some(s) => println(s), Opt::Nil => println(\"n\") }\n return 0 }\n",
                module: None,
                expect: Expect::Balanced("<ec>\nec\n"),
            },
        ],
    );
}
