mod common;

use aelys_common::WarningKind;
use aelys_driver::{RuntimeVariant, compile_file_with_llvm_with_warnings};
use aelys_opt::OptimizationLevel;
use common::{
    exe_path_for, exit_code, linker_unavailable, note_leg, pin_legs, require_linker_skip, slug,
    warm_core_archive,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

const MODULES: &[&str] = &["prelude.aelys", "result.aelys", "slice.aelys", "str.aelys"];

#[derive(PartialEq, Eq, Clone, Copy)]
enum Bucket {
    Anchor,
    Prefix,
    Neither,
}

// every row of this file, with the point its outcome discriminates and, for a control, why it must not
const CENSUS: &[(&str, Bucket, &str)] = &[
    ("r01_array_const_length_runs", Bucket::Anchor, ""),
    ("r02_array_length_mismatch_refused", Bucket::Anchor, ""),
    ("r03_const_index_outside_array_refused", Bucket::Anchor, ""),
    ("r04_array_nonconst_length_refused", Bucket::Anchor, ""),
    ("r05_rc_carrier_from_a_call_refused", Bucket::Anchor, ""),
    ("r06_rc_carrier_from_a_conditional_runs", Bucket::Anchor, ""),
    ("r07_werror_input_is_the_same_at_every_level", Bucket::Anchor, ""),
    ("r08_net_names_both_verdicts", Bucket::Anchor, ""),
    ("r09_file_scope_and_runs", Bucket::Anchor, ""),
    ("r10_char_class_eab", Bucket::Anchor, ""),
    ("r11_char_literals_and_six_operators", Bucket::Anchor, ""),
    ("r12_char_from_i64_outside_the_range_panics", Bucket::Anchor, ""),
    ("r13_char_from_i64_inside_the_range_runs", Bucket::Anchor, ""),
    ("r14_string_index_refused", Bucket::Anchor, ""),
    ("r15_bytes_of_a_call_result_refused", Bucket::Anchor, ""),
    ("r16_bytes_through_an_rc_temporary_refused", Bucket::Anchor, ""),
    (
        "r17_bytes_of_a_literal_runs",
        Bucket::Neither,
        "declared control: a literal's bytes are static storage, so the no-place refusal must \
         never reach them, and it did not reach them at the anchor either",
    ),
    (
        "r18_bytes_of_a_bound_rc_runs",
        Bucket::Neither,
        "declared control: the receiver is a binding, so the place rule must leave it alone at \
         every measured point",
    ),
    ("r19_generic_library_over_two_element_types", Bucket::Anchor, ""),
    ("r20_ord_excludes_string", Bucket::Anchor, ""),
    ("r21_eq_bound_over_two_element_types", Bucket::Anchor, ""),
    ("r22_expected_found_order", Bucket::Anchor, ""),
    ("r23_character_loop_values", Bucket::Anchor, ""),
    (
        "r24_plain_arithmetic_control",
        Bucket::Neither,
        "declared control: a block that is simply red against an older compiler discriminates \
         nothing, and this row is what says it is not",
    ),
    (
        "r25_byte_view_loop_control",
        Bucket::Neither,
        "declared control: the byte view is the string surface this run did not touch",
    ),
    (
        "r26_character_loop_is_linear",
        Bucket::Prefix,
        "the anchor refuses this program, so a cost cannot be measured there; the point that \
         falsifies it is the stage 2 tip, where the same bytes give the same answers 16 times \
         slower per doubling pair",
    ),
    (
        "w1_generic_struct_carrier_refused",
        Bucket::Neither,
        "declared control: a type parameter in a struct field is refused at every measured \
         point, so the row holds the boundary of what stage 2 made generic",
    ),
    (
        "w2_generic_enum_payload_runs",
        Bucket::Neither,
        "declared control: a concrete instantiation of a generic enum payload already worked at \
         the anchor",
    ),
    ("w3_file_scope_binary_initializer_runs", Bucket::Anchor, ""),
    (
        "w4_generic_function_over_a_struct_runs",
        Bucket::Neither,
        "declared control: a plain struct crossing a type parameter already worked at the anchor",
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the aelys package must sit inside the workspace root")
        .to_path_buf()
}

fn library_root() -> PathBuf {
    let dir = repo_root().join("std");
    for module in MODULES {
        let file = dir.join(module);
        let size = fs::metadata(&file)
            .unwrap_or_else(|e| panic!("std/{module} must be readable at {}: {e}", file.display()))
            .len();
        assert!(size > 0, "std/{module} must not be empty");
    }
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create staged directory");
    for entry in fs::read_dir(from).expect("read library directory") {
        let entry = entry.expect("read library entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("library entry kind").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy library file");
        }
    }
}

fn stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.trim().starts_with("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
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
    library: PathBuf,
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
            library: library_root(),
        }
    }

    // `needs std.x` resolves under the root file's own directory first, so the library is copied there
    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&self.library, &dir.join("std"));
        let root = dir.join("root.aelys");
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn build(&self, root: &Path, opt: OptimizationLevel) -> Result<Vec<WarningKind>, String> {
        match compile_file_with_llvm_with_warnings(root, opt, false, RuntimeVariant::Rc) {
            Ok(warnings) => Ok(warnings.into_iter().map(|w| w.kind).collect()),
            Err(err) => Err(err.to_string()),
        }
    }

    fn run(&self, exe: &Path, alloc: Option<&str>) -> Ran {
        let mut cmd = Command::new(exe);
        cmd.env("AELYS_RC_STATS", "1");
        if let Some(a) = alloc {
            cmd.env("AELYS_ALLOC", a);
        }
        let out = cmd.output().expect("run compiled exe");
        note_leg();
        Ran {
            exit: exit_code(&out.status),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    fn compiled(
        &self,
        id: &str,
        tag: &str,
        opt: OptimizationLevel,
        src: &str,
    ) -> Option<(PathBuf, Vec<WarningKind>)> {
        let root = self.stage(id, tag, src);
        let warnings = match self.build(&root, opt) {
            Ok(warnings) => warnings,
            Err(err) if linker_unavailable(&err) => {
                require_linker_skip(&format!("{id} at {tag} could not be linked"));
                return None;
            }
            Err(err) => panic!("{id} at {tag} MUST compile:\n{err}"),
        };
        let exe = exe_path_for(&root);
        if !exe.is_file() {
            require_linker_skip(&format!("{id} at {tag} produced no executable"));
            return None;
        }
        Some((exe, warnings))
    }

    fn runs(&self, id: &str, src: &str, exit: i32, stdout: &str, managed: Option<(i64, i64)>) {
        for (tag, opt) in LEVELS {
            let Some((exe, _)) = self.compiled(id, tag, *opt, src) else {
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let ran = self.run(&exe, *alloc);
                assert_eq!(
                    ran.exit, exit,
                    "{id} at {tag}/{alloc_name}: the artifact MUST exit {exit}\nstderr:\n{}",
                    ran.stderr
                );
                assert_eq!(
                    ran.stdout, stdout,
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}\nstderr:\n{}",
                    ran.stderr
                );
                if let Some(want) = managed {
                    let got = stats(&ran.stderr).unwrap_or_else(|| {
                        panic!("{id} at {tag}/{alloc_name}: no [rc] line\nstderr:\n{}", ran.stderr)
                    });
                    assert_eq!(
                        got, want,
                        "{id} at {tag}/{alloc_name}: the managed counter MUST read allocs={} \
                         frees={}",
                        want.0, want.1
                    );
                }
            }
        }
    }

    fn refuses(&self, id: &str, src: &str, code: &str, says: &str) {
        // one staging for the four levels, or the rendered path differs and the comparison is vacuous
        let root = self.stage(id, "refused", src);
        let mut rendered = Vec::new();
        for (tag, opt) in LEVELS {
            rendered.push((tag, self.refusal(id, tag, *opt, &root, code, says)));
        }
        let (first_tag, first) = &rendered[0];
        for (tag, other) in &rendered[1..] {
            assert_eq!(
                other, first,
                "{id}: the refusal at {tag} differs from the one at {first_tag}; a rule that \
                 reads the optimizer is not a rule"
            );
        }
    }

    fn refusal(
        &self,
        id: &str,
        tag: &str,
        opt: OptimizationLevel,
        root: &Path,
        code: &str,
        says: &str,
    ) -> String {
        let rendered = match self.build(root, opt) {
            Ok(_) => panic!("{id} at {tag}: MUST be refused"),
            Err(err) => err,
        };
        note_leg();
        assert!(
            !linker_unavailable(&rendered),
            "{id} at {tag}: the refusal must come from the compiler, not the linker\n{rendered}"
        );
        assert!(
            rendered.contains(&format!("[{code}]")),
            "{id} at {tag}: the refusal MUST be {code}\nrendered:\n{rendered}"
        );
        assert!(
            rendered.contains(says),
            "{id} at {tag}: the refusal MUST say {says:?}\nrendered:\n{rendered}"
        );
        rendered
    }
}

#[test]
fn group_v01_r01_array_const_length_runs() {
    let h = Harness::new();
    let _pin = pin_legs("r01", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r01",
        "fn main() -> i64 {\n\
         \x20   let a = [7; 2 + 2]\n\
         \x20   println(a[3])\n\
         \x20   return 0\n\
         }\n",
        0,
        "7\n",
        None,
    );
}

#[test]
fn group_v01_r02_array_length_mismatch_refused() {
    let h = Harness::new();
    let _pin = pin_legs("r02", LEVELS.len());
    h.refuses(
        "r02",
        "fn main() -> i64 {\n\
         \x20   let a: [i64; 2] = [1, 2, 3, 4]\n\
         \x20   println(a[0])\n\
         \x20   return 0\n\
         }\n",
        "E0301",
        "expected `[i64; 2]`, found `[i64; 4]`",
    );
}

#[test]
fn group_v01_r03_const_index_outside_array_refused() {
    let h = Harness::new();
    let _pin = pin_legs("r03", LEVELS.len());
    h.refuses(
        "r03",
        "fn main() -> i64 {\n\
         \x20   let a: [i64; 4] = [1, 2, 3, 4]\n\
         \x20   println(a[7])\n\
         \x20   return 0\n\
         }\n",
        "E0433",
        "[const-index] index 7 is outside `[_; 4]`, whose elements are 0 to 3",
    );
}

#[test]
fn group_v01_r04_array_nonconst_length_refused() {
    let h = Harness::new();
    let _pin = pin_legs("r04", LEVELS.len());
    h.refuses(
        "r04",
        "fn main() -> i64 {\n\
         \x20   let n: i64 = 4\n\
         \x20   let a = [0; n]\n\
         \x20   println(a[0])\n\
         \x20   return 0\n\
         }\n",
        "E0902",
        "unsupported non-constant array size",
    );
}

const RC_CARRIER: &str = "struct W { r: Rc<i64> }\n\n";

#[test]
fn group_v01_r05_rc_carrier_from_a_call_refused() {
    let h = Harness::new();
    let _pin = pin_legs("r05", LEVELS.len());
    h.refuses(
        "r05",
        &format!(
            "{RC_CARRIER}fn mkrc() -> Rc<i64> {{ return Rc::new(7) }}\n\
             \n\
             fn main() -> i64 {{\n\
             \x20   let w: W = W {{ r: mkrc() }}\n\
             \x20   println(Rc::get(w.r))\n\
             \x20   return 0\n\
             }}\n"
        ),
        "E0410",
        "is initialized from a call returning an `Rc<T>`-bearing value",
    );
}

#[test]
fn group_v01_r06_rc_carrier_from_a_conditional_runs() {
    let h = Harness::new();
    let _pin = pin_legs("r06", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r06",
        &format!(
            "{RC_CARRIER}fn main() -> i64 {{\n\
             \x20   let a: Rc<i64> = Rc::new(7)\n\
             \x20   let b: Rc<i64> = Rc::new(9)\n\
             \x20   let c: bool = true\n\
             \x20   let w: W = W {{ r: if c {{ a }} else {{ b }} }}\n\
             \x20   println(Rc::get(w.r))\n\
             \x20   return 0\n\
             }}\n"
        ),
        0,
        "7\n",
        Some((2, 2)),
    );
}

const INLINE_RECURSIVE: &str = "@inline\n\
                                fn probe(n: i64) -> i64 {\n\
                                \x20   if n <= 0 { return 0 }\n\
                                \x20   return probe(n - 1)\n\
                                }\n\
                                \n\
                                fn main() -> i64 {\n\
                                \x20   println(probe(3))\n\
                                \x20   return 0\n\
                                }\n";

#[test]
fn group_v01_r07_werror_input_is_the_same_at_every_level() {
    let h = Harness::new();
    let _pin = pin_legs("r07", LEVELS.len() * ALLOCATORS.len());
    // -werror is a policy over this set, so the set deciding the verdict is what has to be level invariant
    for (tag, opt) in LEVELS {
        let Some((exe, warnings)) = h.compiled("r07", tag, *opt, INLINE_RECURSIVE) else {
            return;
        };
        assert!(
            warnings.contains(&WarningKind::InlineRecursive),
            "r07 at {tag}: the inline refusal MUST be reported here, or -Werror answers one way \
             at this level and another elsewhere; got {warnings:?}"
        );
        for (alloc_name, alloc) in ALLOCATORS {
            let ran = h.run(&exe, *alloc);
            assert_eq!(ran.exit, 0, "r07 at {tag}/{alloc_name} must exit 0");
            assert_eq!(ran.stdout, "0\n", "r07 at {tag}/{alloc_name}");
        }
    }
}

const NET_FIXTURE: &str = "enum Opt<T> { Some(T), Nil }\n\
                           \n\
                           fn main() -> i64 {\n\
                           \x20   let mut i1 = Vec::new()\n\
                           \x20   Vec::push(i1, 1)\n\
                           \x20   let a = Opt::Some(i1)\n\
                           \x20   return 0\n\
                           }\n";

#[test]
fn group_v01_r08_net_names_both_verdicts() {
    let h = Harness::new();
    let _pin = pin_legs("r08", LEVELS.len());
    // this is a tracked cross-o fixture and it was accepted in silence above -o1, so the codes differ by level on purpose
    let root = h.stage("r08", "refused", NET_FIXTURE);
    for (tag, opt) in LEVELS {
        let code = if *opt <= OptimizationLevel::Basic {
            "E0412"
        } else {
            "E0432"
        };
        let rendered = h.refusal(
            "r08",
            tag,
            *opt,
            &root,
            code,
            "which holds a `Vec<T>` by value inside another container",
        );
        if code == "E0432" {
            assert!(
                rendered.contains("the -O levels do not agree about it")
                    && rendered.contains(&format!(
                        "that refusal is raised at -O0 and not at {tag}"
                    )),
                "r08 at {tag}: the net MUST name both verdicts\nrendered:\n{rendered}"
            );
        }
    }
}

#[test]
fn group_v01_r09_file_scope_and_runs() {
    let h = Harness::new();
    let _pin = pin_legs("r09", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r09",
        "pub let g: bool = true and false\n\
         \n\
         fn main() -> i64 {\n\
         \x20   if g { println(1) } else { println(0) }\n\
         \x20   return 0\n\
         }\n",
        0,
        "0\n",
        None,
    );
}

#[test]
fn group_v01_r10_char_class_eab() {
    let h = Harness::new();
    let _pin = pin_legs("r10", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r10",
        "fn main() -> i64 {\n\
         \x20   let s: string = \"éab\"\n\
         \x20   println(s.bytes.len)\n\
         \x20   for c in s {\n\
         \x20       println(c)\n\
         \x20       println(c as i64)\n\
         \x20       println(\"{c}\")\n\
         \x20   }\n\
         \x20   return 0\n\
         }\n",
        0,
        "4\né\n233\né\na\n97\na\nb\n98\nb\n",
        Some((0, 0)),
    );
}

#[test]
fn group_v01_r11_char_literals_and_six_operators() {
    let h = Harness::new();
    let _pin = pin_legs("r11", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r11",
        "fn main() -> i64 {\n\
         \x20   let a: char = 'é'\n\
         \x20   let b: char = 'a'\n\
         \x20   if a == b { println(1) } else { println(0) }\n\
         \x20   if a != b { println(1) } else { println(0) }\n\
         \x20   if a < b { println(1) } else { println(0) }\n\
         \x20   if a <= b { println(1) } else { println(0) }\n\
         \x20   if a > b { println(1) } else { println(0) }\n\
         \x20   if a >= b { println(1) } else { println(0) }\n\
         \x20   return 0\n\
         }\n",
        0,
        "0\n1\n0\n0\n1\n1\n",
        Some((0, 0)),
    );
}

#[test]
fn group_v01_r12_char_from_i64_outside_the_range_panics() {
    let h = Harness::new();
    let _pin = pin_legs("r12", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r12",
        "fn main() -> i64 {\n\
         \x20   let c: char = char::from_i64(1114112)\n\
         \x20   println(c)\n\
         \x20   return 0\n\
         }\n",
        134,
        "",
        None,
    );
}

#[test]
fn group_v01_r13_char_from_i64_inside_the_range_runs() {
    let h = Harness::new();
    let _pin = pin_legs("r13", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r13",
        "fn main() -> i64 {\n\
         \x20   println(char::from_i64(233))\n\
         \x20   println(char::from_i64(97))\n\
         \x20   return 0\n\
         }\n",
        0,
        "é\na\n",
        None,
    );
}

#[test]
fn group_v01_r14_string_index_refused() {
    let h = Harness::new();
    let _pin = pin_legs("r14", LEVELS.len());
    h.refuses(
        "r14",
        "fn main() -> i64 {\n\
         \x20   let s: string = \"éab\"\n\
         \x20   println(s[0])\n\
         \x20   return 0\n\
         }\n",
        "E0304",
        "`.len` counts bytes while `s[i]` counted characters",
    );
}

const NO_PLACE_BYTES: &str = "[no-place] the receiver of `.bytes` denotes no storage";

#[test]
fn group_v01_r15_bytes_of_a_call_result_refused() {
    let h = Harness::new();
    let _pin = pin_legs("r15", LEVELS.len());
    h.refuses(
        "r15",
        "fn mkheap(k: i64) -> string { return \"hel\" + \"lo\" }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let s: &[u8] = mkheap(4).bytes\n\
         \x20   println(s[0])\n\
         \x20   return 0\n\
         }\n",
        "E0421",
        NO_PLACE_BYTES,
    );
}

const RC_STRUCT: &str = "struct N { n: string }\n\n";

#[test]
fn group_v01_r16_bytes_through_an_rc_temporary_refused() {
    let h = Harness::new();
    let _pin = pin_legs("r16", LEVELS.len());
    h.refuses(
        "r16",
        &format!(
            "{RC_STRUCT}fn mkrc() -> Rc<N> {{ return Rc::new(N {{ n: \"hello\" }}) }}\n\
             \n\
             fn main() -> i64 {{\n\
             \x20   let s: &[u8] = mkrc().n.bytes\n\
             \x20   println(s[0])\n\
             \x20   return 0\n\
             }}\n"
        ),
        "E0421",
        NO_PLACE_BYTES,
    );
}

#[test]
fn group_v01_r17_bytes_of_a_literal_runs() {
    let h = Harness::new();
    let _pin = pin_legs("r17", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r17",
        "fn main() -> i64 {\n\
         \x20   let l: &[u8] = \"hi\".bytes\n\
         \x20   println(l[0])\n\
         \x20   println(l.len)\n\
         \x20   return 0\n\
         }\n",
        0,
        "104\n2\n",
        Some((0, 0)),
    );
}

#[test]
fn group_v01_r18_bytes_of_a_bound_rc_runs() {
    let h = Harness::new();
    let _pin = pin_legs("r18", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r18",
        &format!(
            "{RC_STRUCT}fn main() -> i64 {{\n\
             \x20   let r: Rc<N> = Rc::new(N {{ n: \"hello\" }})\n\
             \x20   let s: &[u8] = r.n.bytes\n\
             \x20   println(s[0])\n\
             \x20   return 0\n\
             }}\n"
        ),
        0,
        "104\n",
        Some((1, 1)),
    );
}

#[test]
fn group_v01_r19_generic_library_over_two_element_types() {
    let h = Harness::new();
    let _pin = pin_legs("r19", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r19",
        "needs std.slice\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let mut vi: Vec<i64> = Vec::new()\n\
         \x20   Vec::push(vi, 3)\n\
         \x20   Vec::push(vi, 9)\n\
         \x20   Vec::push(vi, 5)\n\
         \x20   let si: &[i64] = Vec::as_slice(vi)\n\
         \x20   match slice.max(si) { Option::Some(m) => println(m), Option::None => println(-1) }\n\
         \x20   match slice.min(si) { Option::Some(m) => println(m), Option::None => println(-1) }\n\
         \x20   let mut vc: Vec<char> = Vec::new()\n\
         \x20   Vec::push(vc, 'a')\n\
         \x20   Vec::push(vc, 'z')\n\
         \x20   Vec::push(vc, 'm')\n\
         \x20   let sc: &[char] = Vec::as_slice(vc)\n\
         \x20   match slice.max(sc) { Option::Some(m) => println(m), Option::None => println('?') }\n\
         \x20   match slice.min(sc) { Option::Some(m) => println(m), Option::None => println('?') }\n\
         \x20   return 0\n\
         }\n",
        0,
        "9\n3\nz\na\n",
        None,
    );
}

#[test]
fn group_v01_r20_ord_excludes_string() {
    let h = Harness::new();
    let _pin = pin_legs("r20", LEVELS.len());
    h.refuses(
        "r20",
        "fn mx<T: ord>(a: T, b: T) -> T { if a > b { return a } return b }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   println(mx(\"abc\", \"abd\"))\n\
         \x20   return 0\n\
         }\n",
        "E0732",
        "is bound `ord`, but this call instantiates it with `string`, which is not `ord`",
    );
}

#[test]
fn group_v01_r21_eq_bound_over_two_element_types() {
    let h = Harness::new();
    let _pin = pin_legs("r21", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r21",
        "needs std.slice\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let mut vc: Vec<char> = Vec::new()\n\
         \x20   Vec::push(vc, 'a')\n\
         \x20   Vec::push(vc, 'z')\n\
         \x20   Vec::push(vc, 'm')\n\
         \x20   let sc: &[char] = Vec::as_slice(vc)\n\
         \x20   println(slice.index_of(sc, 'm'))\n\
         \x20   if slice.contains(sc, 'q') { println(1) } else { println(0) }\n\
         \x20   let mut vi: Vec<i64> = Vec::new()\n\
         \x20   Vec::push(vi, 4)\n\
         \x20   Vec::push(vi, 8)\n\
         \x20   let si: &[i64] = Vec::as_slice(vi)\n\
         \x20   println(slice.index_of(si, 8))\n\
         \x20   return 0\n\
         }\n",
        0,
        "2\n0\n1\n",
        None,
    );
}

#[test]
fn group_v01_r22_expected_found_order() {
    let h = Harness::new();
    let _pin = pin_legs("r22", LEVELS.len());
    h.refuses(
        "r22",
        "fn main() -> i64 {\n\
         \x20   let x: i64 = true\n\
         \x20   println(x)\n\
         \x20   return 0\n\
         }\n",
        "E0301",
        "expected `i64`, found `bool`",
    );
}

#[test]
fn group_v01_r23_character_loop_values() {
    let h = Harness::new();
    let _pin = pin_legs("r23", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r23",
        "fn main() -> i64 {\n\
         \x20   let s: string = \"héllo ca été là\"\n\
         \x20   let mut n: i64 = 0\n\
         \x20   let mut acc: i64 = 0\n\
         \x20   for c in s {\n\
         \x20       n = n + 1\n\
         \x20       acc = acc + (c as i64)\n\
         \x20   }\n\
         \x20   println(n)\n\
         \x20   println(acc)\n\
         \x20   println(s.bytes.len)\n\
         \x20   return 0\n\
         }\n",
        0,
        "15\n1870\n19\n",
        Some((0, 0)),
    );
}

#[test]
fn group_v01_r24_plain_arithmetic_control() {
    let h = Harness::new();
    let _pin = pin_legs("r24", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r24",
        "fn main() -> i64 {\n\
         \x20   let mut i: i64 = 0\n\
         \x20   let mut t: i64 = 0\n\
         \x20   while i < 5 {\n\
         \x20       t = t + i * i\n\
         \x20       i = i + 1\n\
         \x20   }\n\
         \x20   println(t)\n\
         \x20   return 0\n\
         }\n",
        0,
        "30\n",
        Some((0, 0)),
    );
}

#[test]
fn group_v01_r25_byte_view_loop_control() {
    let h = Harness::new();
    let _pin = pin_legs("r25", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "r25",
        "fn main() -> i64 {\n\
         \x20   let s: string = \"héllo\"\n\
         \x20   let mut sum: i64 = 0\n\
         \x20   for b in s.bytes {\n\
         \x20       sum = sum + (b as i64)\n\
         \x20   }\n\
         \x20   println(sum)\n\
         \x20   println(s.bytes.len)\n\
         \x20   return 0\n\
         }\n",
        0,
        "795\n6\n",
        Some((0, 0)),
    );
}

const LOOP_SEED: &str = "éabéabéabéabéabéabéabéab";
const LOOP_SIZES: &[(i64, i32)] = &[(9, 104), (11, 165)];
const LOOP_REPEATS: usize = 5;
// a four times size step reads two on a floor bound run and sixteen when the loop re-walks
const LOOP_SPAN_BOUND: f64 = 6.0;

fn loop_source(doubles: i64) -> String {
    format!(
        "fn main() -> i64 {{\n\
         \x20   let mut s: string = \"{LOOP_SEED}\"\n\
         \x20   let mut k: i64 = 0\n\
         \x20   while k < {doubles} {{\n\
         \x20       s = s + s\n\
         \x20       k = k + 1\n\
         \x20   }}\n\
         \x20   let mut acc: i64 = 0\n\
         \x20   for c in s {{\n\
         \x20       acc = acc + (c as i64)\n\
         \x20   }}\n\
         \x20   return acc % 251\n\
         }}\n"
    )
}

#[test]
fn group_v01_r26_character_loop_is_linear() {
    let h = Harness::new();
    let _pin = pin_legs("r26", LEVELS.len() * LOOP_SIZES.len() * LOOP_REPEATS);
    for (tag, opt) in LEVELS {
        let mut best = Vec::new();
        for (doubles, exit) in LOOP_SIZES {
            let src = loop_source(*doubles);
            let Some((exe, _)) = h.compiled(&format!("r26_{doubles}"), tag, *opt, &src) else {
                return;
            };
            let mut fastest = f64::MAX;
            for _ in 0..LOOP_REPEATS {
                let at = Instant::now();
                let ran = h.run(&exe, None);
                fastest = fastest.min(at.elapsed().as_secs_f64());
                assert_eq!(
                    ran.exit, *exit,
                    "r26 at {tag}: {doubles} doublings MUST exit {exit}\nstderr:\n{}",
                    ran.stderr
                );
            }
            best.push(fastest);
        }
        let span = best[1] / best[0];
        assert!(
            span <= LOOP_SPAN_BOUND,
            "r26 at {tag}: four times the characters costs {span:.2}x, over the \
             {LOOP_SPAN_BOUND}x bound; the traversal is re-walking the string ({:.4}s then \
             {:.4}s)",
            best[0],
            best[1]
        );
    }
}

#[test]
fn group_v01_w1_generic_struct_carrier_refused() {
    let h = Harness::new();
    let _pin = pin_legs("w1", LEVELS.len());
    h.refuses(
        "w1",
        "struct Wrapper<T> { v: T }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let w: Wrapper<i64> = Wrapper { v: 5 }\n\
         \x20   println(w.v)\n\
         \x20   return 0\n\
         }\n",
        "E0304",
        "unresolved generic type parameter 'T' escaped generic context",
    );
}

#[test]
fn group_v01_w2_generic_enum_payload_runs() {
    let h = Harness::new();
    let _pin = pin_legs("w2", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "w2",
        "enum Box<T> { Full(T), Empty }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let b: Box<i64> = Box::Full(6)\n\
         \x20   match b { Box::Full(v) => println(v), Box::Empty => println(-1) }\n\
         \x20   return 0\n\
         }\n",
        0,
        "6\n",
        None,
    );
}

#[test]
fn group_v01_w3_file_scope_binary_initializer_runs() {
    let h = Harness::new();
    let _pin = pin_legs("w3", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "w3",
        "pub let g: i64 = 2 + 2\n\
         \n\
         fn main() -> i64 {\n\
         \x20   println(g)\n\
         \x20   return 0\n\
         }\n",
        0,
        "4\n",
        None,
    );
}

#[test]
fn group_v01_w4_generic_function_over_a_struct_runs() {
    let h = Harness::new();
    let _pin = pin_legs("w4", LEVELS.len() * ALLOCATORS.len());
    h.runs(
        "w4",
        "struct P { a: i64 }\n\
         \n\
         fn pick<T>(p: P, x: T) -> T { return x }\n\
         \n\
         fn main() -> i64 {\n\
         \x20   let p: P = P { a: 3 }\n\
         \x20   println(pick(p, 42))\n\
         \x20   return 0\n\
         }\n",
        0,
        "42\n",
        None,
    );
}

#[test]
fn group_v01_census_covers_every_row_and_declares_every_control() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("group_v01_tests.rs");
    let text = fs::read_to_string(&source)
        .unwrap_or_else(|e| panic!("this file must be readable at {}: {e}", source.display()));
    let mut rows: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("fn group_v01_")
            && let Some(name) = rest.strip_suffix("() {")
            && name != "census_covers_every_row_and_declares_every_control"
        {
            rows.push(name.to_string());
        }
    }
    assert_eq!(
        rows.len(),
        CENSUS.len(),
        "the census has {} entries and the file has {} rows: {rows:?}",
        CENSUS.len(),
        rows.len()
    );
    for (name, bucket, reason) in CENSUS {
        assert!(
            rows.iter().any(|r| r == name),
            "the census names `{name}`, which is not a row of this file"
        );
        if *bucket == Bucket::Neither {
            assert!(
                reason.len() > 40,
                "`{name}` is a neither, so it must carry the reason it discriminates nothing; an \
                 undeclared neither is a dead row"
            );
        } else if *bucket == Bucket::Anchor {
            assert!(
                reason.is_empty(),
                "`{name}` discriminates the anchor, so it needs no note beyond its own assertions"
            );
        }
    }
    let anchor = CENSUS.iter().filter(|r| r.1 == Bucket::Anchor).count();
    let prefix = CENSUS.iter().filter(|r| r.1 == Bucket::Prefix).count();
    let neither = CENSUS.iter().filter(|r| r.1 == Bucket::Neither).count();
    assert_eq!(
        (anchor, prefix, neither),
        (22, 1, 7),
        "the census partition moved; re-measure at `v01/anchor` before editing this number"
    );
}
