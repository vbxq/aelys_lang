use aelys_driver::{
    LinkRequirement, RuntimeVariant, compile_file_with_llvm, compile_file_with_llvm_linked,
};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

mod common;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const PINNED: u32 = u32::MAX;
const NO_TRACE: u32 = 2;

// the reader is the only way to see the header: the counters read zero while a raw malloc leaked
const PROBE_C: &str = r#"#include <stdint.h>
#include <string.h>

long long aelys_hdr_refcount(const unsigned char *p) {
    uint32_t v = 0;
    memcpy(&v, p - 16, 4);
    return (long long)v;
}

long long aelys_hdr_flags(const unsigned char *p) {
    uint8_t v = 0;
    memcpy(&v, p - 16 + 4, 1);
    return (long long)v;
}

long long aelys_hdr_type_id(const unsigned char *p) {
    uint32_t v = 0;
    memcpy(&v, p - 16 + 8, 4);
    return (long long)v;
}

/* the seven bytes no producer reads: a residue here is the only thing that tells the three writers apart */
long long aelys_hdr_reserved(const unsigned char *p) {
    const unsigned char *h = p - 16;
    return (long long)h[5] + h[6] + h[7] + h[12] + h[13] + h[14] + h[15];
}

/* the measured strings must land on a recycled slab, or a fresh mmap answers zero and the row proves nothing */
extern void *__aelys_alloc(long long);
extern void __aelys_free(void *);

void aelys_dirty_heap(long long bytes) {
    void *blocks[64];
    for (int i = 0; i < 64; i++) {
        blocks[i] = __aelys_alloc(bytes);
        memset(blocks[i], 0xFF, (size_t)bytes);
    }
    for (int i = 0; i < 64; i++) {
        __aelys_free(blocks[i]);
    }
}

long long aelys_opaque(long long x) { return x; }

/* the frame slot inherits whatever the previous call left, so a zeroed header must be written and not found */
void aelys_dirty_stack(void) {
    volatile unsigned char scratch[4096];
    for (int i = 0; i < 4096; i++) {
        scratch[i] = 0xFF;
    }
}
"#;

// every operand the optimizer could fold is seeded from aelys_opaque, or -o1 replaces a
const READER: &str = r#"unsafe extern fn aelys_hdr_refcount(p: &u8) -> i64
unsafe extern fn aelys_hdr_flags(p: &u8) -> i64
unsafe extern fn aelys_hdr_type_id(p: &u8) -> i64
unsafe extern fn aelys_hdr_reserved(p: &u8) -> i64
unsafe extern fn aelys_dirty_heap(bytes: i64)
unsafe extern fn aelys_opaque(x: i64) -> i64

let G: string = "a global"

fn show(tag: string, s: string) {
    let b: &[u8] = s.bytes
    unsafe {
        print(tag)
        print("|")
        print(aelys_hdr_refcount(&b[0]))
        print("|")
        print(aelys_hdr_flags(&b[0]))
        print("|")
        print(aelys_hdr_type_id(&b[0]))
        print("|")
        println(aelys_hdr_reserved(&b[0]))
    }
}

fn borrow_once(s: string) {
    show("P5-borrow", s)
}

fn main() -> i64 {
    let mut k: i64 = 0
    unsafe {
        k = aelys_opaque(7)
        aelys_dirty_heap(20)
        aelys_dirty_heap(24)
        aelys_dirty_heap(32)
    }

    let heap_concat: string = "row " + "{k}"
    let heap_char: string = string::from_char('A')
    let heap_sub: string = string::substring_bytes(G, k - 5, k - 1)
    let lit: string = "a literal"
    let b: bool = true
    let boolstr: string = "{b}"
    let f: f64 = 1.5
    let c: char = 'A'
    let from_i64: string = "{k}"
    let from_f64: string = "{f}"
    let from_char: string = "{c}"

    show("P1-concat", heap_concat)
    show("P1-from-char", heap_char)
    show("P1-substring", heap_sub)
    show("P2-literal", lit)
    show("P2-global", G)
    show("P3-bool", boolstr)
    show("P4-i64", from_i64)
    show("P4-f64", from_f64)
    show("P4-char", from_char)
    borrow_once(lit)
    return 0
}
"#;

// a `let` binding of an interpolation takes the non-elided family, which is the one that mallocs
const MALLOC_READER: &str = r#"unsafe extern fn aelys_hdr_refcount(p: &u8) -> i64
unsafe extern fn aelys_hdr_flags(p: &u8) -> i64
unsafe extern fn aelys_hdr_type_id(p: &u8) -> i64
unsafe extern fn aelys_hdr_reserved(p: &u8) -> i64
unsafe extern fn aelys_dirty_heap(bytes: i64)
unsafe extern fn aelys_opaque(x: i64) -> i64

fn show(tag: string, s: string) {
    let b: &[u8] = s.bytes
    unsafe {
        print(tag)
        print("|")
        print(aelys_hdr_refcount(&b[0]))
        print("|")
        print(aelys_hdr_flags(&b[0]))
        print("|")
        print(aelys_hdr_type_id(&b[0]))
        print("|")
        println(aelys_hdr_reserved(&b[0]))
    }
}

fn main() -> i64 {
    let mut k: i64 = 0
    unsafe {
        k = aelys_opaque(7)
        aelys_dirty_heap(37)
        aelys_dirty_heap(80)
        aelys_dirty_heap(20)
    }
    let f: f64 = 1.5
    let c: char = 'A'

    let from_i64: string = "{k}"
    let from_f64: string = "{f}"
    let from_char: string = "{c}"

    show("M1-i64", from_i64)
    show("M2-f64", from_f64)
    show("M3-char", from_char)
    return 0
}
"#;

const DIRTY_FRAME: &str = r#"unsafe extern fn aelys_dirty_stack()

fn emit(i: i64) {
    println("{i}")
}

fn main() -> i64 {
    unsafe { aelys_dirty_stack() }
    emit(42)
    return 0
}
"#;

fn expected_rows() -> String {
    let mut out = String::new();
    for (tag, rc) in [
        ("P1-concat", 1),
        ("P1-from-char", 1),
        ("P1-substring", 1),
        ("P2-literal", PINNED),
        ("P2-global", PINNED),
        ("P3-bool", PINNED),
        ("P4-i64", 1),
        ("P4-f64", 1),
        ("P4-char", 1),
        ("P5-borrow", PINNED),
    ] {
        out.push_str(&format!("{tag}|{rc}|{NO_TRACE}|0|0\n"));
    }
    out
}

// a missing toolchain must redden the row, never skip it: this suite links a real archive
fn tool(name: &str) -> String {
    let found = Command::new(name).arg("--version").output();
    assert!(
        found.map(|out| out.status.success()).unwrap_or(false),
        "the header reader is built with `{name}`, so its absence is a failure and not a skip"
    );
    name.to_string()
}

fn probe_archive() -> TempDir {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("probe.c"), PROBE_C).expect("write probe");
    fs::create_dir_all(dir.path().join("lib")).expect("lib dir");
    let clang = Command::new(tool("clang"))
        .args(["-fPIC", "-c", "-o", "probe.o", "probe.c"])
        .current_dir(dir.path())
        .output()
        .expect("run clang");
    assert!(clang.status.success(), "clang failed: {clang:?}");
    let ar = Command::new(tool("ar"))
        .args(["rcs", "lib/libaelyshdrprobe.a", "probe.o"])
        .current_dir(dir.path())
        .output()
        .expect("run ar");
    assert!(ar.status.success(), "ar failed: {ar:?}");
    dir
}

fn link_probe(lib: &Path) -> LinkRequirement {
    LinkRequirement {
        search_paths: vec![lib.join("lib")],
        libraries: vec!["aelyshdrprobe".to_string()],
    }
}

fn build_linked(dir: &Path, lib: &Path, stem: &str, source: &str, opt: OptimizationLevel) -> PathBuf {
    let path = dir.join(format!("{stem}.aelys"));
    fs::write(&path, source).expect("write source");
    compile_file_with_llvm_linked(&path, opt, false, RuntimeVariant::Rc, &link_probe(lib))
        .unwrap_or_else(|err| panic!("{stem}: the probe program must compile and link: {err}"));
    common::exe_path_for(&path)
}

fn run_reader(dir: &Path, lib: &Path, level: &str, opt: OptimizationLevel) -> String {
    let exe = build_linked(dir, lib, &format!("reader{}", level.trim_start_matches("-O")), READER, opt);
    common::note_leg();
    let out = Command::new(&exe).output().expect("run the header reader");
    assert!(
        out.status.success(),
        "{level}: the header reader must exit 0, got {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_nine_headered_string_provenances_answer_a_valid_rc_header() {
    common::warm_core_archive();
    let _pin = common::pin_legs(
        "the_nine_headered_string_provenances_answer_a_valid_rc_header",
        4,
    );
    let lib = probe_archive();
    let dir = tempdir().expect("tempdir");
    let want = expected_rows();
    for (level, opt) in LEVELS {
        let got = run_reader(dir.path(), lib.path(), level, opt);
        assert_eq!(
            got, want,
            "{level}: all nine string provenances are headered, and each must answer a valid rc \
             header at ptr-16, reserved bytes included; the three that used to malloc a bare \
             buffer are measured again, with their counters, by \
             the_three_former_raw_malloc_string_producers_are_headered_and_managed. the empty \
             string is the one provenance no row here can reach: an extern refuses a `string` \
             parameter with E0615 and `&b[0]` on a zero-length byte view panics with `index out \
             of bounds` before it reads anything, so its header is argued from the \
             `bytes > 0 ? bytes : 1` floor in aelys_str_alloc and is never measured"
        );
    }
}

fn is_valid_header(refcount: u64, flags: u64, type_id: u64) -> bool {
    (refcount == 1 || refcount == u64::from(PINNED)) && flags == u64::from(NO_TRACE) && type_id == 0
}

#[test]
fn the_three_former_raw_malloc_string_producers_are_headered_and_managed() {
    common::warm_core_archive();
    let _pin = common::pin_legs(
        "the_three_former_raw_malloc_string_producers_are_headered_and_managed",
        4,
    );
    let lib = probe_archive();
    let dir = tempdir().expect("tempdir");
    for (level, opt) in LEVELS {
        let exe = build_linked(
            dir.path(),
            lib.path(),
            &format!("malloc{}", level.trim_start_matches("-O")),
            MALLOC_READER,
            opt,
        );
        common::note_leg();
        let out = Command::new(&exe)
            .env("AELYS_RC_STATS", "1")
            .output()
            .expect("run the malloc reader");
        assert!(
            out.status.success(),
            "{level}: the malloc reader must exit 0, got {:?}",
            out.status
        );
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let rows: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(
            rows.len(),
            3,
            "{level}: the three raw malloc producers must each print one row\n{stdout}"
        );
        for row in &rows {
            let cells: Vec<&str> = row.split('|').collect();
            assert_eq!(cells.len(), 5, "{level}: malformed row `{row}`");
            let n = |i: usize| -> u64 {
                cells[i]
                    .parse()
                    .unwrap_or_else(|_| panic!("{level}: `{row}` field {i} is not a number"))
            };
            assert!(
                is_valid_header(n(1), n(2), n(3)),
                "{level}: `{}` answers no valid rc header at ptr-16. to_string_i64, to_string_f64 \
                 and to_string_char allocate through aelys_str_alloc now, which is what widens \
                 the title property from six provenances to nine and keeps __aelys_rc_release \
                 from writing into a glibc chunk word\n{stdout}",
                cells[0]
            );
            assert_eq!(
                n(4),
                0,
                "{level}: `{}` leaves residue in the seven bytes no producer reads; the caller \
                 dirties the heap first, so an unwritten byte comes back as 0xFF\n{stdout}",
                cells[0]
            );
        }
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let rc_allocs = counter(&stderr, "[rc] allocs=");
        let raw_allocs = counter(&stderr, "[raw] allocs=");
        assert!(
            rc_allocs >= 3,
            "{level}: the three producers must be counted by the managed allocator, got \
             {rc_allocs}\n{stderr}"
        );
        assert_eq!(
            raw_allocs, 0,
            "{level}: nothing in this program may reach the raw allocator any more; a non-zero \
             count means a string producer still mallocs a bare buffer\n{stderr}"
        );
    }
}

fn counter(stats: &str, key: &str) -> u64 {
    let line = stats
        .lines()
        .find(|l| l.starts_with(key))
        .unwrap_or_else(|| panic!("AELYS_RC_STATS printed no `{key}` line:\n{stats}"));
    line[key.len()..]
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("`{key}` is not a number: {line}"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn c_define(source: &str, name: &str) -> u32 {
    let line = source
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("#define {name} ")))
        .unwrap_or_else(|| panic!("core/src/aelys_core_common.c no longer defines {name}"));
    line.rsplit(' ')
        .next()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("{name} is no longer a plain integer: {line}"))
}

fn ir_for(source: &str, opt: OptimizationLevel) -> String {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("module.aelys");
    fs::write(&path, source).expect("write source");
    compile_file_with_llvm(&path, opt, true).expect("the llvm backend must accept this program");
    fs::read_to_string(path.with_extension("ll")).expect("emitted ir must be readable")
}

// gdb reads the shipped artifact, so nothing here depends on ir text the linker never saw
fn header_at_callee_entry(exe: &Path, symbol: &str) -> [u8; 16] {
    let out = Command::new(tool("gdb"))
        .args([
            "-batch",
            "-ex",
            &format!("break {symbol}"),
            "-ex",
            "run",
            "-ex",
            "x/16xb (char*)$rdi-16",
        ])
        .arg(exe)
        .output()
        .expect("run gdb");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("Breakpoint 1,"),
        "gdb never stopped in {symbol}, so nothing was measured:\n{text}"
    );
    let bytes: Vec<u8> = text
        .split_whitespace()
        .filter_map(|tok| {
            let hex = tok.strip_prefix("0x")?;
            (hex.len() == 2)
                .then(|| u8::from_str_radix(hex, 16).ok())
                .flatten()
        })
        .collect();
    assert!(
        bytes.len() >= 16,
        "gdb printed fewer than sixteen bytes at $rdi-16:\n{text}"
    );
    let mut header = [0u8; 16];
    header.copy_from_slice(&bytes[bytes.len() - 16..]);
    header
}

#[test]
fn the_interpolation_frame_slot_is_headered_and_both_sides_still_agree_on_its_size() {
    common::warm_core_archive();
    let _pin = common::pin_legs(
        "the_interpolation_frame_slot_is_headered_and_both_sides_still_agree_on_its_size",
        4,
    );
    let core = fs::read_to_string(repo_root().join("core/src/aelys_core_common.c"))
        .expect("the runtime source must be readable");
    let f64_buf = c_define(&core, "AELYS_F64_STR_BUF");
    let i64_buf = c_define(&core, "AELYS_I64_STR_BUF");
    let char_buf = c_define(&core, "AELYS_CHAR_STR_BUF");
    assert!(
        i64_buf <= f64_buf && char_buf <= f64_buf,
        "the widest `_into` callee writes {f64_buf} bytes, the others {i64_buf} and {char_buf}, \
         and codegen sizes one slot for all three"
    );
    let slot = format!("{{ i32, i8, i8, i8, i8, i32, i32, [{f64_buf} x i8] }}");

    let lib = probe_archive();
    let dir = tempdir().expect("tempdir");
    for (level, opt) in LEVELS {
        let ir = ir_for("fn main() -> i64 {\n    let i: i64 = 42\n    println(\"{i}\")\n    return 0\n}\n", opt);
        assert!(
            ir.contains(&format!("alloca {slot}, align 16")),
            "{level}: the frame slot must reserve the rc header ahead of {f64_buf} bytes, aligned \
             like every other header\n{ir}"
        );
        assert!(
            ir.contains(&format!("{slot}, ptr %interp_buf, i64 0, i32 7"))
                || ir.contains(&format!("{slot}, ptr %interp_buf, i32 0, i32 7")),
            "{level}: the callee must be handed the data pointer, not the header\n{ir}"
        );
        assert!(
            ir.contains("@__aelys_to_string_i64_into(ptr"),
            "{level}: this program must still take the elided `_into` path\n{ir}"
        );

        let exe = build_linked(
            dir.path(),
            lib.path(),
            &format!("frame{}", level.trim_start_matches("-O")),
            DIRTY_FRAME,
            opt,
        );
        common::note_leg();
        let header = header_at_callee_entry(&exe, "__aelys_to_string_i64_into");
        assert_eq!(
            header,
            [0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            "{level}: the frame slot handed to the callee must carry a pinned, untraced header \
             over all sixteen bytes; the caller dirties the stack with 0xFF first, so a byte that \
             is not written here is read back as residue"
        );
    }
}

#[test]
fn a_string_literal_is_emitted_behind_a_pinned_header() {
    for (level, opt) in LEVELS {
        let ir = ir_for("fn main() -> i64 {\n    println(\"hello world\")\n    return 0\n}\n", opt);
        assert!(
            ir.contains("{ i32, i8, i8, i8, i8, i32, i32, [12 x i8] } { i32 -1, i8 2, i8 0, i8 0, i8 0, i32 0, i32 0, [12 x i8] c\"hello world\\00\" }, align 16"),
            "{level}: a literal must be emitted behind a pinned, untraced rc header, aligned so \
             the header word is never split across a cache line the loader placed it on\n{ir}"
        );
        assert!(
            ir.contains("i32 0, i32 7, i32 0)") || ir.contains("i64 0, i32 7, i64 0)"),
            "{level}: the program must hold the data pointer, never the header\n{ir}"
        );
    }
}

#[test]
fn the_interior_view_producer_is_still_unreachable_from_the_compiler() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("index.aelys");
    fs::write(
        &path,
        "fn main() -> i64 {\n    let s: string = \"abc\"\n    let mut i: i64 = 0\n    i = i + 1\n    let c: char = s[i]\n    return 0\n}\n",
    )
    .expect("write source");
    let err = compile_file_with_llvm(&path, OptimizationLevel::None, false)
        .expect_err("indexing a string must be refused")
        .to_string();
    assert!(
        err.contains("E0304"),
        "the only surface that would need an interior view is `s[i]`, and the refusal that closes \
         it is E0304; codegen keeps the same refusal as a backstop, asserted by \
         llvm_air_index_tests::string_index_below_sema_is_refused_by_codegen:\n{err}"
    );

    let root = repo_root();
    let mut found = Vec::new();
    for member in [
        "aelys", "core", "common", "syntax", "frontend", "sema", "opt", "air", "codegen", "driver",
        "cli",
    ] {
        for entry in walkdir(&root.join(member).join("src")) {
            let text = fs::read_to_string(&entry).unwrap_or_default();
            if text.contains("\"__aelys_str_char_at\"") {
                found.push(entry.display().to_string());
            }
        }
    }
    assert!(
        found.is_empty(),
        "second line only, and a weaker one: this grep reads the eleven crate sources for the \
         quoted symbol and would miss a name built by format!. what closes the class is the \
         refusal asserted above. __aelys_str_char_at hands back a pointer into another buffer; \
         declaring it would put an interior view behind a header that is not its own: {found:?}"
    );
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walkdir(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}
