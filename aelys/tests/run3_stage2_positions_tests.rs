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

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    PreNarrowing,
    PostNarrowing,
}

const PHASE: Phase = Phase::PostNarrowing;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    MustBecomeAccepted,
    MustStayRejected,
    MustNotMoveAccepted,
    /// a position pre-empted by a fence that is not the effect system
    MustNotMoveRejected(&'static str),
    /// a genuinely managed operation that a fence reaches before the effect system does, so the
    MustStayRejectedBehind(&'static str),
    /// this fence still refuses. unlike muststayrejectedbehind the node rule is not firing
    OnlyTheFenceHolds(&'static str),
    /// a position the rule accepts whose lowering has never been executed and cannot be: no
    AcceptedButInert {
        fence: &'static str,
        callers: &'static [&'static str],
    },
    MovesToTheValidator,
    /// a genuinely managed store that no rule and no fence refuses. the row pins the acceptance
    AcceptedAndOwed(&'static str),
}

#[derive(Clone, Copy)]
struct Twin {
    src: &'static str,
    stdout: &'static str,
    exit: i32,
    allocs: i64,
    frees: i64,
}

#[derive(Clone, Copy)]
struct Row {
    id: &'static str,
    position: &'static str,
    class: Class,
    src: &'static str,
    twin: Option<Twin>,
}

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

struct Outcome {
    exit: i32,
    stdout: String,
    stderr: String,
    stats: Option<(i64, i64)>,
}

struct Harness {
    dir: TempDir,
    compiled_legs: Cell<usize>,
    linker_skips: Cell<usize>,
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

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
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
            compiled_legs: Cell::new(0),
            linker_skips: Cell::new(0),
        }
    }

    fn write(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let path = self.dir.path().join(format!("{}.aelys", slug(id, tag)));
        fs::write(&path, src).expect("write fixture");
        path
    }

    // none means the toolchain cannot link here, which is a skip and not a failure
    fn compile(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> Option<PathBuf> {
        let path = self.write(id, tag, src);
        match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
            Ok(()) => self.compiled_legs.set(self.compiled_legs.get() + 1),
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    self.linker_skips.set(self.linker_skips.get() + 1);
                    return None;
                }
                panic!("{id} at {tag} must compile:\n{src}\nerror: {err}");
            }
        }
        let exe = exe_path_for(&path);
        exe.is_file().then_some(exe)
    }

    fn run(&self, exe: &Path, alloc: Option<&str>) -> Outcome {
        let mut cmd = Command::new(exe);
        cmd.env("AELYS_RC_STATS", "1");
        if let Some(a) = alloc {
            cmd.env("AELYS_ALLOC", a);
        }
        let out = cmd.output().expect("run compiled exe");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Outcome {
            exit: exit_code(&out.status),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stats: parse_stats(&stderr),
            stderr,
        }
    }

    fn reject(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> String {
        let path = self.write(id, tag, src);
        match lower_file_to_air(&path, opt) {
            Ok(_) => panic!("{id} at {tag} MUST be rejected, but it was accepted:\n{src}"),
            Err(rendered) => rendered,
        }
    }

    fn accepts(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) {
        let path = self.write(id, tag, src);
        if let Err(rendered) = lower_file_to_air(&path, opt) {
            panic!("{id} at {tag} MUST be accepted:\n{src}\nrejected with:\n{rendered}");
        }
    }
}

fn expect_reject(h: &Harness, id: &str, src: &str, code: &str) {
    for (tag, opt) in LEVELS {
        let rendered = h.reject(id, tag, src, *opt);
        assert!(
            rendered.contains(&format!("[{code}]")),
            "{id} at {tag} MUST be rejected with {code} specifically, got:\n{rendered}"
        );
    }
}

fn expect_accept(h: &Harness, id: &str, src: &str) {
    for (tag, opt) in LEVELS {
        h.accepts(id, tag, src, *opt);
    }
}

fn run_twin(h: &Harness, id: &str, t: &Twin) {
    for (tag, opt) in LEVELS {
        let Some(exe) = h.compile(id, &format!("twin{tag}"), t.src, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run(&exe, *alloc);
            assert_eq!(
                o.exit, t.exit,
                "{id} twin at {tag}/{alloc_name}: the answer MUST be {}\nsource:{}\nstdout: {:?}\nstderr:\n{}",
                t.exit, t.src, o.stdout, o.stderr
            );
            assert_eq!(
                o.stdout, t.stdout,
                "{id} twin at {tag}/{alloc_name}: stdout MUST be {:?}\nsource:{}",
                t.stdout, t.src
            );
            let (a, f) = o
                .stats
                .unwrap_or_else(|| panic!("{id} twin at {tag}/{alloc_name}: no [rc] stats line"));
            assert_eq!(
                (a, f),
                (t.allocs, t.frees),
                "{id} twin at {tag}/{alloc_name}: MUST be exactly allocs={} frees={}, got allocs={a} frees={f}\nsource:{}",
                t.allocs,
                t.frees,
                t.src
            );
        }
    }
}

fn required_verdict(class: Class, phase: Phase) -> Option<&'static str> {
    match (class, phase) {
        (Class::MustBecomeAccepted, Phase::PreNarrowing) => Some("E0727"),
        (Class::MustBecomeAccepted, Phase::PostNarrowing) => None,
        (Class::MustStayRejected, _) => Some("E0727"),
        (Class::MustNotMoveAccepted, _) => None,
        (Class::MustNotMoveRejected(code), _) => Some(code),
        (Class::MustStayRejectedBehind(fence), _) => Some(fence),
        (Class::OnlyTheFenceHolds(fence), _) => Some(fence),
        (Class::AcceptedButInert { .. }, Phase::PreNarrowing) => Some("E0727"),
        (Class::AcceptedButInert { .. }, Phase::PostNarrowing) => None,
        (Class::MovesToTheValidator, Phase::PreNarrowing) => Some("E0727"),
        (Class::MovesToTheValidator, Phase::PostNarrowing) => Some("E0901"),
        (Class::AcceptedAndOwed(_), Phase::PreNarrowing) => Some("E0727"),
        (Class::AcceptedAndOwed(_), Phase::PostNarrowing) => None,
    }
}

fn check_row(h: &Harness, row: &Row) {
    match required_verdict(row.class, PHASE) {
        Some(code) => expect_reject(h, row.id, row.src, code),
        None => expect_accept(h, row.id, row.src),
    }
    if let Some(t) = row.twin {
        run_twin(h, row.id, &t);
    }
}

fn linker_skip_declared() -> bool {
    std::env::var("AELYS_ALLOW_LINKER_SKIP").is_ok()
}

fn run_rows(rows: &[Row], expected_legs: usize) {
    let h = Harness::new();
    for row in rows {
        check_row(&h, row);
    }
    let (built, skipped) = (h.compiled_legs.get(), h.linker_skips.get());
    if skipped > 0 {
        assert!(
            linker_skip_declared(),
            "{skipped} leg(s) could not link, so this group has no runtime evidence; \
             set AELYS_ALLOW_LINKER_SKIP=1 to declare that on purpose"
        );
        return;
    }
    assert_eq!(
        built, expected_legs,
        "this group must compile and run exactly {expected_legs} executable legs"
    );
}

const GROUP_A: &[Row] = &[
    Row {
        id: "A01",
        position: "Return <- Index.object",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 { return (*r)[0] }\n\
              fn main() -> i64 {\n\
              \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
              \x20   return peek(&v)\n\
              }\n",
        twin: Some(Twin {
            src: "fn peek(r: &Vec<i64>) -> i64 { return (*r)[0] }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A02",
        position: "Member.object",
        class: Class::MustBecomeAccepted,
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn peek(s: &S) -> i64 { return (*s).n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "struct S { r: Rc<i64>, n: i64 }\n\
                  fn peek(s: &S) -> i64 { return (*s).n }\n\
                  fn main() -> i64 {\n\
                  \x20   let s = S{r: Rc::new(11), n: 7919}\n\
                  \x20   println(peek(&s))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A03",
        position: "Member.object nested/chained",
        class: Class::MustBecomeAccepted,
        src: "struct I { r: Rc<i64>, n: i64 }\n\
              struct S { i: I, m: i64 }\n\
              nogc fn peek(s: &S) -> i64 { return (*s).i.n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "struct I { r: Rc<i64>, n: i64 }\n\
                  struct S { i: I, m: i64 }\n\
                  fn peek(s: &S) -> i64 { return (*s).i.n }\n\
                  fn main() -> i64 {\n\
                  \x20   let s = S{i: I{r: Rc::new(11), n: 7919}, m: 3}\n\
                  \x20   println(peek(&s))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    // detaches nothing, which is what makes the side-of-assignment rule wrong on the left
    Row {
        id: "A04",
        position: "FieldAssign.object, unmanaged field",
        class: Class::MustBecomeAccepted,
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn poke(d: &mut S) -> i64 {\n\
              \x20   (*d).n = 5\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "struct S { r: Rc<i64>, n: i64 }\n\
                  fn poke(d: &mut S) -> i64 {\n\
                  \x20   (*d).n = 101\n\
                  \x20   return 0\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let mut s = S{r: Rc::new(11), n: 7919}\n\
                  \x20   let q = poke(&mut s)\n\
                  \x20   println(s.n)\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "101\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A05",
        position: "Let.initializer",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let x: i64 = (*r)[0]\n\
              \x20   return x\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn peek(r: &Vec<i64>) -> i64 {\n\
                  \x20   let x: i64 = (*r)[0]\n\
                  \x20   return x\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A06",
        position: "Grouping.inner",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 { return ((*r))[0] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn peek(r: &Vec<i64>) -> i64 { return ((*r))[0] }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A07",
        position: "Cast.expr",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 { return (*r)[0] as i64 }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn peek(r: &Vec<i64>) -> i64 { return (*r)[0] as i64 }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A08",
        position: "Slice.object",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let s: &[i64] = (*r)[0..2]\n\
              \x20   return s[0]\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn peek(r: &Vec<i64>) -> i64 {\n\
                  \x20   let s: &[i64] = (*r)[0..2]\n\
                  \x20   return s[0]\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A09",
        position: "Call.args, unmanaged projection",
        class: Class::MustBecomeAccepted,
        src: "nogc fn g(n: i64) -> i64 { return n }\n\
              nogc fn peek(r: &Vec<i64>) -> i64 { return g((*r)[0]) }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A10",
        position: "Binary.left",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 { return (*r)[0] + 1 }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A11",
        position: "Unary.operand",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 { return -(*r)[0] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A12",
        position: "If(stmt).condition",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   if (*r)[0] > 0 { return 1 }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A13",
        position: "While.condition",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   while (*r)[0] > 0 { return 1 }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A14",
        position: "For.end",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let mut t: i64 = 0\n\
              \x20   for i in 0..(*r)[0] { t = t + 1 }\n\
              \x20   return t\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A15",
        position: "Range.start and Range.end",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let mut t: i64 = 0\n\
              \x20   for i in (*r)[0]..(*r)[1] { t = t + 1 }\n\
              \x20   return t\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A16",
        position: "Index.index",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>, w: &Vec<i64>) -> i64 { return (*w)[(*r)[0]] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A17",
        position: "FieldAssign.value",
        class: Class::MustBecomeAccepted,
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn poke(d: &mut S, s: &S) -> i64 {\n\
              \x20   (*d).n = (*s).n\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A18",
        position: "Expression(stmt)",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   (*r)[0]\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A19",
        position: "Block.tail",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let x: i64 = { (*r)[0] }\n\
              \x20   return x\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A20",
        position: "And.left and And.right",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   if (*r)[0] > 0 && (*r)[1] > 0 { return 1 }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A21",
        position: "Reference.operand, interior",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let p: &i64 = &(*r)[0]\n\
              \x20   return *p\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn peek(r: &Vec<i64>) -> i64 {\n\
                  \x20   let p: &i64 = &(*r)[0]\n\
                  \x20   return *p\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A22",
        position: "Reference.operand, whole",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let p: &Vec<i64> = &(*r)\n\
              \x20   return (*p)[0]\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn peek(r: &Vec<i64>) -> i64 {\n\
                  \x20   let p: &Vec<i64> = &(*r)\n\
                  \x20   return (*p)[0]\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A23",
        position: "ArraySized.fill_value",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let a: [i64; 3] = [(*r)[0]; 3]\n\
              \x20   return a[0]\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A24",
        position: "Member.object over Index over Deref",
        class: Class::MustBecomeAccepted,
        src: "struct S { n: i64 }\n\
              nogc fn peek(r: &Vec<S>) -> i64 { return (*r)[0].n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "struct S { n: i64 }\n\
                  fn peek(r: &Vec<S>) -> i64 { return (*r)[0].n }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
                  \x20   println(peek(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "A25",
        position: "Reference.operand on a field of an element",
        class: Class::MustBecomeAccepted,
        src: "struct S { n: i64 }\n\
              nogc fn peek(r: &Vec<S>) -> i64 {\n\
              \x20   let p: &i64 = &(*r)[0].n\n\
              \x20   return *p\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A26",
        position: "Slice.object over a Vec of structs",
        class: Class::MustBecomeAccepted,
        src: "struct S { n: i64 }\n\
              nogc fn peek(r: &Vec<S>) -> i64 {\n\
              \x20   let s: &[S] = (*r)[0..2]\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A27",
        position: "If(expr).condition",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let x: i64 = if (*r)[0] > 0 { 1 } else { 2 }\n\
              \x20   return x\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A28",
        position: "Block.stmts, a non-tail statement",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let x: i64 = { let y: i64 = (*r)[0]\n\
              \x20   y }\n\
              \x20   return x\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A29",
        position: "For.step",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   let mut t: i64 = 0\n\
              \x20   for i in 0..4 step (*r)[0] { t = t + 1 }\n\
              \x20   return t\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A30",
        position: "Or.left and Or.right",
        class: Class::MustBecomeAccepted,
        src: "nogc fn peek(r: &Vec<i64>) -> i64 {\n\
              \x20   if (*r)[0] > 0 || (*r)[1] > 0 { return 1 }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // no caller exists or can exist, so the acceptance has never been executed and cannot be
    Row {
        id: "A32",
        position: "Index.object over a slice whose element type is managed",
        class: Class::AcceptedButInert {
            fence: "E0410",
            callers: CALLERS_VEC_OF_MANAGED_STRUCT,
        },
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn peek(s: &[S]) -> i64 { return s[0].n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A31",
        position: "Index.object over a slice taken from a Vec",
        class: Class::MustBecomeAccepted,
        src: "struct S { n: i64 }\n\
              nogc fn peek(r: &Vec<S>) -> i64 {\n\
              \x20   let s: &[S] = (*r)[0..2]\n\
              \x20   return s[0].n\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A33",
        position: "Member.object over Index into a Vec whose element type is managed",
        class: Class::AcceptedButInert {
            fence: "E0410",
            callers: CALLERS_VEC_OF_MANAGED_STRUCT,
        },
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn peek(v: &Vec<S>) -> i64 { return (*v)[0].n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A34",
        position: "the same read re-rooted through a local slice",
        class: Class::AcceptedButInert {
            fence: "E0410",
            callers: CALLERS_VEC_OF_MANAGED_STRUCT,
        },
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn peek(v: &Vec<S>) -> i64 {\n\
              \x20   let s: &[S] = (*v)[0..2]\n\
              \x20   return s[0].n\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "A35",
        position: "Index.object over a local slice of a Vec of Vecs",
        class: Class::AcceptedButInert {
            fence: "E0412",
            callers: CALLERS_VEC_OF_VEC,
        },
        src: "nogc fn peek(r: &Vec<Vec<i64>>) -> i64 {\n\
              \x20   let s: &[Vec<i64>] = (*r)[0..2]\n\
              \x20   return s[0][1]\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
];

const CALLERS_VEC_OF_MANAGED_STRUCT: &[&str] = &[
    "struct S { r: Rc<i64>, n: i64 }\n\
     fn main() -> i64 {\n\
     \x20   let v: Vec<S> = vec[S{r: Rc::new(11), n: 7919}]\n\
     \x20   return v[0].n\n\
     }\n",
    "struct S { r: Rc<i64>, n: i64 }\n\
     fn main() -> i64 {\n\
     \x20   let a: [S; 1] = [S{r: Rc::new(11), n: 7919}]\n\
     \x20   return a[0].n\n\
     }\n",
];

const CALLERS_VEC_OF_VEC: &[&str] = &[
    "fn main() -> i64 {\n\
     \x20   let v: Vec<Vec<i64>> = vec[vec[7919, 2], vec[3, 4]]\n\
     \x20   return v[0][0]\n\
     }\n",
    "fn main() -> i64 {\n\
     \x20   let mut v: Vec<Vec<i64>> = Vec::new()\n\
     \x20   return 0\n\
     }\n",
];

const CONTROL_PLAIN_ELEMENT: (&str, &str, i64, i64) = (
    "struct S { n: i64 }\n\
     fn main() -> i64 {\n\
     \x20   let v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
     \x20   println(v[0].n)\n\
     \x20   return 0\n\
     }\n",
    "7919\n",
    1,
    1,
);


const GROUP_B: &[Row] = &[
    // second allocation is the cow detach that a04's twin does not have
    Row {
        id: "B01",
        position: "IndexAssign.object, cow detach",
        class: Class::MustStayRejected,
        src: "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
              \x20   (*r)[0] = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn poke(r: &mut Vec<i64>) -> i64 {\n\
                  \x20   (*r)[0] = 101\n\
                  \x20   return 0\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   let w: Vec<i64> = v\n\
                  \x20   let q = poke(&mut v)\n\
                  \x20   println(v[0])\n\
                  \x20   println(w[0])\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "101\n7919\n",
            exit: 0,
            allocs: 2,
            frees: 2,
        }),
    },
    Row {
        id: "B02",
        position: "IndexAssign.object with a projected value",
        class: Class::MustStayRejected,
        src: "nogc fn poke(r: &Vec<i64>, w: &mut Vec<i64>) -> i64 {\n\
              \x20   (*w)[0] = (*r)[0]\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: Some(Twin {
            src: "fn poke(r: &Vec<i64>, w: &mut Vec<i64>) -> i64 {\n\
                  \x20   (*w)[0] = (*r)[0]\n\
                  \x20   return 0\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   let mut u: Vec<i64> = vec[1, 2, 3]\n\
                  \x20   let t: Vec<i64> = u\n\
                  \x20   let q = poke(&v, &mut u)\n\
                  \x20   println(u[0])\n\
                  \x20   println(t[0])\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n1\n",
            exit: 0,
            allocs: 3,
            frees: 3,
        }),
    },
    Row {
        id: "B03",
        position: "DerefAssign.value, Rc to Rc",
        class: Class::MustStayRejected,
        src: "nogc fn xfer(d: &mut Rc<i64>, s: &Rc<i64>) -> i64 {\n\
              \x20   *d = *s\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B04",
        position: "DerefAssign.value, struct holding an Rc",
        class: Class::MustStayRejected,
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn xfer(d: &mut S, s: &S) -> i64 {\n\
              \x20   *d = *s\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B05",
        position: "Return of an owned managed value",
        class: Class::MustStayRejected,
        src: "nogc fn take(x: &Rc<i64>) -> Rc<i64> { return *x }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B06",
        position: "If(expr) arms yielding managed",
        class: Class::MustStayRejected,
        src: "nogc fn pick(d: &mut Rc<i64>, a: &Rc<i64>, b: &Rc<i64>, c: bool) -> i64 {\n\
              \x20   *d = if c { *a } else { *b }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B07",
        position: "Call.args, managed by value",
        class: Class::MustStayRejected,
        src: "nogc fn g(v: Vec<i64>) -> i64 { return 0 }\n\
              nogc fn f(r: &Vec<i64>) -> i64 { return g(*r) }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B08",
        position: "parameter, managed by value",
        class: Class::MustStayRejected,
        src: "nogc fn f(v: Vec<i64>) -> i64 { return 0 }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B09",
        position: "Let of a managed local",
        class: Class::MustStayRejected,
        src: "nogc fn f() -> i64 {\n\
              \x20   let v: Vec<i64> = vec[1, 2]\n\
              \x20   return v[0]\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B10",
        position: "EnumVariant, Rc::new",
        class: Class::MustStayRejected,
        src: "nogc fn f() -> i64 {\n\
              \x20   let a: Rc<i64> = Rc::new(7)\n\
              \x20   return Rc::get(a)\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B11",
        position: "EnumVariant, Vec::push",
        class: Class::MustStayRejected,
        src: "nogc fn f(v: Vec<i64>) -> i64 {\n\
              \x20   Vec::push(v, 9)\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B12",
        position: "indirect call through a general fn pointer",
        class: Class::MustStayRejected,
        src: "nogc fn f(g: fn(i64) -> i64) -> i64 { return g(1) }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B13",
        position: "FmtString",
        class: Class::MustStayRejected,
        src: "nogc fn f(r: &Vec<i64>) -> i64 {\n\
              \x20   println(\"{(*r)[0]}\")\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // the lambda body is never walked, so this rejection comes from the indirect call and not
    Row {
        id: "B14",
        position: "call of a locally bound lambda",
        class: Class::MustStayRejected,
        src: "nogc fn f() -> i64 {\n\
              \x20   let g = fn() -> i64 {\n\
              \x20       let v: Vec<i64> = vec[1, 2]\n\
              \x20       return v[0]\n\
              \x20   }\n\
              \x20   return g()\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B15",
        position: "callee summary propagation, nested fn",
        class: Class::MustStayRejected,
        src: "nogc fn f() -> i64 {\n\
              \x20   fn inner() -> i64 {\n\
              \x20       let v: Vec<i64> = vec[1, 2]\n\
              \x20       return v[0]\n\
              \x20   }\n\
              \x20   return inner()\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B16",
        position: "callee summary propagation, sibling fn",
        class: Class::MustStayRejected,
        src: "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
              \x20   (*r)[0] = 101\n\
              \x20   return 0\n\
              }\n\
              nogc fn f(r: &mut Vec<i64>) -> i64 { return poke(r) }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B17",
        position: "Return of a managed field read",
        class: Class::MustStayRejected,
        src: "struct S { r: Rc<i64> }\n\
              nogc fn take(s: &S) -> Rc<i64> {\n\
              \x20   return (*s).r\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B18",
        position: "Expression(stmt), discarded managed value",
        class: Class::MustStayRejected,
        src: "struct S { r: Rc<i64> }\n\
              nogc fn dump(s: &S) -> i64 {\n\
              \x20   (*s).r\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B19",
        position: "Let of a managed field read",
        class: Class::MustStayRejected,
        src: "struct S { r: Rc<i64> }\n\
              nogc fn f(s: &S) -> i64 {\n\
              \x20   let q: Rc<i64> = (*s).r\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // derived from a04's shape puts this row in group a and is wrong. the store does not detach
    Row {
        id: "B20",
        position: "FieldAssign.object through Index into a managed container",
        class: Class::MustStayRejected,
        src: "struct S { n: i64 }\n\
              nogc fn poke(r: &mut Vec<S>) -> i64 {\n\
              \x20   (*r)[0].n = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B21",
        position: "IndexAssign.index projected, target is a Vec",
        class: Class::MustStayRejected,
        src: "nogc fn poke(r: &mut Vec<i64>, w: &Vec<i64>) -> i64 {\n\
              \x20   (*r)[(*w)[0]] = 1\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // reachable through .unwrap(), .expect(), .into_ok() and .unwrap_unchecked(), never through
    Row {
        id: "B22",
        position: "ResultAssert.scrutinee, managed payload",
        class: Class::MustStayRejected,
        src: "enum Result<T, E> { Ok(T), Err(E) }\n\
              enum E { X }\n\
              nogc fn take(r: &Result<Vec<i64>, E>) -> i64 { return (*r).unwrap()[0] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B23",
        position: "Binary with a String operand",
        class: Class::MustStayRejected,
        src: "nogc fn f(a: string, b: string) -> string { return a + b }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // projection chain: index cannot repeat through a vec of vecs because e0412 stands behind it
    Row {
        id: "B24",
        position: "Index.object over a Vec of Vecs",
        class: Class::MustStayRejected,
        src: "nogc fn f(r: &Vec<Vec<i64>>) -> i64 { return (*r)[0][0] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B25",
        position: "IndexAssign.object, an array field of a Vec element",
        class: Class::MustStayRejected,
        src: "struct S { a: [i64; 2] }\n\
              nogc fn f(r: &mut Vec<S>) -> i64 {\n\
              \x20   (*r)[0].a[1] = 5\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // b26 and b27 were the store-side re-rooting escape: the chain the store clause reads is
    Row {
        id: "B26",
        position: "IndexAssign through a local slice of a Vec",
        class: Class::MustStayRejected,
        src: "nogc fn f(r: &mut Vec<i64>) -> i64 {\n\
              \x20   let s: &mut [i64] = (*r)[0..2]\n\
              \x20   s[0] = 1\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B27",
        position: "FieldAssign under a local slice of a Vec",
        class: Class::MustStayRejected,
        src: "struct S { n: i64 }\n\
              nogc fn f(r: &mut Vec<S>) -> i64 {\n\
              \x20   let s: &mut [S] = (*r)[0..2]\n\
              \x20   s[0].n = 1\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B28",
        position: "Member.object over a borrowed Rc of a non-scalar",
        class: Class::MovesToTheValidator,
        src: "struct S { n: i64 }\n\
              nogc fn f(x: &Rc<S>) -> i64 { return (*x).n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // a reference's mutability, so `&t` reaching a `&mut t` position is e0416. the eight rows stay
    Row {
        id: "B29",
        position: "Reference into a Vec element, mutable by let annotation",
        class: Class::MustNotMoveRejected("E0416"),
        src: "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
              \x20   let p: &mut i64 = &(*r)[0]\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B30",
        position: "the same, mutable by a call argument",
        class: Class::MustNotMoveRejected("E0416"),
        src: "nogc fn w(p: &mut i64) -> i64 {\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              nogc fn poke(r: &mut Vec<i64>) -> i64 { return w(&(*r)[0]) }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B31",
        position: "the same, mutable by a return type",
        class: Class::MustNotMoveRejected("E0416"),
        src: "nogc fn g(r: &mut Vec<i64>) -> &mut i64 { return &(*r)[0] }\n\
              nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
              \x20   let p: &mut i64 = g(r)\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B32",
        position: "the same, mutable by assignment to an existing &mut",
        class: Class::MustNotMoveRejected("E0416"),
        src: "nogc fn poke(r: &mut Vec<i64>, q: &mut i64) -> i64 {\n\
              \x20   let mut p: &mut i64 = q\n\
              \x20   p = &(*r)[0]\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B33",
        position: "the same, mutable through if-expression arms",
        class: Class::MustNotMoveRejected("E0416"),
        src: "nogc fn poke(r: &mut Vec<i64>, c: bool) -> i64 {\n\
              \x20   let p: &mut i64 = if c { &(*r)[0] } else { &(*r)[1] }\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B34",
        position: "the same, mutable through a block tail",
        class: Class::MustNotMoveRejected("E0416"),
        src: "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
              \x20   let p: &mut i64 = { &(*r)[0] }\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B35",
        position: "the same, onto a field of an element",
        class: Class::MustNotMoveRejected("E0416"),
        src: "struct S { n: i64 }\n\
              nogc fn poke(r: &mut Vec<S>) -> i64 {\n\
              \x20   let p: &mut i64 = &(*r)[0].n\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "B36",
        position: "the same, through a slice bound to a local",
        class: Class::MustNotMoveRejected("E0416"),
        src: "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
              \x20   let s: &mut [i64] = (*r)[0..2]\n\
              \x20   let p: &mut i64 = &s[0]\n\
              \x20   *p = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
];

const GROUP_C: &[Row] = &[
    Row {
        id: "C01",
        position: "Member.object, plain struct",
        class: Class::MustNotMoveAccepted,
        src: "struct P { n: i64 }\n\
              nogc fn peek(s: &P) -> i64 { return (*s).n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C02",
        position: "FieldAssign.object, plain struct",
        class: Class::MustNotMoveAccepted,
        src: "struct P { n: i64 }\n\
              nogc fn poke(d: &mut P) -> i64 {\n\
              \x20   (*d).n = 5\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C03",
        position: "Index.object, slice parameter",
        class: Class::MustNotMoveAccepted,
        src: "nogc fn peek(s: &[i64]) -> i64 { return s[0] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C04",
        position: "Call.args, managed by reference",
        class: Class::MustNotMoveAccepted,
        src: "nogc fn g(r: &Vec<i64>) -> i64 { return 0 }\n\
              nogc fn f(r: &Vec<i64>) -> i64 { return g(r) }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C05",
        position: "Call.args, nogc fn pointer",
        class: Class::MustNotMoveAccepted,
        src: "nogc fn ok() -> i64 { return 7 }\n\
              nogc fn apply(g: nogc fn() -> i64) -> i64 { return g() }\n\
              fn main() -> i64 { return apply(ok) }\n",
        twin: None,
    },
    Row {
        id: "C06",
        position: "A8 W-read",
        class: Class::MustNotMoveAccepted,
        src: "nogc fn peek(r: &i64) -> i64 { return *r }\n\
              fn main() -> i64 {\n\
              \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
              \x20   let w: Vec<i64> = v\n\
              \x20   println(peek(&v[0]))\n\
              \x20   println(peek(&w[0]))\n\
              \x20   return 0\n\
              }\n",
        twin: Some(Twin {
            src: "nogc fn peek(r: &i64) -> i64 { return *r }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   let w: Vec<i64> = v\n\
                  \x20   println(peek(&v[0]))\n\
                  \x20   println(peek(&w[0]))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
    Row {
        id: "C07",
        position: "A8 W-transfer",
        class: Class::MustNotMoveAccepted,
        src: "fn poke(r: &mut Vec<i64>) -> i64 {\n\
              \x20   (*r)[0] = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
              \x20   let w: Vec<i64> = v\n\
              \x20   let q = poke(&mut v)\n\
              \x20   println(v[0])\n\
              \x20   println(w[0])\n\
              \x20   return 0\n\
              }\n",
        twin: Some(Twin {
            src: "fn poke(r: &mut Vec<i64>) -> i64 {\n\
                  \x20   (*r)[0] = 101\n\
                  \x20   return 0\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   let w: Vec<i64> = v\n\
                  \x20   let q = poke(&mut v)\n\
                  \x20   println(v[0])\n\
                  \x20   println(w[0])\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "101\n7919\n",
            exit: 0,
            allocs: 2,
            frees: 2,
        }),
    },
    // the effect walk never enters lambdainner.body or lambdainner.params, so a vec literal and
    Row {
        id: "C08",
        position: "LambdaInner.body, uncalled",
        class: Class::MustNotMoveAccepted,
        src: "nogc fn f() -> i64 {\n\
              \x20   let g = fn() -> i64 {\n\
              \x20       let v: Vec<i64> = vec[1, 2]\n\
              \x20       return v[0]\n\
              \x20   }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return f() }\n",
        twin: None,
    },
    Row {
        id: "C09",
        position: "LambdaInner.params, uncalled",
        class: Class::MustNotMoveAccepted,
        src: "nogc fn f() -> i64 {\n\
              \x20   let g = fn(v: Vec<i64>) -> i64 { return 0 }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return f() }\n",
        twin: None,
    },
    Row {
        id: "C10",
        position: "Index.object without a deref",
        class: Class::MustNotMoveRejected("E0304"),
        src: "nogc fn peek(r: &Vec<i64>) -> i64 { return r[0] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C11",
        position: "Member.object without a deref",
        class: Class::MustNotMoveRejected("E0304"),
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn peek(s: &S) -> i64 { return s.n }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C12",
        position: "Deref.inner, managed",
        class: Class::MustNotMoveRejected("E0304"),
        src: "nogc fn peek(rr: & &Vec<i64>) -> i64 { return (*(*rr))[0] }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C13",
        position: "ForEach.iterable",
        class: Class::MustNotMoveRejected("E0414"),
        src: "nogc fn f(r: &Vec<i64>) -> i64 {\n\
              \x20   for x in *r { return 1 }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C14",
        position: "FieldAssign.object, managed field",
        class: Class::MustNotMoveRejected("E0410"),
        src: "struct S { r: Rc<i64>, n: i64 }\n\
              nogc fn poke(d: &mut S, s: &S) -> i64 {\n\
              \x20   (*d).r = (*s).r\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C15",
        position: "DerefAssign.value, Vec",
        class: Class::MustNotMoveRejected("E0412"),
        src: "nogc fn xfer(d: &mut Vec<i64>, s: &Vec<i64>) -> i64 {\n\
              \x20   *d = *s\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C16",
        position: "Return of a Vec by value",
        class: Class::MustNotMoveRejected("E0412"),
        src: "nogc fn take(r: &Vec<i64>) -> Vec<i64> { return *r }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C17",
        position: "Assign.value, managed name target",
        class: Class::MustNotMoveRejected("E0412"),
        src: "nogc fn f(s: &Vec<i64>) -> i64 {\n\
              \x20   let mut v: Vec<i64> = vec[1]\n\
              \x20   v = *s\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C18",
        position: "Match.scrutinee, managed",
        class: Class::MustNotMoveRejected("E0301"),
        src: "nogc fn f(r: &Vec<i64>) -> i64 {\n\
              \x20   return match *r { _ => 0 }\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C19",
        position: "Slice in assignment-target position",
        class: Class::MustNotMoveRejected("E0104"),
        src: "nogc fn f(r: &mut Vec<i64>, s: &[i64]) -> i64 {\n\
              \x20   (*r)[0..2] = s\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C20",
        position: "Call.args, lambda where a nogc fn pointer is expected",
        class: Class::MustNotMoveRejected("E0729"),
        src: "nogc fn apply(g: nogc fn() -> i64) -> i64 { return g() }\n\
              fn main() -> i64 {\n\
              \x20   return apply(fn() -> i64 {\n\
              \x20       let v: Vec<i64> = vec[1, 2]\n\
              \x20       return v[0]\n\
              \x20   })\n\
              }\n",
        twin: None,
    },
    Row {
        id: "C21",
        position: "ArrayLiteral.elements, projected",
        class: Class::MustNotMoveRejected("E0714"),
        src: "nogc fn f(r: &Vec<i64>) -> i64 {\n\
              \x20   let a: [i64; 2] = [(*r)[0], 1]\n\
              \x20   return a[0]\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C22",
        position: "StructLiteral.fields, projected",
        class: Class::MustNotMoveRejected("E0714"),
        src: "struct P { n: i64 }\n\
              nogc fn f(r: &Vec<i64>) -> i64 {\n\
              \x20   let p = P{n: (*r)[0]}\n\
              \x20   return p.n\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C23",
        position: "Tuple, no surface spelling",
        class: Class::MustNotMoveRejected("E0101"),
        src: "nogc fn f(r: &Vec<i64>) -> i64 {\n\
              \x20   let t: (i64, i64) = ((*r)[0], 1)\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C24",
        position: "Reference.operand, &mut to an element",
        class: Class::MustNotMoveRejected("E0415"),
        src: "nogc fn f(r: &mut Vec<i64>) -> i64 {\n\
              \x20   let p: &mut i64 = &mut (*r)[0]\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C25",
        position: "a nogc fn type on a local binding",
        class: Class::MustNotMoveRejected("E0728"),
        src: "nogc fn f() -> i64 {\n\
              \x20   let g: nogc fn() -> i64 = fn() -> i64 { return 7 }\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C26",
        position: "ResultAssert.scrutinee, unmanaged payload",
        class: Class::MustNotMoveAccepted,
        src: "enum Result<T, E> { Ok(T), Err(E) }\n\
              enum E { X }\n\
              nogc fn take(r: &Result<i64, E>) -> i64 { return (*r).unwrap() }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // the second of the two rows that bound the projection chain: a vec cannot sit under a field
    Row {
        id: "C27",
        position: "a struct field of Vec type",
        class: Class::MustNotMoveRejected("E0410"),
        src: "struct S { v: Vec<i64> }\n\
              nogc fn f(s: &S) -> i64 { return 0 }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // and e0304 is a type error rather than a fence on this run's relax list
    Row {
        id: "C28",
        position: "IndexAssign.object of Rc type",
        class: Class::MustNotMoveRejected("E0304"),
        src: "nogc fn f(x: &Rc<Vec<i64>>) -> i64 {\n\
              \x20   (*x)[0] = 1\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    // because the typetable cannot resolve one, so a nogc signature mentioning it is accepted.
    Row {
        id: "C29",
        position: "F3 inert, a generic struct by value",
        class: Class::MustNotMoveAccepted,
        src: F3_GENERIC_STRUCT_BY_VALUE,
        twin: Some(INERT_TWIN(F3_GENERIC_STRUCT_BY_VALUE)),
    },
    Row {
        id: "C30",
        position: "F3 inert, a generic struct by reference",
        class: Class::MustNotMoveAccepted,
        src: F3_GENERIC_STRUCT_BY_REF,
        twin: Some(INERT_TWIN(F3_GENERIC_STRUCT_BY_REF)),
    },
    Row {
        id: "C31",
        position: "F3 inert, a generic struct as the return type",
        class: Class::MustNotMoveAccepted,
        src: F3_GENERIC_STRUCT_AS_RETURN,
        twin: Some(INERT_TWIN(F3_GENERIC_STRUCT_AS_RETURN)),
    },
    Row {
        id: "C32",
        position: "F3 inert, a generic enum carrying a Vec",
        class: Class::MustNotMoveAccepted,
        src: F3_GENERIC_ENUM,
        twin: Some(INERT_TWIN(F3_GENERIC_ENUM)),
    },
    Row {
        id: "C33",
        position: "F3 inert, an explicit nogc bound, uncalled",
        class: Class::MustNotMoveAccepted,
        src: F3_EXPLICIT_NOGC_BOUND,
        twin: Some(INERT_TWIN(F3_EXPLICIT_NOGC_BOUND)),
    },
    // c34 to c37 are the callable side, and they are the three fences the a6 booking names.
    Row {
        id: "C34",
        position: "F3 callable, the call site of a nogc generic",
        class: Class::MustNotMoveRejected("E0730"),
        src: "struct G<T> { n: T }\n\
              nogc fn f<T>(g: G<T>) -> i64 { return 0 }\n\
              fn main() -> i64 {\n\
              \x20   let g: G<i64> = G{n: 1}\n\
              \x20   return f(g)\n\
              }\n",
        twin: None,
    },
    Row {
        id: "C35",
        position: "F3 callable, a concrete signature over a managed carrier",
        class: Class::MustNotMoveRejected("E0412"),
        src: "struct G<T> { n: Vec<T> }\n\
              nogc fn f(g: G<i64>) -> i64 { return 0 }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C36",
        position: "F3 callable, the declaration of a carrier holding a Vec",
        class: Class::MustNotMoveRejected("E0410"),
        src: "struct G { n: Vec<i64> }\n\
              nogc fn f(g: G) -> i64 { return 0 }\n\
              fn main() -> i64 { return 0 }\n",
        twin: None,
    },
    Row {
        id: "C37",
        position: "F3 callable, the literal of a carrier holding an Rc",
        class: Class::MustNotMoveRejected("E0410"),
        src: "struct G<T> { n: T }\n\
              fn main() -> i64 {\n\
              \x20   let g: G<Rc<i64>> = G{n: Rc::new(1)}\n\
              \x20   return 0\n\
              }\n",
        twin: None,
    },
    Row {
        id: "C38",
        position: "Reference.operand through an Index into a Vec, then a read-only local",
        class: Class::MustNotMoveAccepted,
        src: "nogc fn peek_local(r: &Vec<i64>) -> i64 {\n\
              \x20   let p: &i64 = &(*r)[0]\n\
              \x20   return *p\n\
              }\n\
              fn main() -> i64 {\n\
              \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
              \x20   println(peek_local(&v))\n\
              \x20   return 0\n\
              }\n",
        twin: Some(Twin {
            src: "nogc fn peek_local(r: &Vec<i64>) -> i64 {\n\
                  \x20   let p: &i64 = &(*r)[0]\n\
                  \x20   return *p\n\
                  }\n\
                  fn main() -> i64 {\n\
                  \x20   let v: Vec<i64> = vec[7919, 2, 3]\n\
                  \x20   println(peek_local(&v))\n\
                  \x20   return 0\n\
                  }\n",
            stdout: "7919\n",
            exit: 0,
            allocs: 1,
            frees: 1,
        }),
    },
];

const F3_GENERIC_STRUCT_BY_VALUE: &str = "struct G<T> { n: T }\n\
                                          nogc fn f<T>(g: G<T>) -> i64 { return 0 }\n\
                                          fn main() -> i64 { return 0 }\n";
const F3_GENERIC_STRUCT_BY_REF: &str = "struct G<T> { n: T }\n\
                                        nogc fn f<T>(g: &G<T>) -> i64 { return 0 }\n\
                                        fn main() -> i64 { return 0 }\n";
const F3_GENERIC_STRUCT_AS_RETURN: &str = "struct G<T> { n: T }\n\
                                           nogc fn f<T>(g: G<T>) -> G<T> { return g }\n\
                                           fn main() -> i64 { return 0 }\n";
const F3_GENERIC_ENUM: &str = "enum E<T> { One(Vec<T>), None }\n\
                               nogc fn f<T>(e: E<T>) -> i64 { return 0 }\n\
                               fn main() -> i64 { return 0 }\n";
const F3_EXPLICIT_NOGC_BOUND: &str = "struct G<T> { n: T }\n\
                                      nogc fn f<T: nogc>(g: G<T>) -> i64 { return 0 }\n\
                                      fn main() -> i64 { return 0 }\n";

#[allow(non_snake_case)]
const fn INERT_TWIN(src: &'static str) -> Twin {
    Twin {
        src,
        stdout: "",
        exit: 0,
        allocs: 0,
        frees: 0,
    }
}

// group d. the store-side cow detach under a projection, now fenced.

#[derive(Clone, Copy)]
enum Verdict {
    Fenced(&'static str),
    /// still compiles, and the detach is observable in the numbers
    Runs {
        stdout: &'static str,
        allocs: i64,
        frees: i64,
    },
}

#[derive(Clone, Copy)]
struct Store {
    id: &'static str,
    position: &'static str,
    src: &'static str,
    verdict: Verdict,
    /// what the store owes the day the fence is retired
    owed: &'static str,
}

const GROUP_D: &[Store] = &[
    Store {
        id: "D01",
        position: "Field under Index, static index",
        src: "struct S { n: i64 }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   v[0].n = 101\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the detach now fires under a projection, and this is what it owed",
    },
    Store {
        id: "D02",
        position: "Field under Field under Index",
        src: "struct I { n: i64 }\n\
              struct S { i: I }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{i: I{n: 7919}}, S{i: I{n: 2}}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   v[0].i.n = 101\n\
              \x20   println(v[0].i.n)\n\
              \x20   println(w[0].i.n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the detach now fires under a projection, and this is what it owed",
    },
    Store {
        id: "D03",
        position: "Index under Field under Index, an array field of an element",
        src: "struct S { a: [i64; 2] }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{a: [7919, 1]}, S{a: [2, 3]}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   v[0].a[0] = 101\n\
              \x20   println(v[0].a[0])\n\
              \x20   println(w[0].a[0])\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the detach now fires under a projection, and this is what it owed",
    },
    Store {
        id: "D04",
        position: "Index under Index, a Vec of arrays",
        src: "fn main() -> i64 {\n\
              \x20   let mut v: Vec<[i64; 2]> = vec[[7919, 1], [2, 3]]\n\
              \x20   let w: Vec<[i64; 2]> = v\n\
              \x20   v[0][0] = 101\n\
              \x20   println(v[0][0])\n\
              \x20   println(w[0][0])\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the detach now fires under a projection, and this is what it owed",
    },
    Store {
        id: "D05",
        position: "Field under Index through a &mut parameter",
        src: "struct S { n: i64 }\n\
              fn poke(r: &mut Vec<S>) -> i64 {\n\
              \x20   (*r)[0].n = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   let q = poke(&mut v)\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the detach now fires under a projection, and this is what it owed",
    },
    Store {
        id: "D06",
        position: "Field under Index, compound store",
        src: "struct S { n: i64 }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   v[0].n = v[0].n + 1\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "7920\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the detach now fires under a projection, and this is what it owed",
    },
    Store {
        id: "D07",
        position: "Field under Index, dynamic index",
        src: "struct S { n: i64 }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   let mut i: i64 = 0\n\
              \x20   v[i].n = 101\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the detach now fires under a projection, and this is what it owed",
    },
    // the two controls. same buffer, same sharing, same shape, and the detach does fire, so the
    Store {
        id: "D08",
        position: "control, whole element store",
        src: "struct S { n: i64 }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   v[0] = S{n: 101}\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the whole-element store already detaches",
    },
    Store {
        id: "D09",
        position: "control, whole element store through a &mut parameter",
        src: "struct S { n: i64 }\n\
              fn poke(r: &mut Vec<S>) -> i64 {\n\
              \x20   (*r)[0] = S{n: 101}\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   let q = poke(&mut v)\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: the whole-element store already detaches",
    },
];

#[test]
fn the_requirement_table_is_pinned_in_both_phases() {
    use Class::*;
    use Phase::*;
    assert_eq!(
        required_verdict(MustBecomeAccepted, PreNarrowing),
        Some("E0727")
    );
    assert_eq!(required_verdict(MustBecomeAccepted, PostNarrowing), None);
    assert_eq!(
        required_verdict(MustStayRejected, PreNarrowing),
        Some("E0727")
    );
    assert_eq!(
        required_verdict(MustStayRejected, PostNarrowing),
        Some("E0727")
    );
    assert_eq!(required_verdict(MustNotMoveAccepted, PreNarrowing), None);
    assert_eq!(required_verdict(MustNotMoveAccepted, PostNarrowing), None);
    assert_eq!(
        required_verdict(MustNotMoveRejected("E0304"), PostNarrowing),
        Some("E0304")
    );
    assert_eq!(
        required_verdict(MustStayRejectedBehind("E0429"), PreNarrowing),
        Some("E0429")
    );
    assert_eq!(
        required_verdict(MustStayRejectedBehind("E0429"), PostNarrowing),
        Some("E0429")
    );
}

#[test]
fn group_a_must_become_accepted() {
    run_rows(GROUP_A, 33);
}

#[test]
fn group_b_must_stay_rejected() {
    run_rows(GROUP_B, 6);
}

#[test]
fn group_c_must_not_move() {
    run_rows(GROUP_C, 24);
}

// the a6 hook, re-aimed rather than retired. the fence is gone because its obligation was
#[test]
fn the_fence_is_discharged_and_what_replaced_it_is_named() {
    use aelys_common::diagnostic::registry;
    assert!(
        registry::lookup("E0429").is_none(),
        "E0429 must be gone from the registry: stage 3 emits the detach under a projection, so \
         the fence has nothing left to stand in for"
    );
    assert!(
        registry::lookup("E0426").is_none(),
        "E0426 must be gone from the registry: its obligation was discharged at the formation of \
         the view, where the `Vec` header is still in hand, and B26/B27 carry E0727 now"
    );
    let root = repo_root();
    let bir = fs::read_to_string(root.join("air/src/bir/loans.rs")).expect("bir loans");
    assert!(
        !bir.contains("E0429"),
        "the bir decided this fence because the projection chain still existed there; the \
         decision moved to air lowering, and two live mechanisms could double-fire"
    );
    let air = fs::read_to_string(root.join("air/src/lib.rs")).expect("air");
    let place = air
        .split("pub enum Place {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("the air place enum");
    let variants = place
        .lines()
        .filter(|l| l.starts_with("    ") && !l.trim_start().starts_with("//"))
        .count();
    // false: e0429 retired with the place still flat at 5 variants, because the detach is
    assert_eq!(
        variants, 5,
        "the air place gained or lost a variant. a place that carries the chain would let the \
         consumer detach too, and two mechanisms deciding one detach can double-fire"
    );
    let stores =
        fs::read_to_string(root.join("codegen/src/lowering/stmts.rs")).expect("store lowering");
    assert!(
        !stores.contains("fn vec_root_of("),
        "vec_root_of was the detach's only discriminator and stage 3's job was to change it; it \
         is deleted, and `run3_stage3_cow_tests::s3_25` is where its replacement is pinned"
    );
}

// group would read green off a fence alone, so the kept form is executed rather than asserted
fn run_stores(group: &[Store], expected: (usize, usize), expected_legs: usize) {
    let h = Harness::new();
    let mut fenced = 0;
    let mut runs = 0;
    for m in group {
        let (stdout, allocs, frees) = match m.verdict {
            Verdict::Fenced(code) => {
                for (tag, opt) in LEVELS {
                    let rendered = h.reject(m.id, tag, m.src, *opt);
                    assert!(
                        rendered.contains(&format!("[{code}]")),
                        "{} ({}) at {tag} MUST be refused by {code} specifically; it owed {}\ngot:\n{rendered}",
                        m.id,
                        m.position,
                        m.owed
                    );
                }
                fenced += 1;
                continue;
            }
            Verdict::Runs {
                stdout,
                allocs,
                frees,
            } => (stdout, allocs, frees),
        };
        for (tag, opt) in LEVELS {
            let Some(exe) = h.compile(m.id, tag, m.src, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{} could not link, so this group has no runtime evidence; \
                     set AELYS_ALLOW_LINKER_SKIP=1 to declare that on purpose",
                    m.id
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let o = h.run(&exe, *alloc);
                assert_eq!(
                    o.exit, 0,
                    "{} at {tag}/{alloc_name} must run\nstderr:\n{}",
                    m.id, o.stderr
                );
                assert_eq!(
                    o.stdout, stdout,
                    "{} ({}) at {tag}/{alloc_name}: it owed {}\nsource:{}",
                    m.id, m.position, m.owed, m.src
                );
                let (a, f) = o.stats.unwrap_or_else(|| {
                    panic!("{} at {tag}/{alloc_name}: no [rc] stats line", m.id)
                });
                assert_eq!(
                    (a, f),
                    (allocs, frees),
                    "{} ({}) at {tag}/{alloc_name}: allocs/frees moved, so the detach this row \
                     turns on changed behaviour",
                    m.id,
                    m.position
                );
            }
        }
        runs += 1;
    }
    assert_eq!(
        (fenced, runs),
        expected,
        "the fenced/running split moved; if it did, say which row and why"
    );
    assert_eq!(
        h.compiled_legs.get(),
        expected_legs,
        "this group must compile and run exactly {expected_legs} executable legs"
    );
}

#[test]
fn group_d_the_cow_detach_under_a_projection() {
    run_stores(GROUP_D, (0, 9), 27);
}

#[test]
fn group_l_the_same_stores_inside_a_lambda_body() {
    run_stores(GROUP_L, (1, 4), 12);
}

// const with no bir trace (`air/src/bir/build.rs:504`), so the fence cannot see a store written

const GROUP_L: &[Store] = &[
    Store {
        id: "L01",
        position: "Field under Index, inside a lambda body",
        src: "struct S { n: i64 }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   let g = fn(r: &mut Vec<S>) -> i64 {\n\
              \x20       (*r)[0].n = 101\n\
              \x20       return 0\n\
              \x20   }\n\
              \x20   let q = g(&mut v)\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing, and the debt was discharged rather than deferred: the condition this \
               row was waiting for was never met, the detach moved to air lowering instead, \
               which traverses lambda bodies",
    },
    Store {
        id: "L02",
        position: "control, whole element inside the same lambda",
        src: "struct S { n: i64 }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   let g = fn(r: &mut Vec<S>) -> i64 {\n\
              \x20       (*r)[0] = S{n: 101}\n\
              \x20       return 0\n\
              \x20   }\n\
              \x20   let q = g(&mut v)\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "already correct, and it is what proves the lambda is not the cause",
    },
    Store {
        id: "L03",
        position: "the same store one frame out, at top level",
        src: "struct S { n: i64 }\n\
              fn poke(r: &mut Vec<S>) -> i64 {\n\
              \x20   (*r)[0].n = 101\n\
              \x20   return 0\n\
              }\n\
              fn main() -> i64 {\n\
              \x20   let mut v: Vec<S> = vec[S{n: 7919}, S{n: 2}]\n\
              \x20   let w: Vec<S> = v\n\
              \x20   let q = poke(&mut v)\n\
              \x20   println(v[0].n)\n\
              \x20   println(w[0].n)\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "101\n7919\n",
            allocs: 2,
            frees: 2,
        },
        owed: "nothing: there is no boundary left to escape across, which is what closed L01",
    },
    Store {
        id: "L04",
        position: "nogc reaching the lambda that stores",
        src: "struct S { n: i64 }\n\
              nogc fn f(r: &mut Vec<S>) -> i64 {\n\
              \x20   let g = fn(q: &mut Vec<S>) -> i64 {\n\
              \x20       (*q)[0].n = 101\n\
              \x20       return 0\n\
              \x20   }\n\
              \x20   return g(r)\n\
              }\n\
              fn main() -> i64 { return 0 }\n",
        verdict: Verdict::Fenced("E0727"),
        owed: "nothing, the indirect call keeps nogc out and is why this is not gating",
    },
    Store {
        id: "L05",
        position: "the lambda body is not walked at all, uncalled",
        src: "struct S { n: i64 }\n\
              nogc fn f() -> i64 {\n\
              \x20   let g = fn(q: &mut Vec<S>) -> i64 {\n\
              \x20       (*q)[0].n = 101\n\
              \x20       return 0\n\
              \x20   }\n\
              \x20   return 7\n\
              }\n\
              fn main() -> i64 {\n\
              \x20   println(f())\n\
              \x20   return 0\n\
              }\n",
        verdict: Verdict::Runs {
            stdout: "7\n",
            allocs: 0,
            frees: 0,
        },
        owed: "nothing today, and it is the sharpest A14 witness: a store E0429 refuses one \
               frame out is not even seen here",
    },
];

#[test]
fn the_corpus_census_is_exact() {
    let mut becomes = 0;
    let mut stay = 0;
    let mut still_accepted = 0;
    let mut still_rejected = 0;
    let mut behind = 0;
    let mut only_fence = 0;
    let mut inert = 0;
    let mut validator = 0;
    let mut owed_rows = 0;
    let mut ids: Vec<&str> = Vec::new();
    for row in GROUP_A.iter().chain(GROUP_B).chain(GROUP_C) {
        ids.push(row.id);
        assert!(
            !row.position.is_empty(),
            "{} must name the position it occupies",
            row.id
        );
        match row.class {
            Class::MustBecomeAccepted => becomes += 1,
            Class::MustStayRejected => stay += 1,
            Class::MustNotMoveAccepted => still_accepted += 1,
            Class::MustNotMoveRejected(_) => still_rejected += 1,
            Class::MustStayRejectedBehind(_) => behind += 1,
            Class::OnlyTheFenceHolds(_) => only_fence += 1,
            Class::AcceptedButInert { .. } => inert += 1,
            Class::MovesToTheValidator => validator += 1,
            Class::AcceptedAndOwed(owed) => {
                assert!(!owed.is_empty(), "{} must say what it owes", row.id);
                owed_rows += 1;
            }
        }
    }
    let total = ids.len();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "row ids must be unique");
    assert_eq!(
        (
            becomes,
            stay,
            behind,
            only_fence,
            still_accepted,
            still_rejected,
            inert,
            validator,
            owed_rows,
            total
        ),
        (31, 27, 0, 0, 16, 30, 4, 1, 0, 109),
        "the class split changed"
    );
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn variant_names(src: &str, enum_name: &str) -> Vec<String> {
    let head = format!("pub enum {enum_name} {{");
    let start = src
        .find(&head)
        .unwrap_or_else(|| panic!("{enum_name} not found"))
        + head.len();
    let body = &src[start..];
    let end = body.find("\n}").expect("enum end");
    body[..end]
        .lines()
        .filter_map(|l| {
            let t = l.strip_prefix("    ")?;
            let mut c = t.chars();
            if !c.next()?.is_ascii_uppercase() {
                return None;
            }
            let name: String = t
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// keyed by the variant that owns it. `chain_coverage` is the second: the sequence of place
const SLOT_COVERAGE: &[(&str, &str)] = &[
    ("Int", "excluded: a literal cannot carry a managed type"),
    ("Float", "excluded: a literal cannot carry a managed type"),
    ("Bool", "excluded: a literal cannot carry a managed type"),
    (
        "String",
        "excluded: a string literal local is accepted in nogc, measured; the managed site \
         for strings is Binary with a String operand, row B23",
    ),
    ("Null", "excluded: a literal cannot carry a managed type"),
    ("FmtString", "B13"),
    ("Identifier", "B08 B09"),
    ("Binary", "A10 B23"),
    ("Unary", "A11"),
    ("And", "A20"),
    ("Or", "A30"),
    ("Call", "A09 B07 B12 C04 C05"),
    ("Assign", "C17"),
    ("Grouping", "A06"),
    ("If", "A27 B06"),
    ("Lambda", "B14 C08 C09 C25"),
    ("LambdaInner", "B14 C08 C09 C25"),
    ("Member", "A02 A03 A24 A33 A34 B17 B18 B19 B28 C01 C11"),
    ("ArrayLiteral", "C21"),
    ("ArraySized", "A23"),
    ("VecLiteral", "B09"),
    ("Index", "A01 A16 A31 A35 B24 C03 C10"),
    ("IndexAssign", "B01 B02 B21 B25 B26 C28"),
    ("FieldAssign", "A04 A17 B20 B27 C02 C14"),
    ("Range", "A15"),
    ("Slice", "A08 A26 A34 A35 B26 B27 C19"),
    (
        "Reference",
        "A21 A22 A25 B29 B30 B31 B32 B33 B34 B35 B36 C24 C38",
    ),
    ("Deref", "A01 C12"),
    ("DerefAssign", "B03 B04 B29 B32 C15"),
    ("StructLiteral", "C22"),
    ("Cast", "A07"),
    ("EnumVariant", "B10 B11"),
    ("Match", "C18"),
    ("ResultAssert", "B22 C26"),
    ("Block", "A19 A28"),
    ("stmt.Expression", "A18 B18"),
    ("stmt.Let", "A05 B09 B19"),
    ("stmt.Block", "A19"),
    ("stmt.If", "A12"),
    ("stmt.While", "A13"),
    ("stmt.For", "A14 A29"),
    ("stmt.ForEach", "C13"),
    ("stmt.Return", "A01 B05 B17 C16"),
    (
        "stmt.Break",
        "excluded: a unit variant, it carries no expression",
    ),
    (
        "stmt.Continue",
        "excluded: a unit variant, it carries no expression",
    ),
    ("stmt.Function", "B15"),
    (
        "stmt.Needs",
        "excluded: NeedsStmt is path, kind and span, checked against the definition rather than \
         against one spelling",
    ),
    (
        "stmt.StructDecl",
        "excluded: its fields are names and InferTypes, no expression; the Vec-field case is \
         row C27",
    ),
    (
        "stmt.EnumDecl",
        "excluded: its variants are names, tags and InferTypes, no expression",
    ),
];

/// three bounds are executable rather than argued. `deref` cannot repeat: a second dereference
const CHAIN_COVERAGE: &[(&str, &str)] = &[
    ("Deref", "B03 B05 B07"),
    ("Deref.Field", "A02 A04 A17 B17 B18 B19 B28 C14"),
    ("Deref.Field.Field", "A03"),
    ("Deref.Index", "A01 A16 B01 B02 B21"),
    ("Deref.Index.Field", "A24 A25 A33 B20 D05"),
    ("Deref.Index.Index", "B24 B25 C28 D03 D04"),
    ("Deref.Slice", "A08 A26"),
    ("Deref.Slice.Index", "A31 A32 A34 A35 B26 B27"),
];

#[test]
fn the_chain_space_is_bounded_by_rows_that_exist() {
    let ids: Vec<&str> = GROUP_A
        .iter()
        .chain(GROUP_B)
        .chain(GROUP_C)
        .map(|r| r.id)
        .collect();
    let d_ids: Vec<&str> = GROUP_D.iter().map(|m| m.id).collect();
    for (chain, rows) in CHAIN_COVERAGE {
        assert!(!rows.is_empty(), "chain {chain} has no row");
        for id in rows.split(' ') {
            assert!(
                ids.contains(&id) || d_ids.contains(&id),
                "chain {chain} cites row {id}, which does not exist"
            );
        }
    }
    for bound in ["C12", "C27", "B24", "A31"] {
        assert!(
            ids.contains(&bound),
            "{bound} bounds the chain space and must exist"
        );
    }
    assert_eq!(
        CHAIN_COVERAGE.len(),
        8,
        "the number of shape classes this corpus commits to changed. this is a classification, \
         not a derived bound: nothing computes a program's real chain, so a ninth shape is found \
         by writing one, not by this number moving"
    );
}

#[test]
fn the_slot_coverage_map_is_total_over_the_variant_list() {
    let root = repo_root();
    let ast = fs::read_to_string(root.join("sema/src/typed_ast/mod.rs")).expect("typed_ast");
    let mut expected: Vec<String> = variant_names(&ast, "TypedExprKind");
    expected.extend(
        variant_names(&ast, "TypedStmtKind")
            .into_iter()
            .map(|v| format!("stmt.{v}")),
    );
    let mut mapped: Vec<String> = SLOT_COVERAGE
        .iter()
        .map(|(k, _)| (*k).to_string())
        .collect();
    let mut want = expected.clone();
    mapped.sort();
    want.sort();
    assert_eq!(
        mapped, want,
        "the coverage map and the variant list disagree, so a position is unclassified"
    );

    let ids: Vec<&str> = GROUP_A
        .iter()
        .chain(GROUP_B)
        .chain(GROUP_C)
        .map(|r| r.id)
        .collect();
    let mut excluded = 0;
    for (variant, note) in SLOT_COVERAGE {
        if let Some(reason) = note.strip_prefix("excluded: ") {
            assert!(!reason.is_empty(), "{variant} is excluded without a reason");
            excluded += 1;
            continue;
        }
        for id in note.split(' ') {
            assert!(
                ids.contains(&id),
                "{variant} cites row {id}, which does not exist"
            );
        }
    }
    assert_eq!(excluded, 10, "the number of declared exclusions changed");
}

#[test]
fn the_language_defined_position_space_is_the_variant_list() {
    let root = repo_root();
    let ast = fs::read_to_string(root.join("sema/src/typed_ast/mod.rs")).expect("typed_ast");
    let exprs = variant_names(&ast, "TypedExprKind");
    let stmts = variant_names(&ast, "TypedStmtKind");
    assert_eq!(
        (exprs.len(), stmts.len()),
        (35, 14),
        "the typed ast variant list moved:\nexpr {exprs:?}\nstmt {stmts:?}"
    );
    for required in [
        "Deref",
        "DerefAssign",
        "Index",
        "IndexAssign",
        "Member",
        "FieldAssign",
        "Slice",
        "Reference",
        "Grouping",
        "Cast",
        "LambdaInner",
        "Match",
    ] {
        assert!(
            exprs.iter().any(|v| v == required),
            "{required} must still be a TypedExprKind variant"
        );
    }
}

#[test]
fn the_effect_decision_surface_is_one_file() {
    let root = repo_root();
    let effects = fs::read_to_string(root.join("air/src/bir/effects.rs")).expect("effects.rs");
    assert_eq!(
        effects.matches("Effect::Managed").count(),
        10,
        "the number of mentions of the managed effect changed. this is a drift detector over the \
         whole file, not a count of inserting sites"
    );
    assert_eq!(
        effects.matches("set.insert(Effect::Managed)").count(),
        7,
        "the inserting sites are the parameter rule, alloc_managed, the managed-local rule, the \
         node rule, the store clause's two arms and the mutable-view clause. the other three \
         mentions read or shift the bit"
    );
    assert!(
        effects.contains("if pos == Pos::Value && category(&expr.ty, tt) == Category::Managed {"),
        "the per-node type test this corpus is written against is gone or reshaped"
    );
    assert!(
        effects.contains("TypedExprKind::LambdaInner { captures, .. }"),
        "the lambda arm changed shape; re-measure whether the body is walked"
    );
}

#[test]
fn the_ablation_matrix_is_stated_over_rows_that_exist() {
    let root = repo_root();
    let guard =
        fs::read_to_string(root.join("scripts/s4_retro_discrimination.sh")).expect("guard script");
    let extractor =
        fs::read_to_string(root.join("scripts/extract_corpus.py")).expect("fixture extractor");
    assert!(
        extractor.contains("const %s: &[Row] = &["),
        "the guard must generate its fixtures from this file, not carry copies of the programs"
    );
    for group in ["GROUP_A", "GROUP_B", "GROUP_C"] {
        assert!(
            extractor.contains(group),
            "the fixture generator must read {group} too, or the matrix cannot see it move"
        );
    }
    assert!(
        guard.contains("scripts/extract_corpus.py"),
        "the guard must call the generator rather than embed fixtures"
    );

    let ids: Vec<&str> = GROUP_A
        .iter()
        .chain(GROUP_B)
        .chain(GROUP_C)
        .map(|r| r.id)
        .collect();
    let mut declared = 0;
    let mut becomes_accepted = 0;
    for table in guard.split("EXPECT_").skip(1) {
        let body = table
            .split_once("=\"")
            .and_then(|(_, rest)| rest.split("\"\n").next())
            .expect("an EXPECT_<id> table ends with a quote on its own");
        for token in body.split_whitespace() {
            let Some((id, verdict)) = token.split_once(':') else {
                continue;
            };
            assert!(
                ids.contains(&id),
                "the ablation matrix declares {id}, which is not a corpus row"
            );
            declared += 1;
            if verdict == "ACCEPT" && id.starts_with('B') {
                becomes_accepted += 1;
            }
        }
    }
    assert!(
        declared > 0,
        "the ablation matrix declares no moved row at all"
    );
    assert!(
        becomes_accepted > 0,
        "no ablation is declared to turn a must-stay-rejected row into an acceptance, so the \
         matrix cannot show a silent retain"
    );
}

#[test]
fn every_must_stay_rejected_row_names_e0727() {
    let h = Harness::new();
    let mut behind = 0;
    let mut only_fence = 0;
    let mut validator = 0;
    let mut owed_rows = 0;
    for row in GROUP_B {
        if let Class::AcceptedAndOwed(owed) = row.class {
            owed_rows += 1;
            h.accepts(row.id, "-O0", row.src, OptimizationLevel::None);
            assert!(
                owed.contains("&mut"),
                "{} must name the mechanism it waits on, not just a wish",
                row.id
            );
            continue;
        }
        let rendered = h.reject(row.id, "-O0", row.src, OptimizationLevel::None);
        match row.class {
            Class::MustStayRejected => assert!(
                rendered.contains("[E0727]"),
                "{} MUST be rejected by the effect system specifically:\n{rendered}",
                row.id
            ),
            // a fence in front of the node rule renders first, and the node rule is still firing
            Class::MustStayRejectedBehind(fence) => {
                behind += 1;
                assert!(
                    rendered.contains(&format!("[{fence}]")),
                    "{} MUST be refused by {fence}:\n{rendered}",
                    row.id
                );
                assert!(
                    rendered.contains("[E0727]"),
                    "{} must still be reached by the effect system underneath {fence}, \
                     otherwise the fence is the only thing holding a managed store:\n{rendered}",
                    row.id
                );
            }
            Class::OnlyTheFenceHolds(fence) => {
                only_fence += 1;
                assert!(
                    rendered.contains(&format!("[{fence}]")),
                    "{} MUST be refused by {fence}:\n{rendered}",
                    row.id
                );
                if PHASE == Phase::PostNarrowing {
                    assert!(
                        !rendered.contains("[E0727]"),
                        "{} is now held by the effect system too, so {fence} is no longer the \
                         only thing refusing this store and its A6 booking must be \
                         rewritten:\n{rendered}",
                        row.id
                    );
                }
            }
            Class::MovesToTheValidator => {
                validator += 1;
                let want = required_verdict(row.class, PHASE).expect("still rejected");
                assert!(
                    rendered.contains(&format!("[{want}]")),
                    "{} MUST be refused by {want}:\n{rendered}",
                    row.id
                );
            }
            Class::AcceptedAndOwed(owed) => {
                owed_rows += 1;
                assert!(
                    owed.contains("&mut"),
                    "{} must name the mechanism it waits on, not just a wish",
                    row.id
                );
            }
            // the row says so instead of a fence quietly covering for it
            Class::MustNotMoveRejected(code) => {
                assert!(
                    rendered.contains(&format!("[{code}]")),
                    "{} MUST be refused by {code}:\n{rendered}",
                    row.id
                );
            }
            _ => panic!(
                "{} is in group b and must carry a stay-rejected class",
                row.id
            ),
        }
    }
    assert_eq!(
        (behind, only_fence, validator, owed_rows),
        (0, 0, 1, 0),
        "the split of group b rows not held by the effect system alone changed"
    );
}

// fail once the narrowing was implemented and never on a phase flip at head
#[test]
fn the_a13_counter_witnesses_all_reach_the_effect_rule() {
    let h = Harness::new();
    for id in ["A04", "B03", "B05"] {
        let row = GROUP_A
            .iter()
            .chain(GROUP_B)
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("{id} must exist"));
        match required_verdict(row.class, PHASE) {
            Some(code) => {
                assert_eq!(
                    code, "E0727",
                    "{id} is held by the effect system or by nothing"
                );
                let rendered = h.reject(id, "-O0", row.src, OptimizationLevel::None);
                assert!(
                    rendered.contains("[E0727]"),
                    "{id} must reach the effect rule, not a fence:\n{rendered}"
                );
            }
            None => h.accepts(id, "-O0", row.src, OptimizationLevel::None),
        }
    }
}

// caller can be built, so the day the fence is relaxed this fails, the row fails with it, and the
#[test]
fn the_inert_rows_have_no_caller_and_their_plain_twin_runs() {
    let h = Harness::new();
    let mut rows = 0;
    let mut refusals = 0;
    for row in GROUP_A.iter().chain(GROUP_B).chain(GROUP_C) {
        let Class::AcceptedButInert { fence, callers } = row.class else {
            continue;
        };
        rows += 1;
        assert!(
            !callers.is_empty(),
            "{} names no construction route",
            row.id
        );
        for (n, caller) in callers.iter().enumerate() {
            for (tag, opt) in LEVELS {
                let id = format!("{}_caller{}", row.id, n);
                let rendered = h.reject(&id, tag, caller, *opt);
                assert!(
                    rendered.contains(&format!("[{fence}]")),
                    "{} at {tag}: this construction route must still be refused by {fence}, or \
                     the row is producible and owes a runtime twin:\n{caller}\ngot:\n{rendered}",
                    id
                );
            }
            refusals += 1;
        }
    }
    assert_eq!(rows, 4, "the number of inert rows changed");
    assert_eq!(
        refusals, 8,
        "the number of asserted construction routes changed"
    );

    let (src, stdout, allocs, frees) = CONTROL_PLAIN_ELEMENT;
    let mut legs = 0;
    for (tag, opt) in LEVELS {
        let Some(exe) = h.compile("inert_control", tag, src, *opt) else {
            assert!(
                linker_skip_declared(),
                "the inert class's control could not link, so the fence reads as blanket when it \
                 is element-type-specific; set AELYS_ALLOW_LINKER_SKIP=1 to declare that"
            );
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run(&exe, *alloc);
            assert_eq!(o.exit, 0, "the control must run at {tag}/{alloc_name}");
            assert_eq!(
                o.stdout, stdout,
                "the control's answer moved at {tag}/{alloc_name}"
            );
            let (a, f) = o
                .stats
                .unwrap_or_else(|| panic!("the control has no [rc] stats at {tag}/{alloc_name}"));
            assert_eq!(
                (a, f),
                (allocs, frees),
                "the control's allocs/frees moved at {tag}/{alloc_name}"
            );
            legs += 1;
        }
    }
    assert_eq!(
        legs, 6,
        "the control must run at three levels and two allocators"
    );
}

#[test]
fn the_a9_transfer_witness_discriminates() {
    const READ: &str = "nogc fn get(r: &Vec<i64>) -> i64 { return (*r)[0] }\n\
                        fn main() -> i64 { return 0 }\n";
    const TRANSFER: &str = "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
                            \x20   (*r)[0] = 101\n\
                            \x20   return 0\n\
                            }\n\
                            fn main() -> i64 { return 0 }\n";
    let h = Harness::new();
    for (tag, opt) in LEVELS {
        let rendered = h.reject("W_transfer_nogc", tag, TRANSFER, *opt);
        assert!(
            rendered.contains("[E0727]"),
            "the transfer half must be refused by the effect system at {tag}:\n{rendered}"
        );
        assert!(
            rendered.contains("a store into a buffer that may be shared"),
            "the witness must name the operation at {tag}, not the operand's type:\n{rendered}"
        );
        assert!(
            !rendered.contains("a managed value"),
            "the witness fell back to the type test at {tag}, so the pair stops \
             discriminating:\n{rendered}"
        );
        match PHASE {
            Phase::PostNarrowing => h.accepts("W_read_nogc", tag, READ, *opt),
            Phase::PreNarrowing => {
                h.reject("W_read_nogc", tag, READ, *opt);
            }
        }
    }
    assert!(
        GROUP_A.iter().any(|r| r.id == "A01" && r.twin.is_some()),
        "the read half's runtime twin must still exist"
    );
    assert!(
        GROUP_C.iter().any(|r| r.id == "C07" && r.twin.is_some()),
        "the transfer half's runtime twin must still exist"
    );
}

#[test]
fn the_fail_open_category_boundary_is_where_the_scans_stop() {
    let root = repo_root();
    let tt = fs::read_to_string(root.join("sema/src/types/type_table.rs")).expect("type table");
    assert_eq!(
        tt.matches("type_params.is_empty()").count(),
        4,
        "the early-outs that make a generic nominal type read as Copy moved. F3's inert rows are \
         representatives of the class they bound, so the row set has to be re-decided"
    );
    for scan in ["fn scan_vec_by_value(", "fn scan_rc_nominal("] {
        assert!(
            tt.contains(scan),
            "{scan} is what `category` consults; if it is gone, say what decides the category now"
        );
    }
    // the three fences the a6 booking names, and the rows that hold them
    for (code, row) in [("E0730", "C34"), ("E0412", "C35"), ("E0410", "C36")] {
        let held = GROUP_C
            .iter()
            .find(|r| r.id == row)
            .unwrap_or_else(|| panic!("{row} must exist"));
        assert!(
            matches!(held.class, Class::MustNotMoveRejected(c) if c == code),
            "{row} must still be the {code} route into the fail-open category"
        );
    }
    assert!(
        aelys_common::diagnostic::registry::lookup("E0410").is_none(),
        "E0410 gained an --explain entry; that is an improvement, and this row is where the gap          was recorded, so retire it here"
    );
    for code in ["E0730", "E0412"] {
        assert!(
            aelys_common::diagnostic::registry::lookup(code).is_some(),
            "{code} fences something this stage books under A6 and must keep its --explain entry"
        );
    }
    assert!(
        aelys_common::diagnostic::registry::lookup("E0429").is_none(),
        "E0429 came back; the store-side detach under a projection is emitted in air lowering \
         now, so a second mechanism refusing the same form is a double hold, not a fence"
    );
}

