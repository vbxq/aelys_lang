mod common;

use aelys_cli::cli::args::{Command as CliCommand, parse_args};
use aelys_driver::{
    LinkRequirement, RuntimeVariant, SourceOptions, compile_file_with_llvm_sources,
};
use aelys_opt::OptimizationLevel;
use common::{
    exe_path_for, exit_code, linker_unavailable, note_leg, pin_legs, require_linker_skip, slug,
    warm_core_archive,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
];

// the population the nondeterminism was measured over, one verdict is not a measurement of it
const REPEATS: usize = 40;

const MULTI_BYTE: &str = "héllo ca été là";
const MULTI_BYTE_BYTES: i64 = 19;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the aelys package must sit inside the workspace root")
        .to_path_buf()
}

fn library_include_root() -> PathBuf {
    let root = repo_root();
    let module = root.join("std").join("str.aelys");
    let size = fs::metadata(&module)
        .unwrap_or_else(|e| {
            panic!(
                "std/str.aelys must be readable at {}: {e}",
                module.display()
            )
        })
        .len();
    assert!(size > 0, "std/str.aelys must not be empty");
    root
}

fn sources_with(extra: Option<&Path>) -> SourceOptions {
    let mut roots = Vec::new();
    if let Some(dir) = extra {
        roots.push(dir.to_path_buf());
    }
    roots.push(library_include_root());
    SourceOptions::with_include(roots)
}

fn stats(stderr: &str, tag: &str) -> Option<(i64, i64)> {
    let head = format!("[{tag}] allocs=");
    let line = stderr.lines().find(|l| l.trim().starts_with(&head))?;
    let rest = line.trim().strip_prefix(&head)?;
    let (a, f) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, f.trim().parse().ok()?))
}

struct Ran {
    exit: i32,
    stdout: String,
    stderr: String,
}

struct Harness {
    dir: TempDir,
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
        }
    }

    // the root file's own directory outranks every include root, so it must hold no `std`
    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let name = slug(id, tag);
        let dir = self.dir.path().join(&name);
        fs::create_dir_all(&dir).expect("stage dir");
        assert!(
            !dir.join("std").exists(),
            "{id}: a `std` beside the root file would answer before any include root does"
        );
        let root = dir.join(format!("{name}.aelys"));
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn build(
        &self,
        root: &Path,
        opt: OptimizationLevel,
        extra: Option<&Path>,
    ) -> Result<Option<PathBuf>, String> {
        match compile_file_with_llvm_sources(
            root,
            opt,
            false,
            RuntimeVariant::Rc,
            &LinkRequirement::default(),
            &sources_with(extra),
        ) {
            Ok(_) => {
                let exe = exe_path_for(root);
                Ok(exe.is_file().then_some(exe))
            }
            Err(err) => Err(err.to_string()),
        }
    }

    fn run(&self, exe: &Path) -> Ran {
        let out = Command::new(exe)
            .env("AELYS_RC_STATS", "1")
            .output()
            .expect("run compiled exe");
        note_leg();
        Ran {
            exit: exit_code(&out.status),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    fn accepts_at(
        &self,
        id: &str,
        tag: &str,
        opt: OptimizationLevel,
        src: &str,
        extra: Option<&Path>,
    ) -> Option<Ran> {
        let root = self.stage(id, tag, src);
        let exe = match self.build(&root, opt, extra) {
            Ok(Some(exe)) => exe,
            Ok(None) => {
                require_linker_skip(&format!("{id} at {tag} produced no executable"));
                return None;
            }
            Err(err) if linker_unavailable(&err) => {
                require_linker_skip(&format!("{id} at {tag} could not be linked"));
                return None;
            }
            Err(err) => panic!("{id} at {tag} MUST compile:\n{err}"),
        };
        let ran = self.run(&exe);
        assert_eq!(
            ran.exit, 0,
            "{id} at {tag} must exit 0\nstderr:\n{}",
            ran.stderr
        );
        Some(ran)
    }

    fn value_row(&self, id: &str, src: &str, stdout: &str) {
        self.counted_row(id, src, stdout, None, None);
    }

    fn counted_row(
        &self,
        id: &str,
        src: &str,
        stdout: &str,
        managed: Option<(i64, i64)>,
        raw: Option<(i64, i64)>,
    ) {
        for (tag, opt) in LEVELS {
            let Some(ran) = self.accepts_at(id, tag, *opt, src, None) else {
                return;
            };
            assert_eq!(
                ran.stdout, stdout,
                "{id} at {tag}: stdout MUST be {stdout:?}\nstderr:\n{}",
                ran.stderr
            );
            if let Some(want) = managed {
                let got = stats(&ran.stderr, "rc").unwrap_or_else(|| {
                    panic!("{id} at {tag}: no [rc] line\nstderr:\n{}", ran.stderr)
                });
                assert_eq!(
                    got, want,
                    "{id} at {tag}: the managed counter MUST read allocs={} frees={}",
                    want.0, want.1
                );
            }
            if let Some(want) = raw {
                let got = stats(&ran.stderr, "raw").unwrap_or_else(|| {
                    panic!(
                        "{id} at {tag}: no [raw] line; a counter that is absent cannot see the \
                         allocation it was added to see\nstderr:\n{}",
                        ran.stderr
                    )
                });
                assert_eq!(
                    got, want,
                    "{id} at {tag}: the raw counter MUST read allocs={} frees={}",
                    want.0, want.1
                );
            }
        }
    }

    fn refuses(&self, id: &str, src: &str, code: &str, says: &str) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let rendered = match self.build(&root, *opt, None) {
                Ok(_) => panic!("{id} at {tag}: MUST be refused\n{src}"),
                Err(err) => err,
            };
            self.check_refusal(id, tag, &rendered, code, says, src);
        }
    }

    fn check_refusal(
        &self,
        id: &str,
        tag: &str,
        rendered: &str,
        code: &str,
        says: &str,
        src: &str,
    ) {
        assert!(
            !linker_unavailable(rendered),
            "{id} at {tag}: the refusal must come from the compiler, not the linker\n{rendered}"
        );
        assert!(
            rendered.contains(&format!("[{code}]")),
            "{id} at {tag}: the refusal MUST be {code}\n{src}\nrendered:\n{rendered}"
        );
        assert!(
            rendered.contains(says),
            "{id} at {tag}: the refusal MUST say {says:?}\n{src}\nrendered:\n{rendered}"
        );
        // e0901 reads "your program is not at fault", so a refusable program landing there is a lie
        assert!(
            !rendered.contains("[E0901]"),
            "{id} at {tag}: this program is refusable, it must not be reported as a compiler \
             bug\n{src}\nrendered:\n{rendered}"
        );
    }
}

#[test]
fn group_surface_two_generic_enums_that_prefix_one_another_answer_the_same_way_forty_times() {
    let h = Harness::new();
    let _pin = pin_legs("prefix-enum determinism", REPEATS * LEVELS.len());
    let src = "enum Q<T> { A(T) }\n\
               enum Q_ptr<T> { B(T), C(i64) }\n\
               \n\
               fn main() -> i64 {\n\
               \x20   let x: Q_ptr<i64> = Q_ptr::C(99)\n\
               \x20   match x { Q_ptr::B(v) => { println(v) } Q_ptr::C(w) => { println(w) } }\n\
               \x20   return 0\n\
               }\n";
    for (tag, opt) in LEVELS {
        let mut seen: Vec<String> = Vec::with_capacity(REPEATS);
        for round in 0..REPEATS {
            let id = format!("prefix_enums_{round}");
            let Some(ran) = h.accepts_at(&id, tag, *opt, src, None) else {
                return;
            };
            seen.push(ran.stdout);
        }
        let first = seen[0].clone();
        let agreeing = seen.iter().filter(|s| **s == first).count();
        assert_eq!(
            agreeing, REPEATS,
            "at {tag} the same program must answer the same way {REPEATS} times, {agreeing} \
             agreed with {first:?}"
        );
        assert_eq!(
            first, "99\n",
            "at {tag} the variant tag must be the one the program wrote"
        );
    }
}

const GENERIC_PRELUDE: &str = "struct P { a: i64 }\n\
                               \n\
                               enum Res<T, E> { Ok(T), Err(E) }\n\
                               \n\
                               enum Opt<T> { Some(T), None }\n\
                               \n";

#[test]
fn group_surface_a_partially_concrete_argument_list_monomorphizes() {
    let h = Harness::new();
    let _pin = pin_legs("is_ok_partial", LEVELS.len());
    let src = format!(
        "{GENERIC_PRELUDE}fn is_ok_partial<T>(r: Res<T, i64>) -> bool {{\n\
         \x20   match r {{ Res::Ok(v) => {{ return true }} Res::Err(e) => {{ return false }} }}\n\
         }}\n\
         \n\
         fn main() -> i64 {{\n\
         \x20   let a: Res<i64, i64> = Res::Ok(7)\n\
         \x20   let b: Res<i64, i64> = Res::Err(3)\n\
         \x20   if is_ok_partial(a) {{ println(1) }} else {{ println(0) }}\n\
         \x20   if is_ok_partial(b) {{ println(1) }} else {{ println(0) }}\n\
         \x20   return 0\n\
         }}\n"
    );
    h.value_row("is_ok_partial", &src, "1\n0\n");
}

#[test]
fn group_surface_a_generic_over_two_parameters_monomorphizes_from_its_arguments() {
    let h = Harness::new();
    let _pin = pin_legs("ok_or", LEVELS.len());
    let src = format!(
        "{GENERIC_PRELUDE}fn ok_or<T, E>(o: Opt<T>, e: E) -> Res<T, E> {{\n\
         \x20   match o {{ Opt::Some(v) => {{ return Res::Ok(v) }} Opt::None => {{ return Res::Err(e) }} }}\n\
         }}\n\
         \n\
         fn main() -> i64 {{\n\
         \x20   let t: Res<i64, i64> = ok_or(Opt::Some(5), 3)\n\
         \x20   match t {{ Res::Ok(v) => println(v), Res::Err(e) => println(e) }}\n\
         \x20   let u: Opt<i64> = Opt::None\n\
         \x20   let w: Res<i64, i64> = ok_or(u, 4)\n\
         \x20   match w {{ Res::Ok(v) => println(v), Res::Err(e) => println(e) }}\n\
         \x20   return 0\n\
         }}\n"
    );
    h.value_row("ok_or", &src, "5\n4\n");
}

#[test]
fn group_surface_a_type_parameter_no_argument_names_is_the_call_sites_fault() {
    let h = Harness::new();
    let src = format!(
        "{GENERIC_PRELUDE}fn ok<T, E>(v: T) -> Res<T, E> {{ return Res::Ok(v) }}\n\
         \n\
         fn main() -> i64 {{\n\
         \x20   let s: Res<i64, bool> = ok(9)\n\
         \x20   match s {{ Res::Ok(v) => println(v), Res::Err(b) => println(0) }}\n\
         \x20   return 0\n\
         }}\n"
    );
    h.refuses(
        "ok_two_params",
        &src,
        "E0902",
        "the call site does not determine its type arguments",
    );
}

#[test]
fn group_surface_a_plain_struct_crossing_a_generic_keeps_its_own_name() {
    let h = Harness::new();
    let _pin = pin_legs("pick", LEVELS.len());
    let src = format!(
        "{GENERIC_PRELUDE}fn pick<T>(p: P, x: T) -> T {{ return x }}\n\
         \n\
         fn main() -> i64 {{\n\
         \x20   let p: P = P {{ a: 3 }}\n\
         \x20   println(pick(p, 42))\n\
         \x20   return 0\n\
         }}\n"
    );
    h.value_row("pick", &src, "42\n");
}

#[test]
fn group_surface_a_duplicated_global_is_refused_at_both_optimization_levels() {
    let h = Harness::new();
    // the two levels read different values out of it, so one level alone cannot see the defect
    h.refuses(
        "dup_global",
        "let g: i64 = 1\n\
         let g: i64 = 2\n\
         \n\
         fn main() -> i64 {\n\
         \x20   println(g)\n\
         \x20   return 0\n\
         }\n",
        "E0204",
        "the name `g` is defined twice as a global",
    );
}

#[test]
fn group_surface_an_interpolation_that_goes_nowhere_allocates_no_raw_bytes() {
    let h = Harness::new();
    let _pin = pin_legs("interpolation raw counter", LEVELS.len() * 2);
    h.counted_row(
        "interp_alone",
        "fn main() -> i64 {\n\
         \x20   let n: i64 = 7\n\
         \x20   println(\"{n}\")\n\
         \x20   return 0\n\
         }\n",
        "7\n",
        Some((0, 0)),
        Some((0, 0)),
    );
    h.counted_row(
        "interp_joined",
        "fn main() -> i64 {\n\
         \x20   let n: i64 = 7\n\
         \x20   println(\"é{n}\")\n\
         \x20   return 0\n\
         }\n",
        "é7\n",
        None,
        Some((1, 0)),
    );
}

#[test]
fn group_surface_a_vec_temporary_read_for_its_length_is_freed() {
    let h = Harness::new();
    let _pin = pin_legs("vec length temporary", LEVELS.len());
    h.counted_row(
        "vec_len_temp",
        "fn main() -> i64 {\n\
         \x20   println(Vec::len(vec[1, 2]))\n\
         \x20   return 0\n\
         }\n",
        "2\n",
        Some((1, 1)),
        None,
    );
}

#[test]
fn group_surface_a_string_carries_a_byte_view() {
    let h = Harness::new();
    let _pin = pin_legs("string byte view", LEVELS.len());
    let src = format!(
        "fn main() -> i64 {{\n\
         \x20   let s: string = \"{MULTI_BYTE}\"\n\
         \x20   println(s.bytes.len)\n\
         \x20   println(s.bytes[0])\n\
         \x20   return 0\n\
         }}\n"
    );
    h.value_row("str_bytes", &src, &format!("{MULTI_BYTE_BYTES}\n104\n"));
}

#[test]
fn group_surface_ends_with_does_not_abort_on_a_multi_byte_haystack() {
    let h = Harness::new();
    let _pin = pin_legs("ends_with abort", LEVELS.len());
    h.value_row(
        "ends_with_abort",
        "needs std.str\n\
         \n\
         fn main() -> i64 {\n\
         \x20   if str.ends_with(\"éab\", \"b\") { println(1) } else { println(0) }\n\
         \x20   return 0\n\
         }\n",
        "1\n",
    );
}

#[test]
fn group_surface_ends_with_answers_true_on_a_multi_byte_haystack() {
    let h = Harness::new();
    let _pin = pin_legs("ends_with answer", LEVELS.len());
    h.value_row(
        "ends_with_answer",
        "needs std.str\n\
         \n\
         fn main() -> i64 {\n\
         \x20   if str.ends_with(\"éab\", \"ab\") { println(1) } else { println(0) }\n\
         \x20   return 0\n\
         }\n",
        "1\n",
    );
}

#[test]
fn group_surface_an_interpolation_of_a_dead_local_is_refused() {
    let h = Harness::new();
    h.refuses(
        "dead_interp",
        "fn main() -> i64 {\n\
         \x20   let anchor: i64 = 42\n\
         \x20   let mut r: &i64 = &anchor\n\
         \x20   {\n\
         \x20       let inner: i64 = 7\n\
         \x20       r = &inner\n\
         \x20   }\n\
         \x20   println(\"{*r}\")\n\
         \x20   return 0\n\
         }\n",
        "E0722",
        "`inner` does not live long enough",
    );
}

#[test]
fn group_surface_an_array_repeat_of_a_dead_local_is_refused() {
    let h = Harness::new();
    h.refuses(
        "dead_array_repeat",
        "struct P { n: i64 }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let anchor: P = P{n: 7}\n\
         \x20   let mut esc: [&P; 2] = [&anchor; 2]\n\
         \x20   {\n\
         \x20       let inner: P = P{n: 42}\n\
         \x20       esc = [&inner; 2]\n\
         \x20   }\n\
         \x20   println((*esc[0]).n)\n\
         \x20   return 0\n\
         }\n",
        "E0714",
        "references stored into aggregate containers",
    );
}

#[test]
fn group_surface_an_if_expression_carrying_a_dead_local_is_refused() {
    let h = Harness::new();
    h.refuses(
        "dead_if_expr",
        "fn pick() -> bool { return true }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let anchor: i64 = 7\n\
         \x20   let c: bool = pick()\n\
         \x20   let mut r: &i64 = &anchor\n\
         \x20   {\n\
         \x20       let inner: i64 = 42\n\
         \x20       r = if c { &inner } else { &anchor }\n\
         \x20   }\n\
         \x20   println(\"{*r}\")\n\
         \x20   return 0\n\
         }\n",
        "E0722",
        "`inner` does not live long enough",
    );
}

#[test]
fn group_surface_an_i64_copied_out_of_a_slice_is_not_a_stored_reference() {
    let h = Harness::new();
    let _pin = pin_legs("slice projection", LEVELS.len());
    h.value_row(
        "slice_projection",
        "needs std.result\n\
         \n\
         fn take(s: &[i64]) -> result.Option<i64> {\n\
         \x20   return result.Option::Some(s[0])\n\
         }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let a: [i64; 3] = [4, 5, 6]\n\
         \x20   println(result.some_or(take(a[..]), -1))\n\
         \x20   return 0\n\
         }\n",
        "4\n",
    );
}

#[test]
fn group_surface_a_slice_iterates() {
    let h = Harness::new();
    let _pin = pin_legs("slice for-each", LEVELS.len());
    h.value_row(
        "slice_for_each",
        "fn total(s: &[i64]) -> i64 {\n\
         \x20   let mut acc: i64 = 0\n\
         \x20   for x in s {\n\
         \x20       acc = acc + x\n\
         \x20   }\n\
         \x20   return acc\n\
         }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let a: [i64; 3] = [4, 5, 6]\n\
         \x20   println(total(a[..]))\n\
         \x20   return 0\n\
         }\n",
        "15\n",
    );
}

#[test]
fn group_surface_a_library_prints_a_multi_byte_string() {
    let h = Harness::new();
    let _pin = pin_legs("library print", LEVELS.len());
    let src = format!(
        "needs std.io\n\
         \n\
         fn main() -> i64 {{\n\
         \x20   let s: string = \"{MULTI_BYTE}\"\n\
         \x20   println(io.print_out(s))\n\
         \x20   println(s.bytes.len)\n\
         \x20   return 0\n\
         }}\n"
    );
    h.value_row(
        "library_print",
        &src,
        &format!("{MULTI_BYTE}{MULTI_BYTE_BYTES}\n{MULTI_BYTE_BYTES}\n"),
    );
}

#[test]
fn group_surface_a_module_resolves_through_an_include_root_and_only_through_it() {
    let h = Harness::new();
    let _pin = pin_legs("include root", LEVELS.len());

    let flag = ["aelys", "compile", "main.aelys", "-I", "/lib/one"].map(String::from);
    let parsed = parse_args(&flag).expect("`-I` must be a flag the driver understands");
    assert!(
        matches!(parsed.command, CliCommand::Compile { .. }),
        "`-I` must not swallow the command"
    );
    assert_eq!(
        parsed.sources.include,
        vec![PathBuf::from("/lib/one")],
        "`-I` must land in the module search roots"
    );

    let library = h.dir.path().join("elsewhere");
    fs::create_dir_all(&library).expect("include root dir");
    fs::write(
        library.join("greet.aelys"),
        format!(
            "needs std.io\n\
             \n\
             pub fn hello() -> i64 {{ return io.print_out(\"{MULTI_BYTE}\") }}\n"
        ),
    )
    .expect("write library module");

    let src = "needs greet\n\
               \n\
               fn main() -> i64 {\n\
               \x20   println(greet.hello())\n\
               \x20   return 0\n\
               }\n";

    for (tag, opt) in LEVELS {
        let alone = h.stage("include_root_absent", tag, src);
        let rendered = match h.build(&alone, *opt, None) {
            Ok(_) => panic!("without its include root `greet` must not resolve at {tag}"),
            Err(err) => err,
        };
        assert!(
            !linker_unavailable(&rendered),
            "the refusal must come from module resolution, not the linker\n{rendered}"
        );

        let Some(ran) = h.accepts_at("include_root", tag, *opt, src, Some(&library)) else {
            return;
        };
        assert_eq!(
            ran.stdout,
            format!("{MULTI_BYTE}{MULTI_BYTE_BYTES}\n"),
            "at {tag} the module reached only through `-I` must run"
        );
    }
}

#[test]
fn group_surface_option_reaches_a_program_that_imports_nothing() {
    let h = Harness::new();
    let _pin = pin_legs("prelude Option", LEVELS.len());
    h.value_row(
        "prelude_option",
        "fn main() -> i64 {\n\
         \x20   let o: Option<i64> = Option::Some(9)\n\
         \x20   match o { Option::Some(v) => println(v), Option::None => println(0) }\n\
         \x20   return 0\n\
         }\n",
        "9\n",
    );
}

#[test]
fn group_surface_strings_order_through_a_capability_and_the_refusal_names_it() {
    let h = Harness::new();
    let _pin = pin_legs("string ordering", LEVELS.len());
    h.refuses(
        "string_order_operator",
        "fn main() -> i64 {\n\
         \x20   if \"é\" < \"ê\" { println(1) } else { println(0) }\n\
         \x20   return 0\n\
         }\n",
        "E0301",
        "`string` has no ordering operator",
    );
    h.value_row(
        "string_order_capability",
        "needs std.sort\n\
         needs std.str\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let mut a: [string; 3] = [\"ê\", \"é\", \"e\"]\n\
         \x20   sort.strings(a[..])\n\
         \x20   println(a[0])\n\
         \x20   println(a[1])\n\
         \x20   println(a[2])\n\
         \x20   println(str.compare(\"é\", \"ê\"))\n\
         \x20   return 0\n\
         }\n",
        "e\né\nê\n-1\n",
    );
}

#[test]
fn group_surface_a_vec_gives_back_and_a_missing_method_names_what_exists() {
    let h = Harness::new();
    let _pin = pin_legs("Vec::pop", LEVELS.len());
    h.counted_row(
        "vec_pop",
        "fn main() -> i64 {\n\
         \x20   let mut v: Vec<i64> = vec[1, 2, 3]\n\
         \x20   let x: i64 = Vec::pop(v)\n\
         \x20   println(x)\n\
         \x20   println(Vec::len(v))\n\
         \x20   return 0\n\
         }\n",
        "3\n2\n",
        Some((1, 1)),
        None,
    );
    h.refuses(
        "vec_unknown_method",
        "fn main() -> i64 {\n\
         \x20   let v: Vec<i64> = vec[1, 2]\n\
         \x20   println(Vec::nope(v))\n\
         \x20   return 0\n\
         }\n",
        "E0301",
        "unknown method 'nope' on Vec; supported:",
    );
}

#[test]
fn group_surface_the_most_negative_i64_is_a_literal() {
    let h = Harness::new();
    let _pin = pin_legs("min i64", LEVELS.len());
    h.value_row(
        "min_i64",
        "fn main() -> i64 {\n\
         \x20   let x: i64 = -9223372036854775808\n\
         \x20   println(x)\n\
         \x20   println(x + 1)\n\
         \x20   return 0\n\
         }\n",
        "-9223372036854775808\n-9223372036854775807\n",
    );
}

#[test]
fn group_surface_a_name_that_is_a_module_under_another_alias_says_so() {
    let h = Harness::new();
    h.refuses(
        "module_alias",
        "needs std.math as m\n\
         \n\
         fn main() -> i64 {\n\
         \x20   println(m.abs(-5))\n\
         \x20   println(math.abs(-6))\n\
         \x20   return 0\n\
         }\n",
        "E0623",
        "the module `std.math` is imported under the name `m`",
    );
}

#[test]
fn group_surface_a_file_that_cannot_be_read_is_not_a_compiler_bug() {
    let h = Harness::new();
    let missing = h.dir.path().join("no_such_root.aelys");
    assert!(
        !missing.exists(),
        "the row needs a path that does not exist"
    );
    for (tag, opt) in LEVELS {
        let rendered = match h.build(&missing, *opt, None) {
            Ok(_) => panic!("a missing root file must be refused at {tag}"),
            Err(err) => err,
        };
        h.check_refusal(
            "missing_file",
            tag,
            &rendered,
            "E0008",
            "could not read",
            "<a path that does not exist>",
        );
    }
}

#[test]
fn group_surface_a_struct_with_infinite_size_is_not_a_compiler_bug() {
    let h = Harness::new();
    h.refuses(
        "infinite_struct",
        "struct Node { next: Node, v: i64 }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   println(1)\n\
         \x20   return 0\n\
         }\n",
        "E0904",
        "struct `Node` has infinite size",
    );
}

#[test]
fn group_surface_a_main_with_a_parameter_is_not_a_compiler_bug() {
    let h = Harness::new();
    h.refuses(
        "main_with_param",
        "fn main(x: i64) -> i64 {\n\
         \x20   println(x)\n\
         \x20   return 0\n\
         }\n",
        "E0904",
        "main must have no parameters",
    );
}

// a control: the base refuses or aborts every other string row, this one shows its compiler works
#[test]
fn group_surface_control_a_multi_byte_literal_reaches_the_program_intact() {
    let h = Harness::new();
    let _pin = pin_legs("multi-byte control", LEVELS.len());
    let src = format!(
        "needs std.str\n\
         \n\
         fn main() -> i64 {{\n\
         \x20   let s: string = \"{MULTI_BYTE}\"\n\
         \x20   println(s.len)\n\
         \x20   if str.is_empty(s) {{ println(1) }} else {{ println(0) }}\n\
         \x20   return 0\n\
         }}\n"
    );
    h.value_row(
        "control_multi_byte",
        &src,
        &format!("{MULTI_BYTE_BYTES}\n0\n"),
    );
}

#[test]
fn group_surface_control_the_managed_counter_reads_zero_when_nothing_is_allocated() {
    let h = Harness::new();
    let _pin = pin_legs("zero allocation control", LEVELS.len());
    let src = format!(
        "fn main() -> i64 {{\n\
         \x20   println(\"{MULTI_BYTE}\")\n\
         \x20   return 0\n\
         }}\n"
    );
    h.counted_row(
        "control_zero_alloc",
        &src,
        &format!("{MULTI_BYTE}\n"),
        Some((0, 0)),
        None,
    );
}

#[test]
fn group_surface_a_string_for_each_yields_characters_not_bytes() {
    let h = Harness::new();
    let _pin = pin_legs("string for-each", LEVELS.len());
    // `joined` reads the element and `count` never does, and only the first shape aborts
    let src = format!(
        "fn count(s: string) -> i64 {{\n\
         \x20   let mut n: i64 = 0\n\
         \x20   for c in s {{ n = n + 1 }}\n\
         \x20   return n\n\
         }}\n\
         \n\
         fn joined(s: string) -> string {{\n\
         \x20   let mut acc: string = \"|\"\n\
         \x20   for c in s {{ acc = acc + c + \"|\" }}\n\
         \x20   return acc\n\
         }}\n\
         \n\
         fn main() -> i64 {{\n\
         \x20   println(count(\"abc\"))\n\
         \x20   println(joined(\"abc\"))\n\
         \x20   println(count(\"é\"))\n\
         \x20   println(joined(\"é\"))\n\
         \x20   println(count(\"\"))\n\
         \x20   println(joined(\"\"))\n\
         \x20   println(count(\"abé\"))\n\
         \x20   println(joined(\"abé\"))\n\
         \x20   println(count(\"{MULTI_BYTE}\"))\n\
         \x20   println(\"{MULTI_BYTE}\".len)\n\
         \x20   println(joined(\"{MULTI_BYTE}\"))\n\
         \x20   return 0\n\
         }}\n"
    );
    h.value_row(
        "string_for_each",
        &src,
        "3\n|a|b|c|\n1\n|é|\n0\n|\n3\n|a|b|é|\n15\n19\n|h|é|l|l|o| |c|a| |é|t|é| |l|à|\n",
    );
}

#[test]
fn group_surface_one_file_under_two_dotted_names_is_one_unit() {
    let h = Harness::new();
    let legs = if cfg!(unix) { 2 } else { 1 };
    let _pin = pin_legs("two dotted names", LEVELS.len() * legs);
    let counter = format!(
        "pub let mut n: i64 = 0\n\
         \n\
         pub fn bump() {{ n = n + 1 }}\n\
         \n\
         pub fn read() -> i64 {{ return n }}\n\
         \n\
         pub fn label() -> string {{ return \"{MULTI_BYTE}\" }}\n"
    );
    let expected = format!("3\n3\n{MULTI_BYTE}\n");

    for (tag, opt) in LEVELS {
        let nested_src = "needs lib.counter as a\n\
                          needs counter as b\n\
                          \n\
                          fn main() -> i64 {\n\
                          \x20   a.bump()\n\
                          \x20   a.bump()\n\
                          \x20   a.bump()\n\
                          \x20   println(a.read())\n\
                          \x20   println(b.read())\n\
                          \x20   println(b.label())\n\
                          \x20   return 0\n\
                          }\n";
        let nested = h.dir.path().join(slug("two_names_nested", tag)).join("lib");
        fs::create_dir_all(&nested).expect("nested root dir");
        fs::write(nested.join("counter.aelys"), &counter).expect("write nested module");
        let Some(ran) = h.accepts_at("two_names_nested", tag, *opt, nested_src, Some(&nested))
        else {
            return;
        };
        assert_eq!(
            ran.stdout, expected,
            "at {tag} a root nested inside another root must not split the module's state\n\
             stderr:\n{}",
            ran.stderr
        );

        #[cfg(unix)]
        {
            let symlink_src = "needs pkg.counter as a\n\
                               needs pkg.sub.counter as b\n\
                               \n\
                               fn main() -> i64 {\n\
                               \x20   a.bump()\n\
                               \x20   a.bump()\n\
                               \x20   a.bump()\n\
                               \x20   println(a.read())\n\
                               \x20   println(b.read())\n\
                               \x20   println(b.label())\n\
                               \x20   return 0\n\
                               }\n";
            let pkg = h
                .dir
                .path()
                .join(slug("two_names_symlink", tag))
                .join("pkg");
            fs::create_dir_all(pkg.join("sub")).expect("symlink package dir");
            fs::write(pkg.join("counter.aelys"), &counter).expect("write linked module");
            std::os::unix::fs::symlink("../counter.aelys", pkg.join("sub").join("counter.aelys"))
                .expect("link the module under a second dotted name");
            let Some(ran) = h.accepts_at("two_names_symlink", tag, *opt, symlink_src, None) else {
                return;
            };
            assert_eq!(
                ran.stdout, expected,
                "at {tag} a symlinked module file must not split the module's state\n\
                 stderr:\n{}",
                ran.stderr
            );
        }
    }
}
