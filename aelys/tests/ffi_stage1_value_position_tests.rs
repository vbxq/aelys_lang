use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const LABS: &str = "unsafe extern fn labs(x: i64) -> i64\n";

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

fn rejects_as_a_value(id: &str, body: &str) {
    rejects_as_a_value_n(id, body, 1);
}

fn rejects_as_a_value_n(id: &str, body: &str, hits: usize) {
    let source = format!("{LABS}\n{body}");
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root.aelys");
        fs::write(&root, &source).expect("write fixture");
        let rendered = match lower_file_to_air(&root, *opt) {
            Ok(_) => panic!("{id} at {level}: naming the declaration MUST be rejected\n{source}"),
            Err(rendered) => rendered,
        };
        let message = message_only(&rendered);
        assert!(
            message.contains("E0616"),
            "{id} at {level}: the rejection MUST be E0616\nrendered:\n{rendered}"
        );
        assert!(
            message.contains(
                "`labs` is an external declaration, so it may only be called directly, never used \
                 as a value"
            ),
            "{id} at {level}: the message MUST name the remedy\nrendered:\n{rendered}"
        );
        assert_eq!(
            message.matches("error[E0616]").count(),
            hits,
            "{id} at {level}: {hits} rejection(s) expected\nrendered:\n{rendered}"
        );
    }
}

#[test]
fn f3_23_an_unannotated_let_is_a_value_use() {
    rejects_as_a_value(
        "F3-23",
        "fn main() -> i64 {\n    let g = labs\n    return g(-37)\n}\n",
    );
}

// gates the call site on , so the accepted spelling now carries its own block
#[test]
fn f3_24_the_direct_call_stays_accepted() {
    let source = format!("{LABS}\nfn main() -> i64 {{\n    unsafe {{ return labs(-37) }}\n}}\n");
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root.aelys");
        fs::write(&root, &source).expect("write fixture");
        if let Err(err) = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
            panic!(
                "{} at {level}: the direct call MUST stay accepted, a rule that rejected it \
                 would break the whole feature\nerror:\n{err}",
                "F3-24"
            );
        }
    }
}

#[test]
fn f3_24_bis_the_old_spelling_of_the_direct_call_is_now_e0617() {
    let source = format!("{LABS}\nfn main() -> i64 {{\n    return labs(-37)\n}}\n");
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root.aelys");
        fs::write(&root, &source).expect("write fixture");
        let rendered = match lower_file_to_air(&root, *opt) {
            Ok(_) => panic!("F3-24-bis at {level}: the bare direct call MUST be rejected\n{source}"),
            Err(rendered) => rendered,
        };
        assert!(
            message_only(&rendered).contains("E0617"),
            "F3-24-bis at {level}: the rejection MUST be E0617\nrendered:\n{rendered}"
        );
    }
}

#[test]
fn f3_25_a_parenthesised_callee_is_a_value_use() {
    rejects_as_a_value("F3-25", "fn main() -> i64 {\n    return (labs)(-37)\n}\n");
}

#[test]
fn f3_26_an_annotated_let_is_a_value_use() {
    rejects_as_a_value(
        "F3-26",
        "fn main() -> i64 {\n    let g: fn(i64) -> i64 = labs\n    return g(-37)\n}\n",
    );
}

#[test]
fn f3_27_an_argument_is_a_value_use() {
    rejects_as_a_value(
        "F3-27",
        "fn apply(f: fn(i64) -> i64) -> i64 {\n    return f(-37)\n}\n\nfn main() -> i64 {\n    \
         return apply(labs)\n}\n",
    );
}

#[test]
fn f3_28_a_returned_name_is_a_value_use() {
    rejects_as_a_value(
        "F3-28",
        "fn pick() -> fn(i64) -> i64 {\n    return labs\n}\n\nfn main() -> i64 {\n    return \
         pick()(-37)\n}\n",
    );
}

#[test]
fn f3_29_a_struct_field_is_a_value_use() {
    rejects_as_a_value(
        "F3-29",
        "struct Box {\n    cb: fn(i64) -> i64,\n}\n\nfn main() -> i64 {\n    let b = Box { cb: \
         labs }\n    return b.cb(-37)\n}\n",
    );
}

#[test]
fn f3_30_an_array_element_is_a_value_use() {
    rejects_as_a_value_n(
        "F3-30",
        "fn main() -> i64 {\n    let a = [labs, labs]\n    return a[0](-37)\n}\n",
        2,
    );
}

#[test]
fn f3_31_a_lambda_body_is_a_value_use() {
    rejects_as_a_value(
        "F3-31",
        "fn main() -> i64 {\n    let mk = fn() -> fn(i64) -> i64 { return labs }\n    return \
         mk()(-37)\n}\n",
    );
}

#[test]
fn f3_32_an_assignment_is_a_value_use() {
    rejects_as_a_value(
        "F3-32",
        "fn other(x: i64) -> i64 {\n    return x\n}\n\nfn main() -> i64 {\n    let mut g = \
         other\n    g = labs\n    return g(-37)\n}\n",
    );
}

#[test]
fn f3_33_an_unsafe_block_hides_nothing() {
    rejects_as_a_value(
        "F3-33",
        "fn main() -> i64 {\n    unsafe {\n        let g = labs\n    }\n    return 0\n}\n",
    );
}

#[test]
fn f3_34_the_library_route_rejects_it_too() {
    let source = format!("{LABS}\nfn main() -> i64 {{\n    let g = labs\n    return g(-37)\n}}\n");
    let err = aelys_driver::compile_to_typed_ast(&source)
        .expect_err("F3-34: the typed route MUST reject it as well");
    let rendered = err.to_string();
    assert!(
        rendered.contains("E0616"),
        "F3-34: the typed route MUST carry E0616\nrendered:\n{rendered}"
    );
}

#[test]
fn f3_35_a_nogc_generic_still_answers_with_its_own_code() {
    let source = "nogc fn keep<T: nogc>(t: T) -> T {\n    return t\n}\n\nfn main() -> i64 {\n    \
                  let g = keep\n    return 0\n}\n";
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("F3-35: the twin predicate MUST still bite"),
        Err(rendered) => rendered,
    };
    let message = message_only(&rendered);
    assert!(
        message.contains("E0731") && !message.contains("E0616"),
        "F3-35: the two membership sets stay disjoint\nrendered:\n{rendered}"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn validate_expr_has_no_catch_all_arm() {
    let text = fs::read_to_string(repo_root().join("sema/src/infer/validate.rs"))
        .expect("read validate.rs");
    let start = text
        .find("fn validate_expr")
        .expect("validate_expr must exist");
    let body = &text[start..];
    assert_eq!(
        body.matches("_ => {}").count(),
        0,
        "a catch-all arm would let a new expression form skip the traversal without a compiler \
         error"
    );
}

#[test]
fn lower_callee_names_two_forms() {
    let text = fs::read_to_string(repo_root().join("air/src/lower/expr.rs")).expect("read expr.rs");
    let start = text
        .find("fn lower_callee")
        .expect("lower_callee must exist");
    let body = &text[start..];
    let end = body
        .find("\n    fn ")
        .or_else(|| body.find("\n    pub fn "))
        .expect("lower_callee must end");
    assert_eq!(
        body[..end].matches("Callee::Named(").count(),
        2,
        "a third source form rendered as a named callee would need its own exemption in \
         `validate_expr`"
    );
}

#[test]
fn f3_31_a_parameter_spelling_the_declaration_is_not_the_declaration() {
    let source = format!(
        "{LABS}\nfn g(labs: fn(i64) -> i64) -> i64 {{\n    let f = labs\n    return f(3)\n}}\n         fn seven(x: i64) -> i64 {{ return 7 }}\n         fn main() -> i64 {{ return g(seven) }}\n"
    );
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root.aelys");
        fs::write(&root, &source).expect("write fixture");
        if let Err(err) = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
            panic!(
                "F3-31 at {level}: a parameter that spells the declaration MUST stay accepted\n                 {source}\nerror:\n{err}"
            );
        }
        let exe = root.with_extension("");
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("{} >/dev/null 2>&1; echo $?", exe.display()))
            .output()
            .expect("shell should run the compiled executable");
        let got = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<i32>()
            .expect("a shell status is an integer below 256");
        assert_eq!(
            got, 7,
            "F3-31 at {level}: the parameter answers, not the foreign symbol\n{source}"
        );
    }
}
