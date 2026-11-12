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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Collections {
    Unchecked,
    Exactly(i64),
    AtLeastOne,
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
        self.run_row(id, src, stdout, counted, Collections::Unchecked, None);
    }

    fn running_row(&self, id: &str, src: &str, stdout: &str, counted: (i64, i64)) {
        self.run_row(id, src, stdout, counted, Collections::AtLeastOne, None);
    }

    fn stressed_row(&self, id: &str, src: &str, stdout: &str, counted: (i64, i64)) {
        self.run_row(id, src, stdout, counted, Collections::AtLeastOne, Some("1"));
    }

    fn collected_row(
        &self,
        id: &str,
        src: &str,
        stdout: &str,
        counted: (i64, i64),
        collections: i64,
    ) {
        self.run_row(
            id,
            src,
            stdout,
            counted,
            Collections::Exactly(collections),
            None,
        );
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
        collections: Collections,
        threshold: Option<&str>,
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
                cmd.env_remove("AELYS_CYCLE_THRESHOLD");
                if let Some(t) = threshold {
                    cmd.env("AELYS_CYCLE_THRESHOLD", t);
                }
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
                if collections == Collections::Unchecked {
                    continue;
                }
                let seen = parse_cycles(&seen_err)
                    .unwrap_or_else(|| panic!("{id} at {leg}: no [cycles] line\n{seen_err}"));
                assert_eq!(seen.1, 0, "{id} at {leg}: clamps");
                match collections {
                    Collections::Exactly(n) => {
                        assert_eq!(seen.0, n, "{id} at {leg}: collections");
                    }
                    Collections::AtLeastOne => {
                        assert!(seen.0 >= 1, "{id} at {leg}: no collection ran");
                    }
                    Collections::Unchecked => {}
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
    h.running_row(
        "release01-15",
        RELEASE01_15,
        "1000000\n",
        (1_000_000, 1_000_000),
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

const RELEASE01_18: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn churn(i: i64) -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: a })
    a.next = b
    return Rc::get(Rc::get(a).next).val
}
fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 20000 {
        n = n + churn(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_18_a_two_node_cycle_churned_in_a_loop_is_collected_during_execution() {
    let h = Harness::new();
    h.running_row("release01-18", RELEASE01_18, "40000\n", (40_000, 40_000));
    h.assert_legs(4);
}

const RELEASE01_19: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node> }
fn poke(h: &H) -> i64 { return Rc::get((*h).r).val }
fn work(i: i64) -> i64 {
    let mut h = H { r: Rc::null() }
    let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
    a.next = a
    h.r = a
    let t = poke(&h)
    h.r = Rc::new(Node { val: 7, next: Rc::null() })
    return t + poke(&h)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_19_a_stack_holder_field_overwritten_then_a_safepoint_stays_balanced() {
    let h = Harness::new();
    h.stressed_row("release01-19", RELEASE01_19, "24\n", (6, 6));
    h.assert_legs(4);
}

const RELEASE01_20: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node> }
fn poke(h: Rc<H>) -> i64 { return Rc::get(Rc::get(h).r).val }
fn peek(h: Rc<H>) -> i64 {
    let c = h
    return 1
}
fn work(i: i64) -> i64 {
    let hold: Rc<H> = Rc::new(H { r: Rc::null() })
    Rc::get(hold).r = Rc::new(Node { val: i, next: Rc::null() })
    let t = poke(hold) + peek(hold)
    Rc::get(hold).r = Rc::new(Node { val: 7, next: Rc::null() })
    return t + poke(hold)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_20_an_rc_get_field_overwrite_of_a_candidate_holder_then_a_safepoint_stays_sound() {
    let h = Harness::new();
    h.stressed_row("release01-20", RELEASE01_20, "27\n", (9, 9));
    h.assert_legs(4);
}

const RELEASE01_21: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node> }
struct W { k: i64, h: H }
fn poke(w: &W) -> i64 { return Rc::get((*w).h.r).val }
fn work(i: i64) -> i64 {
    let mut w = W { k: 1, h: H { r: Rc::null() } }
    let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
    a.next = a
    w.h.r = a
    let t = poke(&w)
    w.h.r = Rc::new(Node { val: 7, next: Rc::null() })
    return t + poke(&w)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_21_a_nested_stack_holder_field_overwrite_then_a_safepoint_stays_balanced() {
    let h = Harness::new();
    h.stressed_row("release01-21", RELEASE01_21, "24\n", (6, 6));
    h.assert_legs(4);
}

const RELEASE01_22: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node> }
fn poke(h: &H) -> i64 { return Rc::get((*h).r).val }
fn set(h: &mut H, v: i64) {
    (*h).r = Rc::new(Node { val: v, next: Rc::null() })
    let k = poke(&(*h))
}
fn work(i: i64) -> i64 {
    let mut h = H { r: Rc::null() }
    let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
    a.next = a
    h.r = a
    let t = poke(&h)
    set(&mut h, 7)
    return t + poke(&h)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_22_a_field_overwrite_through_a_mutable_reference_then_a_safepoint_stays_balanced() {
    let h = Harness::new();
    h.stressed_row("release01-22", RELEASE01_22, "24\n", (6, 6));
    h.assert_legs(4);
}

const RELEASE01_23: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { k: i64, r: Rc<Node> }
fn poke(h: Rc<H>) -> i64 { return Rc::get(Rc::get(h).r).val }
fn peek(h: Rc<H>) -> i64 {
    let c = h
    return 1
}
fn work(i: i64) -> i64 {
    let hold: Rc<H> = Rc::new(H { k: 0, r: Rc::null() })
    let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
    Rc::get(hold).r = a
    let t = poke(hold) + peek(hold)
    let p = &mut Rc::get(hold)
    *p = H { k: 1, r: Rc::new(Node { val: 7, next: Rc::null() }) }
    return t + poke(hold)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_23_a_whole_payload_overwrite_of_a_candidate_holder_then_a_safepoint_stays_balanced() {
    let h = Harness::new();
    h.stressed_row("release01-23", RELEASE01_23, "27\n", (9, 9));
    h.assert_legs(4);
}

const RELEASE01_24: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node> }
fn poke(h: Rc<H>) -> i64 { return Rc::get(Rc::get(h).r).val }
fn peek(h: Rc<H>) -> i64 {
    let c = h
    return 1
}
fn work(i: i64) -> i64 {
    let hold: Rc<H> = Rc::new(H { r: Rc::null() })
    let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
    a.next = Rc::new(Node { val: 1, next: Rc::null() })
    hold.r = a
    let t = poke(hold) + peek(hold)
    Rc::get(Rc::get(hold).r).next = Rc::new(Node { val: 7, next: Rc::null() })
    return t + Rc::get(Rc::get(Rc::get(hold).r).next).val + poke(hold)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_24_a_nested_field_overwrite_under_a_candidate_holder_then_a_safepoint_stays_sound() {
    let h = Harness::new();
    h.stressed_row("release01-24", RELEASE01_24, "30\n", (12, 12));
    h.assert_legs(4);
}

const RELEASE01_25: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node>, me: Rc<H> }
fn poke(h: Rc<H>) -> i64 { return Rc::get(Rc::get(h).r).val }
fn peek(h: Rc<H>) -> i64 {
    let c = h
    return 1
}
fn work(i: i64) -> i64 {
    let hold: Rc<H> = Rc::new(H { r: Rc::null(), me: Rc::null() })
    hold.me = hold
    hold.r = Rc::new(Node { val: i, next: Rc::null() })
    let t = poke(hold) + peek(hold)
    hold.r = Rc::new(Node { val: 7, next: Rc::null() })
    return t + poke(hold)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_25_a_field_overwrite_of_a_self_cyclic_candidate_holder_then_a_safepoint_stays_sound() {
    let h = Harness::new();
    h.stressed_row("release01-25", RELEASE01_25, "27\n", (9, 9));
    h.assert_legs(4);
}

const RELEASE01_26: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct H { r: Rc<Node> }
fn poke(h: Rc<H>) -> i64 { return Rc::get(Rc::get(h).r).val }
fn peek(h: Rc<H>) -> i64 {
    let c = h
    return 1
}
fn set(h: &mut H, v: i64) {
    (*h).r = Rc::new(Node { val: v, next: Rc::null() })
}
fn work(i: i64) -> i64 {
    let hold: Rc<H> = Rc::new(H { r: Rc::null() })
    Rc::get(hold).r = Rc::new(Node { val: i, next: Rc::null() })
    let t = poke(hold) + peek(hold)
    set(&mut Rc::get(hold), 7)
    return t + poke(hold)
}
fn main() -> i64 {
    let mut i = 0
    let mut n = 0
    while i < 3 {
        n = n + work(i)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_26_a_mutable_reference_into_an_rc_payload_overwrite_then_a_safepoint_stays_balanced() {
    let h = Harness::new();
    h.stressed_row("release01-26", RELEASE01_26, "27\n", (9, 9));
    h.assert_legs(4);
}

const RELEASE01_27: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string, me: Rc<W> }
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }

fn use_view(bs: &[u8]) -> i64 {
    let p = poke(1)
    return bs[0] + bs.len
}
fn main() -> i64 {
    let mut a = Rc::new(W { s: victim(12345), me: Rc::null() })
    a.me = a
    let bs = Rc::get(a).s.bytes
    a = poke(2)
    let n = use_view(bs)
    let t: string = other(88888)
    println("{n + t.len}")
    return 0
}
"#;

#[test]
fn release01_27_a_bytes_view_held_across_a_call_survives_the_collection_of_its_cycle() {
    let h = Harness::new();
    h.stressed_row("release01-27", RELEASE01_27, "111\n", (11, 11));
    h.assert_legs(4);
}

const RELEASE01_28: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string, me: Rc<W> }
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }

fn main() -> i64 {
    let mut a = Rc::new(W { s: victim(12345), me: Rc::null() })
    a.me = a
    let mut n: i64 = 0
    for b in Rc::get(a).s.bytes {
        a = poke(b)
        let t: string = other(88888)
        n = n + b + t.len
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn release01_28_a_for_over_the_bytes_of_a_cycle_member_survives_the_body_dropping_the_cycle() {
    let h = Harness::new();
    h.stressed_row("release01-28", RELEASE01_28, "499\n", (38, 38));
    h.assert_legs(4);
}

const RELEASE01_29: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string, me: Rc<W> }
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }

fn take(s: string, k: i64) -> i64 { return s.len + s.bytes[0] + k }
fn main() -> i64 {
    let mut a = Rc::new(W { s: victim(12345), me: Rc::null() })
    a.me = a
    println(take(Rc::get(a).s, { a = poke(5); let p = poke(6); 1 }))
    return 0
}
"#;

#[test]
fn release01_29_a_string_argument_of_a_cycle_member_survives_a_block_argument_dropping_the_cycle() {
    let h = Harness::new();
    h.stressed_row("release01-29", RELEASE01_29, "105\n", (9, 9));
    h.assert_legs(4);
}

const RELEASE01_30: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { s: string, a: [string; 2], e: E, me: Rc<W> }
fn mk(n: i64) -> Rc<W> {
    let w = Rc::new(W { s: victim(n), a: [victim(1), victim(n)], e: E::A(victim(n)), me: Rc::null() })
    w.me = w
    return w
}
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), a: [other(1), other(2)], e: E::B, me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn take(s: string, k: i64) -> i64 { return s.len + s.bytes[0] + k }

fn main() -> i64 {
    let mut a = mk(12345)
    println(take("{Rc::get(a).s}", { a = poke(5); let p = poke(6); 1 }))
    return 0
}
"#;

#[test]
fn release01_30_an_interpolation_of_a_cycle_member_survives_a_block_argument_dropping_the_cycle() {
    let h = Harness::new();
    h.stressed_row("release01-30", RELEASE01_30, "105\n", (23, 23));
    h.assert_legs(4);
}

const RELEASE01_31: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { s: string, a: [string; 2], e: E, me: Rc<W> }
fn mk(n: i64) -> Rc<W> {
    let w = Rc::new(W { s: victim(n), a: [victim(1), victim(n)], e: E::A(victim(n)), me: Rc::null() })
    w.me = w
    return w
}
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), a: [other(1), other(2)], e: E::B, me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn take(s: string, k: i64) -> i64 { return s.len + s.bytes[0] + k }

fn take_e(e: E, k: i64) -> i64 {
    let p = poke(7)
    return match e { E::A(x) => x.bytes[0] + x.len + k, E::B => 0 }
}
fn main() -> i64 {
    let mut a = mk(12345)
    println(take_e(Rc::get(a).e, { a = poke(5); 1 }))
    return 0
}
"#;

#[test]
fn release01_31_an_enum_field_argument_survives_a_block_argument_dropping_its_cycle() {
    let h = Harness::new();
    h.stressed_row("release01-31", RELEASE01_31, "105\n", (23, 23));
    h.assert_legs(4);
}

const RELEASE01_32: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { s: string, a: [string; 2], e: E, me: Rc<W> }
fn mk(n: i64) -> Rc<W> {
    let w = Rc::new(W { s: victim(n), a: [victim(1), victim(n)], e: E::A(victim(n)), me: Rc::null() })
    w.me = w
    return w
}
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), a: [other(1), other(2)], e: E::B, me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn take(s: string, k: i64) -> i64 { return s.len + s.bytes[0] + k }

fn use_s(s: string) -> i64 {
    let p = poke(8)
    return s.bytes[0] + s.len
}
fn main() -> i64 {
    let mut a = mk(12345)
    let v = Rc::get(a).a[0..2]
    a = poke(5)
    let n = use_s(v[1])
    let t: string = other(88888)
    println("{n + t.len}")
    return 0
}
"#;

#[test]
fn release01_32_a_slice_element_of_a_cycle_member_survives_the_cycle_dropped_before_a_call() {
    let h = Harness::new();
    h.stressed_row("release01-32", RELEASE01_32, "111\n", (25, 25));
    h.assert_legs(4);
}

const RELEASE01_33: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { s: string, a: [string; 2], e: E, me: Rc<W> }
fn mk(n: i64) -> Rc<W> {
    let w = Rc::new(W { s: victim(n), a: [victim(1), victim(n)], e: E::A(victim(n)), me: Rc::null() })
    w.me = w
    return w
}
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), a: [other(1), other(2)], e: E::B, me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn take(s: string, k: i64) -> i64 { return s.len + s.bytes[0] + k }

fn main() -> i64 {
    let mut a = mk(12345)
    let mut n: i64 = 0
    for b in bytes_of(&Rc::get(a)) {
        a = poke(b)
        let t: string = other(88888)
        n = n + b + t.len
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn release01_33_a_for_over_a_view_returned_by_a_call_survives_the_body_dropping_the_cycle() {
    let h = Harness::new();
    h.stressed_row("release01-33", RELEASE01_33, "499\n", (72, 72));
    h.assert_legs(4);
}

const RELEASE01_34: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { s: string, a: [string; 2], e: E, me: Rc<W> }
fn mk(n: i64) -> Rc<W> {
    let w = Rc::new(W { s: victim(n), a: [victim(1), victim(n)], e: E::A(victim(n)), me: Rc::null() })
    w.me = w
    return w
}
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), a: [other(1), other(2)], e: E::B, me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn take(s: string, k: i64) -> i64 { return s.len + s.bytes[0] + k }

fn clob(bs: &[u8], k: i64) -> i64 {
    let p = poke(9)
    let t: string = other(88888)
    return bs[0] + bs.len + t.len + k
}
fn main() -> i64 {
    let mut a = mk(12345)
    println(clob(bytes_of(&Rc::get(a)), { a = poke(5); 1 }))
    return 0
}
"#;

#[test]
fn release01_34_a_view_argument_returned_by_a_call_survives_a_block_argument_dropping_the_cycle() {
    let h = Harness::new();
    h.stressed_row("release01-34", RELEASE01_34, "112\n", (25, 25));
    h.assert_legs(4);
}

const RELEASE01_35: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { s: string, a: [string; 2], e: E, me: Rc<W> }
fn mk(n: i64) -> Rc<W> {
    let w = Rc::new(W { s: victim(n), a: [victim(1), victim(n)], e: E::A(victim(n)), me: Rc::null() })
    w.me = w
    return w
}
fn poke(k: i64) -> Rc<W> { return Rc::new(W { s: other(k), a: [other(1), other(2)], e: E::B, me: Rc::null() }) }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn take(s: string, k: i64) -> i64 { return s.len + s.bytes[0] + k }

fn main() -> i64 {
    let mut a = mk(12345)
    let mut i = 0
    let mut n = 0
    let bs = Rc::get(a).s.bytes
    while i < 3 {
        a = poke(i)
        let t: string = other(88888)
        n = n + bs[i] + t.len
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn release01_35_a_bytes_view_held_across_iterations_survives_the_cycle_dropped_in_the_loop() {
    let h = Harness::new();
    h.stressed_row("release01-35", RELEASE01_35, "265\n", (36, 36));
    h.assert_legs(4);
}

const RELEASE01_36: &str = r#"
nogc fn take(h: fn() -> i64, s: string) -> i64 { return s.len }
nogc fn spin(f: fn() -> i64, s: string, n: i64) -> i64 {
    let mut i = 0
    let mut t = 0
    let mut acc: string = "x"
    let mut g = f
    while i < n {
        g = f
        acc = s
        t = t + take(g, acc)
        i = i + 1
    }
    return t
}
fn main() -> i64 {
    let k: i64 = 5
    let f = fn() -> i64 { return k }
    let mut s: string = "a"
    let mut j = 0
    while j < 3 {
        s = s + "b"
        j = j + 1
    }
    println(spin(f, s, 10))
    return 0
}
"#;

#[test]
fn release01_36_a_nogc_loop_releasing_closure_envs_and_strings_never_collects() {
    let h = Harness::new();
    h.run_row(
        "release01-36",
        RELEASE01_36,
        "40\n",
        (4, 4),
        Collections::Exactly(0),
        Some("1"),
    );
    h.assert_legs(4);
}

const RELEASE01_37: &str = r#"
struct S { s: string, k: i64 }
struct Node { val: i64, next: Rc<Node> }
nogc fn fill(p: &mut S, src: string, n: i64) {
    let mut i = 0
    while i < n {
        (*p).s = src
        (*p).k = (*p).k + 1
        i = i + 1
    }
}
struct Tags { names: [string; 4], n: i64 }
nogc fn rot(a: &mut Tags, n: i64) {
    let mut i = 0
    while i < n {
        (*a).names[i % 4] = (*a).names[(i + 1) % 4]
        i = i + 1
    }
}
nogc fn relabel(s: string, n: i64) -> i64 {
    let mut acc: string = "x"
    let mut i = 0
    while i < n {
        acc = s
        i = i + 1
    }
    return acc.len
}
nogc fn take(h: fn() -> i64, s: string) -> i64 { return s.len }
nogc fn spin(f: fn() -> i64, s: string, n: i64) -> i64 {
    let mut i = 0
    let mut t = 0
    let mut g = f
    while i < n {
        g = f
        t = t + take(g, s)
        i = i + 1
    }
    return t
}
fn churn(n: i64) -> i64 {
    let mut i = 0
    let mut t = 0
    while i < n {
        let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
        a.next = a
        t = t + Rc::get(a).val
        i = i + 1
    }
    return t
}
fn main() -> i64 {
    let mut p = S { s: "a", k: 0 }
    fill(&mut p, "b" + "c", 5)
    let mut a = Tags { names: ["a", "bb", "ccc", "dddd"], n: 4 }
    rot(&mut a, 7)
    let k = 3
    let f = fn() -> i64 { return k }
    let t = spin(f, "xy" + "z", 4) + relabel("pq" + "r", 3) + churn(5)
    println("{p.s} {p.k} {a.names[0]}{a.names[1]}{a.names[2]}{a.names[3]} {t}")
    return 0
}
"#;

const RELEASE01_37_GLOBAL: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
let mut G: string = "seed"
nogc fn w(n: i64) {
    let mut i = 0
    while i < n {
        G = "q"
        i = i + 1
    }
}
nogc fn k(s: string) -> i64 {
    w(3)
    return s.bytes[0] + s.len
}
fn main() -> i64 {
    G = victim(12345)
    println(k(G))
    return 0
}
"#;

const SAFEPOINT_MARKS: &[&str] = &["__aelys_cycle_poll", "__aelys_cycle_due", "__aelys_collect"];

fn ir_bodies(ir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in ir.lines() {
        if let Some((name, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
            if line == "}" {
                out.push((std::mem::take(name), std::mem::take(body)));
                current = None;
            }
            continue;
        }
        if line.starts_with("define ")
            && let Some(at) = line.find('@')
        {
            let rest = line[at + 1..].trim_start_matches('"');
            let end = rest.find(['"', '(']).unwrap_or(rest.len());
            current = Some((rest[..end].to_string(), String::new()));
        }
    }
    out
}

fn nogc_names(src: &str) -> Vec<String> {
    src.split("nogc fn ")
        .skip(1)
        .filter_map(|rest| rest.split('(').next())
        .map(|name| name.trim().to_string())
        .collect()
}

fn polled_nogc_functions(ir: &str, nogc: &[String]) -> (usize, Vec<String>, usize) {
    let bodies = ir_bodies(ir);
    let mut seen = 0;
    let mut polled = Vec::new();
    for (name, body) in &bodies {
        if nogc.contains(name) {
            seen += 1;
            if SAFEPOINT_MARKS.iter().any(|m| body.contains(m)) {
                polled.push(name.clone());
            }
        }
    }
    let managed_polled = bodies
        .iter()
        .filter(|(name, body)| !nogc.contains(name) && body.contains("__aelys_cycle_poll"))
        .count();
    (seen, polled, managed_polled)
}

#[test]
fn release01_37_no_nogc_function_holds_a_safepoint_in_the_llvm_ir() {
    let h = Harness::new();
    let mut checked = 0;
    for (id, src) in [
        ("release01-37", RELEASE01_37),
        ("release01-37-global", RELEASE01_37_GLOBAL),
    ] {
        let nogc = nogc_names(src);
        for (tag, opt) in LEVELS {
            let root = h.stage(id, tag, src);
            compile_file_with_llvm_variant(&root, *opt, true, RuntimeVariant::RcCycles)
                .unwrap_or_else(|e| panic!("{id} at {tag}: emit ir\n{e}"));
            let ir = fs::read_to_string(root.with_extension("ll")).expect("read emitted ir");
            let (seen, polled, managed_polled) = polled_nogc_functions(&ir, &nogc);
            assert!(
                polled.is_empty(),
                "{id} at {tag}: safepoint in nogc {polled:?}"
            );
            assert!(managed_polled >= 1, "{id} at {tag}: no safepoint anywhere");
            if *tag == "-O0" {
                assert_eq!(seen, nogc.len(), "{id} at {tag}: nogc functions in the ir");
            }
            checked += seen;
        }
    }
    assert!(checked >= 10, "nogc function bodies checked: {checked}");
}

#[test]
fn release01_38_a_safepoint_planted_in_nogc_functions_is_caught_by_the_ir_check() {
    let h = Harness::new();
    let root = h.stage("release01-38", "-O0", RELEASE01_37);
    let nogc = nogc_names(RELEASE01_37);
    let mut air = aelys_driver::lower_file_to_air(&root, OptimizationLevel::None)
        .unwrap_or_else(|e| panic!("lower\n{e}"));
    for f in &mut air.functions {
        if nogc.contains(&f.name) {
            assert!(
                f.attributes.no_gc,
                "{} is not marked nogc in the air",
                f.name
            );
            f.attributes.no_gc = false;
        }
    }
    aelys_air::passes::safepoints::insert_cycle_polls(&mut air);
    let mut codegen = aelys_codegen::CodegenContext::new("release01_38");
    codegen.compile(&air).expect("codegen of the planted air");
    let ll = root.with_extension("ll");
    codegen
        .emit_ir(&ll.to_string_lossy())
        .expect("emit planted ir");
    let ir = fs::read_to_string(&ll).expect("read planted ir");
    let (seen, polled, _) = polled_nogc_functions(&ir, &nogc);
    assert_eq!(seen, nogc.len(), "nogc functions in the planted ir");
    assert_eq!(
        polled.len(),
        nogc.len(),
        "planted nogc functions caught: {polled:?}"
    );
}

fn windowed_air(
    h: &Harness,
    id: &str,
    tag: &str,
    opt: OptimizationLevel,
    src: &str,
) -> aelys_air::AirProgram {
    let root = h.stage(id, tag, src);
    let mut air = aelys_driver::lower_file_to_air(&root, opt)
        .unwrap_or_else(|e| panic!("{id} at {tag}: lower\n{e}"));
    aelys_air::passes::safepoints::insert_cycle_polls(&mut air);
    air
}

#[test]
fn release01_39_no_user_call_or_safepoint_sits_between_a_field_release_and_its_store() {
    let h = Harness::new();
    let mut checked = 0;
    for (id, src) in [
        ("release01-19", RELEASE01_19),
        ("release01-20", RELEASE01_20),
        ("release01-21", RELEASE01_21),
        ("release01-22", RELEASE01_22),
        ("release01-24", RELEASE01_24),
        ("release01-25", RELEASE01_25),
        ("release01-26", RELEASE01_26),
        ("release01-13", &RELEASE01_13.replace("    MID\n", "")),
    ] {
        for (tag, opt) in LEVELS {
            let air = windowed_air(&h, id, tag, *opt, src);
            let windows = aelys_air::passes::safepoints::release_store_windows(&air);
            assert!(
                windows.violations.is_empty(),
                "{id} at {tag}: {:?}",
                windows.violations
            );
            assert!(
                windows.checked >= 1,
                "{id} at {tag}: no release before a store"
            );
            checked += windows.checked;
        }
    }
    assert!(checked >= 16, "windows checked: {checked}");
}

#[test]
fn release01_40_a_safepoint_planted_between_a_field_release_and_its_store_is_caught() {
    use aelys_air::{AirStmt, AirStmtKind, Callee, Place};
    let h = Harness::new();
    let mut air = windowed_air(
        &h,
        "release01-40",
        "-O0",
        OptimizationLevel::None,
        RELEASE01_20,
    );
    let mut planted = 0;
    for f in &mut air.functions {
        for b in &mut f.blocks {
            let mut i = 0;
            while i + 1 < b.stmts.len() {
                let releases = matches!(
                    &b.stmts[i].kind,
                    AirStmtKind::CallVoid { func: Callee::Named(n), .. } if n == "__aelys_rc_release"
                );
                let stores = matches!(
                    &b.stmts[i + 1].kind,
                    AirStmtKind::Assign {
                        place: Place::Field(..),
                        ..
                    }
                );
                if releases && stores {
                    b.stmts.insert(
                        i + 1,
                        AirStmt {
                            kind: AirStmtKind::CallVoid {
                                func: Callee::Named(
                                    aelys_air::passes::safepoints::CYCLE_POLL.to_string(),
                                ),
                                args: Vec::new(),
                            },
                            span: None,
                        },
                    );
                    planted += 1;
                    i += 1;
                }
                i += 1;
            }
        }
    }
    assert!(planted >= 2, "planted safepoints: {planted}");
    let windows = aelys_air::passes::safepoints::release_store_windows(&air);
    assert_eq!(
        windows.violations.len(),
        planted,
        "{:?}",
        windows.violations
    );
    assert!(
        windows
            .violations
            .iter()
            .all(|v| v.contains("__aelys_cycle_poll")),
        "{:?}",
        windows.violations
    );
}

const RELEASE01_41: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let base: i64 = 0
    let f = fn(n: i64) -> i64 {
        let mut i = base

        let mut s = 0
        while i < n {
            let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
            let b: Rc<Node> = Rc::new(Node { val: 1, next: a })
            a.next = b
            s = s + Rc::get(b).val
            i = i + 1
        }
        return s
    }
    println(f(30000))
    return 0
}
"#;

#[test]
fn release01_41_a_cycle_churned_in_a_loop_inside_a_lambda_is_collected_during_execution() {
    let h = Harness::new();
    h.running_row("release01-41", RELEASE01_41, "30000\n", (60_001, 60_001));
    h.assert_legs(4);
}
