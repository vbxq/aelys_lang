#[path = "level_legs/mod.rs"]
mod level_legs;

use aelys_driver::{
    EMBEDDED_STD, LinkRequirement, RuntimeVariant, SourceOptions, compile_file_with_llvm_sources,
    embedded_std_hash, std_hash,
};
use aelys_opt::OptimizationLevel;
use level_legs::{LEVELS, LevelLeg, legs_of, level_disagreements};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

#[derive(Clone, Copy)]
enum Expect {
    Refused {
        code: &'static str,
        at: (u32, u32),
        says: &'static str,
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

fn codes(rendered: &str) -> Vec<String> {
    let mut found: Vec<String> = rendered
        .match_indices("error[E")
        .map(|(i, _)| rendered[i + 6..i + 11].to_string())
        .collect();
    found.sort();
    found
}

fn env_of(expect: Expect) -> &'static [(&'static str, &'static str)] {
    match expect {
        Expect::Balanced(_) => &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "malloc")],
        _ => &[],
    }
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
            Expect::Runs(out),
            LevelLeg::Ran {
                stdout,
                stderr,
                exit,
            },
        ) => stdout == out && stderr.is_empty() && *exit == 0,
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
        let legs = legs_of(
            dir.path(),
            case.name,
            case.src,
            case.module,
            env_of(case.expect),
        );
        assert_eq!(legs.len(), LEVELS.len(), "{row}: legs of {}", case.name);
        if matches!(case.expect, Expect::Balanced(_)) {
            for (tag, leg) in &legs {
                if !met(case.expect, leg) {
                    failures.push(format!("{} at {tag}: {}", case.name, leg.render()));
                }
            }
            continue;
        }
        failures.extend(level_disagreements(case.name, &legs));
        if !met(case.expect, &legs[0].1) {
            failures.push(format!("{} at -O0: {}", case.name, legs[0].1.render()));
        }
    }
    assert!(failures.is_empty(), "{row}:\n{}", failures.join("\n"));
}

const PRELUDE_VARIANTS: [&str; 4] = ["Some", "None", "Ok", "Err"];

fn unqualified(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while !rest.is_empty() {
        let head = ["Option::", "Result::"]
            .into_iter()
            .find(|head| rest.starts_with(head));
        let before = out.chars().last();
        let bare = !before.is_some_and(|c| c.is_alphanumeric() || c == '.' || c == '_');
        if let Some(head) = head
            && bare
            && PRELUDE_VARIANTS.iter().any(|v| {
                rest[head.len()..].starts_with(v)
                    && !rest[head.len() + v.len()..].starts_with(|c: char| c.is_alphanumeric())
            })
        {
            rest = &rest[head.len()..];
            continue;
        }
        let c = rest.chars().next().expect("non-empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

struct Pair {
    name: &'static str,
    qualified: &'static str,
    module: Option<(&'static str, &'static str)>,
    expect: Expect,
}

fn check_pairs(row: &str, pairs: &[Pair]) {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    for pair in pairs {
        let bare_src = unqualified(pair.qualified);
        let bare_module = pair.module.map(|(file, text)| (file, unqualified(text)));
        if bare_src == pair.qualified
            && bare_module.as_ref().map(|m| m.1.as_str()) == pair.module.map(|m| m.1)
        {
            failures.push(format!(
                "{}: the twin spells nothing differently",
                pair.name
            ));
            continue;
        }
        let env = env_of(pair.expect);
        let spelled = legs_of(
            dir.path(),
            &format!("{}_q", pair.name),
            pair.qualified,
            pair.module,
            env,
        );
        let bare = legs_of(
            dir.path(),
            &format!("{}_u", pair.name),
            &bare_src,
            bare_module.as_ref().map(|(f, t)| (*f, t.as_str())),
            env,
        );
        for ((tag, q), (_, u)) in spelled.iter().zip(bare.iter()) {
            let same = match (q, u) {
                (LevelLeg::Refused(a), LevelLeg::Refused(b)) => codes(a) == codes(b),
                (
                    LevelLeg::Ran {
                        stdout: a,
                        exit: x,
                        stderr: e,
                    },
                    LevelLeg::Ran {
                        stdout: b,
                        exit: y,
                        stderr: f,
                    },
                ) => a == b && x == y && e == f,
                _ => false,
            };
            if !same {
                failures.push(format!(
                    "{} at {tag}: qualified {} but unqualified {}",
                    pair.name,
                    q.render(),
                    u.render()
                ));
            }
        }
        if !matches!(pair.expect, Expect::Balanced(_)) {
            failures.extend(level_disagreements(&format!("{}_u", pair.name), &bare));
        }
        let legs_to_check: Vec<&LevelLeg> = match pair.expect {
            Expect::Balanced(_) => bare.iter().chain(spelled.iter()).map(|(_, l)| l).collect(),
            _ => vec![&bare[0].1],
        };
        for leg in legs_to_check {
            if !met(pair.expect, leg) {
                failures.push(format!("{} unqualified: {}", pair.name, leg.render()));
            }
        }
    }
    assert!(failures.is_empty(), "{row}:\n{}", failures.join("\n"));
}

#[test]
fn release01_82_the_rewrite_of_a_qualified_variant_drops_only_the_prelude_heads() {
    assert_eq!(
        unqualified(
            "let a: Option<i64> = Option::Some(m.Option::None) + Option::Both + MyOption::None + Result::Err(1);"
        ),
        "let a: Option<i64> = Some(m.Option::None) + Option::Both + MyOption::None + Err(1);"
    );
}

#[test]
fn release01_83_each_unqualified_variant_form_answers_like_its_qualified_spelling() {
    check_pairs(
        "release01_83",
        &[
            Pair {
                name: "some_bare",
                qualified: "fn main() {\n    let o: Option<i64> = Option::Some;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 26),
                    says: "enum variant 'Option::Some' expects 1 argument, got 0",
                },
            },
            Pair {
                name: "some_call0",
                qualified: "fn main() {\n    let o: Option<i64> = Option::Some();\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 26),
                    says: "enum variant 'Option::Some' expects 1 argument, got 0",
                },
            },
            Pair {
                name: "some_call1",
                qualified: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Option::Some(1);\n    return ExitCode::from((match o { Option::Some(v) => v - 1, Option::None => 9 }) as u8);\n}\n",
                module: None,
                expect: Expect::Runs(""),
            },
            Pair {
                name: "some_call2",
                qualified: "fn main() {\n    let o: Option<i64> = Option::Some(1, 2);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 26),
                    says: "enum variant 'Option::Some' expects 1 argument, got 2",
                },
            },
            Pair {
                name: "none_bare",
                qualified: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Option::None;\n    return ExitCode::from((match o { Option::Some(v) => v, Option::None => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Runs(""),
            },
            Pair {
                name: "none_call0",
                qualified: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Option::None();\n    return ExitCode::from((match o { Option::Some(v) => v, Option::None => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Runs(""),
            },
            Pair {
                name: "none_call1",
                qualified: "fn main() {\n    let o: Option<i64> = Option::None(1);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 26),
                    says: "enum variant 'Option::None' expects 0 arguments, got 1",
                },
            },
            Pair {
                name: "none_call2",
                qualified: "fn main() {\n    let o: Option<i64> = Option::None(1, 2);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 26),
                    says: "enum variant 'Option::None' expects 0 arguments, got 2",
                },
            },
            Pair {
                name: "ok_bare",
                qualified: "fn main() {\n    let r: Result<i64, i64> = Result::Ok;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 31),
                    says: "enum variant 'Result::Ok' expects 1 argument, got 0",
                },
            },
            Pair {
                name: "ok_call0",
                qualified: "fn main() {\n    let r: Result<i64, i64> = Result::Ok();\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 31),
                    says: "enum variant 'Result::Ok' expects 1 argument, got 0",
                },
            },
            Pair {
                name: "ok_call1",
                qualified: "use std::process::ExitCode; fn main() -> ExitCode {\n    let r: Result<i64, i64> = Result::Ok(4);\n    return ExitCode::from((match r { Result::Ok(v) => v - 4, Result::Err(e) => e }) as u8);\n}\n",
                module: None,
                expect: Expect::Runs(""),
            },
            Pair {
                name: "ok_call2",
                qualified: "fn main() {\n    let r: Result<i64, i64> = Result::Ok(1, 2);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 31),
                    says: "enum variant 'Result::Ok' expects 1 argument, got 2",
                },
            },
            Pair {
                name: "err_bare",
                qualified: "fn main() {\n    let r: Result<i64, i64> = Result::Err;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 31),
                    says: "enum variant 'Result::Err' expects 1 argument, got 0",
                },
            },
            Pair {
                name: "err_call0",
                qualified: "fn main() {\n    let r: Result<i64, i64> = Result::Err();\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 31),
                    says: "enum variant 'Result::Err' expects 1 argument, got 0",
                },
            },
            Pair {
                name: "err_call1",
                qualified: "use std::process::ExitCode; fn main() -> ExitCode {\n    let r: Result<i64, i64> = Result::Err(6);\n    return ExitCode::from((match r { Result::Ok(v) => v, Result::Err(e) => e - 6 }) as u8);\n}\n",
                module: None,
                expect: Expect::Runs(""),
            },
            Pair {
                name: "err_call2",
                qualified: "fn main() {\n    let r: Result<i64, i64> = Result::Err(1, 2);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0302",
                    at: (2, 31),
                    says: "enum variant 'Result::Err' expects 1 argument, got 2",
                },
            },
        ],
    );
}

#[test]
fn release01_84_error_handling_reads_an_unqualified_carrier_like_a_qualified_one() {
    check_pairs(
        "release01_84",
        &[
            Pair {
                name: "try_result",
                qualified: "fn f(x: i64) -> Result<i64, i64> {\n    if x > 0 { return Result::Ok(x); }\n    Result::Err(7)\n}\nfn g(x: i64) -> Result<i64, i64> {\n    let v: i64 = f(x)?;\n    Result::Ok(v + 1)\n}\nfn main() {\n    println!(\"{} {}\", match g(2) { Ok(v) => v, Err(e) => e }, match g(0) { Ok(v) => v, Err(e) => e });\n}\n",
                module: None,
                expect: Expect::Runs("3 7\n"),
            },
            Pair {
                name: "try_option",
                qualified: "fn f(x: i64) -> Option<i64> {\n    if x > 0 { return Option::Some(x); }\n    Option::None\n}\nfn g(x: i64) -> Option<i64> {\n    let v: i64 = f(x)?;\n    Option::Some(v + 1)\n}\nfn main() {\n    let a: i64 = match g(4) { Option::Some(v) => v, Option::None => 0 };\n    let b: i64 = match g(0) { Option::Some(v) => v, Option::None => 9 };\n    println!(\"{a} {b}\");\n}\n",
                module: None,
                expect: Expect::Runs("5 9\n"),
            },
            Pair {
                name: "catch_arms",
                qualified: "enum E { Lo, Hi }\nfn f(x: i64) -> Result<i64, E> {\n    if x > 0 { return Result::Ok(x); }\n    Result::Err(E::Hi)\n}\nfn main() {\n    let v: i64 = match f(0) { Ok(v) => v, Err(E::Lo) => 1, Err(E::Hi) => 2 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("2\n"),
            },
            Pair {
                name: "map_error",
                qualified: "enum EA { Lo, Hi }\nenum EB { Mild, Worse }\nfn widen(e: EA) -> EB {\n    match e {\n        EA::Lo => return EB::Mild,\n        EA::Hi => return EB::Worse,\n    }\n}\nfn get() -> Result<i64, EA> { Result::Err(EA::Hi) }\nfn main() {\n    let r: Result<i64, EB> = get().map_error(widen);\n    let v: i64 = match r {\n        Result::Ok(v) => v,\n        Result::Err(e) => match e { EB::Mild => 1, EB::Worse => 2 },\n    };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("2\n"),
            },
            Pair {
                name: "unwrap_expect",
                qualified: "fn f() -> Result<i64, i64> { Result::Ok(5) }\nfn main() {\n    let a: i64 = f().unwrap();\n    let b: i64 = f().expect(\"no\");\n    println!(\"{}\", a + b);\n}\n",
                module: None,
                expect: Expect::Runs("10\n"),
            },
            Pair {
                name: "into_ok",
                qualified: "fn f() -> Result<i64, Never> { Result::Ok(3) }\nfn main() {\n    println!(\"{}\", f().into_ok());\n}\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Pair {
                name: "unwrap_unchecked",
                qualified: "fn f() -> Result<i64, i64> { Result::Ok(8) }\nfn main() {\n    let v: i64 = unsafe { f().unwrap_unchecked() };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("8\n"),
            },
            Pair {
                name: "unwrap_on_option",
                qualified: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Option::Some(4);\n    return ExitCode::from(o.unwrap() as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (3, 12),
                    says: "unwrap",
                },
            },
            Pair {
                name: "dropped_result",
                qualified: "fn main() {\n    Result::Ok(3);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0411",
                    at: (2, 5),
                    says: "[must-use] this `Result` can fail; use `?` to propagate",
                },
            },
            Pair {
                name: "dropped_annotated_result",
                qualified: "fn f() -> Result<i64, i64> { Result::Ok(1) }\nfn main() {\n    f();\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0411",
                    at: (3, 5),
                    says: "[must-use] this `Result` can fail with `i64`",
                },
            },
            Pair {
                name: "discarded_result",
                qualified: "fn main() {\n    let _ = Result::Ok(3);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 13),
                    says: "type annotations needed: the type of this expression is never inferred",
                },
            },
            Pair {
                name: "discarded_bound_result",
                qualified: "fn main() {\n    let r: Result<i64, i64> = Result::Ok(3);\n    let _ = r;\n    let _ = Result::Err(4).unwrap();\n    println!(\"ok\");\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (4, 13),
                    says: "type annotations needed",
                },
            },
        ],
    );
}

#[test]
fn release01_85_a_nogc_function_infers_the_same_with_either_spelling() {
    check_pairs(
        "release01_85",
        &[
            Pair {
                name: "nogc_result",
                qualified: "nogc fn f(x: i64) -> Result<i64, i64> {\n    if x > 0 { return Result::Ok(x); }\n    Result::Err(1)\n}\nnogc fn g(x: i64) -> Result<i64, i64> {\n    let v: i64 = f(x)?;\n    Result::Ok(v + 1)\n}\nnogc fn h(x: i64) -> i64 {\n    match f(x) { Ok(v) => v, Err(e) => e }\n}\nfn main() {\n    println!(\"{}\", h(3) + (match g(2) { Ok(v) => v, Err(e) => e }));\n}\n",
                module: None,
                expect: Expect::Runs("6\n"),
            },
            Pair {
                name: "nogc_option",
                qualified: "nogc fn k(x: i64) -> Option<i64> {\n    if x > 0 { return Option::Some(x); }\n    Option::None\n}\nfn main() {\n    let a: i64 = match k(5) { Option::Some(v) => v, Option::None => 0 };\n    let b: i64 = match k(0) { Option::Some(v) => v, Option::None => 1 };\n    println!(\"{}\", a + b);\n}\n",
                module: None,
                expect: Expect::Runs("6\n"),
            },
            Pair {
                name: "nogc_string_payload",
                qualified: "nogc fn f(x: i64) -> Result<String, i64> {\n    if x > 0 { return Result::Ok(\"a\"); }\n    Result::Err(1)\n}\nfn main() { }\n",
                module: None,
                expect: Expect::Runs(""),
            },
        ],
    );
}

#[test]
fn release01_86_generics_lambdas_globals_and_nesting_take_the_unqualified_spelling() {
    check_pairs(
        "release01_86",
        &[
            Pair {
                name: "generic_id",
                qualified: "fn id<T>(x: T) -> Option<T> { Option::Some(x) }\nfn main() {\n    let v: i64 = match id(5) { Option::Some(v) => v, Option::None => 0 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("5\n"),
            },
            Pair {
                name: "lambda_capture",
                qualified: "fn main() {\n    let limit: i64 = 3;\n    let f = move |x: i64| -> Option<i64> { if x > limit { return Option::Some(x); } Option::None };\n    let a: i64 = match f(5) { Option::Some(v) => v, Option::None => 0 };\n    let b: i64 = match f(1) { Option::Some(v) => v, Option::None => 7 };\n    println!(\"{a} {b}\");\n}\n",
                module: None,
                expect: Expect::Runs("5 7\n"),
            },
            Pair {
                name: "global_none",
                qualified: "static G: Option<i64> = Option::None;\nfn main() {\n    let v: i64 = match G { Option::Some(v) => v, Option::None => 7 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("7\n"),
            },
            Pair {
                name: "let_none_unannotated",
                qualified: "fn main() {\n    let x = Option::None;\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0304",
                    at: (2, 13),
                    says: "type annotations needed: cannot infer type parameter <T> for `Option::None`",
                },
            },
            Pair {
                name: "let_mut_none_then_some",
                qualified: "fn main() {\n    let mut x = Option::None;\n    x = Option::Some(1);\n    let v: i64 = match x { Option::Some(v) => v, Option::None => 0 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Pair {
                name: "nested_some",
                qualified: "fn main() {\n    let o: Option<Option<i64>> = Option::Some(Option::Some(2));\n    let v: i64 = match o { Option::Some(i) => match i { Option::Some(v) => v, Option::None => 0 }, Option::None => 0 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("2\n"),
            },
            Pair {
                name: "ok_of_none",
                qualified: "fn main() {\n    let r: Result<Option<i64>, i64> = Result::Ok(Option::None);\n    let v: i64 = match r { Result::Ok(o) => match o { Option::Some(v) => v, Option::None => 4 }, Result::Err(e) => e };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Pair {
                name: "string_payload_counters",
                qualified: "fn wrap(s: String) -> Option<String> { Option::Some(s + \"!\") }\nfn main() {\n    let o: Option<String> = wrap(\"hi\");\n    let s: String = match o { Option::Some(v) => v, Option::None => \"\" };\n    println!(\"{}\", s);\n}\n",
                module: None,
                expect: Expect::Balanced("hi!\n"),
            },
        ],
    );
}

#[test]
fn release01_87_match_arms_of_either_spelling_are_checked_as_one() {
    check(
        "release01_87",
        &[
            Case {
                name: "mixed_arms",
                src: "fn main() {\n    let o: Option<i64> = Some(4);\n    let v: i64 = match o { Some(v) => v, Option::None => 0 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Case {
                name: "mixed_duplicate",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Some(1);\n    return ExitCode::from((match o { Some(v) => v, Option::Some(w) => w + 1, None => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0310",
                    at: (3, 36),
                    says: "unreachable match arm: duplicate pattern for variant 'Option::Some'",
                },
            },
            Case {
                name: "wildcard_not_last",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Some(1);\n    return ExitCode::from((match o { None => 0, _ => 1, Some(v) => v }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0310",
                    at: (3, 41),
                    says: "unreachable match arm: wildcard pattern must be the last match arm",
                },
            },
            Case {
                name: "non_exhaustive_local",
                src: "use std::process::ExitCode; enum Option<T> { Some(T), None, Both }\nfn main() -> ExitCode {\n    let o: Option<i64> = Option::Both;\n    return ExitCode::from((match o { Some(v) => v, None => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (4, 12),
                    says: "non-exhaustive match: missing variant 'Option::Both'",
                },
            },
            Case {
                name: "local_option_takes_the_names",
                src: "enum Option<T> { Some(T), None, Both }\nfn main() {\n    let o: Option<i64> = Some(3);\n    let v: i64 = match o { Some(v) => v, None => 0, Option::Both => 9 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "pattern_of_another_enum",
                src: "use std::process::ExitCode; enum MyOpt { Some(i64), None }\nfn main() -> ExitCode {\n    let a: MyOpt = MyOpt::Some(4);\n    return ExitCode::from((match a { Some(v) => v, MyOpt::None => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (4, 22),
                    says: "`Some` here is `Option::Some`; the scrutinee is `MyOpt`; write `MyOpt::Some`",
                },
            },
            Case {
                name: "not_a_variant",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Some(1);\n    return ExitCode::from((match o { Some(v) => v, Nnoe => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0101",
                    at: (3, 36),
                    says: "`Nnoe` is not a variant of `Option`, whose variants are `Some` and `None`",
                },
            },
            Case {
                name: "variant_of_the_scrutinee_written_bare",
                src: "use std::process::ExitCode; enum Color { Red, Green }\nfn main() -> ExitCode {\n    let c: Color = Color::Red;\n    return ExitCode::from((match c { Red => 1, Color::Green => 2 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0101",
                    at: (4, 22),
                    says: "`Red` alone does not name a variant; write `Color::Red`",
                },
            },
            Case {
                name: "nested_unit_variant",
                src: "fn main() {\n    let o: Option<Option<i64>> = Some(None);\n    let v: i64 = match o { Some(None) => 1, Some(x) => 2, None => 3 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("1\n"),
            },
            Case {
                name: "nested_qualified_variant",
                src: "fn main() {\n    let o: Option<Option<i64>> = Option::Some(Option::None);\n    let v: i64 = match o { Option::Some(Option::Some(v)) => v, _ => 7 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("7\n"),
            },
            Case {
                name: "lowercase_variant_declaration",
                src: "enum E { foo, bar(i64) }\nfn main() -> i64 { return 0 }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0101",
                    at: (1, 13),
                    says: "expected capitalized variant name",
                },
            },
            Case {
                name: "lowercase_catch_all",
                src: "fn main() {\n    let o: Option<i64> = None;\n    let v: i64 = match o { Some(v) => v, other => 5 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("5\n"),
            },
        ],
    );
}

const SHADOWING_MODULE: &str = "pub enum Option<T> { Some(T), None, Many(T) }\npub fn wrap<T>(x: T) -> Option<T> { Option::Some(x) }\npub fn get<T>(o: Option<T>, d: T) -> T {\n    match o { Option::Some(v) => v, Option::None => d, Option::Many(v) => v }\n}\n";

#[test]
fn release01_88_a_module_resolves_the_names_against_its_own_scope() {
    check_pairs(
        "release01_88",
        &[
            Pair {
                name: "module_shadows_option",
                qualified: "mod m;\nfn main() {\n    let o: m::Option<i64> = m::wrap(5);\n    println!(\"{}\", m::get(o, 0));\n}\n",
                module: Some(("m.aelys", SHADOWING_MODULE)),
                expect: Expect::Runs("5\n"),
            },
            Pair {
                name: "module_uses_prelude",
                qualified: "mod m;\nfn main() {\n    let v: i64 = match m::half(8) { Option::Some(v) => v, Option::None => 0 };\n    println!(\"{v}\");\n}\n",
                module: Some((
                    "m.aelys",
                    "pub fn half(x: i64) -> Option<i64> {\n    if x % 2 == 0 { return Option::Some(x / 2); }\n    Option::None\n}\n",
                )),
                expect: Expect::Runs("4\n"),
            },
        ],
    );
    check(
        "release01_88",
        &[Case {
            name: "module_scrutinee_unqualified_arms",
            src: "use std::process::ExitCode; mod m;\nfn main() -> ExitCode {\n    let o: m::Option<i64> = m::wrap(5);\n    return ExitCode::from((match o { Some(v) => v, None => 0, m::Option::Many(v) => v }) as u8);\n}\n",
            module: Some(("m.aelys", SHADOWING_MODULE)),
            expect: Expect::Refused {
                code: "E0301",
                at: (4, 22),
                says: "`Some` here is `Option::Some`; the scrutinee is `m.Option`; write `m.Option::Some`",
            },
        }],
    );
}

#[test]
fn release01_89_the_names_resolve_to_the_enum_in_scope_or_say_why_not() {
    check(
        "release01_89",
        &[
            Case {
                name: "local_result_without_ok",
                src: "enum Result<T> { Yes(T), No }\nfn main() {\n    let r: Result<i64> = Ok(3);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (3, 26),
                    says: "`Ok` names a variant of the `Result` in scope, which is the enum `Result` declared in this module; it has no variant `Ok`",
                },
            },
            Case {
                name: "pattern_on_local_result_without_err",
                src: "use std::process::ExitCode; enum Result<T> { Yes(T), No }\nfn main() -> ExitCode {\n    let r: Result<i64> = Result::Yes(3);\n    return ExitCode::from((match r { Err(e) => e, Result::Yes(v) => v, Result::No => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (4, 22),
                    says: "`Err` names a variant of the `Result` in scope, which is the enum `Result` declared in this module; it has no variant `Err`",
                },
            },
            Case {
                name: "struct_named_option",
                src: "struct Option { x: i64 }\nfn main() {\n    let r = Some(3);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (3, 13),
                    says: "`Some` names a variant of the `Option` in scope, which is the struct `Option` declared in this module, not an enum",
                },
            },
            Case {
                name: "imported_option_without_none",
                src: "mod m; use m::Option;\nfn main() {\n    let o: Option = None;\n}\n",
                module: Some(("m.aelys", "pub enum Option { Some(i64), Nothing }\n")),
                expect: Expect::Refused {
                    code: "E0301",
                    at: (3, 21),
                    says: "`None` names a variant of the `Option` in scope, which is the enum `Option` imported from `m`; it has no variant `None`",
                },
            },
            Case {
                name: "unqualified_dotted_type_head",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    let r: std.result.Result<i64, i64> = Result::Ok(3);\n    return ExitCode::from((match r { Result::Ok(x) => x, Result::Err(_) => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0301",
                    at: (2, 12),
                    says: "unknown type `std.result.Result<i64, i64>`",
                },
            },
        ],
    );
}

#[test]
fn release01_90_no_prelude_leaves_the_names_unresolved_with_a_named_error() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(
        &root,
        "enum Choice { A, B }\nfn main() {\n    let c: Choice = Choice::A;\n    let r = Some(1);\n}\n",
    )
    .expect("write root");
    let no_prelude = SourceOptions {
        include: Vec::new(),
        prelude: None,
    };
    let rendered = compile_file_with_llvm_sources(
        &root,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
        &LinkRequirement::default(),
        &no_prelude,
        None,
    )
    .expect_err("Some without a prelude must be refused")
    .to_string();
    assert!(
        rendered.contains("error[E0301]")
            && rendered.contains("root.aelys:4:13")
            && rendered.contains("`Some` names a variant of `Option`, and no `Option` is in scope"),
        "release01_90: {rendered}"
    );
}

fn reserved(name: &'static str, src: &'static str, at: (u32, u32), says: &'static str) -> Case {
    Case {
        name,
        src,
        module: Some(("lib.aelys", "pub fn seven() -> i64 { 7 }\n")),
        expect: Expect::Refused {
            code: "E0205",
            at,
            says,
        },
    }
}

#[test]
fn release01_91_the_four_names_are_reserved_at_every_value_binding_site() {
    const NONE: &str =
        "`None` is reserved for the prelude variant `Option::None`; choose another name";
    const SOME: &str =
        "`Some` is reserved for the prelude variant `Option::Some`; choose another name";
    const OK: &str = "`Ok` is reserved for the prelude variant `Result::Ok`; choose another name";
    const ERR: &str =
        "`Err` is reserved for the prelude variant `Result::Err`; choose another name";
    check(
        "release01_91",
        &[
            reserved(
                "fn_name",
                "fn Some(x: i64) -> i64 { return x }\nfn main() -> i64 { return 0 }\n",
                (1, 4),
                SOME,
            ),
            reserved(
                "extern_name",
                "unsafe extern fn None(x: i64) -> i64\nfn main() -> i64 { return 0 }\n",
                (1, 18),
                NONE,
            ),
            reserved(
                "global",
                "let None: i64 = 9\nfn main() -> i64 { return 0 }\n",
                (1, 5),
                NONE,
            ),
            reserved(
                "let",
                "fn main() -> i64 {\n    let Ok: i64 = 4\n    return 0\n}\n",
                (2, 9),
                OK,
            ),
            reserved(
                "parameter",
                "fn f(Err: i64) -> i64 { return 1 }\nfn main() -> i64 { return f(1) }\n",
                (1, 6),
                ERR,
            ),
            reserved(
                "lambda_parameter",
                "fn main() -> i64 {\n    let f = fn(Some: i64) -> i64 { return 1 }\n    return f(2)\n}\n",
                (2, 16),
                SOME,
            ),
            reserved(
                "for_variable",
                "fn main() -> i64 {\n    for None in 0..3 { }\n    return 0\n}\n",
                (2, 9),
                NONE,
            ),
            reserved(
                "catch_binding",
                "fn f() -> Result<i64, i64> { return Err(5) }\nfn main() -> i64 { return f() catch |Err| 1 }\n",
                (2, 38),
                ERR,
            ),
            reserved(
                "imported_symbol",
                "needs Ok from lib\nfn main() -> i64 { return 0 }\n",
                (1, 7),
                OK,
            ),
            reserved(
                "namespace_alias",
                "needs lib as Err\nfn main() -> i64 { return 0 }\n",
                (1, 14),
                ERR,
            ),
            Case {
                name: "implicit_namespace_name",
                src: "needs None\nfn main() -> i64 { return 0 }\n",
                module: Some(("None.aelys", "pub fn seven() -> i64 { 7 }\n")),
                expect: Expect::Refused {
                    code: "E0205",
                    at: (1, 7),
                    says: NONE,
                },
            },
            Case {
                name: "capitalized_pattern_binding",
                src: "use std::process::ExitCode; fn main() -> ExitCode {\n    let o: Option<i64> = Some(4);\n    return ExitCode::from((match o { Some(V) => 1, None => 0 }) as u8);\n}\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0101",
                    at: (3, 27),
                    says: "the binding `V` starts with an uppercase letter; a capitalized name in a pattern names a variant, never a binding",
                },
            },
            Case {
                name: "capitalized_catch_binding",
                src: "fn f() -> Result<i64, i64> { return Err(5) }\nfn main() -> i64 { return f() catch |E| 1 }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0101",
                    at: (2, 38),
                    says: "the binding `E` starts with an uppercase letter; a capitalized name in a pattern names a variant, never a binding",
                },
            },
            Case {
                name: "not_values_stay_free",
                src: "struct S { None: i64 }\nstruct Some { x: i64 }\nenum MyOpt { Some(i64), None }\nfn main() {\n    let s: S = S { None: 2 };\n    let t: Some = Some { x: 3 };\n    let o: MyOpt = MyOpt::None;\n    let k: i64 = match o { MyOpt::Some(v) => v, MyOpt::None => 4 };\n    println!(\"{}\", s.None + t.x + k);\n}\n",
                module: None,
                expect: Expect::Runs("9\n"),
            },
        ],
    );
}

#[test]
fn release01_92_a_unit_function_named_like_a_prelude_type_keeps_the_type() {
    check(
        "release01_92",
        &[
            Case {
                name: "fn_option_and_type_option",
                src: "fn Option() -> i64 { 2 }\nfn main() {\n    let o: Option<i64> = Option::Some(1);\n    let v: i64 = match o { Option::Some(v) => v, Option::None => 0 };\n    println!(\"{}\", v + Option());\n}\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
            Case {
                name: "local_duplicate_is_still_one_namespace",
                src: "struct Foo { x: i64 }\nfn Foo() -> i64 { 1 }\nfn main() { }\n",
                module: None,
                expect: Expect::Refused {
                    code: "E0204",
                    at: (2, 1),
                    says: "the name `Foo` is defined twice, once as a struct and once as a function",
                },
            },
        ],
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn stage(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    for (name, body) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, body).expect("write");
    }
    dir
}

fn compiled(root: &Path, sources: &SourceOptions) -> Result<(i32, String), String> {
    compile_file_with_llvm_sources(
        root,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
        &LinkRequirement::default(),
        sources,
        None,
    )
    .map_err(|err| err.to_string())?;
    let out = Command::new(root.with_extension(""))
        .output()
        .expect("run executable");
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

#[test]
fn release01_93_the_embedded_std_is_the_std_directory() {
    let mut on_disk: Vec<(String, String)> = fs::read_dir(repo_root().join("std"))
        .expect("read std")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "aelys"))
        .map(|path| {
            let stem = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .into_owned();
            (stem, fs::read_to_string(&path).expect("read module"))
        })
        .collect();
    on_disk.sort();
    let mut embedded: Vec<(String, String)> = EMBEDDED_STD
        .iter()
        .map(|(name, text)| (name.to_string(), text.to_string()))
        .collect();
    embedded.sort();
    assert_eq!(
        embedded, on_disk,
        "the embedded std must list every std/*.aelys file with its text"
    );

    let mut changed = embedded.clone();
    changed[0].1.push_str("\n");
    let hashed = |modules: &[(String, String)]| -> String {
        let refs: Vec<(&str, &str)> = modules
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        std_hash(&refs)
    };
    assert_eq!(hashed(&on_disk), embedded_std_hash());
    assert_ne!(
        hashed(&changed),
        embedded_std_hash(),
        "a changed std must change the hash"
    );

    let line = aelys_cli::cli::version_line();
    let hash = embedded_std_hash();
    assert!(
        hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()) && line.contains(&hash),
        "the version line must carry the embedded std hash: {line}"
    );
}

const ABS: &str = "use std::process::ExitCode; use std::math;\nfn main() -> ExitCode {\n    return ExitCode::from(math::abs(0 - 7) as u8);\n}\n";
const LOUD_MATH: &str = "pub nogc fn abs(x: i64) -> i64 {\n    99\n}\n";
const OWN_PRELUDE: &str = "pub fn seven() -> i64 { 7 }\n";

#[test]
fn release01_94_the_std_namespace_comes_from_one_source() {
    let bare = stage(&[("root.aelys", ABS), ("std/math.aelys", LOUD_MATH)]);
    assert_eq!(
        compiled(&bare.path().join("root.aelys"), &SourceOptions::default()),
        Ok((7, String::new())),
        "a std directory without a prelude is not a std, the embedded one answers"
    );

    let owned = stage(&[
        ("root.aelys", "use std::process::ExitCode; fn main() -> ExitCode {\n    return ExitCode::from(seven() as u8);\n}\n"),
        ("std/prelude.aelys", OWN_PRELUDE),
    ]);
    assert_eq!(
        compiled(&owned.path().join("root.aelys"), &SourceOptions::default()),
        Ok((7, String::new())),
        "the root directory's std/prelude.aelys is the prelude"
    );

    let partial = stage(&[("root.aelys", ABS), ("std/prelude.aelys", OWN_PRELUDE)]);
    let rendered = compiled(
        &partial.path().join("root.aelys"),
        &SourceOptions::default(),
    )
    .expect_err("a std root without math must not borrow the embedded math");
    let searched = partial.path().join("std").join("math.aelys");
    assert!(
        rendered.contains("error[E0601]")
            && rendered.contains(&format!("searched in: {}", searched.display()))
            && !rendered.contains("<embedded>"),
        "release01_94 partial: {rendered}"
    );

    let lib = stage(&[
        ("std/prelude.aelys", OWN_PRELUDE),
        ("std/math.aelys", LOUD_MATH),
    ]);
    let user = stage(&[("root.aelys", ABS)]);
    assert_eq!(
        compiled(
            &user.path().join("root.aelys"),
            &SourceOptions::with_include(vec![lib.path().to_path_buf()])
        ),
        Ok((99, String::new())),
        "an include root holding std/prelude.aelys is the whole std"
    );

    let ignored = stage(&[
        (
            "root.aelys",
            "use std::process::ExitCode; use std::foo;\nfn main() -> ExitCode { return ExitCode::from(foo::mine() as u8); }\n",
        ),
        ("std/foo.aelys", OWN_PRELUDE),
    ]);
    let rendered = compiled(
        &ignored.path().join("root.aelys"),
        &SourceOptions::default(),
    )
    .expect_err("a std copy without a prelude is not a std source");
    let copy = ignored.path().join("std").join("foo.aelys");
    assert!(
        rendered.contains("error[E0601]")
            && rendered.contains(&format!(
                "std.foo comes from one source: no search root holds std/prelude.aelys, so {} is ignored",
                copy.display()
            )),
        "release01_94 ignored: {rendered}"
    );

    let nope = stage(&[(
        "root.aelys",
        "use std::nope;\nfn main() { }\n",
    )]);
    let rendered = compiled(&nope.path().join("root.aelys"), &SourceOptions::default())
        .expect_err("std.nope does not exist");
    assert!(
        rendered.contains("error[E0601]")
            && rendered.contains("searched in: <embedded>/std/nope.aelys"),
        "release01_94 nope: {rendered}"
    );
}

#[test]
fn release01_95_a_binding_without_letter_case_is_a_binding() {
    check_pairs(
        "release01_95",
        &[
            Pair {
                name: "caseless_pattern_binding",
                qualified: "fn main() {\n    let o: Option<i64> = Option::Some(4);\n    let v: i64 = match o { Option::Some(値) => 値, Option::None => 0 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("4\n"),
            },
            Pair {
                name: "caseless_catch_binding",
                qualified: "fn g() -> Result<i64, i64> { Result::Err(2) }\nfn main() {\n    let v: i64 = match g() { Ok(v) => v, Err(値) => 値 + 1 };\n    println!(\"{v}\");\n}\n",
                module: None,
                expect: Expect::Runs("3\n"),
            },
        ],
    );
}
