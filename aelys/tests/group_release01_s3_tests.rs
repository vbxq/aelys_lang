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
