use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
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

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

fn parse_cycles(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr
        .lines()
        .find(|l| l.contains("[cycles] collections="))?;
    let rest = line.trim().strip_prefix("[cycles] collections=")?;
    let (c, k) = rest.split_once(" clamped=")?;
    Some((c.trim().parse().ok()?, k.trim().parse().ok()?))
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
    legs: Cell<usize>,
    linker_skips: Cell<usize>,
}

impl Harness {
    fn new() -> Self {
        Harness {
            dir: tempdir().expect("tempdir"),
            legs: Cell::new(0),
            linker_skips: Cell::new(0),
        }
    }

    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&repo_root().join("std"), &dir.join("std"));
        let root = dir.join("root.aelys");
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn cycles_row(&self, id: &str, src: &str, stdout: &str, counted: (i64, i64)) {
        self.run_row(id, src, stdout, counted, None);
    }

    fn collected_row(
        &self,
        id: &str,
        src: &str,
        stdout: &str,
        counted: (i64, i64),
        collections: i64,
    ) {
        self.run_row(id, src, stdout, counted, Some(collections));
    }

    fn exit_and_mid_rows(
        &self,
        id: &str,
        src: &str,
        stdout: &str,
        counted: (i64, i64),
        collections: i64,
    ) {
        let exit = src.replace("    MID\n", "");
        let mid = src.replace("MID", "__aelys_collect()");
        self.collected_row(&format!("{id}-exit"), &exit, stdout, counted, collections);
        self.collected_row(&format!("{id}-mid"), &mid, stdout, counted, collections + 1);
    }

    fn run_row(
        &self,
        id: &str,
        src: &str,
        stdout: &str,
        counted: (i64, i64),
        collections: Option<i64>,
    ) {
        for (tag, opt) in LEVELS {
            for (alloc_name, alloc) in ALLOCATORS {
                let leg = format!("{tag}_{alloc_name}");
                let root = self.stage(id, &leg, src);
                if let Err(err) =
                    compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::RcCycles)
                {
                    if linker_unavailable(&err.to_string()) {
                        self.linker_skips.set(self.linker_skips.get() + 1);
                        assert!(
                            linker_skip_declared(),
                            "{id}: no linker and AELYS_ALLOW_LINKER_SKIP is unset"
                        );
                        return;
                    }
                    panic!("{id} at {leg}: compile\n{err}");
                }
                let mut cmd = Command::new(exe_path_for(&root));
                cmd.env("AELYS_RC_STATS", "1");
                if let Some(a) = alloc {
                    cmd.env("AELYS_ALLOC", a);
                }
                let out = cmd.output().expect("run compiled exe");
                let seen_err = String::from_utf8_lossy(&out.stderr).into_owned();
                self.legs.set(self.legs.get() + 1);
                assert_eq!(exit_code(&out.status), 0, "{id} at {leg}: exit\n{seen_err}");
                assert_eq!(
                    String::from_utf8_lossy(&out.stdout),
                    stdout,
                    "{id} at {leg}: answer"
                );
                let got = parse_stats(&seen_err)
                    .unwrap_or_else(|| panic!("{id} at {leg}: no [rc] stats line\n{seen_err}"));
                assert_eq!(got, counted, "{id} at {leg}: allocs and frees");
                if let Some(collections) = collections {
                    let seen = parse_cycles(&seen_err)
                        .unwrap_or_else(|| panic!("{id} at {leg}: no [cycles] line\n{seen_err}"));
                    assert_eq!(
                        seen,
                        (collections, 0),
                        "{id} at {leg}: collections and clamps"
                    );
                }
            }
        }
    }

    fn assert_legs(&self, expected: usize) {
        if self.linker_skips.get() > 0 && linker_skip_declared() {
            return;
        }
        assert_eq!(self.legs.get(), expected, "legs run");
    }
}

const RELEASE01_1: &str = r#"
struct Leaf { k: i64 }
enum E { A(i32, Rc<Leaf>) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(20, Rc::new(Leaf { k: 300 })) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_1_an_rc_payload_field_after_an_i32_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-1", RELEASE01_1, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_2: &str = r#"
struct Leaf { k: i64 }
enum E { A(u8, Rc<Leaf>) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(20, Rc::new(Leaf { k: 300 })) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_2_an_rc_payload_field_after_a_u8_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-2", RELEASE01_2, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_3: &str = r#"
struct Leaf { k: i64 }
enum E { A(i64, Rc<Leaf>) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(20, Rc::new(Leaf { k: 300 })) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_3_an_rc_payload_field_after_an_i64_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-3", RELEASE01_3, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_4: &str = r#"
struct Leaf { k: i64 }
enum E { A(Rc<Leaf>, i32) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(r, x) => { return Rc::get(r).k + x }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(Rc::new(Leaf { k: 300 }), 20) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_4_an_rc_first_payload_field_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-4", RELEASE01_4, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_5: &str = r#"
struct Leaf { k: i64 }
enum E { A(Rc<Leaf>) }
struct Inner { tag: u8, e: E }
struct N { v: i64, w: i32, inner: Inner }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    let n = Rc::get(a)
    match n.inner.e {
        E::A(r) => { return n.w + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, w: 20, inner: Inner { tag: 7, e: E::A(Rc::new(Leaf { k: 300 })) } })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_5_an_enum_two_struct_levels_deep_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-5", RELEASE01_5, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_6: &str = r#"
struct Leaf { k: i64 }
struct P { k: i32, r: Rc<Leaf> }
enum E { A(P) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(p) => { return p.k + Rc::get(p.r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(P { k: 20, r: Rc::new(Leaf { k: 300 }) }) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_6_a_struct_in_an_enum_payload_carrying_an_rc_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-6", RELEASE01_6, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_7: &str = r#"
struct Leaf { k: i64 }
enum E { A(i32, Rc<Leaf>) }
fn peek(a: Rc<E>) -> i64 {
    let c = a
    return 1
}
fn read(a: Rc<E>) -> i64 {
    match Rc::get(a) {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<E> = Rc::new(E::A(20, Rc::new(Leaf { k: 300 })))
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_7_an_enum_held_directly_in_an_rc_node_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-7", RELEASE01_7, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_8: &str = r#"
struct T { val: i64, parent: Rc<T>, child: Rc<T> }
fn work() -> i64 {
    let root: Rc<T> = Rc::new(T { val: 1, parent: Rc::null(), child: Rc::null() })
    root.child = Rc::new(T { val: 2, parent: root, child: Rc::null() })
    return Rc::get(Rc::get(root).child).val
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_8_a_parent_pointer_pair_is_collected() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-8", RELEASE01_8, "2\n2\n", (4, 4), 0);
    h.assert_legs(8);
}

const RELEASE01_9: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn work() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::new(Node { val: 2, next: Rc::new(Node { val: 3, next: Rc::null() }) }) })
    Rc::get(Rc::get(a).next).next.next = a
    return Rc::get(Rc::get(a).next).val
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_9_an_inline_three_cycle_is_collected() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-9", RELEASE01_9, "2\n2\n", (6, 6), 0);
    h.assert_legs(8);
}

const RELEASE01_10: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn build(n: i64, first: Rc<Node>) -> Rc<Node> {
    if n == 0 { return first }
    return Rc::new(Node { val: n, next: build(n - 1, first) })
}
fn work() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 0, next: Rc::null() })
    a.next = build(5, a)
    return Rc::get(Rc::get(a).next).val
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_10_a_ring_built_by_recursion_is_collected() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-10", RELEASE01_10, "5\n5\n", (12, 12), 0);
    h.assert_legs(8);
}

const RELEASE01_11: &str = r#"
struct DNode { val: i64, next: Rc<DNode>, prev: Rc<DNode> }
fn build(n: i64, prev: Rc<DNode>) -> Rc<DNode> {
    if n == 0 { return Rc::null() }
    let me: Rc<DNode> = Rc::new(DNode { val: n, next: Rc::null(), prev: prev })
    me.next = build(n - 1, me)
    return me
}
fn work() -> i64 {
    let h = build(10, Rc::null())
    return Rc::get(Rc::get(h).next).val
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_11_a_doubly_linked_list_built_by_recursion_is_collected() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-11", RELEASE01_11, "9\n9\n", (20, 20), 0);
    h.assert_legs(8);
}

const RELEASE01_12: &str = r#"
struct N { v: i64, next: Rc<Rc<N>> }
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, next: Rc::null() })
    let b: Rc<N> = Rc::new(N { v: 2, next: Rc::new(a) })
    a.next = Rc::new(b)
    return Rc::get(b).v
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_12_a_cycle_through_an_rc_of_an_rc_is_collected() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-12", RELEASE01_12, "2\n2\n", (8, 8), 0);
    h.assert_legs(8);
}

const RELEASE01_13: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node> }
fn mk(h: &mut H, v: i64) {
    let a: Rc<Node> = Rc::new(Node { val: v, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: a })
    a.next = b
    (*h).r = a
}
fn work() -> i64 {
    let mut h = H { r: Rc::null() }
    mk(&mut h, 7)
    __aelys_collect()
    let v = Rc::get(h.r).val
    h.r = Rc::null()
    return v
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_13_a_survivor_dying_through_one_member_is_collected() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-13", RELEASE01_13, "7\n7\n", (4, 4), 2);
    h.assert_legs(8);
}

const RELEASE01_14: &str = r#"
struct N { k: i64, a: Rc<N>, b: Rc<N> }
fn work() -> i64 {
    let c: Rc<N> = Rc::new(N { k: 5, a: Rc::null(), b: Rc::null() })
    let mut x: Rc<N> = Rc::new(N { k: 1, a: c, b: Rc::null() })
    let mut y: Rc<N> = Rc::new(N { k: 2, a: c, b: x })
    x.b = y
    x = Rc::null()
    y = Rc::null()
    __aelys_collect()
    return Rc::get(c).k
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_14_a_live_child_of_two_garbage_parents_survives_their_collection() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-14", RELEASE01_14, "5\n5\n", (6, 6), 2);
    h.assert_legs(8);
}

const RELEASE01_15: &str = r#"
struct DNode { val: i64, next: Rc<DNode>, prev: Rc<DNode> }
struct DList { head: Rc<DNode>, len: i64 }
fn push_front(l: &mut DList, v: i64) {
    (*l).head = Rc::new(DNode { val: v, next: (*l).head, prev: Rc::null() })
    if !Rc::is_null(Rc::get((*l).head).next) {
        Rc::get(Rc::get((*l).head).next).prev = (*l).head
    }
    (*l).len = (*l).len + 1
}
fn work() -> i64 {
    let mut l = DList { head: Rc::null(), len: 0 }
    let mut i = 0
    while i < 1000000 {
        push_front(&mut l, i)
        i = i + 1
    }
    return l.len
}
fn main() -> i64 {
    let t = work()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_15_a_dead_doubly_linked_list_of_a_million_nodes_is_collected_at_exit() {
    let h = Harness::new();
    h.collected_row(
        "release01-15",
        RELEASE01_15,
        "1000000\n",
        (1_000_000, 1_000_000),
        0,
    );
    h.assert_legs(4);
}

const RELEASE01_16: &str = r#"
struct N { s: string, next: Rc<N> }
struct H { r: Rc<N> }
fn mk(h: &mut H, p: string) {
    let a: Rc<N> = Rc::new(N { s: p + "cdefgh", next: Rc::null() })
    a.next = a
    (*h).r = a
}
fn take(s: string, k: i64) -> i64 {
    return s.len + s.bytes[0] + k
}
fn main() -> i64 {
    let mut h = H { r: Rc::null() }
    mk(&mut h, "ab")
    println(take(Rc::get(h.r).s, { h.r = Rc::null(); __aelys_collect(); 1 }))
    return 0
}
"#;

#[test]
fn release01_16_an_explicit_collect_in_a_block_argument_after_a_borrowed_read_of_a_cycle_member_stays_balanced() {
    let h = Harness::new();
    h.collected_row("release01-16", RELEASE01_16, "106\n", (2, 2), 1);
    h.assert_legs(4);
}

const RELEASE01_17: &str = r#"
struct N { k: i64, a: Rc<N>, b: Rc<N> }
struct H { r: Rc<N> }
fn work() -> i64 {
    let mut h = H { r: Rc::null() }
    let mut y: Rc<N> = Rc::new(N { k: 7, a: Rc::null(), b: Rc::null() })
    let mut x: Rc<N> = Rc::new(N { k: 1, a: y, b: Rc::null() })
    h.r = x
    let mut p: Rc<N> = Rc::new(N { k: 2, a: y, b: Rc::null() })
    let mut q: Rc<N> = Rc::new(N { k: 3, a: p, b: Rc::null() })
    p.b = q
    x = Rc::null()
    y = Rc::null()
    p = Rc::null()
    q = Rc::null()
    __aelys_collect()
    return Rc::get(Rc::get(h.r).a).k
}
fn main() -> i64 {
    let t = work()
    MID
    let u = work()
    println(t)
    println(u)
    return 0
}
"#;

#[test]
fn release01_17_an_object_live_only_by_propagation_is_kept() {
    let h = Harness::new();
    h.exit_and_mid_rows("release01-17", RELEASE01_17, "7\n7\n", (8, 8), 2);
    h.assert_legs(8);
}
