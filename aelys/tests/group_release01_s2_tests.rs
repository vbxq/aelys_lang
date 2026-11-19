#[path = "level_legs/mod.rs"]
mod level_legs;

use aelys_air::passes::validate::{AirValidationDetail, validate_air};
use aelys_air::*;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::OptimizationLevel;
use aelys_opt::OptimizationPass;
use aelys_opt::passes::{GlobalConstantPropagator, LocalConstantPropagator};
use aelys_sema::TypedStmtKind;
use aelys_sema::{InferType, TypeInference, TypedExpr, TypedExprKind, TypedProgram, TypedStmt};
use aelys_syntax::Source;
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

enum Expect {
    Runs { stdout: &'static str, exit: i32 },
    Refused(&'static str),
}

struct Member {
    name: &'static str,
    src: &'static str,
    module: Option<(&'static str, &'static str)>,
    expect: Expect,
}

fn run_row(row: &str, members: &[Member]) {
    let dir = tempdir().expect("tempdir");
    let mut legs_run = 0;
    let mut disagreements = Vec::new();
    for member in members {
        let legs = level_legs::legs_of(dir.path(), member.name, member.src, member.module, &[]);
        for (tag, leg) in &legs {
            match (&member.expect, leg) {
                (Expect::Refused(code), level_legs::LevelLeg::Refused(rendered)) => {
                    assert!(
                        rendered.contains(code),
                        "{row} {} at {tag}: refusal must be {code}\n{rendered}",
                        member.name
                    );
                }
                (Expect::Refused(code), other) => {
                    panic!(
                        "{row} {} at {tag}: expected {code}, got {}",
                        member.name,
                        other.render()
                    )
                }
                (Expect::Runs { .. }, level_legs::LevelLeg::Refused(rendered)) => {
                    panic!("{row} {} at {tag}: refused\n{rendered}", member.name)
                }
                (Expect::Runs { .. }, level_legs::LevelLeg::DidNotTerminate) => {
                    panic!("{row} {} at {tag}: did not terminate", member.name);
                }
                (
                    Expect::Runs { stdout, exit },
                    level_legs::LevelLeg::Ran {
                        stdout: out,
                        exit: code,
                        ..
                    },
                ) => {
                    assert_eq!(
                        (out.as_str(), *code),
                        (*stdout, *exit),
                        "{row} {} at {tag}: stdout and exit code",
                        member.name
                    );
                }
            }
            legs_run += 1;
        }
        disagreements.extend(level_legs::level_disagreements(member.name, &legs));
    }
    assert!(
        disagreements.is_empty(),
        "{row}: levels disagree\n{}",
        disagreements.join("\n")
    );
    assert_eq!(legs_run, members.len() * LEVELS.len(), "{row}: legs run");
}

const ROW_42: &[Member] = &[
    Member {
        name: "m1_mul_local",
        src: r#"fn main() {
    let a: u8 = 250;
    let b: i64 = 1000;
    let r = b * 10 + a;
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10250\n",
            exit: 0,
        },
    },
    Member {
        name: "m2_mul_param",
        src: r#"fn f(a: u8, b: i64) -> i64 { b * 10 + a }
fn main() { println!("{}", f(250, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10250\n",
            exit: 0,
        },
    },
    Member {
        name: "m3_paren",
        src: r#"fn f(a: u8, b: i64) -> i64 { (b * 10) + a }
fn main() { println!("{}", f(250, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10250\n",
            exit: 0,
        },
    },
    Member {
        name: "m4_sub",
        src: r#"use std::process::ExitCode; fn f(a: u8, b: i64) -> i64 { b - 1 - a }
fn main() -> ExitCode { return ExitCode::from(f(1, 2) as u8); }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 0,
        },
    },
    Member {
        name: "m5_cmp",
        src: r#"use std::process::ExitCode; fn f(a: u8, b: i64) -> bool { b + 1 == a }
fn main() -> ExitCode { if f(3, 2) { return ExitCode::from(1); }
    return ExitCode::SUCCESS; }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
    Member {
        name: "m6_i32",
        src: r#"use std::process::ExitCode; fn f(a: i32, b: i64) -> i64 { b * 10 + a }
fn main() -> ExitCode { return ExitCode::from(f(1, 2) as u8); }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 21,
        },
    },
    Member {
        name: "m7_match",
        src: r#"use std::process::ExitCode; enum E { A(u8, i64) }
fn get(e: E) -> i64 { match e { E::A(a, b) => { return b * 10 + a; } } }
fn main() -> ExitCode { return ExitCode::from(get(E::A(1, 2)) as u8); }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 21,
        },
    },
    Member {
        name: "m8_cast_lit",
        src: r#"use std::process::ExitCode; fn f(b: i64) -> i64 { b * 3 + (1 as u8) }
fn main() -> ExitCode { return ExitCode::from(f(2) as u8); }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 7,
        },
    },
];

const ROW_43: &[Member] = &[
    Member {
        name: "a_shadow",
        src: r#"fn f(a: u8, b: i64) -> i64 { let k = 10;
    let k = b;
    let r = k + a;
    r }
fn main() { println!("{}", f(3, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "b_shadow_lit_ann",
        src: r#"fn f(a: u8, b: i64) -> i64 { let k = 10;
    let k: i64 = b;
    k + a }
fn main() { println!("{}", f(3, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "c_for",
        src: r#"fn f(a: u8) -> i64 { let k = 10;
    let mut s: i64 = 0;
    for k in 0..300 { s = k + a; }
    s }
fn main() { println!("{}", f(3));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "302\n",
            exit: 0,
        },
    },
    Member {
        name: "d_closure",
        src: r#"fn f(a: u8) -> i64 { let k = 10;
    let g = move |k: i64| -> i64 { k + a };
    g(1000) }
fn main() { println!("{}", f(3));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "e_match_bind",
        src: r#"enum E { A(i64) }
fn f(a: u8, e: E) -> i64 { let k = 10;
    match e { E::A(k) => { return k + a; } } }
fn main() { println!("{}", f(3, E::A(1000)));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "f_inner_block_shadow",
        src: r#"fn f(a: u8, b: i64) -> i64 { let k = 10;
    let mut r: i64 = 0;
    if true { let k = b;
        r = k + a; }
    r }
fn main() { println!("{}", f(3, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "x2_copy_then_shadow",
        src: r#"fn main() { let k = 7;
    let j = k;
    let k = 1000;
    let y: u8 = j;
    println!("{y} {k}");
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "7 1000\n",
            exit: 0,
        },
    },
    Member {
        name: "x12_block_expr_scope",
        src: r#"use std::process::ExitCode; fn main() -> ExitCode {
    let x = 5;
    let y = {
        let x = 10;  // shadows outer x
        x * 2
    };
    // x should still be 5 here (inner x was scoped to block)
    if x == 5 && y == 20 {
        return ExitCode::from(1);
    }
    return ExitCode::SUCCESS;
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
];

const ROW_44: &[Member] = &[
    Member {
        name: "g_shadow_u8_ctx",
        src: r#"fn f(a: u8, b: i64) -> u8 { let k = 10;
    let k = b;
    let r: u8 = k;
    r + a }
fn main() { println!("{}", f(3, 1000));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "h_shadow_u8_min",
        src: r#"use std::process::ExitCode; fn f(b: i64) -> i64 { let k = 10;
    let k = b;
    let r: u8 = k;
    println!("{r}");
    0 }
fn main() -> ExitCode { return ExitCode::from(f(1000) as u8); }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "i_closure_param_u8",
        src: r#"fn main() { let k = 10;
    let g = move |k: i64| -> i64 { let r: u8 = k;
        r };
    println!("{}", g(100000));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "j_for_u8",
        src: r#"fn main() { let k = 10;
    for k in 250..260 { let r: u8 = k;
        println!("{r}"); }
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "k_global_shadow",
        src: r#"static k: i64 = 10;
fn f(k: i64) -> i64 { let r: u8 = k;
    r }
fn main() { println!("{}", f(1000));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "x3_block_tail_shadow",
        src: r#"fn f(b: i64) -> u8 { let k = 10;
    let y: u8 = { let k = b;
        k };
    y }
fn main() { println!("{}", f(1000));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "x4_match_arm_shadow",
        src: r#"enum E { A(i64), B }
fn f(e: E) -> u8 { let k = 10;
    let y: u8 = match e { E::A(k) => k, E::B => 1 };
    y }
fn main() { println!("{}", f(E::A(1000)));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "x11_block_expr_shadow",
        src: r#"fn f(b: i64) -> u8 { let k = b;
    let y = { let k = 10;
        k * 2 };
    let z: u8 = k;
    println!("{y}");
    z }
fn main() { println!("{}", f(1000));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "o_if_lit_init",
        src: r#"use std::process::ExitCode; fn f(c: bool) -> i64 { let k = if c { 1 } else { 2 };
    let y: u8 = k;
    println!("{y}");
    0 }
fn main() -> ExitCode { return ExitCode::from(f(true) as u8); }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
];

const ROW_46: &[Member] = &[
    Member {
        name: "a_plain",
        src: r#"fn f(a: u8, b: i64) -> i64 { b + a }
fn main() { println!("{}", f(250, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1250\n",
            exit: 0,
        },
    },
    Member {
        name: "b_lit_left",
        src: r#"fn f(a: u8) -> i64 { 10 + a }
fn main() { println!("{}", f(250));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
    Member {
        name: "c_lit_u8_overflow",
        src: r#"fn f(a: u8) -> u8 { a + 10 }
fn main() { println!("{}", f(250));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
    Member {
        name: "d_cmp",
        src: r#"use std::process::ExitCode; fn f(a: u8, b: i64) -> bool { b < a }
fn main() -> ExitCode { if f(3, 2) { return ExitCode::from(1); }
    return ExitCode::SUCCESS; }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
    Member {
        name: "e_cmp_lit",
        src: r#"use std::process::ExitCode; fn f(a: u8, b: i64) -> bool { b * 2 < a }
fn main() -> ExitCode { if f(3, 1) { return ExitCode::from(1); }
    return ExitCode::SUCCESS; }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
    Member {
        name: "f_mod",
        src: r#"fn f(a: u8, b: i64) -> i64 { b % 7 + a }
fn main() { println!("{}", f(3, 100));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "5\n",
            exit: 0,
        },
    },
    Member {
        name: "g_bitand",
        src: r#"fn f(a: u8, b: i64) -> i64 { (b & 255) | a }
fn main() { println!("{}", f(3, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "235\n",
            exit: 0,
        },
    },
    Member {
        name: "h_shift",
        src: r#"fn f(a: u8, b: i64) -> i64 { b << a }
fn main() { println!("{}", f(3, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "8000\n",
            exit: 0,
        },
    },
    Member {
        name: "h2_shift_lit",
        src: r#"fn f(a: u8, b: i64) -> i64 { (b << 2) + a }
fn main() { println!("{}", f(3, 1000));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4003\n",
            exit: 0,
        },
    },
    Member {
        name: "k_neg",
        src: r#"fn f(a: u8, b: i64) -> i64 { -b * 2 + a }
fn main() { println!("{}", f(3, 10));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "-17\n",
            exit: 0,
        },
    },
    Member {
        name: "l_float",
        src: r#"fn f(a: f32, b: f64) -> f64 { b * 2.0 + a }
fn main() { println!("{}", f(1.5, 2.0));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "s_i32_u8",
        src: r#"fn f(a: u8, b: i32) -> i32 { b * 2 + a }
fn main() { println!("{}", f(3, 7));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "17\n",
            exit: 0,
        },
    },
    Member {
        name: "t_u64_i64",
        src: r#"fn f(a: u64, b: i64) -> i64 { b * 2 + a }
fn main() { println!("{}", f(3, 7));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "k_u8_i8",
        src: r#"fn f(a: u8, c: i8) -> i64 { a + c }
fn main() { println!("{}", f(200, -1));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "m_u32_i32",
        src: r#"fn f(a: u32, c: i32) -> i64 { a + c }
fn main() { println!("{}", f(1, 2));
 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "m_shift_u8_lit",
        src: r#"fn f(a: u8) -> i64 { a << 9 }
fn main() { println!("{}", f(1));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2\n",
            exit: 0,
        },
    },
    Member {
        name: "x5_shift_literal_left",
        src: r#"fn f(n: i64) -> i64 { let y: u8 = 1 << n;
    y }
fn main() { println!("{}", f(3));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "8\n",
            exit: 0,
        },
    },
    Member {
        name: "x8_shift_amount_wide",
        src: r#"fn f(a: u8, n: i64) -> i64 { a << n }
fn main() { println!("{}", f(1, 3));
 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "8\n",
            exit: 0,
        },
    },
];

const ROW_51: &[Member] = &[
    Member {
        name: "f2",
        src: r#"fn id8(x: u8) -> u8 { x }
fn big() -> i64 { 100 }
fn main() {
    let k = 7;
    let v = big();
    println!("{}", id8({ let k = v; k } + k));
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f5",
        src: r#"enum E { A(i64), B }
fn id8(x: u8) -> u8 { x }
fn mk(n: i64) -> E { if n > 0 { return E::A(n); } E::B }
fn main() {
    let k = 7;
    let e = mk(100);
    println!("{}", id8(match e { E::A(k) => k, E::B => k }));
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f6",
        src: r#"enum E { A(i64), B }
fn id8(x: u8) -> u8 { x }
fn mk(n: i64) -> E { if n > 0 { return E::A(n); } E::B }
fn main() {
    let k = 7;
    let e = mk(300);
    println!("{}", id8(match e { E::A(k) => k, E::B => k }));
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f7",
        src: r#"enum E { A(i64), B }
fn id8(x: u8) -> u8 { x }
fn mk(n: i64) -> E { if n > 0 { return E::A(n); } E::B }
fn main() {
    let k = 7;
    let e = mk(100);
    let r = id8(match e { E::A(k) => k, E::B => k });
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f8",
        src: r#"enum E { A(i64), B }
fn mk(n: i64) -> E { if n > 0 { return E::A(n); } E::B }
fn main() {
    let k = 7;
    let e = mk(1000);
    let y: u8 = 1;
    println!("{}", y + match e { E::A(k) => k, E::B => k });
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1001\n",
            exit: 0,
        },
    },
];

const ROW_53: &[Member] = &[
    Member {
        name: "rf1",
        src: r#"fn g(r: &i64) -> i64 { *r + 1 }
fn main() {
    let k = 10;
    let p = &k;
    let a = g(&k);
    let b = g(&(k));
    let c = g(p);
    println!("{a} {b} {c} {}", *p);
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "11 11 11 10\n",
            exit: 0,
        },
    },
    Member {
        name: "shp1",
        src: r#"fn main() {
    let a: i32 = 250;
    let r = a << (7);
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "32000\n",
            exit: 0,
        },
    },
    Member {
        name: "shp2",
        src: r#"fn main() {
    let a: u8 = 3;
    let r = a << (1 + 1);
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "12\n",
            exit: 0,
        },
    },
    Member {
        name: "shp3",
        src: r#"fn main() {
    let a: i32 = 250;
    let k = 7;
    let r = a << (k);
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "32000\n",
            exit: 0,
        },
    },
    Member {
        name: "shp4",
        src: r#"fn main() {
    let a: u8 = 3;
    let r = a << -(-2);
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "12\n",
            exit: 0,
        },
    },
    Member {
        name: "sh_aggk",
        src: r#"fn main() {
    let a: i32 = 250;
    let k = 7;
    let r = a >> (k);
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1\n",
            exit: 0,
        },
    },
    Member {
        name: "sh_all7",
        src: r#"fn main() {
    let a: i32 = 250;
    let k = 7;
    let r = a << (7);
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "32000\n",
            exit: 0,
        },
    },
    Member {
        name: "gp1",
        src: r#"fn main() {
    let a: i32 = 2147483647;
    let r = a + (1);
    let s = a + 1;
    println!("{r} {s}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "-2147483648 -2147483648\n",
            exit: 0,
        },
    },
    Member {
        name: "gp2",
        src: r#"fn main() {
    let a: u8 = 255;
    let r: u8 = a + (1);
    println!("{r}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "0\n",
            exit: 0,
        },
    },
    Member {
        name: "gp3",
        src: r#"fn main() {
    let a: u8 = 255;
    let k = 1;
    let r = a + (k);
    let s = a + k;
    println!("{r} {s}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "0 0\n",
            exit: 0,
        },
    },
];

const ROW_54: &[Member] = &[
    Member {
        name: "t1",
        src: r#"fn main() {
    let x: u8 = 1;
    let n: i64 = 9;
    let y: i64 = x << n;
    println!("{y}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2\n",
            exit: 0,
        },
    },
    Member {
        name: "p8",
        src: r#"fn main() {
    let a: u8 = 1;
    let n: i64 = 9;
    let y: u8 = 1 << n;
    let z: i64 = a << n;
    let w: i64 = 1 << n;
    println!("{y} {z} {w}");
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2 2 512\n",
            exit: 0,
        },
    },
    Member {
        name: "i8_0_lit_shiftamt_558",
        src: r#"struct S { f: i8 }
struct T { g: i64 }
fn idc(x: i64) -> i64 { x }
fn gb() -> i8 { 3 }
static G: i64 = 3;
fn retc(a: i8, p: i8) -> i64 { let s = S { f: p }; let arr = [p, p];  a + 3 }
fn probe(a: i8, p: i8) {
    let s = S { f: p };
    let arr = [p, p];
    let r = 1 << ((a + 3) % 8);
    println!("{r}");
}
fn main() { probe(100, 3); }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "128\n",
            exit: 0,
        },
    },
    Member {
        name: "i8_6_cast_shiftamt_858",
        src: r#"struct S { f: i8 }
struct T { g: i64 }
fn idc(x: i64) -> i64 { x }
fn gb() -> i8 { 3 }
static G: i64 = 3;
fn retc(a: i8, p: i8) -> i64 { let s = S { f: p }; let arr = [p, p];  a | (3 as i8) }
fn probe(a: i8, p: i8) {
    let s = S { f: p };
    let arr = [p, p];
    let r = 1 << ((a | (3 as i8)) % 8);
    println!("{r}");
}
fn main() { probe(100, 3); }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "128\n",
            exit: 0,
        },
    },
];
#[test]
fn release01_42_a_mixed_width_binary_with_a_literal_operand_computes_at_the_widest_operand() {
    run_row("release01_42", ROW_42);
}

#[test]
fn release01_43_a_shadowing_binding_never_inherits_the_literal_init_of_the_name() {
    run_row("release01_43", ROW_43);
}

#[test]
fn release01_44_a_non_literal_binding_never_narrows_into_a_smaller_integer() {
    run_row("release01_44", ROW_44);
}

#[test]
fn release01_46_the_binary_width_rule_holds_for_every_operator() {
    run_row("release01_46", ROW_46);
}

#[test]
fn release01_51_a_binding_use_inside_a_format_string_resolves_in_its_own_scope() {
    run_row("release01_51", ROW_51);
}

#[test]
fn release01_53_a_parenthesized_literal_narrows_as_the_bare_literal() {
    run_row("release01_53", ROW_53);
}

#[test]
fn release01_54_a_shift_has_the_type_of_its_left_operand() {
    run_row("release01_54", ROW_54);
}

fn attribs() -> FunctionAttribs {
    FunctionAttribs {
        inline: InlineHint::Default,
        no_gc: false,
        no_unwind: false,
        cold: false,
    }
}

fn local(id: u32, ty: AirType, name: &str) -> AirLocal {
    AirLocal {
        id: LocalId(id),
        ty,
        name: Some(name.to_string()),
        is_mut: false,
        span: None,
    }
}

fn function(
    id: u32,
    name: &str,
    params: Vec<AirParam>,
    stmts: Vec<AirStmtKind>,
    ret: Operand,
) -> AirFunction {
    AirFunction {
        id: FunctionId(id),
        name: name.to_string(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params,
        ret_ty: AirType::I64,
        locals: vec![local(0, AirType::I64, "_ret"), local(2, AirType::I64, "t")],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: stmts
                .into_iter()
                .map(|kind| AirStmt { kind, span: None })
                .collect(),
            terminator: AirTerminator::Return(Some(ret)),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: attribs(),
        span: None,
    }
}

fn program_with(stmts: Vec<AirStmtKind>, ret: Operand) -> AirProgram {
    let param = AirParam {
        id: LocalId(1),
        ty: AirType::I64,
        name: "b".to_string(),
        span: None,
    };
    let callee_param = AirParam {
        id: LocalId(1),
        ty: AirType::I64,
        name: "x".to_string(),
        span: None,
    };
    AirProgram {
        impls: Vec::new(),
        functions: vec![
            function(0, "f", vec![param], stmts, ret),
            function(
                1,
                "g",
                vec![callee_param],
                vec![],
                Operand::Const(AirConst::Int(0, AirIntSize::I64)),
            ),
        ],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
        dyn_tables: Default::default(),
    }
}

fn int(v: i64, size: AirIntSize) -> Operand {
    Operand::Const(AirConst::Int(v, size))
}

fn assign(rvalue: Rvalue) -> AirStmtKind {
    AirStmtKind::Assign {
        place: Place::Local(LocalId(2)),
        rvalue,
    }
}

fn disagreements(program: &AirProgram) -> Vec<String> {
    match validate_air(program) {
        Ok(()) => Vec::new(),
        Err(errors) => errors
            .into_iter()
            .filter_map(|e| match e.detail {
                AirValidationDetail::TypeAgreement { detail, .. } => Some(detail),
                _ => None,
            })
            .collect(),
    }
}

#[test]
fn release01_49_validate_air_rejects_operands_whose_scalar_types_disagree() {
    let agreeing = program_with(
        vec![
            assign(Rvalue::BinaryOp(
                BinOp::Mul,
                Operand::Copy(LocalId(1)),
                int(10, AirIntSize::I64),
            )),
            assign(Rvalue::BinaryOp(
                BinOp::Add,
                Operand::Copy(LocalId(1)),
                Operand::Const(AirConst::IntLiteral(1)),
            )),
            assign(Rvalue::Call {
                func: Callee::Direct(FunctionId(1)),
                args: vec![int(1, AirIntSize::U64)],
            }),
        ],
        int(0, AirIntSize::I64),
    );
    assert_eq!(disagreements(&agreeing), Vec::<String>::new());

    let cases: Vec<(&str, Vec<AirStmtKind>, Operand)> = vec![
        (
            "binary operands",
            vec![assign(Rvalue::BinaryOp(
                BinOp::Mul,
                Operand::Copy(LocalId(1)),
                int(10, AirIntSize::U8),
            ))],
            int(0, AirIntSize::I64),
        ),
        (
            "store",
            vec![assign(Rvalue::Use(int(1, AirIntSize::U8)))],
            int(0, AirIntSize::I64),
        ),
        ("return", vec![], int(0, AirIntSize::I32)),
        (
            "call argument #0",
            vec![assign(Rvalue::Call {
                func: Callee::Direct(FunctionId(1)),
                args: vec![int(1, AirIntSize::U8)],
            })],
            int(0, AirIntSize::I64),
        ),
    ];
    for (what, stmts, ret) in cases {
        let found = disagreements(&program_with(stmts, ret));
        assert!(
            found.iter().any(|d| d.starts_with(what)),
            "{what}: expected a type disagreement, got {found:?}"
        );
    }
}

fn typed(code: &str) -> TypedProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex");
    let stmts = Parser::new(tokens, src.clone()).parse().expect("parse");
    TypeInference::infer_program(stmts, src).expect("sema")
}

fn visit_idents(stmts: &mut [TypedStmt], name: &str, f: &mut dyn FnMut(&mut TypedExpr)) {
    fn expr(e: &mut TypedExpr, name: &str, f: &mut dyn FnMut(&mut TypedExpr)) {
        match &mut e.kind {
            TypedExprKind::Identifier(n) if n == name => f(e),
            TypedExprKind::Binary { left, right, .. } => {
                expr(left, name, f);
                expr(right, name, f);
            }
            TypedExprKind::Int(_) => f(e),
            _ => {}
        }
    }
    for stmt in stmts {
        match &mut stmt.kind {
            TypedStmtKind::Function(func) => visit_idents(&mut func.body, name, f),
            TypedStmtKind::Return(Some(e)) => expr(e, name, f),
            _ => {}
        }
    }
}

fn propagated_types(mut program: TypedProgram, pass: &mut dyn OptimizationPass) -> Vec<InferType> {
    let mut stale = 0;
    visit_idents(&mut program.stmts, "k", &mut |e| {
        if matches!(e.kind, TypedExprKind::Identifier(_)) {
            e.ty = InferType::U8;
            stale += 1;
        }
    });
    assert_eq!(stale, 1, "the use of `k` is present before propagation");
    pass.run(&mut program);
    let mut found = Vec::new();
    visit_idents(&mut program.stmts, "k", &mut |e| {
        if let TypedExprKind::Int(10) = e.kind {
            found.push(e.ty.clone());
        }
    });
    found
}

#[test]
fn release01_50_constant_propagation_substitutes_a_literal_of_the_binding_type() {
    let local = typed("fn f(b: i64) -> i64 { let k: i64 = 10;\n    b * k }\n");
    assert_eq!(
        propagated_types(local, &mut LocalConstantPropagator::new()),
        vec![InferType::I64]
    );
    let global = typed("static k: i64 = 10;\nfn f(b: i64) -> i64 { b * k }\n");
    assert_eq!(
        propagated_types(global, &mut GlobalConstantPropagator::new()),
        vec![InferType::I64]
    );
}
