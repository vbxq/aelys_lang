use aelys_air::passes::validate::{AirValidationDetail, validate_air};
use aelys_air::*;
use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::OptimizationLevel;
use aelys_opt::OptimizationPass;
use aelys_opt::passes::{GlobalConstantPropagator, LocalConstantPropagator};
use aelys_sema::TypedStmtKind;
use aelys_sema::{InferType, TypeInference, TypedExpr, TypedExprKind, TypedProgram, TypedStmt};
use aelys_syntax::Source;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const RUN_LIMIT: Duration = Duration::from_secs(10);

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

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

fn run_bounded(exe: &Path) -> Option<(String, i32)> {
    let mut child = Command::new(exe)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn compiled exe");
    let stdout = child.stdout.take().expect("child stdout");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let mut stdout = stdout;
        let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
        text
    });
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll compiled exe") {
            let text = reader.join().expect("stdout reader");
            return Some((text, exit_code(&status)));
        }
        if start.elapsed() > RUN_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_row(row: &str, members: &[Member]) {
    let dir = tempdir().expect("tempdir");
    let mut legs = 0;
    for member in members {
        for (tag, opt) in LEVELS {
            let stage = dir.path().join(format!("{}_{}", member.name, &tag[1..]));
            fs::create_dir_all(&stage).expect("stage dir");
            let root = stage.join("root.aelys");
            fs::write(&root, member.src).expect("write root");
            if let Some((file, text)) = member.module {
                fs::write(stage.join(file), text).expect("write module");
            }
            let verdict = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc);
            match (&member.expect, verdict) {
                (Expect::Refused(code), Err(err)) => {
                    let rendered = err.to_string();
                    assert!(
                        rendered.contains(code),
                        "{row} {} at {tag}: refusal must be {code}\n{rendered}",
                        member.name
                    );
                }
                (Expect::Refused(code), Ok(())) => {
                    panic!("{row} {} at {tag}: accepted, expected {code}", member.name)
                }
                (Expect::Runs { .. }, Err(err)) => {
                    panic!("{row} {} at {tag}: refused\n{err}", member.name)
                }
                (Expect::Runs { stdout, exit }, Ok(())) => {
                    let Some((out, code)) = run_bounded(&exe_path_for(&root)) else {
                        panic!("{row} {} at {tag}: did not terminate", member.name);
                    };
                    assert_eq!(
                        (out.as_str(), code),
                        (*stdout, *exit),
                        "{row} {} at {tag}: stdout and exit code",
                        member.name
                    );
                }
            }
            legs += 1;
        }
    }
    assert_eq!(legs, members.len() * LEVELS.len(), "{row}: legs run");
}

const ROW_42: &[Member] = &[
    Member {
        name: "m1_mul_local",
        src: r#"fn main() -> i64 {
    let a: u8 = 250
    let b: i64 = 1000
    let r = b * 10 + a
    println("{r}")
    return 0
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
        src: r#"fn f(a: u8, b: i64) -> i64 { return b * 10 + a }
fn main() -> i64 { println("{f(250, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10250\n",
            exit: 0,
        },
    },
    Member {
        name: "m3_paren",
        src: r#"fn f(a: u8, b: i64) -> i64 { return (b * 10) + a }
fn main() -> i64 { println("{f(250, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10250\n",
            exit: 0,
        },
    },
    Member {
        name: "m4_sub",
        src: r#"fn f(a: u8, b: i64) -> i64 { return b - 1 - a }
fn main() -> i64 { return f(1, 2) }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 0,
        },
    },
    Member {
        name: "m5_cmp",
        src: r#"fn f(a: u8, b: i64) -> bool { return b + 1 == a }
fn main() -> i64 { if f(3, 2) { return 1 }
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
    Member {
        name: "m6_i32",
        src: r#"fn f(a: i32, b: i64) -> i64 { return b * 10 + a }
fn main() -> i64 { return f(1, 2) }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 21,
        },
    },
    Member {
        name: "m7_match",
        src: r#"enum E { A(u8, i64) }
fn get(e: E) -> i64 { match e { E::A(a, b) => { return b * 10 + a } } }
fn main() -> i64 { return get(E::A(1, 2)) }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 21,
        },
    },
    Member {
        name: "m8_cast_lit",
        src: r#"fn f(b: i64) -> i64 { return b * 3 + (1 as u8) }
fn main() -> i64 { return f(2) }
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
        src: r#"fn f(a: u8, b: i64) -> i64 { let k = 10
    let k = b
    let r = k + a
    return r }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "b_shadow_lit_ann",
        src: r#"fn f(a: u8, b: i64) -> i64 { let k = 10
    let k: i64 = b
    return k + a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "c_for",
        src: r#"fn f(a: u8) -> i64 { let k = 10
    let mut s: i64 = 0
    for k in 0..300 { s = k + a }
    return s }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "302\n",
            exit: 0,
        },
    },
    Member {
        name: "d_closure",
        src: r#"fn f(a: u8) -> i64 { let k = 10
    let g = fn(k: i64) -> i64 { return k + a }
    return g(1000) }
fn main() -> i64 { println("{f(3)}")
    return 0 }
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
fn f(a: u8, e: E) -> i64 { let k = 10
    match e { E::A(k) => { return k + a } } }
fn main() -> i64 { println("{f(3, E::A(1000))}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "f_inner_block_shadow",
        src: r#"fn f(a: u8, b: i64) -> i64 { let k = 10
    let mut r: i64 = 0
    if true { let k = b
        r = k + a }
    return r }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "x2_copy_then_shadow",
        src: r#"fn main() -> i64 { let k = 7
    let j = k
    let k = 1000
    let y: u8 = j
    println("{y} {k}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "7 1000\n",
            exit: 0,
        },
    },
    Member {
        name: "x12_block_expr_scope",
        src: r#"fn main() -> i64 {
    let x = 5
    let y = {
        let x = 10  // shadows outer x
        x * 2
    }
    // x should still be 5 here (inner x was scoped to block)
    if x == 5 && y == 20 {
        return 1
    }
    return 0
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
        src: r#"fn f(a: u8, b: i64) -> u8 { let k = 10
    let k = b
    let r: u8 = k
    return r + a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "h_shadow_u8_min",
        src: r#"fn f(b: i64) -> i64 { let k = 10
    let k = b
    let r: u8 = k
    println("{r}")
    return 0 }
fn main() -> i64 { return f(1000) }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "i_closure_param_u8",
        src: r#"fn main() -> i64 { let k = 10
    let g = fn(k: i64) -> i64 { let r: u8 = k
        return r }
    println("{g(100000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "j_for_u8",
        src: r#"fn main() -> i64 { let k = 10
    for k in 250..260 { let r: u8 = k
        println("{r}") }
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "k_global_shadow",
        src: r#"let k = 10
fn f(k: i64) -> i64 { let r: u8 = k
    return r }
fn main() -> i64 { println("{f(1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "x3_block_tail_shadow",
        src: r#"fn f(b: i64) -> u8 { let k = 10
    let y: u8 = { let k = b
        k }
    return y }
fn main() -> i64 { println("{f(1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "x4_match_arm_shadow",
        src: r#"enum E { A(i64), B }
fn f(e: E) -> u8 { let k = 10
    let y: u8 = match e { E::A(k) => k, E::B => 1 }
    return y }
fn main() -> i64 { println("{f(E::A(1000))}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "x11_block_expr_shadow",
        src: r#"fn f(b: i64) -> u8 { let k = b
    let y = { let k = 10
        k * 2 }
    let z: u8 = k
    println("{y}")
    return z }
fn main() -> i64 { println("{f(1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "o_if_lit_init",
        src: r#"fn f(c: bool) -> i64 { let k = if c { 1 } else { 2 }
    let y: u8 = k
    println("{y}")
    return 0 }
fn main() -> i64 { return f(true) }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
];

const ROW_45: &[Member] = &[
    Member {
        name: "d_ident_lit",
        src: r#"fn f(a: u8) -> i64 { let k = 10
    let r = k + a
    return r }
fn main() -> i64 { println("{f(250)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
    Member {
        name: "e_ident_mut",
        src: r#"fn f(a: u8) -> i64 { let mut k = 10
    k = 1000
    let r = k + a
    return r }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "e2_ident_mut_loop",
        src: r#"fn f(a: u8) -> i64 { let mut k = 0
    let mut i = 0
    while i < 300 { k = k + 1
        i = i + 1 }
    return k + a }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "303\n",
            exit: 0,
        },
    },
    Member {
        name: "f_ident_mixed",
        src: r#"fn f(a: u8, b: i64) -> i64 { let k = 10
    return b * k + a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10003\n",
            exit: 0,
        },
    },
    Member {
        name: "h_global_lit",
        src: r#"let K = 10
fn f(a: u8) -> i64 { return K + a }
fn main() -> i64 { println("{f(250)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
    Member {
        name: "k_f32_mixed",
        src: r#"fn f(a: f32, x: f32) -> f32 { let k = 2.5
    return a * k + x }
fn main() -> i64 { println("{f(1.5, 1.0)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4.75\n",
            exit: 0,
        },
    },
    Member {
        name: "l_f32_shadow",
        src: r#"fn f(a: f32, b: f64) -> f64 { let k = 2.5
    let k = b
    return k + a }
fn main() -> i64 { println("{f(1.5, 1.0)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "p_copy_lit",
        src: r#"fn main() -> i64 { let k = 7
    let j = k
    let y: u8 = j
    println("{y}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "7\n",
            exit: 0,
        },
    },
    Member {
        name: "x1_mod_literal_global",
        src: r#"needs m
fn main() -> i64 { let a: u8 = 250
    let y: u8 = m.K
    println("{m.K + a} {y}")
    return 0 }
"#,
        module: Some((
            r#"m.aelys"#,
            r#"pub let K = 10
"#,
        )),
        expect: Expect::Runs {
            stdout: "4 10\n",
            exit: 0,
        },
    },
    Member {
        name: "x7_closure_literal",
        src: r#"fn main() -> i64 { let k = 10
    let g = fn(a: u8) -> u8 { return a + k }
    println("{g(250)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
];

const ROW_46: &[Member] = &[
    Member {
        name: "a_plain",
        src: r#"fn f(a: u8, b: i64) -> i64 { return b + a }
fn main() -> i64 { println("{f(250, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1250\n",
            exit: 0,
        },
    },
    Member {
        name: "b_lit_left",
        src: r#"fn f(a: u8) -> i64 { return 10 + a }
fn main() -> i64 { println("{f(250)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
    Member {
        name: "c_lit_u8_overflow",
        src: r#"fn f(a: u8) -> u8 { return a + 10 }
fn main() -> i64 { println("{f(250)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
    Member {
        name: "d_cmp",
        src: r#"fn f(a: u8, b: i64) -> bool { return b < a }
fn main() -> i64 { if f(3, 2) { return 1 }
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
    Member {
        name: "e_cmp_lit",
        src: r#"fn f(a: u8, b: i64) -> bool { return b * 2 < a }
fn main() -> i64 { if f(3, 1) { return 1 }
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
    Member {
        name: "f_mod",
        src: r#"fn f(a: u8, b: i64) -> i64 { return b % 7 + a }
fn main() -> i64 { println("{f(3, 100)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "5\n",
            exit: 0,
        },
    },
    Member {
        name: "g_bitand",
        src: r#"fn f(a: u8, b: i64) -> i64 { return (b & 255) | a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "235\n",
            exit: 0,
        },
    },
    Member {
        name: "h_shift",
        src: r#"fn f(a: u8, b: i64) -> i64 { return b << a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "8000\n",
            exit: 0,
        },
    },
    Member {
        name: "h2_shift_lit",
        src: r#"fn f(a: u8, b: i64) -> i64 { return (b << 2) + a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4003\n",
            exit: 0,
        },
    },
    Member {
        name: "k_neg",
        src: r#"fn f(a: u8, b: i64) -> i64 { return -b * 2 + a }
fn main() -> i64 { println("{f(3, 10)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "-17\n",
            exit: 0,
        },
    },
    Member {
        name: "l_float",
        src: r#"fn f(a: f32, b: f64) -> f64 { return b * 2.0 + a }
fn main() -> i64 { println("{f(1.5, 2.0)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "s_i32_u8",
        src: r#"fn f(a: u8, b: i32) -> i32 { return b * 2 + a }
fn main() -> i64 { println("{f(3, 7)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "17\n",
            exit: 0,
        },
    },
    Member {
        name: "t_u64_i64",
        src: r#"fn f(a: u64, b: i64) -> i64 { return b * 2 + a }
fn main() -> i64 { println("{f(3, 7)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "k_u8_i8",
        src: r#"fn f(a: u8, c: i8) -> i64 { return a + c }
fn main() -> i64 { println("{f(200, -1)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "m_u32_i32",
        src: r#"fn f(a: u32, c: i32) -> i64 { return a + c }
fn main() -> i64 { println("{f(1, 2)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "m_shift_u8_lit",
        src: r#"fn f(a: u8) -> i64 { return a << 9 }
fn main() -> i64 { println("{f(1)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2\n",
            exit: 0,
        },
    },
    Member {
        name: "x5_shift_literal_left",
        src: r#"fn f(n: i64) -> i64 { let y: u8 = 1 << n
    return y }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "8\n",
            exit: 0,
        },
    },
    Member {
        name: "x8_shift_amount_wide",
        src: r#"fn f(a: u8, n: i64) -> i64 { return a << n }
fn main() -> i64 { println("{f(1, 3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "8\n",
            exit: 0,
        },
    },
];

const ROW_47: &[Member] = &[
    Member {
        name: "i_if",
        src: r#"fn f(a: u8, b: i64, c: bool) -> i64 { let r = if c { a } else { b }
    return r }
fn main() -> i64 { println("{f(3, 1000, false)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1000\n",
            exit: 0,
        },
    },
    Member {
        name: "j_compound",
        src: r#"fn f(a: u8) -> u8 { let mut x: u8 = a
    x += 10
    return x }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "13\n",
            exit: 0,
        },
    },
    Member {
        name: "j2_compound_mixed",
        src: r#"fn f(a: u8) -> i64 { let mut x: i64 = 5
    x += a * 2
    return x }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "11\n",
            exit: 0,
        },
    },
    Member {
        name: "m_struct",
        src: r#"struct S { a: u8, b: i64 }
fn f(s: S) -> i64 { return s.b * 10 + s.a }
fn main() -> i64 { println("{f(S { a: 3, b: 7 })}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "73\n",
            exit: 0,
        },
    },
    Member {
        name: "n_index",
        src: r#"fn f(i: u8) -> i64 { let arr = [1, 2, 3, 4, 5]
    return arr[i + 1] }
fn main() -> i64 { println("{f(2)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "n2_index_mixed",
        src: r#"fn f(i: u8, j: i64) -> i64 { let arr = [1, 2, 3, 4, 5]
    return arr[j * 1 + i] }
fn main() -> i64 { println("{f(2, 1)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "4\n",
            exit: 0,
        },
    },
    Member {
        name: "x9_match_arms_join",
        src: r#"enum E { A, B, C }
fn f(e: E, a: u8, b: u16) -> u16 {
    let r = match e { E::A => a, E::B => b, E::C => 300 }
    return r + 65535
}
fn g(e: E, a: u8, b: i64) -> i64 {
    let k = 7
    let r = match e { E::A => a, E::B => k, E::C => 250 }
    return r + 10
}
fn main() { println("{f(E::A, 200, 1)} {f(E::C, 1, 1)} {g(E::A, 250, 0)} {g(E::B, 0, 0)}") }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "199 299 4 17\n",
            exit: 0,
        },
    },
    Member {
        name: "p_let_lit_mix",
        src: r#"fn f(a: u8, b: i64) -> i64 { let t = 10 * b
    return t + a + 1 }
fn main() -> i64 { println("{f(3, 7)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "74\n",
            exit: 0,
        },
    },
    Member {
        name: "q_return_lit",
        src: r#"fn f(a: u8) -> i64 { return a * 200 }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "88\n",
            exit: 0,
        },
    },
    Member {
        name: "r_arg_mixed",
        src: r#"fn g(x: i64) -> i64 { return x }
fn f(a: u8) -> i64 { return g(a + 300) }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "u_while",
        src: r#"fn f(a: u8) -> i64 { let mut s: i64 = 0
    let mut i: i64 = 0
    while i * 1 < a { s = s + i
        i = i + 1 }
    return s }
fn main() -> i64 { println("{f(5)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10\n",
            exit: 0,
        },
    },
    Member {
        name: "a_if_partial",
        src: r#"fn f(a: u8, b: i64, c: bool) -> i64 { return (if c { 10 } else { b }) + a }
fn main() -> i64 { println("{f(3, 1000, false)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1003\n",
            exit: 0,
        },
    },
    Member {
        name: "x10_match_arm_literal_overflow",
        src: r#"enum E { A, B }
fn f(e: E, a: u8) -> i64 { let r = match e { E::A => a, E::B => 300 }
    return r }
fn main() -> i64 { println("{f(E::B, 1)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "c_block_tail",
        src: r#"fn f(a: u8, b: i64) -> i64 { return { b * 10 } + a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10003\n",
            exit: 0,
        },
    },
    Member {
        name: "g_unary",
        src: r#"fn f(a: i8, b: i64) -> i64 { return -(b * 2) + a }
fn main() -> i64 { println("{f(3, 10)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "-17\n",
            exit: 0,
        },
    },
    Member {
        name: "h_cmp_eq",
        src: r#"fn f(a: u8, b: i64) -> bool { return b * 1 == a }
fn main() -> i64 { if f(3, 3) { return 1 }
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "",
            exit: 1,
        },
    },
    Member {
        name: "i_nested",
        src: r#"fn f(a: u8, b: i64) -> i64 { return a + (b + 1) }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1004\n",
            exit: 0,
        },
    },
    Member {
        name: "j_assign_u8",
        src: r#"fn f(a: u8, b: i64) -> i64 { let mut x: u8 = a
    x = x + 1
    return b + x }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1004\n",
            exit: 0,
        },
    },
    Member {
        name: "k_arr_elem",
        src: r#"fn f(a: u8) -> i64 { let arr: [i64; 3] = [1000, 2, 3]
    return arr[0] * 2 + a }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2003\n",
            exit: 0,
        },
    },
    Member {
        name: "l_call_res",
        src: r#"fn g() -> i64 { return 1000 }
fn f(a: u8) -> i64 { return g() * 2 + a }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2003\n",
            exit: 0,
        },
    },
    Member {
        name: "m_global",
        src: r#"let G: i64 = 1000
fn f(a: u8) -> i64 { return G * 2 + a }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2003\n",
            exit: 0,
        },
    },
    Member {
        name: "n_u16",
        src: r#"fn f(a: u16, b: u8) -> u16 { return a * 2 + b }
fn main() -> i64 { println("{f(3, 4)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "10\n",
            exit: 0,
        },
    },
    Member {
        name: "j_u8_from_mixed",
        src: r#"fn f(a: u8, c: i64) -> u8 { let x: u8 = a + c
    return x }
fn main() -> i64 { println("{f(1, 2)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "l_nonlit_ctx",
        src: r#"fn f(a: u8, b: u8) -> i64 { let r: i64 = a * b
    let t = a * b
    let s: i64 = t
    println("{r} {s}")
    return 0 }
fn main() -> i64 { return f(200, 2) }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "144 144\n",
            exit: 0,
        },
    },
    Member {
        name: "n_arg_ctx",
        src: r#"fn g(x: i64) -> i64 { return x }
fn f(a: u8) -> i64 { return g(a * 200) }
fn main() -> i64 { println("{f(3)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "88\n",
            exit: 0,
        },
    },
    Member {
        name: "x6_if_join",
        src: r#"fn f(a: u8, c: bool) -> i64 { let r = if c { 300 } else { a }
    return r }
fn main() -> i64 { println("{f(3, true)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
];

const ROW_48: &[Member] = &[
    Member {
        name: "a_try_u8",
        src: r#"enum Result<T, E> { Ok(T), Err(E) }
fn inner(x: i64) -> Result<u8, i64> { if x > 0 { return Result::Ok(200) }
    return Result::Err(1) }
fn outer() -> Result<i64, i64> { let v = inner(1)?
    return Result::Ok(v * 2) }
fn main() -> i64 { match outer() { Result::Ok(v) => { println("{v}") } Result::Err(e) => { return 1 } }
    return 0 }
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "b_catch_u8",
        src: r#"enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }
fn get(b: i64) -> Result<u8, E> { if b > 0 { return Result::Ok(7) }
    return Result::Err(E::B) }
fn main() -> i64 { let b: i64 = 1000
    let v = get(1) catch { E::A => 1, E::B => 2 }
    println("{b * 2 + v}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2007\n",
            exit: 0,
        },
    },
    Member {
        name: "c_closure_capture_u8",
        src: r#"fn main() -> i64 { let a: u8 = 3
    let b: i64 = 1000
    let g = fn() -> i64 { return b * 2 + a }
    println("{g()}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2003\n",
            exit: 0,
        },
    },
    Member {
        name: "d_string_len_u8",
        src: r#"fn main() -> i64 { let a: u8 = 3
    let s = "hello"
    println("{s.len * 2 + a}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "13\n",
            exit: 0,
        },
    },
    Member {
        name: "e_vec_len_u8",
        src: r#"fn main() -> i64 { let a: u8 = 3
    let v: Vec<i64> = vec[1, 2]
    println("{v.len * 2 + a}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "7\n",
            exit: 0,
        },
    },
    Member {
        name: "f_cast_mix",
        src: r#"fn main() -> i64 { let a: u8 = 3
    let x: f64 = 2.5
    println("{(x as i64) * 2 + a}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "7\n",
            exit: 0,
        },
    },
    Member {
        name: "g_rc_get_u8",
        src: r#"fn main() -> i64 { let a: u8 = 3
    let r: Rc<i64> = Rc::new(1000)
    println("{Rc::get(r) * 2 + a}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2003\n",
            exit: 0,
        },
    },
    Member {
        name: "i_nogc_mix",
        src: r#"nogc fn f(a: u8, b: i64) -> i64 { return b * 2 + a }
fn main() -> i64 { println("{f(3, 1000)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "2003\n",
            exit: 0,
        },
    },
    Member {
        name: "j_f32_lit",
        src: r#"fn f(a: f32, b: f64) -> f64 { return b * 2.0 }
fn g(a: f32) -> f32 { return a * 2.0 }
fn main() -> i64 { println("{g(1.5)}")
    return 0 }
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "3\n",
            exit: 0,
        },
    },
    Member {
        name: "n_module_mix",
        src: r#"needs m
fn main() -> i64 { let a: u8 = 3
    println("{m.B * 2 + a}")
    return 0 }
"#,
        module: Some((
            r#"m.aelys"#,
            r#"pub let B: i64 = 1000
"#,
        )),
        expect: Expect::Runs {
            stdout: "2003\n",
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
    let k = 7
    let v = big()
    println("{id8({ let k = v; k } + k)}")
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f5",
        src: r#"enum E { A(i64), B }
fn id8(x: u8) -> u8 { x }
fn mk(n: i64) -> E { if n > 0 { return E::A(n) } E::B }
fn main() {
    let k = 7
    let e = mk(100)
    println("{id8(match e { E::A(k) => k, E::B => k })}")
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f6",
        src: r#"enum E { A(i64), B }
fn id8(x: u8) -> u8 { x }
fn mk(n: i64) -> E { if n > 0 { return E::A(n) } E::B }
fn main() {
    let k = 7
    let e = mk(300)
    println("{id8(match e { E::A(k) => k, E::B => k })}")
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f7",
        src: r#"enum E { A(i64), B }
fn id8(x: u8) -> u8 { x }
fn mk(n: i64) -> E { if n > 0 { return E::A(n) } E::B }
fn main() {
    let k = 7
    let e = mk(100)
    let r = id8(match e { E::A(k) => k, E::B => k })
    println("{r}")
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "f8",
        src: r#"enum E { A(i64), B }
fn mk(n: i64) -> E { if n > 0 { return E::A(n) } E::B }
fn main() {
    let k = 7
    let e = mk(1000)
    let y: u8 = 1
    println("{y + match e { E::A(k) => k, E::B => k }}")
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "1001\n",
            exit: 0,
        },
    },
];

const ROW_52: &[Member] = &[
    Member {
        name: "p1",
        src: r#"fn main() {
    let k = 5
    let y: u8 = -k
    println("{y}")
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "p2",
        src: r#"fn main() {
    let a: u8 = 10
    let k = 5
    let y = a + -k
    println("{y}")
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "q2",
        src: r#"fn id8(x: u8) -> u8 { x }
fn main() {
    let k = 5
    println("{id8(-k)}")
}
"#,
        module: None,
        expect: Expect::Refused("E0301"),
    },
    Member {
        name: "q4",
        src: r#"fn main() {
    let k = 128
    let y: i8 = -k
    let j: i8 = -128
    println("{y} {j}")
}
"#,
        module: None,
        expect: Expect::Runs {
            stdout: "-128 -128\n",
            exit: 0,
        },
    },
    Member {
        name: "mod1_neg_global",
        src: r#"needs m
fn main() {
    let a: u8 = 10
    let y: u8 = -m.K
    let z: u8 = m.J + a
    println("{y} {z}")
}
"#,
        module: Some((
            r#"m.aelys"#,
            r#"pub let K = 5
pub let J = K
"#,
        )),
        expect: Expect::Refused("E0301"),
    },
];

const ROW_53: &[Member] = &[
    Member {
        name: "rf1",
        src: r#"fn g(r: &i64) -> i64 { return *r + 1 }
fn main() {
    let k = 10
    let p = &k
    let a = g(&k)
    let b = g(&(k))
    let c = g(p)
    println("{a} {b} {c} {*p}")
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
    let a: i32 = 250
    let r = a << (7)
    println("{r}")
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
    let a: u8 = 3
    let r = a << (1 + 1)
    println("{r}")
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
    let a: i32 = 250
    let k = 7
    let r = a << (k)
    println("{r}")
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
    let a: u8 = 3
    let r = a << -(-2)
    println("{r}")
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
    let a: i32 = 250
    let k = 7
    let r = a >> (k)
    println("{r}")
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
    let a: i32 = 250
    let k = 7
    let r = a << (7)
    println("{r}")
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
    let a: i32 = 2147483647
    let r = a + (1)
    let s = a + 1
    println("{r} {s}")
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
    let a: u8 = 255
    let r: u8 = a + (1)
    println("{r}")
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
    let a: u8 = 255
    let k = 1
    let r = a + (k)
    let s = a + k
    println("{r} {s}")
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
    let x: u8 = 1
    let n: i64 = 9
    let y: i64 = x << n
    println("{y}")
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
    let a: u8 = 1
    let n: i64 = 9
    let y: u8 = 1 << n
    let z: i64 = a << n
    let w: i64 = 1 << n
    println("{y} {z} {w}")
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
let G = 3
fn retc(a: i8, p: i8) -> i64 { let s = S { f: p }; let arr = [p, p];  a + 3 }
fn probe(a: i8, p: i8) {
    let s = S { f: p }
    let arr = [p, p]
    let r = 1 << ((a + 3) % 8)
    println("{r}")
}
fn main() { probe(100, 3) }
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
let G = 3
fn retc(a: i8, p: i8) -> i64 { let s = S { f: p }; let arr = [p, p];  a | (3 as i8) }
fn probe(a: i8, p: i8) {
    let s = S { f: p }
    let arr = [p, p]
    let r = 1 << ((a | (3 as i8)) % 8)
    println("{r}")
}
fn main() { probe(100, 3) }
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
fn release01_45_a_literal_initialized_binding_narrows_as_its_literal_value() {
    run_row("release01_45", ROW_45);
}

#[test]
fn release01_46_the_binary_width_rule_holds_for_every_operator() {
    run_row("release01_46", ROW_46);
}

#[test]
fn release01_47_the_binary_width_rule_holds_in_every_context() {
    run_row("release01_47", ROW_47);
}

#[test]
fn release01_48_the_binary_width_rule_holds_for_every_operand_source() {
    run_row("release01_48", ROW_48);
}

#[test]
fn release01_51_a_binding_use_inside_a_format_string_resolves_in_its_own_scope() {
    run_row("release01_51", ROW_51);
}

#[test]
fn release01_52_a_negated_literal_binding_narrows_only_if_the_negated_value_fits() {
    run_row("release01_52", ROW_52);
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
    let local = typed("fn f(b: i64) -> i64 { let k: i64 = 10\n    return b * k }\n");
    assert_eq!(
        propagated_types(local, &mut LocalConstantPropagator::new()),
        vec![InferType::I64]
    );
    let global = typed("let k: i64 = 10\nfn f(b: i64) -> i64 { return b * k }\n");
    assert_eq!(
        propagated_types(global, &mut GlobalConstantPropagator::new()),
        vec![InferType::I64]
    );
}
