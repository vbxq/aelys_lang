
use aelys_driver::{LinkRequirement, RuntimeVariant, compile_file_with_llvm_linked};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const TRIPLE_C: &str = "long aelys_stage4_triple(long x) { return 3 * x; }\n";

const TRIPLE_AE: &str = "unsafe extern fn aelys_stage4_triple(x: i64) -> i64\n\
                         fn main() -> i64 { unsafe { return aelys_stage4_triple(14) } }\n";

// a missing toolchain must redden the row, never skip it
fn tool(name: &str) -> String {
    let found = Command::new(name).arg("--version").output();
    assert!(
        found.map(|out| out.status.success()).unwrap_or(false),
        "this suite builds a real library, so `{name}` is required and its absence is a failure"
    );
    name.to_string()
}

fn run(program: &str, args: &[&str], dir: &Path) -> std::process::Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|err| panic!("failed to run `{program}`: {err}"))
}

fn compile_object(dir: &Path, source: &str, body: &str) {
    fs::write(dir.join(source), body).expect("write c source");
    let object = source.replace(".c", ".o");
    let out = run(&tool("clang"), &["-fPIC", "-c", "-o", &object, source], dir);
    assert!(out.status.success(), "clang failed: {out:?}");
}

fn archive(dir: &Path, name: &str, objects: &[&str]) {
    fs::create_dir_all(dir.join("lib")).expect("lib dir");
    let path = format!("lib/lib{name}.a");
    let mut args = vec!["rcs", path.as_str()];
    args.extend_from_slice(objects);
    let out = run(&tool("ar"), &args, dir);
    assert!(out.status.success(), "ar failed: {out:?}");
}

fn shared(dir: &Path, name: &str, source: &str) {
    fs::create_dir_all(dir.join("lib")).expect("lib dir");
    let path = format!("lib/lib{name}.so");
    let out = run(
        &tool("clang"),
        &["-shared", "-fPIC", "-o", &path, source],
        dir,
    );
    assert!(out.status.success(), "clang -shared failed: {out:?}");
}

fn with_triple_archive() -> TempDir {
    let dir = tempdir().expect("tempdir");
    compile_object(dir.path(), "mylib.c", TRIPLE_C);
    archive(dir.path(), "mystage4", &["mylib.o"]);
    dir
}

fn link_to(dir: &Path, libraries: &[&str]) -> LinkRequirement {
    LinkRequirement {
        search_paths: vec![dir.join("lib")],
        libraries: libraries.iter().map(|name| (*name).to_string()).collect(),
    }
}

fn compile(
    dir: &Path,
    stem: &str,
    source: &str,
    link: &LinkRequirement,
    opt: OptimizationLevel,
) -> Result<(), String> {
    let path = dir.join(format!("{stem}.aelys"));
    fs::write(&path, source).expect("write aelys source");
    compile_file_with_llvm_linked(&path, opt, false, RuntimeVariant::Rc, link)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn exe_path(dir: &Path, stem: &str) -> PathBuf {
    dir.join(stem)
}

fn execute(dir: &Path, stem: &str, env: &[(&str, &str)]) -> (i32, String) {
    let mut command = Command::new(exe_path(dir, stem));
    for (key, value) in env {
        command.env(key, value);
    }
    let out = command.output().expect("the executable should run");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn undefined_symbols(dir: &Path, stem: &str) -> String {
    let exe = exe_path(dir, stem).to_string_lossy().to_string();
    let out = run(&tool("nm"), &["-u", exe.as_str()], dir);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_real_static_archive_links_and_answers_forty_two_at_every_level() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["mystage4"]);
    for (name, opt) in LEVELS {
        compile(dir.path(), "l1", TRIPLE_AE, &link, opt)
            .unwrap_or_else(|err| panic!("L1 {name}: the link must succeed: {err}"));
        let (code, _) = execute(dir.path(), "l1", &[]);
        assert_eq!(
            code, 42,
            "L1 {name}: 3 x 14 is the answer no accident gives"
        );
    }
    // a shape check, not the anti-vacuity: it is green whenever the symbol is not dynamic
    assert!(
        !undefined_symbols(dir.path(), "l1").contains("aelys_stage4_triple"),
        "L1: the archive member should have been absorbed"
    );
}

#[test]
fn a_real_shared_object_links_and_answers_forty_two() {
    let dir = with_triple_archive();
    fs::write(dir.path().join("mydyn.c"), TRIPLE_C).expect("write c source");
    shared(dir.path(), "mystage4dyn", "mydyn.c");
    let link = link_to(dir.path(), &["mystage4dyn"]);
    for (name, opt) in LEVELS {
        compile(dir.path(), "l2", TRIPLE_AE, &link, opt)
            .unwrap_or_else(|err| panic!("L2 {name}: the link must succeed: {err}"));
        let lib = dir.path().join("lib").to_string_lossy().to_string();
        let (code, _) = execute(dir.path(), "l2", &[("LD_LIBRARY_PATH", lib.as_str())]);
        assert_eq!(code, 42, "L2 {name}: the shared object answers too");
    }
    assert!(
        undefined_symbols(dir.path(), "l2").contains("aelys_stage4_triple"),
        "L2: a shared object leaves the symbol undefined in the executable"
    );
    let exe = exe_path(dir.path(), "l2").to_string_lossy().to_string();
    let ldd = run("ldd", &[exe.as_str()], dir.path());
    assert!(
        String::from_utf8_lossy(&ldd.stdout).contains("libmystage4dyn.so"),
        "L2: ldd must name the shared object"
    );
    let (bare, _) = {
        let out = Command::new(exe_path(dir.path(), "l2"))
            .env_remove("LD_LIBRARY_PATH")
            .output()
            .expect("spawn");
        (out.status.code().unwrap_or(-1), out)
    };
    assert_ne!(
        bare, 42,
        "L2: without LD_LIBRARY_PATH the launch cannot work"
    );
}

#[test]
fn the_same_program_without_the_library_is_rejected_and_leaves_no_executable() {
    let dir = with_triple_archive();
    let err = compile(
        dir.path(),
        "l3",
        TRIPLE_AE,
        &LinkRequirement::default(),
        OptimizationLevel::None,
    )
    .expect_err("L3: without `-l` the link cannot resolve the symbol");
    assert!(
        err.contains("E0901"),
        "L3: the verdict is a backend error: {err}"
    );
    assert!(
        err.contains("undefined reference"),
        "L3: LC_ALL=C is what makes this text assertable: {err}"
    );
    assert!(
        !exe_path(dir.path(), "l3").exists(),
        "L3: a rejected link must leave no executable"
    );
}

#[test]
fn a_library_without_a_search_path_is_rejected() {
    let dir = with_triple_archive();
    let link = LinkRequirement {
        search_paths: Vec::new(),
        libraries: vec!["mystage4".to_string()],
    };
    let err = compile(dir.path(), "l4", TRIPLE_AE, &link, OptimizationLevel::None)
        .expect_err("L4: a bare `-l` finds only the system path");
    assert!(err.contains("E0901"), "L4: {err}");
    assert!(!exe_path(dir.path(), "l4").exists(), "L4: no executable");
}

#[test]
fn a_search_path_that_does_not_exist_is_a_rejection_not_a_panic() {
    let dir = with_triple_archive();
    let link = LinkRequirement {
        search_paths: vec![PathBuf::from("/aelys/no/such/directory")],
        libraries: vec!["mystage4".to_string()],
    };
    let err = compile(dir.path(), "l5", TRIPLE_AE, &link, OptimizationLevel::None)
        .expect_err("L5: the library is not on that path");
    assert!(err.contains("E0901"), "L5: {err}");
    assert!(!exe_path(dir.path(), "l5").exists(), "L5: no executable");
}

#[test]
fn a_library_that_does_not_exist_is_rejected() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["aelys_stage4_no_such_library"]);
    let err = compile(dir.path(), "l6", TRIPLE_AE, &link, OptimizationLevel::None)
        .expect_err("L6: the linker cannot find it");
    assert!(err.contains("E0901"), "L6: {err}");
    assert!(!exe_path(dir.path(), "l6").exists(), "L6: no executable");
}

fn parse(tokens: &[&str]) -> Result<aelys_cli::cli::args::ParsedArgs, String> {
    let owned: Vec<String> = std::iter::once("aelys".to_string())
        .chain(tokens.iter().map(|token| (*token).to_string()))
        .collect();
    aelys_cli::cli::args::parse_args(&owned)
}

#[test]
fn a_link_flag_without_a_value_is_an_argument_error_before_anything_is_compiled() {
    for tokens in [
        vec!["compile", "a.aelys", "-l"],
        vec!["compile", "a.aelys", "-L"],
        vec!["compile", "a.aelys", "--library"],
        vec!["compile", "a.aelys", "--library-path"],
    ] {
        let err = parse(&tokens).expect_err("a missing value must be refused");
        assert!(err.contains("requires a value"), "L7/L8: {err}");
    }
    for tokens in [
        vec!["compile", "a.aelys", "-l", ""],
        vec!["compile", "a.aelys", "-L", ""],
        vec!["compile", "a.aelys", "--library="],
        vec!["compile", "a.aelys", "--library-path="],
    ] {
        let err = parse(&tokens).expect_err("an empty value must be refused");
        assert!(err.contains("non-empty"), "L9: {err}");
    }
}

#[test]
fn the_glued_and_the_separated_spelling_build_the_same_requirement() {
    let glued = parse(&["compile", "a.aelys", "-Lfoo", "-lbar"]).expect("glued");
    let separated = parse(&["compile", "a.aelys", "-L", "foo", "-l", "bar"]).expect("separated");
    let long = parse(&["compile", "a.aelys", "--library-path=foo", "--library=bar"]).expect("long");
    assert_eq!(glued.link, separated.link, "L10: same link line");
    assert_eq!(glued.link, long.link, "L10: same link line");
    assert_eq!(glued.link.search_paths, vec![PathBuf::from("foo")]);
    assert_eq!(glued.link.libraries, vec!["bar".to_string()]);
}

#[test]
fn the_order_of_appearance_is_preserved() {
    let parsed = parse(&[
        "compile", "a.aelys", "-L", "a", "-L", "b", "-l", "x", "-l", "y",
    ])
    .expect("parse");
    assert_eq!(
        parsed.link.search_paths,
        vec![PathBuf::from("a"), PathBuf::from("b")],
        "L11: -L order is the order written"
    );
    assert_eq!(
        parsed.link.libraries,
        vec!["x".to_string(), "y".to_string()],
        "L11: -l order is the order written"
    );
}

#[test]
fn a_second_useless_library_does_not_break_the_link() {
    let dir = with_triple_archive();
    compile_object(
        dir.path(),
        "spare.c",
        "long aelys_stage4_spare(void) { return 1; }\n",
    );
    archive(dir.path(), "myspare", &["spare.o"]);
    let link = link_to(dir.path(), &["myspare", "mystage4"]);
    compile(dir.path(), "l12", TRIPLE_AE, &link, OptimizationLevel::None).expect("L12");
    assert_eq!(execute(dir.path(), "l12", &[]).0, 42, "L12");
}

#[test]
fn a_library_with_no_external_declaration_at_all_still_compiles() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["mystage4"]);
    compile(
        dir.path(),
        "l13",
        "fn main() -> i64 { return 7 }\n",
        &link,
        OptimizationLevel::None,
    )
    .expect("L13");
    assert_eq!(execute(dir.path(), "l13", &[]).0, 7, "L13");
}

#[test]
fn emitting_ir_ignores_the_link_requirement() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["aelys_stage4_no_such_library"]);
    let path = dir.path().join("l14.aelys");
    fs::write(&path, TRIPLE_AE).expect("write");
    compile_file_with_llvm_linked(
        &path,
        OptimizationLevel::None,
        true,
        RuntimeVariant::Rc,
        &link,
    )
    .expect("L14: emitting ir returns before the link");
    assert!(
        dir.path().join("l14.ll").exists(),
        "L14: the ir was written"
    );
}

#[test]
fn a_program_without_main_is_never_linked() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["aelys_stage4_no_such_library"]);
    compile(
        dir.path(),
        "l15",
        "fn helper() -> i64 { return 1 }\n",
        &link,
        OptimizationLevel::None,
    )
    .expect("L15: no main means no link, so the bad library never matters");
    assert!(
        !exe_path(dir.path(), "l15").exists(),
        "L15: nothing was linked"
    );
}

// l16, and the program must not allocate: on one that does, the wrong order reddens by a duplicate `main`
#[test]
fn a_hostile_archive_defining_main_never_runs() {
    let dir = with_triple_archive();
    compile_object(
        dir.path(),
        "hostile.c",
        "#include <stdio.h>\nint main(void) { printf(\"HOSTILE MAIN\\n\"); return 77; }\n",
    );
    archive(dir.path(), "myhostile", &["hostile.o"]);
    let link = link_to(dir.path(), &["myhostile"]);
    compile(
        dir.path(),
        "l16",
        "fn main() -> i64 { return 5 }\n",
        &link,
        OptimizationLevel::None,
    )
    .expect("L16: the archive is simply not searched for `main`");
    let (code, stdout) = execute(dir.path(), "l16", &[]);
    assert_eq!(code, 5, "L16: the aelys main is the one that runs");
    assert!(!stdout.contains("HOSTILE MAIN"), "L16: {stdout}");
}

const ALLOCATING_AE: &str = "unsafe extern fn aelys_stage4_triple(x: i64) -> i64\n\
                             fn main() -> i64 {\n\
                             \u{20}   let mut v = Vec::new()\n\
                             \u{20}   Vec::push(v, 11)\n\
                             \u{20}   unsafe { return aelys_stage4_triple(14) }\n\
                             }\n";

#[test]
fn an_archive_defining_memcpy_is_named_and_refused() {
    let dir = with_triple_archive();
    compile_object(
        dir.path(),
        "hmemcpy.c",
        "#include <stddef.h>\nvoid *memcpy(void *d, const void *s, size_t n) {\n\
         unsigned char *a = d; const unsigned char *b = s;\n\
         while (n--) *a++ = *b++;\n\
         return d;\n}\n",
    );
    archive(dir.path(), "mymemcpy", &["hmemcpy.o"]);
    let link = link_to(dir.path(), &["mystage4", "mymemcpy"]);
    let err = compile(
        dir.path(),
        "l17",
        ALLOCATING_AE,
        &link,
        OptimizationLevel::None,
    )
    .expect_err("L17: memcpy is one of the sixteen the runtime imports");
    assert!(
        err.contains("E0618") && err.contains("memcpy"),
        "L17: {err}"
    );
    assert!(!exe_path(dir.path(), "l17").exists(), "L17: no executable");
}

#[test]
fn an_archive_defining_malloc_is_named_and_refused() {
    let dir = with_triple_archive();
    compile_object(
        dir.path(),
        "hmalloc.c",
        "#include <stdlib.h>\nextern void *__libc_malloc(size_t);\n\
         void *malloc(size_t n) { return __libc_malloc(n); }\n",
    );
    archive(dir.path(), "mymalloc", &["hmalloc.o"]);
    let link = link_to(dir.path(), &["mystage4", "mymalloc"]);
    let err = compile(
        dir.path(),
        "l18",
        ALLOCATING_AE,
        &link,
        OptimizationLevel::None,
    )
    .expect_err("L18: this is the jemalloc case, named instead of crashed");
    assert!(
        err.contains("E0618") && err.contains("malloc"),
        "L18: {err}"
    );
    assert!(!exe_path(dir.path(), "l18").exists(), "L18: no executable");
}

#[test]
fn an_archive_claiming_nothing_reserved_is_accepted_on_the_same_program() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["mystage4"]);
    compile(
        dir.path(),
        "l19",
        ALLOCATING_AE,
        &link,
        OptimizationLevel::None,
    )
    .expect("L19: the check must not bite the honest library");
    assert_eq!(execute(dir.path(), "l19", &[]).0, 42, "L19");
}

const IFUNC_MEMCMP_C: &str = "#include <stddef.h>\n\
     static int always_equal(const void *a, const void *b, size_t n)\n\
     { (void)a; (void)b; (void)n; return 0; }\n\
     static void *resolve_memcmp(void) { return (void *)always_equal; }\n\
     int memcmp(const void *, const void *, size_t)\n\
     __attribute__((ifunc(\"resolve_memcmp\")));\n";

const STRING_COMPARE_AE: &str = "fn main() {\n\
     \u{20}   if \"abc\" == \"xyz\" { println(\"EQUAL\") } else { println(\"DIFFERENT\") }\n\
     }\n";

#[test]
fn an_archive_defining_memcmp_as_an_ifunc_is_named_and_refused() {
    let dir = with_triple_archive();
    compile_object(dir.path(), "hifunc.c", IFUNC_MEMCMP_C);
    archive(dir.path(), "myifunc", &["hifunc.o"]);
    assert!(
        String::from_utf8_lossy(
            &run(
                &tool("nm"),
                &["--defined-only", "lib/libmyifunc.a"],
                dir.path()
            )
            .stdout
        )
        .contains("i memcmp"),
        "L20: the archive must really carry the ifunc spelling, or the row proves nothing"
    );
    let link = link_to(dir.path(), &["myifunc"]);
    let err = compile(
        dir.path(),
        "l20",
        STRING_COMPARE_AE,
        &link,
        OptimizationLevel::None,
    )
    .expect_err("L20: an ifunc memcmp answers zero for every pair of strings");
    assert!(
        err.contains("E0618") && err.contains("memcmp"),
        "L20: {err}"
    );
    assert!(!exe_path(dir.path(), "l20").exists(), "L20: no executable");
}

#[test]
fn an_archive_whose_reserved_name_is_a_local_symbol_is_accepted() {
    let dir = with_triple_archive();
    compile_object(
        dir.path(),
        "hlocal.c",
        "#include <stddef.h>\nstatic void *local_alloc(size_t n) __asm__(\"malloc\");\n\
         static void *local_alloc(size_t n) { (void)n; return NULL; }\n\
         long aelys_stage4_triple(long x) { return local_alloc(0) ? 0 : 3 * x; }\n",
    );
    archive(dir.path(), "mylocal", &["hlocal.o"]);
    assert!(
        String::from_utf8_lossy(
            &run(
                &tool("nm"),
                &["--defined-only", "lib/libmylocal.a"],
                dir.path()
            )
            .stdout
        )
        .contains("t malloc"),
        "L21: the archive must really carry a local `malloc`, or the row proves nothing"
    );
    let link = link_to(dir.path(), &["mylocal"]);
    compile(dir.path(), "l21", TRIPLE_AE, &link, OptimizationLevel::None)
        .expect("L21: a local definition cannot capture the runtime's own call");
    assert_eq!(execute(dir.path(), "l21", &[]).0, 42, "L21");
}

#[test]
fn a_separated_link_value_that_looks_like_an_option_is_refused() {
    for tokens in [
        vec!["compile", "a.aelys", "-L", "-O0"],
        vec!["compile", "a.aelys", "-l", "-O0"],
        vec!["compile", "a.aelys", "--library-path", "--no-color"],
        vec!["compile", "a.aelys", "--library", "-Wall"],
    ] {
        let err = parse(&tokens).expect_err("L22: the next flag must not be eaten in silence");
        assert!(err.contains("looks like an option"), "L22: {err}");
    }
}

#[test]
fn the_glued_and_the_long_spelling_still_carry_a_value_that_starts_with_a_dash() {
    let glued = parse(&["compile", "a.aelys", "-L-O0", "-l-O0"]).expect("L23: glued");
    assert_eq!(glued.link.search_paths, vec![PathBuf::from("-O0")], "L23");
    assert_eq!(glued.link.libraries, vec!["-O0".to_string()], "L23");
    let long =
        parse(&["compile", "a.aelys", "--library-path=-O0", "--library=-O0"]).expect("L23: long");
    assert_eq!(long.link, glued.link, "L23: the same requirement");
    for tokens in [
        vec!["compile", "a.aelys", "-l="],
        vec!["compile", "a.aelys", "-L="],
    ] {
        let err = parse(&tokens).expect_err("L23: `=` is the long spelling, not a name");
        assert!(err.contains("non-empty"), "L23: {err}");
    }
}

#[test]
fn a_shared_object_twin_of_a_refused_archive_is_accepted_and_the_explain_says_so() {
    let dir = with_triple_archive();
    compile_object(dir.path(), "hifunc2.c", IFUNC_MEMCMP_C);
    archive(dir.path(), "mydual", &["hifunc2.o"]);
    fs::write(dir.path().join("hifunc2s.c"), IFUNC_MEMCMP_C).expect("write c source");
    shared(dir.path(), "mydual", "hifunc2s.c");
    let link = link_to(dir.path(), &["mydual"]);
    compile(
        dir.path(),
        "l24",
        STRING_COMPARE_AE,
        &link,
        OptimizationLevel::None,
    )
    .expect("L24: the linker takes the shared object, so nothing is defined in the executable");
    let explain = aelys_common::diagnostic::registry::lookup("E0618")
        .expect("E0618 is registered")
        .explanation;
    assert!(
        explain.contains("takes `libfoo.so` over `libfoo.a`"),
        "L24: the explain must name the case that disarms the check: {explain}"
    );
}

#[test]
fn the_unresolved_symbol_anchors_on_main() {
    let dir = tempdir().expect("tempdir");
    let err = compile(
        dir.path(),
        "d1",
        TRIPLE_AE,
        &LinkRequirement::default(),
        OptimizationLevel::None,
    )
    .expect_err("D1");
    assert!(err.contains("undefined reference"), "D1: {err}");
    assert!(
        err.contains("fn main() -> i64"),
        "D1: the anchor is the main function, the only one the link can reach: {err}"
    );
}

#[test]
fn structural_row_the_backend_family_has_five_anchor_sites_in_the_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("root");
    let diagnostics =
        fs::read_to_string(root.join("driver/src/api/llvm/diagnostics.rs")).expect("read");
    let module = fs::read_to_string(root.join("driver/src/api/llvm/mod.rs")).expect("read");
    for anchor in [
        "main_function_air_span(air)",
        ".or_else(|| air.functions.iter().find_map(|function| function.span))",
        "unwrap_or_else(|| fallback_source_span(source))",
        "unwrap_or_else(|| program_anchor_span(air, source.as_ref()))",
    ] {
        assert!(
            diagnostics.contains(anchor),
            "D1b: backend-family anchor site missing: {anchor}"
        );
    }
    assert!(
        module.contains("fn anchor_in("),
        "D1b: the module anchor is the fifth site"
    );
}

#[test]
fn the_link_failure_help_names_the_channel_and_the_declaration() {
    let dir = tempdir().expect("tempdir");
    let err = compile(
        dir.path(),
        "d2",
        TRIPLE_AE,
        &LinkRequirement::default(),
        OptimizationLevel::None,
    )
    .expect_err("D2");
    assert!(err.contains("-L <dir> -l <name>"), "D2: the channel: {err}");
    assert!(
        err.contains("`aelys_stage4_triple` at line 1"),
        "D2: the declaration and its line: {err}"
    );
}

#[test]
fn the_help_names_every_declaration_when_more_than_one_is_missing() {
    let dir = with_triple_archive();
    let source = "unsafe extern fn aelys_stage4_triple(x: i64) -> i64\n\
                  unsafe extern fn aelys_stage4_absent(x: i64) -> i64\n\
                  fn main() -> i64 { unsafe { return aelys_stage4_triple(1) + aelys_stage4_absent(2) } }\n";
    let link = link_to(dir.path(), &["mystage4"]);
    let err = compile(dir.path(), "d3", source, &link, OptimizationLevel::None).expect_err("D3");
    assert!(err.contains("aelys_stage4_triple"), "D3: {err}");
    assert!(err.contains("aelys_stage4_absent"), "D3: {err}");
}

#[test]
fn a_program_that_links_carries_no_help_at_all() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["mystage4"]);
    compile(dir.path(), "d4", TRIPLE_AE, &link, OptimizationLevel::None)
        .expect("D4: this one links, so there is no diagnostic to carry a help");
    assert_eq!(execute(dir.path(), "d4", &[]).0, 42, "D4");
}

#[test]
fn a_link_failure_with_no_external_declaration_carries_no_ffi_help() {
    let dir = with_triple_archive();
    let link = link_to(dir.path(), &["aelys_stage4_no_such_library"]);
    let err = compile(
        dir.path(),
        "d5",
        "fn main() -> i64 { return 3 }\n",
        &link,
        OptimizationLevel::None,
    )
    .expect_err("D5: the library is missing, so the link fails");
    assert!(err.contains("E0901"), "D5: {err}");
    assert!(
        !err.contains("an external function is resolved at link time"),
        "D5: the help must not become universal noise: {err}"
    );
}
