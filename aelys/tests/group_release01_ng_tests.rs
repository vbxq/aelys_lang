#[path = "level_legs/mod.rs"]
mod level_legs;

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use level_legs::{LEVELS, LevelLeg, legs_of, level_disagreements};
use std::collections::HashSet;
use std::fs;
use tempfile::tempdir;

#[derive(Clone, Copy)]
enum Expect {
    Refused {
        code: &'static str,
        at: (u32, u32),
        says: &'static str,
    },
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

fn met(expect: Expect, leg: &LevelLeg) -> bool {
    match (expect, leg) {
        (Expect::Refused { code, at, says }, LevelLeg::Refused(rendered)) => {
            rendered.contains(&format!("error[{code}]"))
                && rendered.contains(&format!("root.aelys:{}:{}", at.0, at.1))
                && rendered.contains(says)
                && !rendered.contains("E0901")
        }
        (
            Expect::Balanced(out),
            LevelLeg::Ran {
                stdout,
                stderr,
                exit,
            },
        ) => stdout == out && *exit == 0 && rc_balanced(stderr),
        _ => false,
    }
}

fn check(row: &str, cases: &[Case]) {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    for case in cases {
        let env: &[(&str, &str)] = match case.expect {
            Expect::Balanced(_) => &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "malloc")],
            Expect::Refused { .. } => &[],
        };
        let legs = legs_of(dir.path(), case.name, case.src, case.module, env);
        assert_eq!(legs.len(), LEVELS.len(), "{row}: legs of {}", case.name);
        failures.extend(level_disagreements(case.name, &legs));
        if !met(case.expect, &legs[0].1) {
            failures.push(format!("{} at -O0: {}", case.name, legs[0].1.render()));
        }
    }
    assert!(failures.is_empty(), "{row}:\n{}", failures.join("\n"));
}

const REFUSED: &[Case] = &[
    Case {
        name: "fmt_member",
        src: concat!(
            "fn mk(n: i64) -> string { return \"a{n}\" }\n",
            "nogc fn f(n: i64) -> i64 { return mk(n).len }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (1, 34),
            says: "via `f -> mk -> a format string`",
        },
    },
    Case {
        name: "vec_index",
        src: concat!(
            "fn mk(n: i64) -> Vec<i64> { return vec[n, n] }\n",
            "nogc fn f(n: i64) -> i64 { return mk(n)[0] }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (1, 36),
            says: "via `f -> mk -> a vec literal`",
        },
    },
    Case {
        name: "struct_field",
        src: concat!(
            "struct T { s: string, k: i64 }\n",
            "fn mk(n: i64) -> T { return T { s: \"v{n}\", k: n } }\n",
            "nogc fn f(n: i64) -> i64 { return mk(n).k }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (2, 36),
            says: "via `f -> mk -> a format string`",
        },
    },
    Case {
        name: "result_question",
        src: concat!(
            "fn mk(n: i64) -> Result<string, i64> { return Ok(\"a{n}\") }\n",
            "nogc fn f(n: i64) -> Result<i64, i64> { return Ok(mk(n)?.len) }\n",
            "fn main() -> i64 {\n    match f(7) {\n        Ok(v) => println(v),\n",
            "        Err(e) => println(e)\n    }\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (1, 50),
            says: "via `f -> mk -> a format string`",
        },
    },
    Case {
        name: "std_from_int",
        src: concat!(
            "needs std.str\n",
            "nogc fn f(n: i64) -> i64 { return str.from_int(n).len }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (2, 35),
            says: "via `f -> std.str.from_int -> a format string`",
        },
    },
    Case {
        name: "two_hop_if_arm",
        src: concat!(
            "fn mk0(n: i64) -> string { return \"a\" + \"b{n}\" }\n",
            "fn mk(n: i64) -> string { return mk0(n) }\n",
            "nogc fn f(n: i64) -> i64 { return if n > 0 { mk(n).len } else { 0 } }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (1, 35),
            says: "via `f -> mk -> mk0 -> string concatenation`",
        },
    },
    Case {
        name: "statement_and_argument",
        src: concat!(
            "fn mk(n: i64) -> Vec<i64> {\n    let mut v: Vec<i64> = Vec::new()\n",
            "    Vec::push(v, n)\n    return v\n}\n",
            "nogc fn idn(x: i64) -> i64 { return x }\n",
            "nogc fn f(n: i64) -> i64 {\n    mk(n).len\n    return idn(mk(n)[0])\n}\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (2, 27),
            says: "via `f -> mk -> Vec::new`",
        },
    },
    Case {
        name: "module_function",
        src: concat!(
            "needs m\n",
            "nogc fn f(n: i64) -> i64 { return m.mk(n).len }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: Some((
            "m.aelys",
            "pub fn mk(n: i64) -> string { return \"a{n}\" }\n",
        )),
        expect: Expect::Refused {
            code: "E0727",
            at: (2, 35),
            says: "via `f -> m.mk -> a format string`",
        },
    },
    Case {
        name: "generic_match_arm",
        src: concat!(
            "fn mk<T>(x: T, n: i64) -> string { return \"a{n}\" }\n",
            "nogc fn f(n: i64) -> i64 {\n",
            "    return match Some(n) { Some(v) => mk(1, v).len, None => 0 }\n}\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (1, 43),
            says: "via `f -> mk -> a format string`",
        },
    },
    Case {
        name: "deref_assign_target",
        src: concat!(
            "fn r(x: &mut i64) -> &mut i64 {\n",
            "    let t = \"z\" + \"y\"\n",
            "    return x\n",
            "}\n",
            "nogc fn f(n: i64) -> i64 {\n",
            "    let mut k: i64 = n\n",
            "    *r(&mut k) = 5\n",
            "    return k\n",
            "}\n",
            "fn main() -> i64 {\n",
            "    println(f(3))\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0727",
            at: (2, 13),
            says: "via `f -> r -> string concatenation`",
        },
    },
];

const ACCEPTED: &[Case] = &[
    Case {
        name: "array_index",
        src: concat!(
            "nogc fn mk(n: i64) -> [i64; 2] { return [n, n + 1] }\n",
            "nogc fn f(n: i64) -> i64 { return mk(n)[1] + mk(n + 1)[0] }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Balanced("16\n"),
    },
    Case {
        name: "struct_field_positions",
        src: concat!(
            "struct P { a: i64, b: i64 }\n",
            "nogc fn mk(n: i64) -> P { return P { a: n, b: n * 2 } }\n",
            "nogc fn idn(x: i64) -> i64 { return x }\n",
            "nogc fn f(n: i64) -> i64 {\n    mk(n).a\n",
            "    let r: i64 = if n > 0 { idn(mk(n).b) } else { 0 }\n",
            "    return match Some(r) { Some(v) => v + mk(1).a, None => 0 }\n}\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Balanced("15\n"),
    },
    Case {
        name: "slice_argument",
        src: concat!(
            "struct P { a: i64, b: i64 }\n",
            "nogc fn pick(s: &[i64]) -> P { return P { a: s[0], b: s[1] } }\n",
            "nogc fn f(s: &[i64]) -> i64 { return pick(s).b + pick(s).a }\n",
            "fn main() -> i64 {\n    let v: Vec<i64> = vec[3, 4]\n",
            "    println(f(v[0..2]))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Balanced("7\n"),
    },
    Case {
        name: "result_question",
        src: concat!(
            "struct P { a: i64, b: i64 }\n",
            "nogc fn mk(n: i64) -> Result<P, i64> {\n",
            "    if n < 0 { return Err(n) }\n    return Ok(P { a: n, b: 1 })\n}\n",
            "nogc fn f(n: i64) -> Result<i64, i64> { return Ok(mk(n)?.a + mk(n)?.b) }\n",
            "fn main() -> i64 {\n    match f(7) {\n        Ok(v) => println(v),\n",
            "        Err(e) => println(e)\n    }\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Balanced("8\n"),
    },
    Case {
        name: "managed_caller",
        src: concat!(
            "fn mk(n: i64) -> string { return \"a{n}\" }\n",
            "fn f(n: i64) -> i64 { return mk(n).len }\n",
            "fn main() -> i64 {\n    println(f(7))\n    return 0\n}\n"
        ),
        module: None,
        expect: Expect::Balanced("2\n"),
    },
    Case {
        name: "deref_assign_target",
        src: concat!(
            "fn r(x: &mut i64) -> &mut i64 { return x }\n",
            "nogc fn f(n: i64) -> i64 {\n",
            "    let mut k: i64 = n\n",
            "    *r(&mut k) = 5\n",
            "    return k\n",
            "}\n",
            "fn main() -> i64 {\n",
            "    println(f(3))\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Balanced("5\n"),
    },
];

const FREED_READS: &[Case] = &[
    Case {
        name: "index_after_call_base",
        src: concat!(
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let mut w: Vec<i64> = vec[55]\n",
            "    let x = rr(v[0..3])[{\n",
            "        let mut i: i64 = 0\n",
            "        while i < 200 {\n",
            "            Vec::push(v, 77)\n",
            "            i = i + 1\n",
            "        }\n",
            "        Vec::push(w, 55)\n",
            "        Vec::push(w, 55)\n",
            "        Vec::push(w, 55)\n",
            "        0\n",
            "    }]\n",
            "    println(x)\n",
            "    println(w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (8, 13),
            says: "cannot write to `v` while it is borrowed",
        },
    },
    Case {
        name: "index_after_slice_base",
        src: concat!(
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let mut w: Vec<i64> = vec[55]\n",
            "    let x = v[0..3][{\n",
            "        let mut i: i64 = 0\n",
            "        while i < 200 {\n",
            "            Vec::push(v, 77)\n",
            "            i = i + 1\n",
            "        }\n",
            "        Vec::push(w, 55)\n",
            "        Vec::push(w, 55)\n",
            "        Vec::push(w, 55)\n",
            "        0\n",
            "    }]\n",
            "    println(x)\n",
            "    println(w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (8, 13),
            says: "cannot write to `v` while it is borrowed",
        },
    },
    Case {
        name: "reborrow_through_call",
        src: concat!(
            "fn re(s: &[i64]) -> &i64 { return &s[0] }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let x = &*re(v[0..3])\n",
            "    v[0] = 99\n",
            "    println(*x)\n",
            "    let mut i: i64 = 0\n",
            "    while i < 200 {\n",
            "        Vec::push(v, i)\n",
            "        i = i + 1\n",
            "    }\n",
            "    let mut w: Vec<i64> = vec[77, 77, 77]\n",
            "    let mut j: i64 = 0\n",
            "    while j < 3 {\n",
            "        Vec::push(w, 77)\n",
            "        j = j + 1\n",
            "    }\n",
            "    println(*x)\n",
            "    println(w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (5, 5),
            says: "cannot write to `v[..]` while it is borrowed",
        },
    },
    Case {
        name: "shared_reborrow_then_write",
        src: concat!(
            "fn rc(x: &i64) -> &i64 { return x }\n",
            "fn main() -> i64 {\n",
            "    let mut k: i64 = 1\n",
            "    let x = &*rc(&k)\n",
            "    k = 5\n",
            "    println(*x)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (5, 5),
            says: "cannot write to `k` while it is borrowed",
        },
    },
    Case {
        name: "mut_reborrow_then_read",
        src: concat!(
            "fn ri(x: &mut i64) -> &mut i64 { return x }\n",
            "fn main() -> i64 {\n",
            "    let mut k: i64 = 1\n",
            "    let x = &mut *ri(&mut k)\n",
            "    println(k)\n",
            "    *x = 5\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0713",
            at: (5, 5),
            says: "cannot use `k` while it is mutably borrowed",
        },
    },
    Case {
        name: "slice_of_a_deref_call",
        src: concat!(
            "struct P { x: i64, y: i64 }\n",
            "fn rv(v: &Vec<i64>) -> &Vec<i64> { return v }\n",
            "fn rvm(v: &mut Vec<i64>) -> &mut Vec<i64> { return v }\n",
            "fn rp(p: &P) -> &P { return p }\n",
            "fn rpm(p: &mut P) -> &mut P { return p }\n",
            "fn rs(s: &string) -> &string { return s }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn two(a: &[i64], k: i64) -> i64 { return a[0] + k }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let s = (*rv(&v))[0..2]\n",
            "    let mut i: i64 = 0\n",
            "    while i < 200 {\n",
            "        Vec::push(v, 77)\n",
            "        i = i + 1\n",
            "    }\n",
            "    let mut w: Vec<i64> = vec[55]\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    println(s[0] + w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (14, 9),
            says: "cannot write to `v` while it is borrowed",
        },
    },
    Case {
        name: "mut_slice_of_a_deref_call",
        src: concat!(
            "struct P { x: i64, y: i64 }\n",
            "fn rv(v: &Vec<i64>) -> &Vec<i64> { return v }\n",
            "fn rvm(v: &mut Vec<i64>) -> &mut Vec<i64> { return v }\n",
            "fn rp(p: &P) -> &P { return p }\n",
            "fn rpm(p: &mut P) -> &mut P { return p }\n",
            "fn rs(s: &string) -> &string { return s }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn two(a: &[i64], k: i64) -> i64 { return a[0] + k }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let s = (*rvm(&mut v))[0..2]\n",
            "    let mut i: i64 = 0\n",
            "    while i < 200 {\n",
            "        Vec::push(v, 77)\n",
            "        i = i + 1\n",
            "    }\n",
            "    let mut w: Vec<i64> = vec[55]\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    println(s[0] + w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (14, 9),
            says: "cannot write to `v` while it is borrowed",
        },
    },
    Case {
        name: "as_slice_of_a_deref_call",
        src: concat!(
            "struct P { x: i64, y: i64 }\n",
            "fn rv(v: &Vec<i64>) -> &Vec<i64> { return v }\n",
            "fn rvm(v: &mut Vec<i64>) -> &mut Vec<i64> { return v }\n",
            "fn rp(p: &P) -> &P { return p }\n",
            "fn rpm(p: &mut P) -> &mut P { return p }\n",
            "fn rs(s: &string) -> &string { return s }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn two(a: &[i64], k: i64) -> i64 { return a[0] + k }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let s = Vec::as_slice(*rv(&v))\n",
            "    let mut i: i64 = 0\n",
            "    while i < 200 {\n",
            "        Vec::push(v, 77)\n",
            "        i = i + 1\n",
            "    }\n",
            "    let mut w: Vec<i64> = vec[55]\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    println(s[0] + w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (14, 9),
            says: "cannot write to `v` while it is borrowed",
        },
    },
    Case {
        name: "bytes_of_a_deref_call",
        src: concat!(
            "struct P { x: i64, y: i64 }\n",
            "fn rv(v: &Vec<i64>) -> &Vec<i64> { return v }\n",
            "fn rvm(v: &mut Vec<i64>) -> &mut Vec<i64> { return v }\n",
            "fn rp(p: &P) -> &P { return p }\n",
            "fn rpm(p: &mut P) -> &mut P { return p }\n",
            "fn rs(s: &string) -> &string { return s }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn two(a: &[i64], k: i64) -> i64 { return a[0] + k }\n",
            "fn main() -> i64 {\n",
            "    let mut t: string = \"hello{3}\"\n",
            "    let b = (*rs(&t)).bytes\n",
            "    t = \"zz{4}\"\n",
            "    let u: string = \"QQQQQQ{5}\"\n",
            "    println(b[0] as i64)\n",
            "    println(u)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (12, 5),
            says: "cannot write to `t` while it is borrowed",
        },
    },
    Case {
        name: "index_of_a_slice_of_a_deref_call",
        src: concat!(
            "struct P { x: i64, y: i64 }\n",
            "fn rv(v: &Vec<i64>) -> &Vec<i64> { return v }\n",
            "fn rvm(v: &mut Vec<i64>) -> &mut Vec<i64> { return v }\n",
            "fn rp(p: &P) -> &P { return p }\n",
            "fn rpm(p: &mut P) -> &mut P { return p }\n",
            "fn rs(s: &string) -> &string { return s }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn two(a: &[i64], k: i64) -> i64 { return a[0] + k }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let x = (*rv(&v))[0..3][{\n",
            "        let mut i: i64 = 0\n",
            "        while i < 200 {\n",
            "            Vec::push(v, 77)\n",
            "            i = i + 1\n",
            "        }\n",
            "        0\n",
            "    }]\n",
            "    let mut w: Vec<i64> = vec[55]\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    Vec::push(w, 55)\n",
            "    println(x + w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0711",
            at: (14, 13),
            says: "cannot write to `v` while it is borrowed",
        },
    },
];

const CONFLICTS: &[Case] = &[
    Case {
        name: "bump_while_view",
        src: concat!(
            "fn bump(s: &mut [i64]) -> [i64; 2] {\n",
            "    s[0] = s[0] + 1\n",
            "    return [s[0], s.len]\n",
            "}\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[1, 2, 3]\n",
            "    let view = v[0..3]\n",
            "    let k = bump(v[0..3])[0]\n",
            "    println(view[0] + k)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0713",
            at: (8, 18),
            says: "cannot borrow `v` as mutable while it is already borrowed",
        },
    },
    Case {
        name: "grow_under_projection",
        src: concat!(
            "fn grow(v: &mut Vec<i64>) -> [i64; 1] { return [7] }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[1, 2, 3]\n",
            "    let view = v[0..2]\n",
            "    let k = grow(&mut v)[0]\n",
            "    println(view[0] + k)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0713",
            at: (5, 18),
            says: "cannot borrow `v` as mutable while it is already borrowed",
        },
    },
    Case {
        name: "mut_then_shared",
        src: concat!(
            "struct Resource { id: i64 }\n",
            "fn eat(r: Resource) -> i64 { return r.id }\n",
            "fn mkA(n: i64) -> [i64; 3] { return [n, n + 1, n + 2] }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn rm(a: &mut [i64]) -> &mut [i64] { return a }\n",
            "fn ri(x: &mut i64) -> &mut i64 { return x }\n",
            "fn rc(x: &i64) -> &i64 { return x }\n",
            "fn main() -> i64 {\n",
            "    let mut a: [i64; 3] = [1, 2, 3]\n",
            "    let m = rm(a[0..3])\n",
            "    println(rr(a[0..3])[0])\n",
            "    m[0] = 5\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0713",
            at: (11, 16),
            says: "cannot borrow `a` as shared while it is already mutably borrowed",
        },
    },
    Case {
        name: "view_then_deref_assign_call",
        src: concat!(
            "struct Resource { id: i64 }\n",
            "fn eat(r: Resource) -> i64 { return r.id }\n",
            "fn mkA(n: i64) -> [i64; 3] { return [n, n + 1, n + 2] }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn rm(a: &mut [i64]) -> &mut [i64] { return a }\n",
            "fn ri(x: &mut i64) -> &mut i64 { return x }\n",
            "fn rc(x: &i64) -> &i64 { return x }\n",
            "fn main() -> i64 {\n",
            "    let mut k: i64 = 1\n",
            "    let v = rc(&k)\n",
            "    *ri(&mut k) = 7\n",
            "    println(*v)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0713",
            at: (11, 9),
            says: "cannot borrow `k` as mutable while it is already borrowed",
        },
    },
    Case {
        name: "deref_twice",
        src: concat!(
            "fn rc(x: &i64) -> &i64 { return x }\n",
            "fn ri(x: &mut i64) -> &mut i64 { return x }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "struct P { x: i64, y: i64 }\n",
            "fn main() -> i64 {\n",
            "    let mut k: i64 = 1\n",
            "    let y = ri(&mut k)\n",
            "    let z = *rc(&k)\n",
            "    *y = 4\n",
            "    println(z)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Refused {
            code: "E0713",
            at: (8, 17),
            says: "cannot borrow `k` as shared while it is already mutably borrowed",
        },
    },
];

const ORDERED: &[Case] = &[
    Case {
        name: "index_of_a_place_after_its_growth",
        src: concat!(
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<i64> = vec[11, 22, 33]\n",
            "    let mut w: Vec<i64> = vec[55]\n",
            "    let x = v[{\n",
            "        let mut i: i64 = 0\n",
            "        while i < 200 {\n",
            "            Vec::push(v, 77)\n",
            "            i = i + 1\n",
            "        }\n",
            "        Vec::push(w, 55)\n",
            "        Vec::push(w, 55)\n",
            "        Vec::push(w, 55)\n",
            "        0\n",
            "    }]\n",
            "    println(x)\n",
            "    println(w.len)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Balanced("11\n4\n"),
    },
    Case {
        name: "read_through_returned_reference_beside_a_freeing_call",
        src: concat!(
            "\n",
            "fn victim(n: i64) -> string { return \"ab\" + \"{n}\" }\n",
            "fn churn() -> Vec<string> {\n",
            "    let mut keep: Vec<string> = Vec::new()\n",
            "    let mut i: i64 = 10\n",
            "    while i < 50 {\n",
            "        Vec::push(keep, \"Q\" + \"{i}\")\n",
            "        i = i + 1\n",
            "    }\n",
            "    return keep\n",
            "}\n",
            "\n",
            "fn atc(v: &Vec<string>, i: i64) -> &string { return &(*v)[i] }\n",
            "fn at<T>(v: &Vec<T>, i: i64) -> &T { return &(*v)[i] }\n",
            "fn clob(v: &mut Vec<string>) -> i64 { (*v)[0] = victim(8); let k = churn(); return 1 }\n",
            "fn show2(s: string, n: i64) -> string { return s + \"{n}\" }\n",
            "fn main() -> i64 {\n",
            "    let mut v: Vec<string> = Vec::new()\n",
            "    Vec::push(v, victim(1))\n",
            "    let a: string = show2(*at(&v, 0), clob(&mut v))\n",
            "    let b: string = show2(*atc(&v, 0), clob(&mut v))\n",
            "    let k = churn()\n",
            "    println(a + b)\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Balanced("ab11ab81\n"),
    },
    Case {
        name: "index_store_with_a_borrowed_index",
        src: concat!(
            "struct Resource { id: i64 }\n",
            "fn eat(r: Resource) -> i64 { return r.id }\n",
            "fn mkA(n: i64) -> [i64; 3] { return [n, n + 1, n + 2] }\n",
            "fn rr(a: &[i64]) -> &[i64] { return a }\n",
            "fn rm(a: &mut [i64]) -> &mut [i64] { return a }\n",
            "fn ri(x: &mut i64) -> &mut i64 { return x }\n",
            "fn rc(x: &i64) -> &i64 { return x }\n",
            "fn main() -> i64 {\n",
            "    let mut a: [i64; 3] = [1, 2, 3]\n",
            "    a[rr(a[0..3])[0]] = 7\n",
            "    println(a[1])\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Balanced("7\n"),
    },
    Case {
        name: "slice_of_an_rc_payload",
        src: concat!(
            "\n",
            "fn main() -> i64 {\n",
            "    let base: [i64; 2] = [0, 0]\n",
            "    let mut s: &[i64] = base[0..2]\n",
            "    let mut t: &[i64] = base[0..2]\n",
            "    let mut i: i64 = 0\n",
            "    while i < 2 {\n",
            "        s = t\n",
            "        {\n",
            "            let a: Rc<[i64; 2]> = Rc::new([i + 1, 5])\n",
            "            t = Rc::get(a)[0..2]\n",
            "        }\n",
            "        i = i + 1\n",
            "    }\n",
            "    let p: Rc<[i64; 2]> = Rc::new([9, 9])\n",
            "    println(s[0] * 1000 + Rc::get(p)[0])\n",
            "    return 0\n",
            "}\n",
        ),
        module: None,
        expect: Expect::Balanced("1009\n"),
    },
];

#[test]
fn release01_98_a_managed_call_under_a_projection_refuses_its_nogc_caller() {
    check("release01_98", REFUSED);
}

#[test]
fn release01_99_an_allocation_free_call_under_a_projection_stays_accepted() {
    check("release01_99", ACCEPTED);
}

#[test]
fn release01_101_a_read_through_a_materialized_base_is_ordered_and_borrowed() {
    check("release01_101", FREED_READS);
}

#[test]
fn release01_102_a_call_under_a_projection_carries_its_loans() {
    check("release01_102", CONFLICTS);
}

#[test]
fn release01_103_source_ordered_projections_stay_accepted() {
    check("release01_103", ORDERED);
}

fn ir_bodies(ir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in ir.lines() {
        if let Some((name, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
            if line == "}" {
                out.push((std::mem::take(name), std::mem::take(body)));
                current = None;
            }
            continue;
        }
        if line.starts_with("define ")
            && let Some(at) = line.find('@')
        {
            let rest = line[at + 1..].trim_start_matches('"');
            let end = rest.find(['"', '(']).unwrap_or(rest.len());
            current = Some((rest[..end].to_string(), String::new()));
        }
    }
    out
}

fn callees(body: &str) -> Vec<String> {
    body.match_indices("call ")
        .filter_map(|(i, _)| {
            let rest = &body[i..];
            let line = rest.lines().next()?;
            let at = line.find('@')?;
            let name = line[at + 1..].trim_start_matches('"');
            let end = name.find(['"', '('])?;
            Some(name[..end].to_string())
        })
        .collect()
}

const MANAGED_RUNTIME: &[&str] = &[
    "__aelys_alloc",
    "__aelys_realloc",
    "__aelys_str_concat",
    "__aelys_str_from_char",
    "__aelys_str_substring_bytes",
    "__aelys_to_string_char",
    "__aelys_to_string_f64",
    "__aelys_to_string_i64",
    "__aelys_vec_init",
    "__aelys_vec_new",
    "__aelys_vec_grow",
    "__aelys_vec_push",
    "__aelys_vec_push_glue",
    "__aelys_vec_push_str",
    "__aelys_vec_pop",
    "__aelys_vec_pop_glue",
    "__aelys_vec_pop_str",
    "__aelys_vec_detach",
    "__aelys_vec_detach_glue",
    "__aelys_vec_detach_str",
    "__aelys_cycle_poll",
    "__aelys_cycle_collect",
    "__aelys_collect",
];

fn managed_from_nogc(ir: &str, nogc: &[String]) -> (Vec<String>, usize) {
    let bodies = ir_bodies(ir);
    let mut stack: Vec<String> = bodies
        .iter()
        .map(|(n, _)| n.clone())
        .filter(|n| nogc.contains(n))
        .collect();
    let mut seen = HashSet::new();
    let mut polled = Vec::new();
    while let Some(name) = stack.pop() {
        let Some((_, body)) = bodies.iter().find(|(n, _)| *n == name) else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        for symbol in MANAGED_RUNTIME {
            if body.contains(&format!("@{symbol}(")) {
                polled.push(format!("{name} calls {symbol}"));
            }
        }
        stack.extend(callees(body));
    }
    (polled, seen.len())
}

fn nogc_names(src: &str) -> Vec<String> {
    src.split("nogc fn ")
        .skip(1)
        .filter_map(|rest| rest.split(['(', '<']).next())
        .map(|name| name.trim().to_string())
        .collect()
}

#[test]
fn release01_100_no_function_reachable_from_a_nogc_function_polls_or_allocates_in_the_llvm_ir() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    let mut reached = 0;
    for case in REFUSED.iter().chain(ACCEPTED.iter()) {
        let nogc = nogc_names(case.src);
        for (tag, level) in LEVELS {
            let stage = dir.path().join(format!("{}_{}", case.name, &tag[1..]));
            fs::create_dir_all(&stage).expect("stage dir");
            let root = stage.join("root.aelys");
            fs::write(&root, case.src).expect("write root");
            if let Some((file, text)) = case.module {
                fs::write(stage.join(file), text).expect("write module");
            }
            let accepted =
                compile_file_with_llvm_variant(&root, *level, true, RuntimeVariant::RcCycles)
                    .is_ok();
            if accepted {
                let ir = fs::read_to_string(root.with_extension("ll")).expect("read emitted ir");
                let (polled, seen) = managed_from_nogc(&ir, &nogc);
                if !polled.is_empty() {
                    failures.push(format!(
                        "{} at {tag}: reached from nogc {polled:?}",
                        case.name
                    ));
                }
                reached += seen;
            }
            let expected = matches!(case.expect, Expect::Balanced(_));
            if accepted != expected {
                failures.push(format!("{} at {tag}: accepted={accepted}", case.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "release01_100:\n{}",
        failures.join("\n")
    );
    assert!(reached >= 20, "functions reached from nogc: {reached}");
}
