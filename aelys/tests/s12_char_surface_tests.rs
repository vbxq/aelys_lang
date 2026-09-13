use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::Instant;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

const MODULES: &[&str] = &["result.aelys", "str.aelys"];

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

struct Harness {
    dir: TempDir,
    library: PathBuf,
    legs: Cell<usize>,
    linker_skips: Cell<usize>,
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
            library: library_root(),
            legs: Cell::new(0),
            linker_skips: Cell::new(0),
        }
    }

    // `needs std.str` resolves under the root file, so the library is copied beside every root
    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&self.library, &dir.join("std"));
        let root = dir.join("root.aelys");
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn compile_at(
        &self,
        id: &str,
        tag: &str,
        root: &Path,
        opt: OptimizationLevel,
    ) -> Option<PathBuf> {
        match compile_file_with_llvm_variant(root, opt, false, RuntimeVariant::Rc) {
            Ok(()) => {}
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    self.linker_skips.set(self.linker_skips.get() + 1);
                    return None;
                }
                panic!("{id} at {tag} must compile:\n{err}");
            }
        }
        let exe = exe_path_for(root);
        exe.is_file().then_some(exe)
    }

    fn value_row(&self, id: &str, src: &str, stdout: &str) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let Some(exe) = self.compile_at(id, tag, &root, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped value \
                     row carries no runtime evidence at all"
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let mut cmd = Command::new(&exe);
                cmd.env("AELYS_RC_STATS", "1");
                if let Some(a) = alloc {
                    cmd.env("AELYS_ALLOC", a);
                }
                let out = cmd.output().expect("run compiled exe");
                let seen_out = String::from_utf8_lossy(&out.stdout).into_owned();
                let seen_err = String::from_utf8_lossy(&out.stderr).into_owned();
                self.legs.set(self.legs.get() + 1);
                assert_eq!(
                    exit_code(&out.status),
                    0,
                    "{id} at {tag}/{alloc_name} must exit 0\nstderr:\n{seen_err}"
                );
                assert_eq!(
                    seen_out, stdout,
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}"
                );
            }
        }
    }

    fn panic_row(&self, id: &str, src: &str, says: &str) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let Some(exe) = self.compile_at(id, tag, &root, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped panic \
                     row carries no runtime evidence at all"
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let mut cmd = Command::new(&exe);
                if let Some(a) = alloc {
                    cmd.env("AELYS_ALLOC", a);
                }
                let out = cmd.output().expect("run compiled exe");
                let seen_err = String::from_utf8_lossy(&out.stderr).into_owned();
                self.legs.set(self.legs.get() + 1);
                assert_eq!(
                    exit_code(&out.status),
                    134,
                    "{id} at {tag}/{alloc_name} MUST abort\nstderr:\n{seen_err}"
                );
                assert!(
                    seen_err.contains(says),
                    "{id} at {tag}/{alloc_name}: the panic MUST say {says:?}\nstderr:\n{seen_err}"
                );
            }
        }
    }

    // one refusal that reads the same at every level: a verdict that moves with -o is e0432
    fn rejects(&self, id: &str, src: &str, code: &str, says: &str) {
        // one staging directory for every level so the rendered path cannot differ by itself
        let root = self.stage(id, "reject", src);
        let mut seen: Option<String> = None;
        for (tag, opt) in LEVELS {
            let rendered = match lower_file_to_air(&root, *opt) {
                Ok(_) => panic!("{id} at {tag}: MUST be rejected\n{src}"),
                Err(rendered) => rendered,
            };
            self.legs.set(self.legs.get() + 1);
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {tag}: the rejection MUST be {code}\n{src}\nrendered:\n{rendered}"
            );
            assert!(
                rendered.contains(says),
                "{id} at {tag}: the rejection MUST say {says:?}\n{src}\nrendered:\n{rendered}"
            );
            match &seen {
                None => seen = Some(rendered),
                Some(first) => assert_eq!(
                    *first, rendered,
                    "{id}: the refusal at {tag} does not read like the refusal at -O0"
                ),
            }
        }
    }

    // the compiler's own advice must not hand back the escaping view s[i] was removed for
    fn refuses_without_saying(&self, id: &str, src: &str, forbidden: &str) {
        let root = self.stage(id, "advice", src);
        let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("{id}: MUST be rejected\n{src}"),
            Err(rendered) => rendered,
        };
        self.legs.set(self.legs.get() + 1);
        assert!(
            !rendered.contains(forbidden),
            "{id}: the refusal MUST NOT say {forbidden:?}\n{src}\nrendered:\n{rendered}"
        );
    }

    fn assert_legs(&self, expected: usize) {
        if self.linker_skips.get() > 0 && linker_skip_declared() {
            return;
        }
        assert_eq!(
            self.legs.get(),
            expected,
            "this test must execute exactly {expected} legs; a leg that silently stopped running \
             is a confident zero"
        );
    }
}


const C1_THE_MULTIBYTE_CLASS: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    for c in s {
        println(c)
        println(c as i64)
    }
    return 0
}
"#;

// the class the stage is named after, executed, with the scalar values asserted
#[test]
fn c1_a_multibyte_string_yields_its_scalar_values() {
    let h = Harness::new();
    h.value_row("C-1", C1_THE_MULTIBYTE_CLASS, "é\n233\na\n97\nb\n98\n");
    h.assert_legs(8);
}

const C2_BYTES_AND_CHARACTERS: &str = r#"
needs std.str

fn main() -> i64 {
    let s: string = "éab"
    println(s.bytes.len)
    println(str.char_count(s))
    println(s.len)
    return 0
}
"#;

#[test]
fn c2_the_byte_count_and_the_character_count_are_both_readable() {
    let h = Harness::new();
    h.value_row("C-2", C2_BYTES_AND_CHARACTERS, "4\n3\n4\n");
    h.assert_legs(8);
}

const C3_LITERALS: &str = r#"
fn main() -> i64 {
    println('é' as i64)
    println('日' as i64)
    println('\n' as i64)
    println('\'' as i64)
    println('\\' as i64)
    println('\t' as i64)
    println('\0' as i64)
    println('\r' as i64)
    println('a' as i64)
    return 0
}
"#;

#[test]
fn c3_every_literal_form_is_one_scalar_value() {
    let h = Harness::new();
    h.value_row("C-3", C3_LITERALS, "233\n26085\n10\n39\n92\n9\n0\n13\n97\n");
    h.assert_legs(8);
}

const C4_COMPARISONS: &str = r#"
fn main() -> i64 {
    let a: char = 'a'
    let z: char = 'z'
    let e: char = 'é'
    println(a == a)
    println(a != z)
    println(a < z)
    println(z <= z)
    println(z > a)
    println(a >= a)
    println(e > z)
    println(a < e)
    return 0
}
"#;

// without >= and <= the eleven line program below does not compile, so all six are pinned
#[test]
fn c4_the_six_comparisons_order_by_code_point() {
    let h = Harness::new();
    h.value_row(
        "C-4",
        C4_COMPARISONS,
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n",
    );
    h.assert_legs(8);
}

const C5_IN_AND_OUT: &str = r#"
fn main() -> i64 {
    let e: char = 'é'
    println(e as i64)
    println(char::from_i64(233) == e)
    println(char::from_i64(97))
    println(char::is_scalar(0))
    println(char::is_scalar(1114111))
    println(char::is_scalar(1114112))
    println(char::is_scalar(55296))
    println(char::is_scalar(57343))
    println(char::is_scalar(55295))
    println(char::is_scalar(0 - 1))
    println(string::from_char('日'))
    return 0
}
"#;

#[test]
fn c5_the_way_in_and_the_way_out_round_trip() {
    let h = Harness::new();
    h.value_row(
        "C-5",
        C5_IN_AND_OUT,
        "233\ntrue\na\ntrue\ntrue\nfalse\nfalse\nfalse\ntrue\nfalse\n日\n",
    );
    h.assert_legs(8);
}

const C6_ABOVE_THE_RANGE: &str = r#"
fn main() -> i64 {
    println(char::from_i64(1114112))
    return 0
}
"#;

const C6_A_SURROGATE: &str = r#"
fn main() -> i64 {
    println(char::from_i64(55296))
    return 0
}
"#;

const C6_BELOW_THE_RANGE: &str = r#"
fn main() -> i64 {
    println(char::from_i64(0 - 1))
    return 0
}
"#;

#[test]
fn c6_from_i64_panics_naming_the_value_and_the_ranges() {
    let h = Harness::new();
    let says = "char::from_i64: {} is not a unicode scalar value, the valid ranges are \
                0..=1114111 excluding 55296..=57343";
    h.panic_row("C-6-above", C6_ABOVE_THE_RANGE, &says.replace("{}", "1114112"));
    h.panic_row("C-6-surrogate", C6_A_SURROGATE, &says.replace("{}", "55296"));
    h.panic_row("C-6-below", C6_BELOW_THE_RANGE, &says.replace("{}", "-1"));
    h.assert_legs(24);
}

const C7_BOTH_PRINT_PATHS: &str = r#"
fn main() -> i64 {
    let e: char = 'é'
    let j: char = '日'
    println(e)
    println("{e}")
    println(j)
    println("{j}")
    println("<{e}{j}>")
    return 0
}
"#;

#[test]
fn c7_println_and_interpolation_both_print_the_character() {
    let h = Harness::new();
    h.value_row("C-7", C7_BOTH_PRINT_PATHS, "é\né\n日\n日\n<é日>\n");
    h.assert_legs(8);
}

const C8_A_CHAR_LITERAL_INSIDE_AN_INTERPOLATION: &str = r#"
fn id(c: char) -> char { return c }

fn main() -> i64 {
    println("{ id('}') }")
    println("{ id('{') }")
    println("{ id('\"') }")
    println("{ id('\'') }")
    return 0
}
"#;

// scan_format_expr counts braces, so a literal holding one has to be skipped whole
#[test]
fn c8_a_char_literal_holding_a_delimiter_survives_interpolation() {
    let h = Harness::new();
    h.value_row(
        "C-8",
        C8_A_CHAR_LITERAL_INSIDE_AN_INTERPOLATION,
        "}\n{\n\"\n'\n",
    );
    h.assert_legs(8);
}

const C9_STRING_INDEX_READ: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    println(s[0])
    return 0
}
"#;

const C9_STRING_INDEX_WRITE: &str = r#"
fn main() -> i64 {
    let mut s: string = "éab"
    s[0] = "x"
    return 0
}
"#;

#[test]
fn c9_indexing_a_string_is_refused_and_the_advice_is_not_the_byte_view() {
    let h = Harness::new();
    h.rejects(
        "C-9-read",
        C9_STRING_INDEX_READ,
        "E0304",
        "a `string` cannot be indexed by integer",
    );
    h.rejects(
        "C-9-write",
        C9_STRING_INDEX_WRITE,
        "E0304",
        "a `string` cannot be written through `s[i]`",
    );
    h.refuses_without_saying("C-9-read", C9_STRING_INDEX_READ, ".bytes[");
    h.refuses_without_saying("C-9-write", C9_STRING_INDEX_WRITE, ".bytes[");
    h.assert_legs(10);
}

const C10_CHAR_EQUALS_INT: &str = r#"
fn main() -> i64 {
    println('a' == 97)
    return 0
}
"#;

const C10_INT_AS_CHAR: &str = r#"
fn main() -> i64 {
    let n: i64 = 97
    let c: char = n as char
    return 0
}
"#;

const C10_CHAR_ANNOTATION_TAKES_AN_INT: &str = r#"
fn main() -> i64 {
    let c: char = 97
    return 0
}
"#;

const C10_CHAR_ARITHMETIC: &str = r#"
fn main() -> i64 {
    let c: char = 'a'
    let d: char = c + 'b'
    return 0
}
"#;

const C10_CHAR_ARGUMENT_TAKES_AN_INT: &str = r#"
fn f(c: char) -> i64 { return c as i64 }

fn main() -> i64 {
    return f(97)
}
"#;

// char and i64 do not unify, in either direction and in any position
#[test]
fn c10_a_char_is_not_an_integer() {
    let h = Harness::new();
    h.rejects(
        "C-10-eq",
        C10_CHAR_EQUALS_INT,
        "E0304",
        "requires operands of the same type, found `char` and `i64`",
    );
    h.rejects("C-10-cast-in", C10_INT_AS_CHAR, "E0301", "invalid cast");
    h.rejects(
        "C-10-annotation",
        C10_CHAR_ANNOTATION_TAKES_AN_INT,
        "E0301",
        "expected `char`, found `i64`",
    );
    h.rejects(
        "C-10-arith",
        C10_CHAR_ARITHMETIC,
        "E0301",
        "type `char` is not one of",
    );
    h.rejects(
        "C-10-argument",
        C10_CHAR_ARGUMENT_TAKES_AN_INT,
        "E0301",
        "expected `char`, found `i64`",
    );
    h.assert_legs(20);
}

const C11_TWO_SCALARS: &str = r#"
fn main() -> i64 {
    let c: char = 'ab'
    return 0
}
"#;

const C11_NO_SCALAR: &str = r#"
fn main() -> i64 {
    let c: char = ''
    return 0
}
"#;

const C11_UNTERMINATED: &str = r#"
fn main() -> i64 {
    let c: char = 'a
    return 0
}
"#;

#[test]
fn c11_a_literal_holds_exactly_one_scalar_and_says_so() {
    let h = Harness::new();
    h.rejects(
        "C-11-two",
        C11_TWO_SCALARS,
        "E0010",
        "this one holds 2; write it with `\"` if you meant a string",
    );
    h.rejects(
        "C-11-zero",
        C11_NO_SCALAR,
        "E0010",
        "this one holds 0; write it with `\"` if you meant a string",
    );
    h.rejects(
        "C-11-open",
        C11_UNTERMINATED,
        "E0009",
        "unterminated character literal",
    );
    // a code point escape is capability redundant with char::from_i64, so it is not a literal form
    h.rejects(
        "C-11-escape",
        C11_A_CODE_POINT_ESCAPE,
        "E0005",
        "invalid escape sequence '\\u'",
    );
    h.assert_legs(16);
}

const C11_A_CODE_POINT_ESCAPE: &str = r#"
fn main() -> i64 {
    let c: char = '\u{41}'
    return 0
}
"#;

const C12_NOGC_READING_SURFACE: &str = r#"
nogc fn count_upper(s: string) -> i64 {
    let mut n: i64 = 0
    for c in s {
        if c >= 'A' and c <= 'Z' { n = n + 1 }
    }
    return n
}

nogc fn scalar(c: char) -> i64 { return c as i64 }

nogc fn back(i: i64) -> char { return char::from_i64(i) }

nogc fn admits(i: i64) -> bool { return char::is_scalar(i) }

fn main() -> i64 {
    println(count_upper("aBcDé"))
    println(scalar('é'))
    println(back(97))
    println(admits(233))
    return 0
}
"#;

const C12_A_NOGC_BOUND_AT_CHAR: &str = r#"
nogc fn keep<T: nogc>(x: T) -> i64 { let y = x
 return 0 }

fn main() -> i64 {
    println(keep('a'))
    println(keep(7))
    return 0
}
"#;

const C12_A_NOGC_BOUND_AT_STRING: &str = r#"
nogc fn keep<T: nogc>(x: T) -> i64 { let y = x
 return 0 }

fn main() -> i64 {
    println(keep("a"))
    return 0
}
"#;

const C12_FROM_CHAR_IS_NOT_NOGC: &str = r#"
nogc fn one(c: char) -> string { return string::from_char(c) }

fn main() -> i64 {
    println(one('a'))
    return 0
}
"#;

// the nogc witness is that the fence accepts the body, not a number the allocator reports
#[test]
fn c12_the_reading_surface_crosses_the_nogc_fence_and_the_exit_does_not() {
    let h = Harness::new();
    h.value_row("C-12", C12_NOGC_READING_SURFACE, "2\n233\na\ntrue\n");
    h.value_row("C-12-bound", C12_A_NOGC_BOUND_AT_CHAR, "0\n0\n");
    h.rejects(
        "C-12-bound-string",
        C12_A_NOGC_BOUND_AT_STRING,
        "E0730",
        "instantiates it with `string`, which is not a nogc value",
    );
    h.rejects(
        "C-12-exit",
        C12_FROM_CHAR_IS_NOT_NOGC,
        "E0727",
        "`one -> string::from_char`",
    );
    h.assert_legs(24);
}

const C13_FFI: &str = r#"
unsafe extern fn cfn(c: char) -> i64

fn main() -> i64 { return 0 }
"#;

#[test]
fn c13_the_external_surface_refuses_a_char_by_name() {
    let h = Harness::new();
    h.rejects(
        "C-13",
        C13_FFI,
        "E0615",
        "a `char` is a validated unicode scalar value, and c's `char` is a byte",
    );
    h.assert_legs(4);
}

const C14_UPPER_WITH_BYTES: &str = r#"
needs std.str as str
fn upper(s: string) -> string {
    let up: string = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    let mut t: string = ""
    let mut i: i64 = 0
    while i < s.bytes.len {
        let k: i64 = str.char_len(s, i)
        if k < 1 { return t }
        let b: u8 = s.bytes[i]
        if k == 1 and b >= 97 and b <= 122 {
            let j: i64 = (b as i64) - 97
            t = t + string::substring_bytes(up, j, j + 1)
        } else { t = t + string::substring_bytes(s, i, i + k) }
        i = i + k
    }
    return t
}

fn main() -> i64 {
    println(upper("hello wérld 123!"))
    println("[" + upper("") + "]")
    println(upper("éé"))
    println(upper("aA1é zZ"))
    return 0
}
"#;

const C14_UPPER_WITH_CHAR: &str = r#"
fn upper(s: string) -> string {
    let mut t: string = ""
    for c in s {
        if c >= 'a' and c <= 'z' {
            t = t + string::from_char(char::from_i64(c as i64 - 32))
        } else {
            t = t + string::from_char(c)
        }
    }
    return t
}

fn main() -> i64 {
    println(upper("hello wérld 123!"))
    println("[" + upper("") + "]")
    println(upper("éé"))
    println(upper("aA1é zZ"))
    return 0
}
"#;

const C14_EXPECTED: &str = "HELLO WéRLD 123!\n[]\néé\nAA1é ZZ\n";

#[test]
fn c14_the_ordinary_program_is_shorter_with_char_and_answers_the_same() {
    let h = Harness::new();
    h.value_row("C-14-bytes", C14_UPPER_WITH_BYTES, C14_EXPECTED);
    h.value_row("C-14-char", C14_UPPER_WITH_CHAR, C14_EXPECTED);
    h.assert_legs(16);

    let lines = |src: &str| -> usize {
        src.lines()
            .skip_while(|l| l.trim().is_empty())
            .take_while(|l| !l.starts_with("fn main("))
            .filter(|l| !l.trim().is_empty())
            .count()
    };
    let bytes = lines(C14_UPPER_WITH_BYTES);
    let chars = lines(C14_UPPER_WITH_CHAR);
    assert_eq!(
        (bytes, chars),
        (17, 11),
        "the two spellings of `upper` are {bytes} and {chars} lines; the stage shipped on the \
         claim that the char version is the shorter one, so the number is pinned here"
    );
}

const C15_ONE_PASS_VALUES: &str = r#"
needs std.result
needs std.str

nogc fn scalars(s: string) -> i64 {
    let mut acc: i64 = 0
    for c in s { acc = acc + (c as i64) }
    return acc
}

nogc fn visits(s: string) -> i64 {
    let mut n: i64 = 0
    for c in s { n = n + 1 }
    return n
}

fn main() -> i64 {
    for c in "éab" { println(c) }
    println(visits("héllo"))
    println(scalars("héllo"))
    println(visits(""))
    println(scalars(""))
    for c in "ab日" { println(c as i64) }
    for c in "a𝄞b" { println(c as i64) }
    println(result.some_or(str.char_at("héllo", 1), '?'))
    println(result.some_or(str.char_at("a𝄞b", 1), '?') as i64)
    println(result.some_or(str.char_at("ab日", 2), '?') as i64)
    if result.is_none(str.char_at("ab日", 3)) { println(1) } else { println(0) }
    return 0
}
"#;

const C15_EXPECTED: &str =
    "é\na\nb\n5\n664\n0\n0\n97\n98\n26085\n97\n119070\n98\né\n119070\n26085\n1\n";

// the cursor the loop now carries must land on the same characters the re-walk landed on,
#[test]
fn c15_the_byte_cursor_visits_every_character_and_only_the_characters() {
    let h = Harness::new();
    h.value_row("C-15", C15_ONE_PASS_VALUES, C15_EXPECTED);
    h.assert_legs(8);
}

// the cost is read at -o2 only: a linear traversal is a property of the loop, not of the inliner
const C16_LEVEL: (&str, OptimizationLevel) = ("-O2", OptimizationLevel::Standard);

const C16_REPS: i64 = 60;

const C16_SIZES: &[(u32, i64, i32)] = &[(10, 8192, 6), (11, 16384, 5), (12, 32768, 3), (13, 65536, 6)];

const C16_SAMPLES: usize = 9;

const C16_DOUBLING_BOUND: f64 = 2.8;

const C16_SPAN_BOUND: f64 = 12.0;

const C16_MIN_NET_MS: f64 = 0.20;

const C16_FLOOR: &str = r#"
fn main() -> i64 {
    return 0
}
"#;

fn c16_src(doublings: u32) -> String {
    format!(
        r#"
fn main() -> i64 {{
    let mut s: string = "abcdefgh"
    let mut i: i64 = 0
    while i < {doublings} {{
        s = s + s
        i = i + 1
    }}
    let mut acc: i64 = 0
    let mut r: i64 = 0
    while r < {C16_REPS} {{
        for c in s {{
            acc = acc + (c as i64)
        }}
        r = r + 1
    }}
    return acc % 7
}}
"#
    )
}

fn c16_run(exe: &Path, chars: i64, oracle: i32) -> f64 {
    let started = Instant::now();
    let out = Command::new(exe).output().expect("run the traversal");
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let seen = exit_code(&out.status);
    assert_eq!(
        seen, oracle,
        "C-16 over {chars} characters must exit {oracle}; a traversal that skips, repeats or \
         mis-decodes a character lands on a different sum"
    );
    ms
}

#[test]
fn c16_the_cost_of_the_traversal_doubles_with_its_input() {
    let h = Harness::new();
    let (tag, opt) = C16_LEVEL;

    let floor_root = h.stage("C-16-floor", tag, C16_FLOOR);
    let Some(floor_exe) = h.compile_at("C-16-floor", tag, &floor_root, opt) else {
        assert!(
            linker_skip_declared(),
            "C-16: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped complexity row \
             measures nothing at all"
        );
        return;
    };

    let mut built = Vec::with_capacity(C16_SIZES.len());
    for (doublings, chars, oracle) in C16_SIZES {
        let id = format!("C-16-{chars}");
        let root = h.stage(&id, tag, &c16_src(*doublings));
        let Some(exe) = h.compile_at(&id, tag, &root, opt) else {
            assert!(
                linker_skip_declared(),
                "C-16: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped complexity \
                 row measures nothing at all"
            );
            return;
        };
        built.push((*chars, *oracle, exe));
    }

    // the samples are interleaved so a transient load lands on every size, never on one ratio
    let mut floor_ms = f64::MAX;
    let mut best = vec![f64::MAX; built.len()];
    let mut runs = 0usize;
    for _ in 0..C16_SAMPLES {
        floor_ms = floor_ms.min(c16_run(&floor_exe, 0, 0));
        runs += 1;
        for (i, (chars, oracle, exe)) in built.iter().enumerate() {
            best[i] = best[i].min(c16_run(exe, *chars, *oracle));
            runs += 1;
        }
    }
    assert_eq!(
        runs,
        C16_SAMPLES * (built.len() + 1),
        "a timing leg that silently stopped running is a confident zero"
    );

    let net: Vec<f64> = best.iter().map(|ms| ms - floor_ms).collect();
    let table = built
        .iter()
        .zip(&net)
        .map(|((chars, _, _), ms)| format!("{chars} chars: {ms:.3} ms"))
        .collect::<Vec<_>>()
        .join("\n  ");
    let report = format!("floor {floor_ms:.3} ms, net of the floor:\n  {table}");
    eprintln!("C-16 at {tag}: {report}");

    assert!(
        net[0] >= C16_MIN_NET_MS,
        "C-16: the smallest traversal nets {:.3} ms, under the {C16_MIN_NET_MS} ms this row needs \
         to say anything; raise C16_REPS until it clears the floor\n{report}",
        net[0]
    );

    for i in 1..net.len() {
        let ratio = net[i] / net[i - 1];
        assert!(
            ratio <= C16_DOUBLING_BOUND,
            "C-16: {} characters cost {ratio:.3}x what {} characters cost, over the \
             {C16_DOUBLING_BOUND}x this row allows for twice the input; a quadratic traversal \
             lands near 4x\n{report}",
            built[i].0,
            built[i - 1].0
        );
    }

    let span = net[net.len() - 1] / net[0];
    assert!(
        span <= C16_SPAN_BOUND,
        "C-16: the span from {} to {} characters costs {span:.3}x, over the {C16_SPAN_BOUND}x \
         this row allows for eight times the input; a quadratic traversal lands near 64x\n{report}",
        built[0].0,
        built[built.len() - 1].0
    );
}
