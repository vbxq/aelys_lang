use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

static WARM: Once = Once::new();

fn warm_core_archive() {
    WARM.call_once(|| {
        let Ok(dir) = tempdir() else { return };
        let path = dir.path().join("warmup.aelys");
        if fs::write(&path, "fn main() -> i64 { return 0 }\n").is_err() {
            return;
        }
        let _ = compile_file_with_llvm_variant(
            &path,
            OptimizationLevel::None,
            false,
            RuntimeVariant::Rc,
        );
    });
}

struct Harness {
    dir: TempDir,
    legs: Cell<usize>,
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found") || error.contains("failed to run")
}

fn linker_skip_declared() -> bool {
    std::env::var("AELYS_ALLOW_LINKER_SKIP").is_ok()
}

fn slug(id: &str, tag: &str) -> String {
    let mut s = String::with_capacity(id.len() + tag.len() + 1);
    for c in id.chars().chain(std::iter::once('_')).chain(tag.chars()) {
        s.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    s
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
            legs: Cell::new(0),
        }
    }

    fn write_module(&self, rel: &str, src: &str) {
        let path = self.dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("module parent")).expect("module dir");
        fs::write(&path, src).expect("write module");
    }

    fn value_row(&self, id: &str, src: &str, stdout: &str) {
        for (tag, opt) in LEVELS {
            let path = self.dir.path().join(format!("{}.aelys", slug(id, tag)));
            fs::write(&path, src).expect("write fixture");
            match compile_file_with_llvm_variant(&path, *opt, false, RuntimeVariant::Rc) {
                Ok(()) => {}
                Err(err) => {
                    let text = err.to_string();
                    if linker_unavailable(&text) {
                        assert!(
                            linker_skip_declared(),
                            "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped \
                             value row carries no runtime evidence at all"
                        );
                        return;
                    }
                    panic!("{id} at {tag} must compile and link:\n{text}");
                }
            }
            let exe = exe_path_for(&path);
            assert!(exe.is_file(), "{id} at {tag}: no executable was produced");
            let out = Command::new(&exe).output().expect("run compiled exe");
            self.legs.set(self.legs.get() + 1);
            assert_eq!(
                out.status.code(),
                Some(0),
                "{id} at {tag} must exit 0\nstderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                stdout,
                "{id} at {tag}: stdout MUST be {stdout:?}"
            );
        }
        assert_eq!(self.legs.get(), LEVELS.len(), "{id}: every level must run");
    }
}

fn lower(src: &str) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("module.aelys");
    fs::write(&path, src).expect("write source");
    lower_file_to_air(&path, OptimizationLevel::None)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn refuse(id: &str, src: &str, needles: &[&str]) {
    match lower(src) {
        Ok(()) => panic!("{id}: expected a refusal, but the program compiled"),
        Err(err) => {
            for needle in needles {
                assert!(
                    err.contains(needle),
                    "{id}: refusal must mention {needle:?}, got:\n{err}"
                );
            }
            assert!(
                !err.contains("undefined reference") && !err.contains("[llvm-linker]"),
                "{id}: the refusal must be a compiler diagnostic, not a link failure:\n{err}"
            );
        }
    }
}

#[test]
fn concrete_return_with_match_on_a_generic_enum_runs() {
    Harness::new().value_row(
        "h3",
        "enum Opt<T> { Some(T), None }\n\
         fn is_ok<T>(o: Opt<T>) -> i64 {\n\
         \x20   match o {\n\
         \x20       Opt::Some(x) => { return 1 }\n\
         \x20       Opt::None => { return 0 }\n\
         \x20   }\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   let a: Opt<i64> = Opt::Some(5)\n\
         \x20   println(is_ok(a))\n\
         \x20   return 0\n\
         }\n",
        "1\n",
    );
}

#[test]
fn concrete_return_without_a_match_runs() {
    Harness::new().value_row(
        "h4",
        "enum Opt<T> { Some(T), None }\n\
         fn tag<T>(o: Opt<T>) -> i64 { return 4 }\n\
         fn main() -> i64 {\n\
         \x20   let a: Opt<i64> = Opt::Some(5)\n\
         \x20   println(tag(a))\n\
         \x20   return 0\n\
         }\n",
        "4\n",
    );
}

#[test]
fn two_instantiations_of_one_concrete_return_generic_both_answer() {
    Harness::new().value_row(
        "two_instantiations",
        "enum Opt<T> { Some(T), None }\n\
         fn is_ok<T>(o: Opt<T>) -> i64 {\n\
         \x20   match o {\n\
         \x20       Opt::Some(x) => { return 1 }\n\
         \x20       Opt::None => { return 0 }\n\
         \x20   }\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   let a: Opt<i64> = Opt::Some(5)\n\
         \x20   let b: Opt<bool> = Opt::Some(true)\n\
         \x20   let c: Opt<i64> = Opt::None\n\
         \x20   println(is_ok(a))\n\
         \x20   println(is_ok(b))\n\
         \x20   println(is_ok(c))\n\
         \x20   return 0\n\
         }\n",
        "1\n1\n0\n",
    );
}

#[test]
fn two_type_parameters_with_a_concrete_return_run() {
    Harness::new().value_row(
        "two_type_params",
        "fn pair<A, B>(a: A, b: B) -> i64 {\n\
         \x20   let x: A = a\n\
         \x20   let y: B = b\n\
         \x20   return 12\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   println(pair(1, true))\n\
         \x20   println(pair(true, 2))\n\
         \x20   return 0\n\
         }\n",
        "12\n12\n",
    );
}

#[test]
fn a_generic_that_is_never_called_emits_no_dangling_symbol() {
    Harness::new().value_row(
        "never_called",
        "enum Opt<T> { Some(T), None }\n\
         fn unused<T>(o: Opt<T>) -> i64 { return 1 }\n\
         fn main() -> i64 {\n\
         \x20   println(7)\n\
         \x20   return 0\n\
         }\n",
        "7\n",
    );
}

#[test]
fn a_void_generic_with_a_concrete_signature_runs() {
    Harness::new().value_row(
        "void_return",
        "enum Opt<T> { Some(T), None }\n\
         fn ignore<T>(o: Opt<T>) {\n\
         \x20   let z: i64 = 1\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   let a: Opt<i64> = Opt::Some(5)\n\
         \x20   ignore(a)\n\
         \x20   println(8)\n\
         \x20   return 0\n\
         }\n",
        "8\n",
    );
}

#[test]
fn a_mangling_collision_is_e0427_and_not_a_link_error() {
    refuse(
        "mangling_collision",
        "struct A_B { v: i64 }\n\
         struct B { v: i64 }\n\
         fn g<T>(x: T) -> i64 { return 1 }\n\
         fn g_A<T>(x: T) -> i64 { return 2 }\n\
         fn main() -> i64 {\n\
         \x20   let p: A_B = A_B { v: 1 }\n\
         \x20   let q: B = B { v: 2 }\n\
         \x20   let a = g(p)\n\
         \x20   let b = g_A(q)\n\
         \x20   return a + b\n\
         }\n",
        &["E0427", "__mono_g_A_B"],
    );
}

#[test]
fn a_non_generic_struct_crossing_a_generic_keeps_its_own_name() {
    Harness::new().value_row(
        "plain_struct_through_a_generic",
        "struct P { a: i64 }\n\
         fn pick<T>(p: P, x: T) -> T { return x }\n\
         fn mk<T>(x: T) -> P { return P { a: 1 } }\n\
         fn hold<T>(x: T) -> i64 {\n\
         \x20   let q: P = P { a: 5 }\n\
         \x20   return q.a\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   let p: P = P { a: 3 }\n\
         \x20   println(pick(p, 42))\n\
         \x20   let r: P = mk(7)\n\
         \x20   println(r.a)\n\
         \x20   println(hold(1))\n\
         \x20   return 0\n\
         }\n",
        "42\n1\n5\n",
    );
}

#[test]
fn a_generic_enum_name_that_prefixes_another_never_draws_a_variant_tag() {
    Harness::new().value_row(
        "prefixing_generic_enum_names",
        "enum Q<T> { A(T) }\n\
         enum Q_ptr<T> { B(T), C(i64) }\n\
         fn main() -> i64 {\n\
         \x20   let x: Q_ptr<i64> = Q_ptr::C(99)\n\
         \x20   let y: Q<i64> = Q::A(4)\n\
         \x20   println(match x { Q_ptr::B(v) => { v } Q_ptr::C(w) => { w } })\n\
         \x20   println(match y { Q::A(v) => { v } })\n\
         \x20   return 0\n\
         }\n",
        "99\n4\n",
    );
}

#[test]
fn two_generic_enums_that_do_not_prefix_each_other_both_run() {
    Harness::new().value_row(
        "distinct_generic_enums",
        "enum Alpha<T> { A(T) }\n\
         enum Beta<T> { B(T), C(i64) }\n\
         fn main() -> i64 {\n\
         \x20   let a: Alpha<i64> = Alpha::A(7)\n\
         \x20   let b: Beta<i64> = Beta::C(35)\n\
         \x20   match a { Alpha::A(v) => { println(v) } }\n\
         \x20   match b { Beta::B(v) => { println(v) } Beta::C(w) => { println(w) } }\n\
         \x20   return 0\n\
         }\n",
        "7\n35\n",
    );
}

#[test]
fn a_type_parameter_only_inside_a_generic_enum_is_inferred() {
    Harness::new().value_row(
        "type_param_inside_a_generic_enum",
        "enum Opt<T> { Some(T), None }\n\
         fn f<T, U>(o: Opt<T>, y: U) -> i64 { return 11 }\n\
         fn main() -> i64 {\n\
         \x20   let a: Opt<i64> = Opt::Some(7)\n\
         \x20   println(f(a, 8))\n\
         \x20   return 0\n\
         }\n",
        "11\n",
    );
}

#[test]
fn a_type_parameter_in_no_argument_position_is_a_diagnostic_and_not_a_link_error() {
    refuse(
        "uninferable_generic",
        "fn g<T>(x: i64) -> i64 { return 3 }\n\
         fn main() -> i64 {\n\
         \x20   return g(1)\n\
         }\n",
        &[
            "[monomorphization]",
            "does not determine its type arguments",
        ],
    );
}

#[test]
fn a_scalar_read_through_a_reference_is_accepted_and_its_ref_typed_twin_is_not() {
    lower(
        "enum Opt<T> { Some(T), None }\n\
         nogc fn first(s: &[i64]) -> Opt<i64> { return Opt::Some(s[0]) }\n\
         fn main() -> i64 { return 0 }\n",
    )
    .expect("slice_read_into_enum: an i64 copied out of a slice stores no reference");
    refuse(
        "slice_ref_read_into_enum",
        "enum Opt<T> { Some(T), None }\n\
         nogc fn first(s: &[&i64]) -> Opt<&i64> { return Opt::Some(s[0]) }\n\
         fn main() -> i64 { return 0 }\n",
        &[
            "E0714",
            "[borrow] references stored into aggregate containers are not supported in Run 1",
        ],
    );
}

#[test]
fn a_generic_calling_a_generic_runs() {
    Harness::new().value_row(
        "transitive_mono",
        "fn sink<U>(u: U) -> i64 { return 5 }\n\
         fn forward<T>(x: T) -> i64 { return sink(x) }\n\
         fn main() -> i64 {\n\
         \x20   println(forward(1))\n\
         \x20   println(forward(true))\n\
         \x20   return 0\n\
         }\n",
        "5\n5\n",
    );
}

#[test]
fn a_reference_stored_into_an_enum_payload_still_fires_e0714() {
    refuse(
        "ref_in_enum_payload",
        "enum Opt<T> { Some(T), None }\n\
         nogc fn keep(r: &i64) -> Opt<&i64> { return Opt::Some(r) }\n\
         fn main() -> i64 { return 0 }\n",
        &["E0714", "references stored into aggregate containers"],
    );
}

#[test]
fn a_reference_reached_through_a_projection_still_fires_e0714() {
    refuse(
        "ref_through_projection_in_enum_payload",
        "enum Opt<T> { Some(T), None }\n\
         nogc fn f(a: &[&i64]) -> Opt<&i64> { return Opt::Some(a[0]) }\n\
         fn main() -> i64 { return 0 }\n",
        &["E0714", "references stored into aggregate containers"],
    );
}

#[test]
fn a_reference_stored_into_an_array_still_fires_e0714() {
    refuse(
        "ref_in_array",
        "nogc fn f(x: &i64, y: &i64) -> i64 {\n\
         \x20   let a: [&i64; 2] = [x, y]\n\
         \x20   return 0\n\
         }\n\
         fn main() -> i64 { return 0 }\n",
        &["E0714", "references stored into aggregate containers"],
    );
}

#[test]
fn a_reference_stored_into_a_struct_still_fires_e0714() {
    refuse(
        "ref_in_struct",
        "struct S { p: &i64 }\n\
         nogc fn f(x: &i64) -> i64 {\n\
         \x20   let s: S = S { p: x }\n\
         \x20   return 0\n\
         }\n\
         fn main() -> i64 { return 0 }\n",
        &["E0714", "references stored into aggregate containers"],
    );
}

#[test]
fn a_partly_concrete_generic_enum_parameter_is_inferred() {
    Harness::new().value_row(
        "partly_concrete_result",
        "enum Result<T, E> { Ok(T), Err(E) }\n\
         fn is_ok_partial<T>(r: Result<T, i64>) -> i64 {\n\
         \x20   return match r { Result::Ok(v) => { 1 } Result::Err(e) => { 0 } }\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   let r: Result<i64, i64> = Result::Ok(7)\n\
         \x20   println(is_ok_partial(r))\n\
         \x20   return 0\n\
         }\n",
        "1\n",
    );
}

#[test]
fn two_generic_enums_in_one_signature_are_inferred() {
    Harness::new().value_row(
        "option_to_result",
        "enum Option<T> { Some(T), None }\n\
         enum Result<T, E> { Ok(T), Err(E) }\n\
         fn ok_or<T, E>(o: Option<T>, e: E) -> Result<T, E> {\n\
         \x20   return match o { Option::Some(v) => { Result::Ok(v) } Option::None => { Result::Err(e) } }\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   let o: Option<i64> = Option::Some(5)\n\
         \x20   let r: Result<i64, i64> = ok_or(o, 9)\n\
         \x20   println(match r { Result::Ok(v) => { v } Result::Err(e) => { e } })\n\
         \x20   return 0\n\
         }\n",
        "5\n",
    );
}

#[test]
fn a_two_argument_enum_narrowing_to_a_one_argument_enum_is_inferred() {
    Harness::new().value_row(
        "result_to_option",
        "enum Option<T> { Some(T), None }\n\
         enum Result<T, E> { Ok(T), Err(E) }\n\
         fn ok<T, E>(r: Result<T, E>) -> Option<T> {\n\
         \x20   return match r { Result::Ok(v) => { Option::Some(v) } Result::Err(e) => { Option::None } }\n\
         }\n\
         fn main() -> i64 {\n\
         \x20   let r: Result<i64, i64> = Result::Ok(11)\n\
         \x20   let o: Option<i64> = ok(r)\n\
         \x20   println(match o { Option::Some(v) => { v } Option::None => { 0 } })\n\
         \x20   return 0\n\
         }\n",
        "11\n",
    );
}

#[test]
fn a_generic_enum_nested_in_another_enums_payload_runs() {
    Harness::new().value_row(
        "enum_nested_in_a_payload",
        "enum Inner<T> { Wrap(T) }\n\
         enum Big<A, B, K> { W(Inner<K>, B), Z(A) }\n\
         fn main() -> i64 {\n\
         \x20   let i: Inner<i64> = Inner::Wrap(21)\n\
         \x20   let b: Big<bool, i64, i64> = Big::W(i, 2)\n\
         \x20   println(match b { Big::W(x, y) => { match x { Inner::Wrap(v) => { v * y } } } Big::Z(a) => { 0 } })\n\
         \x20   return 0\n\
         }\n",
        "42\n",
    );
}

#[test]
fn a_generic_enum_crossing_a_module_boundary_runs() {
    let harness = Harness::new();
    harness.write_module(
        "boxmod/lib.aelys",
        "pub enum Box<T> { Hold(T), Empty }\n\
         \n\
         pub fn peek(b: Box<i64>) -> i64 {\n\
         \x20   return match b { Box::Hold(v) => { v } Box::Empty => { 0 } }\n\
         }\n",
    );
    harness.value_row(
        "generic_enum_across_a_module",
        "needs boxmod.lib\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let b: lib.Box<i64> = lib.Box::Hold(23)\n\
         \x20   println(lib.peek(b))\n\
         \x20   return 0\n\
         }\n",
        "23\n",
    );
}
