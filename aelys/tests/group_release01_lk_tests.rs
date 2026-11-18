use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const RUNTIMES: [(&str, RuntimeVariant); 2] = [
    ("rc", RuntimeVariant::Rc),
    ("rc+cycles", RuntimeVariant::RcCycles),
];

struct Case {
    name: &'static str,
    src: &'static str,
    out: &'static str,
}

fn counts(stderr: &str) -> Option<(u64, u64)> {
    let line = stderr.lines().find(|l| l.starts_with("[rc] allocs="))?;
    let mut n = line
        .split_whitespace()
        .filter_map(|w| w.split_once('=').and_then(|(_, v)| v.parse().ok()));
    Some((n.next()?, n.next()?))
}

fn check(row: &str, cases: &[Case]) {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    for case in cases {
        for (rt_tag, rt) in RUNTIMES {
            for (tag, level) in LEVELS {
                let stage = dir.path().join(format!(
                    "{}_{}_{}",
                    case.name,
                    rt_tag.replace('+', "_"),
                    &tag[1..]
                ));
                fs::create_dir_all(&stage).expect("stage dir");
                let root = stage.join("root.aelys");
                fs::write(&root, case.src).expect("write root");
                if let Err(err) = compile_file_with_llvm_variant(&root, level, false, rt) {
                    failures.push(format!("{} {rt_tag} {tag}: refused {err}", case.name));
                    continue;
                }
                let run = Command::new(stage.join("root"))
                    .env("AELYS_RC_STATS", "1")
                    .env("AELYS_ALLOC", "malloc")
                    .output()
                    .expect("run compiled exe");
                let stdout = String::from_utf8_lossy(&run.stdout);
                let stderr = String::from_utf8_lossy(&run.stderr);
                let balanced = counts(&stderr).is_some_and(|(a, f)| a == f);
                if stdout != case.out || run.status.code() != Some(0) || !balanced {
                    failures.push(format!(
                        "{} {rt_tag} {tag}: exit={:?} stdout={stdout:?} stderr={stderr:?}",
                        case.name,
                        run.status.code()
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{row}:\n{}", failures.join("\n"));
}

#[test]
fn release01_96_an_assignment_whose_source_is_its_destination_frees_what_it_held() {
    check(
        "release01_96",
        &[
            Case {
                name: "closure_same",
                src: r#"fn mkc(i: i64) -> fn(i64) -> i64 {
    let k: String = format!("a{i}") + "z";
    move |x: i64| -> i64 { x + k.len() }
}
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut x: fn(i64) -> i64 = mkc(i);
        x = { x; }
        n = n + x(1);
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "5890\n",
            },
            Case {
                name: "enum_same",
                src: r#"enum E { A(String), B }
fn elen(e: &E) -> i64 {
    match *e {
        E::A(s) => return s.len(),
        E::B => return 0,
    }
    0
}
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut x: E = E::A(format!("a{i}") + "z");
        x = x;
        n = n + elen(&x);
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "4890\n",
            },
            Case {
                name: "in_callee",
                src: r#"fn f(s: String) -> i64 {
    let mut t: String = s;
    t = t;
    t.len()
}
fn g(s: String) -> String {
    let mut t = s;
    t = t;
    t = t + t;
    t = t;
    t
}
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let s: String = format!("a{i}") + "z";
        n = n + f(s) + g(s).len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "14670\n",
            },
            Case {
                name: "string_block",
                src: r#"fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut s: String = format!("a{i}") + "z";
        s = { s; }
        n = n + s.len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "4890\n",
            },
            Case {
                name: "string_inlined_identity",
                src: r#"fn id(s: String) -> String { s }
fn gid<T>(x: T) -> T { x }
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut s: String = format!("abc{i}");
        s = id(s);
        s = gid(s);
        n = n + s.len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "5890\n",
            },
            Case {
                name: "string_same",
                src: r#"fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 100000 {
        let mut s: String = format!("abcdefgh{i}");
        s = s;
        n = n + s.len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "1288890\n",
            },
            Case {
                name: "struct_same",
                src: r#"struct P { s: String }
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut p = P { s: format!("a{i}") };
        p = p;
        n = n + p.s.len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "3890\n",
            },
            Case {
                name: "vec_and_rc_same",
                src: r#"struct R { s: String }
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut v: Vec<String> = vec![format!("a{i}"), "b"];
        v = v;
        let mut r: Rc<R> = Rc::new(R { s: format!("a{i}") + "z" });
        r = r;
        n = n + v.len() + Rc::get(r).s.len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "6890\n",
            },
        ],
    );
}

#[test]
fn release01_97_an_array_lent_as_a_slice_frees_its_elements_at_scope_end() {
    check(
        "release01_97",
        &[
            Case {
                name: "element_outlives_array",
                src: r#"fn first(a: &[String]) -> String { a[0] }
fn main() {
    let mut i: i64 = 0;
    let mut keep: String = "";
    let mut kept: String = "";
    while i < 1000 {
        let a: [String; 2] = [format!("a{i}") + "z", "b"];
        keep = first(a[..]);
        let e: String = {
            let b: [String; 2] = [format!("c{i}") + "z", "d"];
            let s = b[..];
            s[0]
        };
        kept = e;
        i = i + 1;
    }
    println!("{}", keep);
    println!("{}", kept);
}
"#,
                out: "a999z\nc999z\n",
            },
            Case {
                name: "foreach_view",
                src: r#"fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    let mut keep: String = "";
    while i < 1000 {
        let a: [String; 2] = [format!("a{i}") + "z", format!("b{i}")];
        for x in a[..] {
            n = n + x.len();
            keep = x;
        }
        i = i + 1;
    }
    println!("{}", keep);
    println!("{}", n);
}
"#,
                out: "b999\n8780\n",
            },
            Case {
                name: "read_views",
                src: r#"fn rm(a: &mut [String]) -> i64 { a[0].len() }
fn rs(a: &[String]) -> i64 { a[1].len() }
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut a: [String; 2] = [format!("a{i}") + "z", format!("b{i}") + "z"];
        n = n + rm(a[..]) + rs(a[..]);
        let s = a[..];
        n = n + s[0].len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "14670\n",
            },
            Case {
                name: "self_store_through_view",
                src: r#"struct P { s: String, k: i64 }
fn gid<T>(x: T) -> T { x }
fn sp(a: &mut [P]) { a[0] = gid(a[0]); }
fn ss(a: &mut [String], i: i64) { a[0] = if i > 5 { a[0] } else { a[0] }; }
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut a: [P; 2] = [P { s: format!("a{i}") + "z", k: i }, P { s: "b", k: 0 }];
        sp(a[..]);
        let mut b: [String; 2] = [format!("c{i}") + "z", "d"];
        ss(b[..], i);
        n = n + a[0].s.len() + b[0].len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "9780\n",
            },
            Case {
                name: "store_outer",
                src: r#"fn seti(a: &mut [String], s: String) { a[0] = s; }
fn main() {
    let mut a: [String; 2] = ["x", "y"];
    let mut i: i64 = 0;
    while i < 1000 {
        seti(a[..], "ab" + format!("{i}"));
        i = i + 1;
    }
    println!("{}", a[0]);
}
"#,
                out: "ab999\n",
            },
            Case {
                name: "swap_through_view",
                src: r#"fn sw(a: &mut [String]) {
    let t = a[0];
    a[0] = a[1];
    a[1] = t;
}
struct H { a: [String; 2], z: i64 }
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 1000 {
        let mut a: [String; 2] = [format!("a{i}") + "z", format!("bb{i}") + "z"];
        sw(a[..]);
        let mut h = H { a: [format!("c{i}") + "z", "d"], z: 0 };
        sw(h.a[..]);
        n = n + a[0].len() + h.a[1].len();
        i = i + 1;
    }
    println!("{}", n);
}
"#,
                out: "10780\n",
            },
        ],
    );
}
