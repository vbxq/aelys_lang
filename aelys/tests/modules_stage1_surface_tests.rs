use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

type Files = &'static [(&'static str, &'static str)];

fn stage(files: Files) -> TempDir {
    let dir = tempdir().expect("tempdir");
    for (name, body) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture directory");
        }
        fs::write(&path, body).expect("write fixture");
    }
    dir
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

fn run_row(id: &str, files: Files, exit: i32, stdout: &str) {
    assert!(
        (0..256).contains(&exit),
        "{id}: an expected exit of {exit} cannot be observed through an 8-bit status"
    );
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        if let Err(err) = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
            panic!("{id} at {level}: the module program MUST compile and link\nerror:\n{err}");
        }
        let exe = exe_path_for(&root);
        assert!(exe.is_file(), "{id} at {level}: no executable was produced");
        let out = Command::new(&exe).output().expect("run executable");
        assert_eq!(
            exit_code(&out.status),
            exit,
            "{id} at {level}: the answer MUST be {exit}\nstdout: {:?}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            stdout,
            "{id} at {level}: stdout MUST be {stdout:?}"
        );
    }
}

fn message_only(rendered: &str) -> String {
    rendered
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            !t.starts_with("-->") && !t.starts_with('|') && !first_field_is_a_gutter(t)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_field_is_a_gutter(line: &str) -> bool {
    match line.split_once('|') {
        Some((head, _)) => !head.is_empty() && head.trim().chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

fn reject_row(id: &str, files: Files, code: &str, needle: &str) {
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        let rendered = match lower_file_to_air(&root, *opt) {
            Ok(_) => panic!("{id} at {level}: MUST be rejected, but it was accepted"),
            Err(rendered) => rendered,
        };
        let message = message_only(&rendered);
        assert!(
            message.contains(code),
            "{id} at {level}: the diagnostic MUST carry {code}\nrendered:\n{rendered}"
        );
        assert!(
            message.contains(needle),
            "{id} at {level}: the diagnostic MUST say {needle:?}\nrendered:\n{rendered}"
        );
    }
}

const SEVEN: &str = "pub fn seven() -> i64 {\n    return 7\n}\n";

#[test]
fn group_mod_d1_diamond_compiles_its_shared_dependency_once() {
    run_row(
        "D-1",
        &[
            (
                "root.aelys",
                "needs l\nneeds r\n\nfn main() -> i64 {\n    return l.viaL() + r.viaR()\n}\n",
            ),
            (
                "l.aelys",
                "needs base\n\npub fn viaL() -> i64 {\n    return base.seed() * 2\n}\n",
            ),
            (
                "r.aelys",
                "needs base\n\npub fn viaR() -> i64 {\n    return base.seed() * 3\n}\n",
            ),
            ("base.aelys", "pub fn seed() -> i64 {\n    return 7\n}\n"),
        ],
        35,
        "",
    );
}

#[test]
fn group_mod_d2_a_module_main_is_an_ordinary_function() {
    run_row(
        "D-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.pick()\n}\n",
            ),
            (
                "m.aelys",
                "fn main() -> i64 {\n    return 99\n}\n\npub fn pick() -> i64 {\n    return 5\n}\n",
            ),
        ],
        5,
        "",
    );
}

#[test]
fn group_mod_d3_one_module_bound_under_two_names() {
    run_row(
        "D-3",
        &[
            (
                "root.aelys",
                "needs m\nneeds m as k\n\nfn main() -> i64 {\n    return m.one() + k.one()\n}\n",
            ),
            ("m.aelys", "pub fn one() -> i64 {\n    return 1\n}\n"),
        ],
        2,
        "",
    );
}

#[test]
fn group_mod_d4_a_path_three_segments_deep() {
    run_row(
        "D-4",
        &[
            (
                "root.aelys",
                "needs a.b.c\n\nfn main() -> i64 {\n    return c.deep()\n}\n",
            ),
            ("a/b/c.aelys", "pub fn deep() -> i64 {\n    return 42\n}\n"),
        ],
        42,
        "",
    );
}

#[test]
fn group_mod_o1_two_modules_define_the_same_struct() {
    run_row(
        "O-1",
        &[
            (
                "root.aelys",
                "needs x\nneeds y\n\nfn main() -> i64 {\n    return x.make() + y.make()\n}\n",
            ),
            (
                "x.aelys",
                "struct Holder { v: i64 }\n\npub fn make() -> i64 {\n    let h = Holder { v: 10 }\n    return h.v\n}\n",
            ),
            (
                "y.aelys",
                "struct Holder { v: i64 }\n\npub fn make() -> i64 {\n    let h = Holder { v: 20 }\n    return h.v\n}\n",
            ),
        ],
        30,
        "",
    );
}

#[test]
fn group_mod_o2_two_modules_define_the_same_data_enum() {
    run_row(
        "O-2",
        &[
            (
                "root.aelys",
                "needs p\nneeds q\n\nfn main() -> i64 {\n    return p.pick() + q.pick()\n}\n",
            ),
            (
                "p.aelys",
                "enum Tag { None, Some(i64) }\n\npub fn pick() -> i64 {\n    return match Tag::Some(6) {\n        Tag::Some(n) => n\n        Tag::None => 0\n    }\n}\n",
            ),
            (
                "q.aelys",
                "enum Tag { None, Some(i64) }\n\npub fn pick() -> i64 {\n    return match Tag::None {\n        Tag::Some(n) => n\n        Tag::None => 50\n    }\n}\n",
            ),
        ],
        56,
        "",
    );
}

#[test]
fn group_mod_o3_two_modules_define_the_same_global() {
    run_row(
        "O-3",
        &[
            (
                "root.aelys",
                "needs p\nneeds q\n\nfn main() -> i64 {\n    return p.read() + q.read()\n}\n",
            ),
            (
                "p.aelys",
                "let shared: i64 = 4\n\npub fn read() -> i64 {\n    return shared\n}\n",
            ),
            (
                "q.aelys",
                "let shared: i64 = 9\n\npub fn read() -> i64 {\n    return shared\n}\n",
            ),
        ],
        13,
        "",
    );
}

#[test]
fn group_mod_o4_two_modules_instantiate_the_same_generic() {
    run_row(
        "O-4",
        &[
            (
                "root.aelys",
                "needs p\nneeds q\n\nfn main() -> i64 {\n    return p.run() + q.run()\n}\n",
            ),
            (
                "p.aelys",
                "fn ident<T>(v: T) -> T {\n    return v\n}\n\npub fn run() -> i64 {\n    return ident(3)\n}\n",
            ),
            (
                "q.aelys",
                "fn ident<T>(v: T) -> T {\n    return v\n}\n\npub fn run() -> i64 {\n    return ident(4)\n}\n",
            ),
        ],
        7,
        "",
    );
}

#[test]
fn group_mod_lb1_two_modules_each_hold_a_lambda() {
    run_row(
        "LB-1",
        &[
            (
                "root.aelys",
                "needs p\nneeds q\n\nfn main() -> i64 {\n    return p.run() + q.run()\n}\n",
            ),
            (
                "p.aelys",
                "pub fn run() -> i64 {\n    let f = fn(a: i64) -> i64 { return a + 1 }\n    return f(1)\n}\n",
            ),
            (
                "q.aelys",
                "pub fn run() -> i64 {\n    let g = fn(a: i64) -> i64 { return a + 10 }\n    return g(1)\n}\n",
            ),
        ],
        13,
        "",
    );
}

#[test]
fn group_mod_cl1_two_modules_each_hold_a_capturing_lambda() {
    run_row(
        "CL-1",
        &[
            (
                "root.aelys",
                "needs p\nneeds q\n\nfn main() -> i64 {\n    println(\"go\")\n    return p.run() + q.run()\n}\n",
            ),
            (
                "p.aelys",
                "pub fn run() -> i64 {\n    let base: i64 = 10\n    let f = fn(a: i64) -> i64 { return a + base }\n    return f(1)\n}\n",
            ),
            (
                "q.aelys",
                "pub fn run() -> i64 {\n    let base: i64 = 20\n    let g = fn(a: i64) -> i64 { return a + base }\n    return g(1)\n}\n",
            ),
        ],
        32,
        "go\n",
    );
}

#[test]
fn group_mod_p1_a_module_path_spelt_like_a_mono_prefix() {
    run_row(
        "P-1",
        &[
            (
                "root.aelys",
                "needs vec_x\n\nfn main() -> i64 {\n    return vec_x.make()\n}\n",
            ),
            (
                "vec_x.aelys",
                "struct Holder { v: i64 }\n\npub fn make() -> i64 {\n    let h = Holder { v: 8 }\n    return h.v\n}\n",
            ),
        ],
        8,
        "",
    );
}

const STRUCT_MODULE: &str = "pub struct P { pub v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n\npub fn take(p: P) -> i64 {\n    return p.v\n}\n";

#[test]
fn group_mod_y1_an_imported_struct_returned_across() {
    run_row(
        "Y-1",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.make(6)\n    return p.v\n}\n",
            ),
            ("m.aelys", STRUCT_MODULE),
        ],
        6,
        "",
    );
}

#[test]
fn group_mod_y2_an_imported_struct_named_in_an_annotation() {
    run_row(
        "Y-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p: m.P = m.make(7)\n    return p.v\n}\n",
            ),
            ("m.aelys", STRUCT_MODULE),
        ],
        7,
        "",
    );
}

#[test]
fn group_mod_y3_an_imported_struct_built_by_the_importer() {
    run_row(
        "Y-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.P { v: 8 }\n    return m.take(p)\n}\n",
            ),
            ("m.aelys", STRUCT_MODULE),
        ],
        8,
        "",
    );
}

#[test]
fn group_mod_y4_an_imported_struct_passed_back_across() {
    run_row(
        "Y-4",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.take(m.make(11))\n}\n",
            ),
            ("m.aelys", STRUCT_MODULE),
        ],
        11,
        "",
    );
}

#[test]
fn group_mod_y5_a_from_imported_struct_binds_a_bare_name() {
    run_row(
        "Y-5",
        &[
            (
                "root.aelys",
                "needs P, make from m\n\nfn main() -> i64 {\n    let p: P = make(12)\n    return p.v\n}\n",
            ),
            ("m.aelys", STRUCT_MODULE),
        ],
        12,
        "",
    );
}

#[test]
fn group_mod_y6_a_from_imported_struct_is_built_by_its_bare_name() {
    run_row(
        "Y-6",
        &[
            (
                "root.aelys",
                "needs P from m\n\nfn main() -> i64 {\n    let p = P { v: 13 }\n    return p.v\n}\n",
            ),
            ("m.aelys", STRUCT_MODULE),
        ],
        13,
        "",
    );
}

const ENUM_MODULE: &str = "pub enum T { None, Some(i64) }\n\npub fn pick() -> T {\n    return T::Some(9)\n}\n\npub fn unwrap(t: T) -> i64 {\n    return match t {\n        T::Some(n) => n\n        T::None => 0\n    }\n}\n";

#[test]
fn group_mod_y7_an_imported_enum_crosses_in_both_directions() {
    run_row(
        "Y-7",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.unwrap(m.pick())\n}\n",
            ),
            ("m.aelys", ENUM_MODULE),
        ],
        9,
        "",
    );
}

#[test]
fn group_mod_y8_a_from_imported_enum_is_built_and_matched() {
    run_row(
        "Y-8",
        &[
            (
                "root.aelys",
                "needs T from m\n\nfn main() -> i64 {\n    return match T::Some(14) {\n        T::Some(n) => n\n        T::None => 0\n    }\n}\n",
            ),
            ("m.aelys", ENUM_MODULE),
        ],
        14,
        "",
    );
}

#[test]
fn group_mod_y9_a_qualified_enum_is_built_and_matched() {
    run_row(
        "Y-9",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return match m.T::Some(15) {\n        m.T::Some(n) => n\n        m.T::None => 0\n    }\n}\n",
            ),
            ("m.aelys", ENUM_MODULE),
        ],
        15,
        "",
    );
}

const VARIANTS: &[(&str, RuntimeVariant)] = &[
    ("leak", RuntimeVariant::Leak),
    ("rc", RuntimeVariant::Rc),
    ("rc+cycles", RuntimeVariant::RcCycles),
];

#[test]
fn group_mod_v1_a_managed_value_crosses_under_every_runtime() {
    let files: Files = &[
        (
            "root.aelys",
            "needs m\n\nfn main() -> i64 {\n    let s = m.greet()\n    println(s)\n    return s.len\n}\n",
        ),
        (
            "m.aelys",
            "pub fn greet() -> string {\n    return \"hello\"\n}\n",
        ),
    ];
    for (level, opt) in LEVELS {
        for (name, variant) in VARIANTS {
            let dir = stage(files);
            let root = dir.path().join("root.aelys");
            if let Err(err) = compile_file_with_llvm_variant(&root, *opt, false, *variant) {
                panic!("V-1 at {level} under {name}: MUST compile and link\nerror:\n{err}");
            }
            let out = Command::new(exe_path_for(&root))
                .output()
                .expect("run executable");
            assert_eq!(
                exit_code(&out.status),
                5,
                "V-1 at {level} under {name}: the length MUST cross as 5\nstderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                "hello\n",
                "V-1 at {level} under {name}: the string MUST be the one m.aelys defines"
            );
        }
    }
}

#[test]
fn group_mod_r1_a_missing_module_names_the_file_it_looked_for() {
    reject_row(
        "R-1",
        &[(
            "root.aelys",
            "needs nope\n\nfn main() -> i64 {\n    return 0\n}\n",
        )],
        "[E0601]",
        "nope.aelys",
    );
}

#[test]
fn group_mod_r2_a_cycle_names_its_chain() {
    reject_row(
        "R-2",
        &[
            (
                "root.aelys",
                "needs a\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
            (
                "a.aelys",
                "needs b\n\npub fn f() -> i64 {\n    return 1\n}\n",
            ),
            (
                "b.aelys",
                "needs a\n\npub fn g() -> i64 {\n    return 2\n}\n",
            ),
        ],
        "[E0602]",
        "a -> b -> a",
    );
}

#[test]
fn group_mod_r3_two_imports_may_not_introduce_one_name() {
    reject_row(
        "R-3",
        &[
            (
                "root.aelys",
                "needs m\nneeds n as m\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
            ("m.aelys", SEVEN),
            ("n.aelys", "pub fn eight() -> i64 {\n    return 8\n}\n"),
        ],
        "[E0603]",
        "'m'",
    );
}

#[test]
fn group_mod_r3b_an_import_may_not_take_a_name_the_module_defines() {
    reject_row(
        "R-3b",
        &[
            (
                "root.aelys",
                "needs seven from m\n\nfn seven() -> i64 {\n    return 1\n}\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
            ("m.aelys", SEVEN),
        ],
        "[E0603]",
        "'seven'",
    );
}

const DUP_FN_A: &str = "fn P() -> i64 {\n    return 1\n}\n";
const DUP_FN_B: &str = "fn P() -> i64 {\n    return 2\n}\n";
const DUP_FN_SIG: &str = "fn P(a: i64) -> i64 {\n    return a\n}\n";
const DUP_LET_A: &str = "pub let P: i64 = 1\n";
const DUP_LET_B: &str = "pub let P: i64 = 2\n";
const DUP_STRUCT_A: &str = "struct P {\n    x: i64,\n}\n";
const DUP_STRUCT_B: &str = "struct P {\n    y: i64,\n}\n";
const DUP_ENUM_A: &str = "enum P {\n    A,\n    B,\n}\n";
const DUP_ENUM_B: &str = "enum P {\n    C,\n    D,\n}\n";
const DUP_MAIN: &str = "\nfn main() -> i64 {\n    return 0\n}\n";

const DUPLICATE_FORMS: &[(&str, &str, &str, &str)] = &[
    ("DUP-1", DUP_FN_A, DUP_FN_B, "twice as a function"),
    ("DUP-1b", DUP_FN_A, DUP_FN_A, "twice as a function"),
    ("DUP-1c", DUP_FN_A, DUP_FN_SIG, "twice as a function"),
    ("DUP-2", DUP_LET_A, DUP_LET_B, "twice as a global"),
    ("DUP-3", DUP_STRUCT_A, DUP_STRUCT_B, "twice as a struct"),
    ("DUP-3b", DUP_STRUCT_A, DUP_STRUCT_A, "twice as a struct"),
    ("DUP-4", DUP_ENUM_A, DUP_ENUM_B, "twice as an enum"),
    ("DUP-4b", DUP_ENUM_A, DUP_ENUM_A, "twice as an enum"),
    ("DUP-5", DUP_FN_A, DUP_LET_B, "once as a function and once as a global"),
    ("DUP-5r", DUP_LET_A, DUP_FN_B, "once as a global and once as a function"),
    ("DUP-6", DUP_FN_A, DUP_STRUCT_A, "once as a function and once as a struct"),
    ("DUP-6r", DUP_STRUCT_A, DUP_FN_B, "once as a struct and once as a function"),
    ("DUP-7", DUP_FN_A, DUP_ENUM_A, "once as a function and once as an enum"),
    ("DUP-7r", DUP_ENUM_A, DUP_FN_B, "once as an enum and once as a function"),
    ("DUP-8", DUP_LET_A, DUP_STRUCT_A, "once as a global and once as a struct"),
    ("DUP-8r", DUP_STRUCT_A, DUP_LET_B, "once as a struct and once as a global"),
    ("DUP-9", DUP_LET_A, DUP_ENUM_A, "once as a global and once as an enum"),
    ("DUP-9r", DUP_ENUM_A, DUP_LET_B, "once as an enum and once as a global"),
    ("DUP-10", DUP_STRUCT_A, DUP_ENUM_A, "once as a struct and once as an enum"),
    ("DUP-10r", DUP_ENUM_A, DUP_STRUCT_A, "once as an enum and once as a struct"),
];

#[test]
fn group_mod_dup_every_pair_of_top_level_binding_forms_is_rejected() {
    for (id, first, second, phrasing) in DUPLICATE_FORMS {
        let source = format!("{first}{second}{DUP_MAIN}");
        for (level, opt) in LEVELS {
            let dir = tempdir().expect("tempdir");
            let root = dir.path().join("root.aelys");
            fs::write(&root, &source).expect("write fixture");
            let rendered = match lower_file_to_air(&root, *opt) {
                Ok(_) => panic!(
                    "{id} at {level}: two top level definitions of `P` MUST be rejected
{source}"
                ),
                Err(rendered) => rendered.to_string(),
            };
            assert!(
                rendered.contains("[E0204]"),
                "{id} at {level}: the diagnostic MUST carry E0204
{source}
rendered:
{rendered}"
            );
            assert!(
                rendered.contains(phrasing),
                "{id} at {level}: the headline MUST say {phrasing:?}
{source}
rendered:
{rendered}"
            );
            assert!(
                rendered.contains("is first defined here"),
                "{id} at {level}: the diagnostic MUST point at the first definition too
{source}
rendered:
{rendered}"
            );
        }
    }
}

#[test]
fn group_mod_dup_a_duplicate_definition_draws_a_single_diagnostic_and_no_consequence() {
    let source = format!("{DUP_FN_A}{DUP_FN_SIG}
fn main() -> i64 {{
    return P(7)
}}
");
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, &source).expect("write fixture");
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("DUP-11: MUST be rejected\n{source}"),
        Err(rendered) => rendered.to_string(),
    };
    assert_eq!(
        rendered.matches("error[").count(),
        1,
        "DUP-11: failing at the resolve layer means the second body is never checked against the \
         first signature, so the arity consequence must not appear\n{source}\nrendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("E0302"),
        "DUP-11: no arity consequence\n{source}\nrendered:\n{rendered}"
    );
}

#[test]
fn group_mod_dup_a_source_duplicate_answers_before_the_symbol_check() {
    let source = format!("{DUP_FN_A}{DUP_FN_B}{DUP_MAIN}");
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, &source).expect("write fixture");
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("DUP-12: MUST be rejected\n{source}"),
        Err(rendered) => rendered.to_string(),
    };
    assert!(
        rendered.contains("[E0204]") && !rendered.contains("[E0427]"),
        "DUP-12: E0204 has both source spans and E0427 only has a mangled symbol, so the source \
         level answer must win\n{source}\nrendered:\n{rendered}"
    );
}

#[test]
fn group_mod_r4_a_reserved_path_segment_is_rejected() {
    reject_row(
        "R-4",
        &[(
            "root.aelys",
            "needs __hidden\n\nfn main() -> i64 {\n    return 0\n}\n",
        )],
        "[E0604]",
        "__hidden",
    );
}

#[test]
fn group_mod_r5_a_private_item_is_not_reachable_through_a_namespace() {
    reject_row(
        "R-5",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.secret()\n}\n",
            ),
            ("m.aelys", "fn secret() -> i64 {\n    return 1\n}\n"),
        ],
        "[E0605]",
        "secret",
    );
}

#[test]
fn group_mod_r5b_a_private_item_is_not_reachable_through_from() {
    reject_row(
        "R-5b",
        &[
            (
                "root.aelys",
                "needs secret from m\n\nfn main() -> i64 {\n    return secret()\n}\n",
            ),
            ("m.aelys", "fn secret() -> i64 {\n    return 1\n}\n"),
        ],
        "[E0605]",
        "secret",
    );
}

#[test]
fn group_mod_r5c_a_private_type_is_not_reachable() {
    reject_row(
        "R-5c",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p: m.P = m.make(1)\n    return p\n}\n",
            ),
            (
                "m.aelys",
                "struct P { v: i64 }\n\npub fn make(n: i64) -> i64 {\n    let p = P { v: n }\n    return p.v\n}\n",
            ),
        ],
        "[E0605]",
        "`P`",
    );
}

#[test]
fn group_mod_r6_an_absent_item_is_named_as_absent_not_as_private() {
    reject_row(
        "R-6",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.absent()\n}\n",
            ),
            ("m.aelys", SEVEN),
        ],
        "[E0606]",
        "absent",
    );
}

#[test]
fn group_mod_r6b_an_absent_item_is_absent_through_from_too() {
    reject_row(
        "R-6b",
        &[
            (
                "root.aelys",
                "needs absent from m\n\nfn main() -> i64 {\n    return absent()\n}\n",
            ),
            ("m.aelys", SEVEN),
        ],
        "[E0606]",
        "absent",
    );
}

#[test]
fn group_mod_r7_a_c_header_target_is_rejected_and_never_miscompiled() {
    reject_row(
        "R-7",
        &[(
            "root.aelys",
            "needs \"GL/glext.h\"\n\nfn main() -> i64 {\n    return 0\n}\n",
        )],
        "[E0607]",
        "GL/glext.h",
    );
}

#[test]
fn group_mod_r7b_an_empty_header_is_still_a_header() {
    reject_row(
        "R-7b",
        &[(
            "root.aelys",
            "needs \"\"\n\nfn main() -> i64 {\n    return 0\n}\n",
        )],
        "[E0607]",
        "not implemented yet",
    );
}

#[test]
fn group_mod_r8_a_needs_after_a_declaration_is_rejected() {
    reject_row(
        "R-8",
        &[
            (
                "root.aelys",
                "fn main() -> i64 {\n    return 0\n}\n\nneeds m\n",
            ),
            ("m.aelys", SEVEN),
        ],
        "[E0608]",
        "before any other top-level declaration",
    );
}

#[test]
fn group_mod_r8b_a_needs_inside_a_body_is_rejected() {
    reject_row(
        "R-8b",
        &[
            (
                "root.aelys",
                "fn main() -> i64 {\n    needs m\n    return 0\n}\n",
            ),
            ("m.aelys", SEVEN),
        ],
        "[E0608]",
        "before any other top-level declaration",
    );
}

#[test]
fn group_mod_r9_a_wildcard_import_is_rejected() {
    reject_row(
        "R-9",
        &[
            (
                "root.aelys",
                "needs m.*\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
            ("m.aelys", SEVEN),
        ],
        "[E0609]",
        "not implemented yet",
    );
}

// ---------- g: generated and mangled names that qualification had cut ----------

#[test]
fn group_mod_g1_a_generic_enum_is_usable_inside_its_own_module() {
    run_row(
        "G-1",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.make()\n}\n",
            ),
            (
                "m.aelys",
                "pub enum Opt<T> { Some(T), None }\n\npub fn make() -> i64 {\n    let o = Opt::Some(5)\n    return match o {\n        Opt::Some(v) => v\n        Opt::None => 0\n    }\n}\n",
            ),
        ],
        5,
        "",
    );
}

#[test]
fn group_mod_g2_a_module_holds_a_whole_result_pipeline() {
    run_row(
        "G-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.plain(20)\n}\n",
            ),
            (
                "m.aelys",
                "enum Result<T, E> { Ok(T), Err(E) }\n\nfn might(n: i64) -> Result<i64, i64> {\n    if n > 0 {\n        return Result::Ok(n * 2)\n    }\n    return Result::Err(1)\n}\n\nfn run(n: i64) -> Result<i64, i64> {\n    let v = might(n)?\n    return Result::Ok(v + 1)\n}\n\npub fn plain(n: i64) -> i64 {\n    return run(n).unwrap()\n}\n",
            ),
        ],
        41,
        "",
    );
}

#[test]
fn group_mod_g3_a_capturing_lambda_under_a_generic_keeps_its_env() {
    run_row(
        "G-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.gen(7, 41)\n}\n",
            ),
            (
                "m.aelys",
                "pub fn gen<T>(x: T, k: i64) -> i64 {\n    let f = fn() -> i64 { return k + 1 }\n    return f()\n}\n",
            ),
        ],
        42,
        "",
    );
}

// the qualified struct literal must not swallow the empty block of a member condition
#[test]
fn group_mod_g4_a_member_condition_over_an_empty_block_still_parses() {
    run_row(
        "G-4",
        &[(
            "root.aelys",
            "struct S { X: bool }\n\nfn main() -> i64 {\n    let s = S { X: true }\n    if s.X { }\n    return 3\n}\n",
        )],
        3,
        "",
    );
}


const DEEP_STRUCT: &str = "pub struct P { pub hi: i64, pub lo: i64 }\n\npub fn make(x: i64, y: i64) -> P {\n    return P { hi: x, lo: y }\n}\n\npub fn read(p: P) -> i64 {\n    return p.hi\n}\n";
const DEEP_MID: &str = "needs a\n\npub fn pass(x: i64, y: i64) -> a.P {\n    return a.make(x, y)\n}\n\npub fn take(p: a.P) -> i64 {\n    return a.read(p)\n}\n";

#[test]
fn group_mod_t1_a_type_survives_two_module_hops() {
    run_row(
        "T-1",
        &[
            (
                "root.aelys",
                "needs mid\n\nfn main() -> i64 {\n    let p = mid.pass(41, 99)\n    return mid.take(p)\n}\n",
            ),
            ("mid.aelys", DEEP_MID),
            ("a.aelys", DEEP_STRUCT),
        ],
        41,
        "",
    );
}

// a transitively reached type must be a type, not a fresh variable that unifies with anything
#[test]
fn group_mod_t2_a_transitive_type_is_not_a_free_variable() {
    reject_row(
        "T-2",
        &[
            (
                "root.aelys",
                "needs mid\n\nfn show(s: string) -> i64 {\n    println(s)\n    return 0\n}\n\nfn main() -> i64 {\n    let p = mid.pass(41, 99)\n    return show(p)\n}\n",
            ),
            ("mid.aelys", DEEP_MID),
            ("a.aelys", DEEP_STRUCT),
        ],
        "[E0301]",
        "a.P",
    );
}

#[test]
fn group_mod_t3_a_module_may_stand_on_another_module() {
    run_row(
        "T-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let x = m.get()\n    return x.v\n}\n",
            ),
            (
                "m.aelys",
                "needs dep\n\npub fn get() -> dep.X {\n    return dep.mk(6)\n}\n",
            ),
            (
                "dep.aelys",
                "pub struct X { pub v: i64 }\n\npub fn mk(n: i64) -> X {\n    return X { v: n }\n}\n",
            ),
        ],
        6,
        "",
    );
}


const PRIVATE_FIELD: &str = "pub struct P { v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n\npub fn read(p: P) -> i64 {\n    return p.v\n}\n";

#[test]
fn group_mod_f1_a_private_field_cannot_be_read_across() {
    reject_row(
        "F-1",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.make(3)\n    return p.v\n}\n",
            ),
            ("m.aelys", PRIVATE_FIELD),
        ],
        "[E0610]",
        "`v`",
    );
}

#[test]
fn group_mod_f2_a_private_field_cannot_be_written_across() {
    reject_row(
        "F-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let mut p = m.make(3)\n    p.v = 9\n    return 0\n}\n",
            ),
            ("m.aelys", PRIVATE_FIELD),
        ],
        "[E0610]",
        "`v`",
    );
}

#[test]
fn group_mod_f3_a_private_field_cannot_be_supplied_across() {
    reject_row(
        "F-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.P { v: 8 }\n    return m.read(p)\n}\n",
            ),
            ("m.aelys", PRIVATE_FIELD),
        ],
        "[E0610]",
        "`v`",
    );
}

#[test]
fn group_mod_f4_an_exported_field_crosses() {
    run_row(
        "F-4",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let mut p = m.make(1)\n    p.v = 3\n    return p.v\n}\n",
            ),
            (
                "m.aelys",
                "pub struct P { pub v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n",
            ),
        ],
        3,
        "",
    );
}

#[test]
fn group_mod_f5_a_module_reads_its_own_private_field() {
    run_row(
        "F-5",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.read(m.make(5))\n}\n",
            ),
            ("m.aelys", PRIVATE_FIELD),
        ],
        5,
        "",
    );
}


#[test]
fn group_mod_v2_a_public_function_may_not_return_a_private_type() {
    reject_row(
        "V-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.make(1)\n    return 0\n}\n",
            ),
            (
                "m.aelys",
                "struct P { v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n",
            ),
        ],
        "[E0611]",
        "`P`",
    );
}

// the escape a public enum payload used to offer
#[test]
fn group_mod_v3_a_public_enum_may_not_carry_a_private_type() {
    reject_row(
        "V-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let e = m.pick()\n    return 0\n}\n",
            ),
            (
                "m.aelys",
                "struct P { v: i64 }\n\npub enum E { None, Some(P) }\n\npub fn pick() -> E {\n    return E::Some(P { v: 5 })\n}\n",
            ),
        ],
        "[E0611]",
        "`P`",
    );
}

#[test]
fn group_mod_v4_a_public_signature_over_public_types_is_kept() {
    run_row(
        "V-4",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.make(4)\n    return p.v\n}\n",
            ),
            (
                "m.aelys",
                "pub struct P { pub v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n",
            ),
        ],
        4,
        "",
    );
}

// ---------- p: a module path spelt like one of monomorphisation's reserved prefixes ----------

#[test]
fn group_mod_p2_a_generic_enum_carries_its_own_struct_across() {
    run_row(
        "P-2",
        &[
            (
                "root.aelys",
                "needs vec_x\n\nfn main() -> i64 {\n    return vec_x.run()\n}\n",
            ),
            (
                "vec_x.aelys",
                "pub struct Holder { pub a: i64, pub b: i64 }\n\nenum Opt<T> { Some(T), None }\n\npub fn run() -> i64 {\n    let h = Holder { a: 10, b: 7 }\n    let o = Opt::Some(h)\n    return match o {\n        Opt::Some(v) => v.a + v.b\n        Opt::None => 0\n    }\n}\n",
            ),
        ],
        17,
        "",
    );
}

#[test]
fn group_mod_p3_a_root_generic_instantiates_over_an_imported_struct() {
    run_row(
        "P-3",
        &[
            (
                "root.aelys",
                "needs vec_x\n\nfn keep<T>(v: T) -> T {\n    return v\n}\n\nfn main() -> i64 {\n    let h = keep(vec_x.make(10, 7))\n    return h.a + h.b\n}\n",
            ),
            (
                "vec_x.aelys",
                "pub struct Holder { pub a: i64, pub b: i64 }\n\npub fn make(x: i64, y: i64) -> Holder {\n    return Holder { a: x, b: y }\n}\n",
            ),
        ],
        17,
        "",
    );
}


// `keep<holder>` from a module named `ptr_x` and `keep<&holder>` from a module named `x` mangle
#[test]
fn group_mod_p4_a_module_path_may_not_be_read_as_a_type_prefix() {
    const HOLDER: &str = "pub struct Holder { pub a: i64, pub b: i64 }\n\npub fn make(x: i64, y: i64) -> Holder {\n    return Holder { a: x, b: y }\n}\n";
    run_row(
        "P-4",
        &[
            (
                "root.aelys",
                "needs ptr_x\nneeds x\n\nfn keep<T>(v: T) -> T {\n    return v\n}\n\nfn main() -> i64 {\n    let h = keep(ptr_x.make(10, 7))\n    let q = x.make(1, 2)\n    let r = &q\n    let s = keep(r)\n    return h.a + h.b + q.a + q.b\n}\n",
            ),
            ("ptr_x.aelys", HOLDER),
            ("x.aelys", HOLDER),
        ],
        20,
        "",
    );
}


#[test]
fn group_mod_w1_a_private_field_may_hold_a_private_type() {
    run_row(
        "W-1",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.read(m.open(7))\n}\n",
            ),
            (
                "m.aelys",
                "struct Raw { v: i64 }\n\npub struct Handle { raw: Raw }\n\npub fn open(n: i64) -> Handle {\n    return Handle { raw: Raw { v: n } }\n}\n\npub fn read(h: Handle) -> i64 {\n    return h.raw.v\n}\n",
            ),
        ],
        7,
        "",
    );
}

#[test]
fn group_mod_w2_an_exported_field_may_not_hold_a_private_type() {
    reject_row(
        "W-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.open(7).raw.v\n}\n",
            ),
            (
                "m.aelys",
                "struct Raw { v: i64 }\n\npub struct Handle { pub raw: Raw }\n\npub fn open(n: i64) -> Handle {\n    return Handle { raw: Raw { v: n } }\n}\n",
            ),
        ],
        "[E0611]",
        "`Raw`",
    );
}

#[test]
fn group_mod_w3_the_root_may_name_its_own_private_types() {
    run_row(
        "W-3",
        &[(
            "root.aelys",
            "struct P { v: i64 }\n\npub fn mk(n: i64) -> P {\n    return P { v: n }\n}\n\nfn main() -> i64 {\n    return mk(1).v\n}\n",
        )],
        1,
        "",
    );
}

// ---------- p-7: the compiler's own spelling never reaches a user ----------

const LEAK_CONVERTERS: &[(&str, &str, &str)] = &[
    (
        "vec-surface",
        "pub struct Holder { pub v: i64 }\n\npub enum Opt<T> { None, Some(T) }\n\npub fn go() -> i64 {\n    let xs: vec<Holder> = vec[Holder { v: 9 }]\n    let o: Opt<vec<Holder>> = Opt::Some(xs)\n    return match o {\n        Opt::Some(h) => h[0].v\n        Opt::None => 0\n    }\n}\n",
        "return m.go()",
    ),
    (
        "air-layout",
        "pub struct Node { next: Node }\n\npub fn make() -> i64 {\n    return 1\n}\n",
        "return m.make()",
    ),
    (
        "rc-surface",
        "pub struct Holder { r: Rc<i64> }\n\npub fn make() -> i64 {\n    return 0\n}\n",
        "let b: vec<m.Holder> = Vec::new()\n    return 0",
    ),
];

const LEAK_MODULE: &str = "pub struct P { pub v: i64 }\n\npub enum E { None, Some(i64) }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n\npub fn pick() -> E {\n    return E::Some(1)\n}\n";

const LEAK_BODIES: &[(&str, &str)] = &[
    (
        "unknown-field-write",
        "let mut p = m.make(1)\n    p.zzz = 2\n    return 0",
    ),
    ("unknown-field-read", "return m.make(1).zzz"),
    (
        "unknown-field-literal",
        "let p = m.P { zzz: 1 }\n    return 0",
    ),
    ("unknown-variant", "let e = m.E::Zzz\n    return 0"),
    (
        "unknown-variant-pattern",
        "return match m.pick() {\n        m.E::Zzz(n) => n\n        m.E::None => 0\n    }",
    ),
    (
        "non-exhaustive-match",
        "return match m.pick() {\n        m.E::Some(n) => n\n    }",
    ),
    ("struct-mismatch", "let q: i64 = m.make(1)\n    return q"),
    ("enum-mismatch", "let q: i64 = m.pick()\n    return q"),
];

#[test]
fn group_mod_p7_no_diagnostic_shows_the_qualification_head() {
    for (shape, module, body) in LEAK_CONVERTERS {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("m.aelys"), module).expect("write module");
        let root = dir.path().join("root.aelys");
        fs::write(
            &root,
            format!("needs m\n\nfn main() -> i64 {{\n    {body}\n}}\n"),
        )
        .expect("write root");
        let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("P-7 {shape}: the probe MUST be rejected, or it proves nothing"),
            Err(rendered) => rendered,
        };
        assert!(
            !rendered.contains("__q"),
            "P-7 {shape}: no diagnostic may show the qualification head\nrendered:\n{rendered}"
        );
    }

    for (shape, body) in LEAK_BODIES {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("m.aelys"), LEAK_MODULE).expect("write module");
        let root = dir.path().join("root.aelys");
        fs::write(
            &root,
            format!("needs m\n\nfn main() -> i64 {{\n    {body}\n}}\n"),
        )
        .expect("write root");
        let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("P-7 {shape}: the probe MUST be rejected, or it proves nothing"),
            Err(rendered) => rendered,
        };
        assert!(
            !rendered.contains("__q"),
            "P-7 {shape}: no diagnostic may show the qualification head\nrendered:\n{rendered}"
        );
    }
}
