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

void aelys_hdr_set_refcount(unsigned char *p, long long v) {
    uint32_t x = (uint32_t)v;
    memcpy(p - 16, &x, 4);
}

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

const NOGC_READERS: [&str; 2] = ["__aelys_str_char_count", "__aelys_str_decode_at"];

const NOGC_COUNTED_SITES: &[(&str, usize, usize)] = &[("std.sort.strings", 3, 3)];

fn std_nogc_functions() -> (Vec<String>, Vec<String>) {
    std_nogc_functions_opening('(')
}

fn std_nogc_functions_opening(open: char) -> (Vec<String>, Vec<String>) {
    let mut names = Vec::new();
    let mut modules = Vec::new();
    let mut files: Vec<PathBuf> = fs::read_dir(repo_root().join("std"))
        .expect("read std")
        .map(|e| e.expect("std entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "aelys"))
        .collect();
    files.sort();
    for path in files {
        let module = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("module name")
            .to_string();
        let text = fs::read_to_string(&path).expect("read a std module");
        let before = names.len();
        for line in text.lines() {
            let t = line.trim_start();
            let t = t.strip_prefix("pub ").unwrap_or(t);
            let Some(rest) = t.strip_prefix("nogc fn ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if rest[name.len()..].starts_with(open) {
                names.push(format!("std.{module}.{name}"));
            }
        }
        if names.len() > before {
            modules.push(module);
        }
    }
    (names, modules)
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

fn named_callees(function: &aelys_air::AirFunction) -> Vec<String> {
    use aelys_air::{AirStmtKind, AirTerminator, Callee, Rvalue};
    let mut out = Vec::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            let func = match &stmt.kind {
                AirStmtKind::CallVoid { func, .. } => Some(func),
                AirStmtKind::Assign {
                    rvalue: Rvalue::Call { func, .. },
                    ..
                } => Some(func),
                _ => None,
            };
            if let Some(Callee::Named(name)) = func {
                out.push(name.clone());
            }
        }
        if let AirTerminator::Invoke {
            func: Callee::Named(name),
            ..
        } = &block.terminator
        {
            out.push(name.clone());
        }
    }
    out
}

fn allocating_statements(function: &aelys_air::AirFunction) -> Vec<String> {
    use aelys_air::{AirStmtKind, AirType, BinOp, Place, Rvalue};
    let is_str = |id: &aelys_air::LocalId| {
        function
            .locals
            .iter()
            .any(|l| l.id == *id && l.ty == AirType::Str)
    };
    let mut out = Vec::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::RcAlloc { .. } => out.push("rc_alloc".to_string()),
                AirStmtKind::GcAlloc { .. } => out.push("gc_alloc".to_string()),
                AirStmtKind::Alloc { .. } => out.push("alloc".to_string()),
                AirStmtKind::Assign {
                    place: Place::Local(dst),
                    rvalue: Rvalue::BinaryOp(BinOp::Add, _, _),
                } if is_str(dst) => out.push("string concatenation".to_string()),
                AirStmtKind::Assign {
                    rvalue: Rvalue::ClosureCreate { .. },
                    ..
                } => out.push("closure creation".to_string()),
                _ => {}
            }
        }
    }
    out
}

#[test]
fn a36_w1_a_std_nogc_function_calls_only_string_counts_readers_externs_and_other_nogc_functions() {
    let (nogc, modules) = std_nogc_functions();
    assert_eq!(
        nogc.len(),
        39,
        "the census must see every non generic `nogc fn` of std, found {nogc:?}"
    );
    let dir = tempdir().expect("tempdir");
    copy_tree(&repo_root().join("std"), &dir.path().join("std"));
    let root = dir.path().join("census.aelys");
    let imports: String = modules.iter().map(|m| format!("needs std.{m}\n")).collect();
    fs::write(
        &root,
        format!("{imports}\nfn main() -> i64 {{\n    return 0\n}}\n"),
    )
    .expect("write census root");
    let air = aelys_driver::lower_file_to_air(&root, OptimizationLevel::None)
        .unwrap_or_else(|err| panic!("the census root must lower: {err}"));
    let externs: std::collections::HashSet<&str> = air
        .functions
        .iter()
        .filter(|f| f.is_extern)
        .map(|f| f.name.as_str())
        .collect();
    let repeat = air
        .functions
        .iter()
        .find(|f| f.name == "std.str.repeat")
        .expect("std.str.repeat must reach the AIR");
    assert!(
        allocating_statements(repeat).contains(&"string concatenation".to_string()),
        "A36: the allocation detector must see the concatenation of std.str.repeat, or it is blind"
    );
    let mut seen = 0;
    for function in &air.functions {
        if !nogc.contains(&function.name) {
            continue;
        }
        seen += 1;
        let callees = named_callees(function);
        for callee in &callees {
            let allowed = callee == "__aelys_str_retain"
                || callee == "__aelys_str_release"
                || NOGC_READERS.contains(&callee.as_str())
                || externs.contains(callee.as_str())
                || nogc.contains(callee);
            assert!(
                allowed,
                "A36: `{}` is nogc and calls `{callee}`, which is neither a string count, a \
                 reader, an extern nor another nogc function",
                function.name
            );
        }
        let allocating = allocating_statements(function);
        assert!(
            allocating.is_empty(),
            "A36: `{}` is nogc and its AIR allocates: {allocating:?}",
            function.name
        );
        let retains = callees
            .iter()
            .filter(|c| *c == "__aelys_str_retain")
            .count();
        let releases = callees
            .iter()
            .filter(|c| *c == "__aelys_str_release")
            .count();
        let pinned = NOGC_COUNTED_SITES
            .iter()
            .find(|(name, _, _)| *name == function.name)
            .map_or((0, 0), |(_, r, l)| (*r, *l));
        assert_eq!(
            (retains, releases),
            pinned,
            "A36: the string retain and release sites of `{}` moved",
            function.name
        );
    }
    assert_eq!(
        seen,
        nogc.len(),
        "every std nogc function must reach the AIR"
    );
}

const A36_W1B_ROOT: &str = r#"
needs std.slice
needs std.sort

fn main() -> i64 {
    let mut a: [i64;4] = [3, 1, 2, 5]
    let b: [i64;4] = [1, 2, 3, 5]
    let f = slice.first(a[..])
    let l = slice.last(a[..])
    let mx = slice.max(a[..])
    let mn = slice.min(a[..])
    let i = slice.index_of(a[..], 2)
    let fd = slice.find(a[..], 2)
    let c = slice.contains(a[..], 2)
    let e = slice.eq(a[..], b[..])
    let s = slice.is_sorted(b[..])
    slice.swap(a[..], 0, 1)
    slice.reverse(a[..])
    sort.insertion_sort(a[..])
    let k = sort.binary_search(a[..], 3)
    slice.fill(a[..], 7)
    return 0
}
"#;

#[test]
fn a36_w1b_a_generic_std_nogc_function_keeps_no_count_once_instantiated() {
    let (generic, _) = std_nogc_functions_opening('<');
    assert_eq!(
        generic.len(),
        14,
        "the census must see every generic `nogc fn` of std, found {generic:?}"
    );
    let dir = tempdir().expect("tempdir");
    copy_tree(&repo_root().join("std"), &dir.path().join("std"));
    let root = dir.path().join("census.aelys");
    fs::write(&root, A36_W1B_ROOT).expect("write census root");
    let air = aelys_driver::lower_file_to_air(&root, OptimizationLevel::None)
        .unwrap_or_else(|err| panic!("the census root must lower: {err}"));
    for name in &generic {
        let prefix = format!("__mono_{name}$");
        let instances: Vec<&aelys_air::AirFunction> = air
            .functions
            .iter()
            .filter(|f| f.name.starts_with(&prefix))
            .collect();
        assert!(
            !instances.is_empty(),
            "A36: the census root must instantiate `{name}`"
        );
        for instance in instances {
            let counts: Vec<String> = named_callees(instance)
                .into_iter()
                .filter(|c| {
                    matches!(
                        c.as_str(),
                        "__aelys_str_retain"
                            | "__aelys_str_release"
                            | "__aelys_dup"
                            | "__aelys_drop"
                    )
                })
                .collect();
            assert!(
                counts.is_empty(),
                "A36: `{}` is nogc and keeps counts once instantiated: {counts:?}",
                instance.name
            );
            let allocating = allocating_statements(instance);
            assert!(
                allocating.is_empty(),
                "A36: `{}` is nogc and its instance allocates: {allocating:?}",
                instance.name
            );
        }
    }
}

const A36_W2_PROBE: &str = r#"
#include <stdio.h>
#include <string.h>

typedef struct {
    const char *ptr;
    long long len;
} AelysString;

extern void *__aelys_alloc(long long);
extern void __aelys_str_retain(void *strptr);
extern void __aelys_str_release(void *strptr);

const unsigned __aelys_rc_type_table[8] = {0};

long long __aelys_user_main(void) {
    char *base = (char *)__aelys_alloc(24);
    memset(base, 0, 24);
    *(unsigned *)base = 1u;
    *(unsigned char *)(base + 4) = 2;
    AelysString s = {base + 16, 3};
    __aelys_str_retain(&s);
    __aelys_str_release(&s);
    printf("%d\n", *(unsigned char *)(base + 4) & 1);
    return 0;
}
"#;

fn candidate_bit_after_a_string_retain_and_release(
    dir: &Path,
    stem: &str,
    cycles_source: &str,
    alloc: &str,
) -> String {
    let src = repo_root().join("core").join("src");
    let rc = dir.join(format!("{stem}_rc.c"));
    fs::write(&rc, cycles_source).expect("write the runtime under test");
    let probe = dir.join(format!("{stem}_probe.c"));
    fs::write(&probe, A36_W2_PROBE).expect("write the probe");
    let exe = dir.join(format!("{stem}_bin"));
    let out = Command::new(tool("clang"))
        .arg("-I")
        .arg(&src)
        .arg(src.join("aelys_alloc_immix.c"))
        .arg(src.join("aelys_core_common.c"))
        .arg(&rc)
        .arg(&probe)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run clang");
    assert!(out.status.success(), "the W2 probe must build: {out:?}");
    let mut cmd = Command::new(&exe);
    if alloc == "malloc" {
        cmd.env("AELYS_ALLOC", "malloc");
    }
    let run = cmd.output().expect("run the W2 probe");
    assert!(run.status.success(), "the W2 probe must exit 0: {run:?}");
    String::from_utf8_lossy(&run.stdout).trim().to_string()
}

#[test]
fn a36_w2_a_string_retained_then_released_under_the_cycle_runtime_never_becomes_a_candidate() {
    let real = fs::read_to_string(repo_root().join("core/src/aelys_rc_cycles.c"))
        .expect("read the cycle runtime");
    let guard = "if ((*flags & AELYS_FLAG_NO_TRACE) != 0) {\n        return;\n    }";
    assert!(
        real.contains(guard),
        "A36 W2: the NO_TRACE early return of __aelys_rc_release moved, so the mutant below no \
         longer removes it"
    );
    let mutant = real.replacen(guard, "", 1);
    let dir = tempdir().expect("tempdir");
    for alloc in ["immix", "malloc"] {
        assert_eq!(
            candidate_bit_after_a_string_retain_and_release(dir.path(), "real", &real, alloc),
            "0",
            "A36 W2 under {alloc}: a string became a cycle candidate through \
             __aelys_str_retain and __aelys_str_release"
        );
        assert_eq!(
            candidate_bit_after_a_string_retain_and_release(dir.path(), "mutant", &mutant, alloc),
            "1",
            "A36 W2 under {alloc}: without its NO_TRACE return the runtime must mark the \
             candidate, or this probe cannot see the rule it guards"
        );
    }
}

const A3_TWO_OWNED_STRINGS: &str = r#"
fn main() -> i64 {
    let a: string = "ab" + "{1}"
    let b: string = "cd" + "{2}"
    println(a)
    println(b)
    return 0
}
"#;

const A3_ONE_BIG_STRING: &str = r#"
fn big() -> string {
    let mut s: string = "x"
    let mut i: i64 = 0
    while i < 18 {
        s = s + s
        i = i + 1
    }
    return s
}
fn main() -> i64 {
    let a: string = big()
    let b: string = "cd" + "{2}"
    println(a.len)
    println(b)
    return 0
}
"#;

const A3_RELEASE_FREED: &str =
    "__aelys_rc_release: the object was already freed, a release with no matching retain";
const A3_RELEASE_UNDERFLOW: &str =
    "__aelys_rc_release: refcount already zero, a release with no matching retain";
const A3_RETAIN_FREED: &str =
    "__aelys_rc_retain: the object was already freed, a retain after its last release";

#[derive(Clone, Copy, PartialEq)]
enum A3Shape {
    ReleaseTwice,
    ReleaseAfterAnotherFree,
    RetainAfterRelease,
    RetainAfterAnotherFree,
}

fn a3_count_call(name: &str, addr: aelys_air::LocalId) -> aelys_air::AirStmt {
    aelys_air::AirStmt {
        kind: aelys_air::AirStmtKind::CallVoid {
            func: aelys_air::Callee::Named(name.to_string()),
            args: vec![aelys_air::Operand::Copy(addr)],
        },
        span: None,
    }
}

fn a3_break_the_counts(air: &mut aelys_air::AirProgram, shape: A3Shape) {
    use aelys_air::{AirStmtKind, Callee, Operand};
    let main = air
        .functions
        .iter_mut()
        .find(|f| f.name == "main")
        .expect("main must reach the AIR");
    let named: Vec<aelys_air::LocalId> = main
        .locals
        .iter()
        .filter(|l| matches!(l.name.as_deref(), Some("a" | "b")))
        .map(|l| l.id)
        .collect();
    let mut addr_of: Vec<(aelys_air::LocalId, aelys_air::LocalId)> = Vec::new();
    let mut releases: Vec<(usize, usize, aelys_air::LocalId)> = Vec::new();
    for (b, block) in main.blocks.iter().enumerate() {
        for (i, stmt) in block.stmts.iter().enumerate() {
            match &stmt.kind {
                AirStmtKind::Assign {
                    place: aelys_air::Place::Local(addr),
                    rvalue: aelys_air::Rvalue::AddressOf(aelys_air::Place::Local(base)),
                } => addr_of.push((*addr, *base)),
                AirStmtKind::CallVoid {
                    func: Callee::Named(name),
                    args,
                } if name == "__aelys_str_release" => {
                    if let [Operand::Copy(addr) | Operand::Move(addr)] = args.as_slice()
                        && addr_of
                            .iter()
                            .any(|(a, base)| a == addr && named.contains(base))
                    {
                        releases.push((b, i, *addr));
                    }
                }
                _ => {}
            }
        }
    }
    assert!(
        releases.len() == 2 && releases[0].0 == releases[1].0 && releases[1].1 > releases[0].1,
        "A3: the bindings `a` and `b` must be released one after the other in one block, found \
         {releases:?}"
    );
    let (block, first_at, first) = releases[0];
    let (_, second_at, _) = releases[1];
    let (at, stmt) = match shape {
        A3Shape::ReleaseTwice => (first_at + 1, a3_count_call("__aelys_str_release", first)),
        A3Shape::ReleaseAfterAnotherFree => {
            (second_at + 1, a3_count_call("__aelys_str_release", first))
        }
        A3Shape::RetainAfterRelease => (first_at + 1, a3_count_call("__aelys_str_retain", first)),
        A3Shape::RetainAfterAnotherFree => {
            (second_at + 1, a3_count_call("__aelys_str_retain", first))
        }
    };
    main.blocks[block].stmts.insert(at, stmt);
}

fn a3_run(
    dir: &Path,
    stem: &str,
    air: &aelys_air::AirProgram,
    runtime: RuntimeVariant,
    outline: bool,
    alloc: &str,
) -> (bool, String) {
    let path = dir.join(format!("{stem}.aelys"));
    aelys_codegen::set_outline_str_counts(Some(outline));
    let built = aelys_driver::compile_air_program_to_executable(
        &path,
        air,
        OptimizationLevel::None,
        runtime,
    );
    aelys_codegen::set_outline_str_counts(None);
    built.unwrap_or_else(|e| panic!("A3 {stem}: the broken AIR must still compile: {e}"));
    let exe = common::exe_path_for(&path);
    let mut cmd = Command::new(&exe);
    if alloc == "malloc" {
        cmd.env("AELYS_ALLOC", "malloc");
    }
    let out = cmd.output().expect("run the A3 program");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn a3_broken_legs(dir: &Path, stem: &str, source: &str, shapes: &[(A3Shape, &str)]) -> usize {
    let src = dir.join(format!("{stem}.aelys"));
    fs::write(&src, source).expect("write the A3 program");
    let clean = aelys_driver::lower_file_to_air(&src, OptimizationLevel::None)
        .expect("the A3 program must lower");
    let runtimes = [
        (RuntimeVariant::Leak, "leak"),
        (RuntimeVariant::Rc, "rc"),
        (RuntimeVariant::RcCycles, "cycles"),
    ];
    let mut legs = 0;
    for &(shape, shape_tag) in shapes {
        let mut air = clean.clone();
        a3_break_the_counts(&mut air, shape);
        for (runtime, rt_tag) in runtimes {
            for outline in [false, true] {
                for alloc in ["immix", "malloc"] {
                    let leg = format!(
                        "{stem}_{shape_tag}_{rt_tag}_{}_{alloc}",
                        if outline { "outline" } else { "inline" }
                    );
                    let (ok, stderr) = a3_run(dir, &leg, &air, runtime, outline, alloc);
                    legs += 1;
                    let stale = matches!(
                        shape,
                        A3Shape::ReleaseAfterAnotherFree | A3Shape::RetainAfterAnotherFree
                    );
                    let retain = matches!(
                        shape,
                        A3Shape::RetainAfterRelease | A3Shape::RetainAfterAnotherFree
                    );
                    if runtime == RuntimeVariant::Leak {
                        if retain {
                            assert!(
                                ok,
                                "A3 {leg}: the leak runtime never frees, so a late retain is \
                                 sound; stderr:\n{stderr}"
                            );
                        } else {
                            assert!(
                                !ok && stderr.contains(A3_RELEASE_UNDERFLOW),
                                "A3 {leg}: a second release must abort on the underflow; \
                                 stderr:\n{stderr}"
                            );
                        }
                        continue;
                    }
                    // libc owns a freed chunk, so once the tombstone moves on nothing is left to read
                    if stale && alloc == "malloc" {
                        continue;
                    }
                    let want = if retain {
                        A3_RETAIN_FREED
                    } else {
                        A3_RELEASE_FREED
                    };
                    assert!(
                        !ok && stderr.contains(want),
                        "A3 {leg}: the broken count must abort with `{want}`; stderr:\n{stderr}"
                    );
                }
            }
        }
    }
    legs
}

#[test]
fn a3_a_broken_count_in_generated_code_is_caught_inline_and_out_of_line() {
    let dir = tempdir().expect("tempdir");
    let small = a3_broken_legs(
        dir.path(),
        "small",
        A3_TWO_OWNED_STRINGS,
        &[
            (A3Shape::ReleaseTwice, "twice"),
            (A3Shape::ReleaseAfterAnotherFree, "stale"),
            (A3Shape::RetainAfterRelease, "retain"),
            (A3Shape::RetainAfterAnotherFree, "retain_stale"),
        ],
    );
    // a mapped string goes back to the kernel on free, so reading its header first faults
    let big = a3_broken_legs(
        dir.path(),
        "big",
        A3_ONE_BIG_STRING,
        &[(A3Shape::ReleaseTwice, "twice")],
    );
    assert_eq!((small, big), (48, 12), "A3: every leg must run");
}

const A3_ONE_BELOW_DEAD: &str = r#"unsafe extern fn aelys_hdr_refcount(p: &u8) -> i64
unsafe extern fn aelys_hdr_set_refcount(p: &u8, v: i64)
unsafe extern fn aelys_opaque(x: i64) -> i64

fn count(s: string) -> i64 {
    let b: &[u8] = s.bytes
    let mut n: i64 = 0
    unsafe {
        n = aelys_hdr_refcount(&b[0])
    }
    return n
}

fn main() -> i64 {
    let mut k: i64 = 0
    unsafe {
        k = aelys_opaque(7)
    }
    let s: string = "row " + "{k}"
    let b: &[u8] = s.bytes
    unsafe {
        aelys_hdr_set_refcount(&b[0], 2920406700)
    }
    let t: string = s
    println(count(t))
    return 0
}
"#;

#[test]
fn a3_a_count_one_below_dead_saturates_instead_of_reading_as_freed() {
    let lib = probe_archive();
    let dir = tempdir().expect("tempdir");
    let runtimes = [
        (RuntimeVariant::Leak, "leak"),
        (RuntimeVariant::Rc, "rc"),
        (RuntimeVariant::RcCycles, "cycles"),
    ];
    let levels = [
        ("o0", OptimizationLevel::None),
        ("o2", OptimizationLevel::Standard),
    ];
    let mut legs = 0;
    for (runtime, rt_tag) in runtimes {
        for (level_tag, opt) in levels {
            for outline in [false, true] {
                let mode = if outline { "outline" } else { "inline" };
                let stem = format!("edge_{rt_tag}_{level_tag}_{mode}");
                let path = dir.path().join(format!("{stem}.aelys"));
                fs::write(&path, A3_ONE_BELOW_DEAD).expect("write the saturation probe");
                aelys_codegen::set_outline_str_counts(Some(outline));
                let built = compile_file_with_llvm_linked(
                    &path,
                    opt,
                    false,
                    runtime,
                    &link_probe(lib.path()),
                );
                aelys_codegen::set_outline_str_counts(None);
                built.unwrap_or_else(|e| panic!("A3 {stem}: the probe must compile and link: {e}"));
                let exe = common::exe_path_for(&path);
                legs += a3_saturation_legs(&stem, &exe);
            }
        }
    }
    assert_eq!(legs, 24, "A3: every saturation leg must run");
}

fn a3_saturation_legs(stem: &str, exe: &Path) -> usize {
    let mut legs = 0;
    for alloc in ["immix", "malloc"] {
        let mut cmd = Command::new(exe);
        if alloc == "malloc" {
            cmd.env("AELYS_ALLOC", "malloc");
        }
        let out = cmd.output().expect("run the saturation probe");
        legs += 1;
        assert!(
            out.status.success(),
            "A3 {stem}/{alloc}: a live count one below DEAD must not read as freed; \
             stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{}\n", u32::MAX),
            "A3 {stem}/{alloc}: the count must saturate to the immortal value"
        );
    }
    legs
}

#[test]
fn a3_string_counts_are_inline_unless_the_asan_knob_asks_for_the_calls() {
    aelys_codegen::set_outline_str_counts(Some(false));
    let inline = ir_for(A3_TWO_OWNED_STRINGS, OptimizationLevel::None);
    aelys_codegen::set_outline_str_counts(Some(true));
    let outline = ir_for(A3_TWO_OWNED_STRINGS, OptimizationLevel::None);
    aelys_codegen::set_outline_str_counts(None);
    assert!(
        inline.contains("str_rc_fast") && !inline.contains("call void @__aelys_str_release"),
        "A3: by default a string count must be emitted inline"
    );
    assert!(
        !outline.contains("str_rc_fast") && outline.contains("call void @__aelys_str_release"),
        "A3: under the asan knob every string count must be a call the runtime makes"
    );
}

#[test]
fn a3_every_asan_tier_that_compiles_aelys_keeps_the_counts_out_of_line() {
    let inline = ir_for(A3_TWO_OWNED_STRINGS, OptimizationLevel::None);
    let outline =
        common::with_outline_str_counts(|| ir_for(A3_TWO_OWNED_STRINGS, OptimizationLevel::None));
    assert!(
        inline.contains("str_rc_fast") && !outline.contains("str_rc_fast"),
        "A3: the asan wrapper must turn the inline string counts into runtime calls"
    );
    let tests = repo_root().join("aelys").join("tests");
    let mut tiers = Vec::new();
    for entry in fs::read_dir(&tests).expect("read aelys/tests") {
        let path = entry.expect("test entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read a test file");
        let starts: Vec<usize> = text
            .match_indices("fn ")
            .map(|(at, _)| at)
            .filter(|at| *at == 0 || text[..*at].ends_with(' ') || text[..*at].ends_with('\n'))
            .collect();
        for (k, start) in starts.iter().enumerate() {
            let body = &text[*start..starts.get(k + 1).copied().unwrap_or(text.len())];
            if body.contains("-fsanitize=address") && body.contains("compile_file_with_llvm") {
                tiers.push(path.clone());
                assert!(
                    body.contains("with_outline_str_counts"),
                    "A3: a function of {} links an Aelys object against an asan core without \
                     `common::with_outline_str_counts`, so the string counts it compiles inline \
                     are invisible to asan",
                    path.display()
                );
            }
        }
    }
    assert!(
        tiers.len() >= 10,
        "A3: the scan found only {} asan tiers that compile Aelys, it no longer looks where \
         they are",
        tiers.len()
    );
}
