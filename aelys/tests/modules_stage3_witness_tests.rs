use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

type Files = &'static [(&'static str, &'static str)];

fn stage(files: Files) -> TempDir {
    let dir = tempdir().expect("tempdir");
    for (name, body) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture directory");
        }
        fs::write(&path, body).expect("write fixture");
    }
    dir
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

fn message_only(rendered: &str) -> String {
    split_rendering(rendered).0
}

fn location_only(rendered: &str) -> String {
    split_rendering(rendered).1
}

fn split_rendering(rendered: &str) -> (String, String) {
    let (mut message, mut location) = (Vec::new(), Vec::new());
    for line in rendered.lines() {
        let t = line.trim_start();
        if t.starts_with("-->") || t.starts_with('|') || first_field_is_a_gutter(t) {
            location.push(line);
        } else {
            message.push(line);
        }
    }
    (message.join("\n"), location.join("\n"))
}

fn first_field_is_a_gutter(line: &str) -> bool {
    match line.split_once('|') {
        Some((head, _)) => !head.is_empty() && head.trim().chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

fn run_nogc_row(id: &str, files: Files, exit: i32) {
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        if let Err(err) = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
            panic!("{id} at {level}: MUST compile and link\nerror:\n{err}");
        }
        let out = Command::new(exe_path_for(&root))
            .env("AELYS_RC_STATS", "1")
            .output()
            .expect("run executable");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            exit_code(&out.status),
            exit,
            "{id} at {level}: the answer MUST be {exit}\nstderr:\n{stderr}"
        );
        let stats = parse_stats(&stderr)
            .unwrap_or_else(|| panic!("{id} at {level}: no rc stats were reported\n{stderr}"));
        assert_eq!(
            stats,
            (0, 0),
            "{id} at {level}: a nogc path across the boundary MUST allocate nothing"
        );
    }
}

fn run_row(id: &str, files: Files, exit: i32) {
    assert!(
        (0..256).contains(&exit),
        "{id}: an expected exit of {exit} cannot be observed through an 8-bit status"
    );
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        if let Err(err) = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
            panic!("{id} at {level}: MUST compile and link\nerror:\n{err}");
        }
        let exe = exe_path_for(&root);
        assert!(exe.is_file(), "{id} at {level}: no executable was produced");
        let out = Command::new(&exe).output().expect("run executable");
        assert_eq!(
            exit_code(&out.status),
            exit,
            "{id} at {level}: the answer MUST be {exit}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

// asserted beside it, or a compiler that keeps the print while leaking still passes
fn run_row_with_stdout(id: &str, files: Files, exit: i32, stdout: &str, stats: (i64, i64)) {
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        if let Err(err) = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
            panic!("{id} at {level}: MUST compile and link\nerror:\n{err}");
        }
        let out = Command::new(exe_path_for(&root))
            .env("AELYS_RC_STATS", "1")
            .output()
            .expect("run executable");
        assert_eq!(
            exit_code(&out.status),
            exit,
            "{id} at {level}: the answer MUST be {exit}"
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            stdout,
            "{id} at {level}: stdout MUST be {stdout:?}"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        let seen = parse_stats(&stderr)
            .unwrap_or_else(|| panic!("{id} at {level}: no rc stats were reported\n{stderr}"));
        assert_eq!(
            seen, stats,
            "{id} at {level}: the exact pair MUST be {stats:?}"
        );
    }
}

fn rejection(id: &str, files: Files, opt: OptimizationLevel) -> String {
    let dir = stage(files);
    let root = dir.path().join("root.aelys");
    match lower_file_to_air(&root, opt) {
        Ok(_) => panic!("{id}: MUST be rejected, but it was accepted"),
        Err(rendered) => rendered,
    }
}

// one level only: `bir::check` runs before the optimizer, so a rejection cannot move with -o.
fn reject_row(id: &str, files: Files, says: &[&str], points_at: &[&str], absent: &[&str]) {
    let rendered = rejection(id, files, OptimizationLevel::None);
    let message = message_only(&rendered);
    for needle in says {
        assert!(
            message.contains(needle),
            "{id}: the diagnostic MUST say {needle:?}\nrendered:\n{rendered}"
        );
    }
    let location = location_only(&rendered);
    for needle in points_at {
        assert!(
            location.contains(needle),
            "{id}: the diagnostic MUST point at {needle:?}\nrendered:\n{rendered}"
        );
    }
    for needle in absent {
        assert!(
            !rendered.contains(needle),
            "{id}: the diagnostic MUST NOT contain {needle:?}\nrendered:\n{rendered}"
        );
    }
}


const CLEAN: &str = "pub nogc fn clean(n: i64) -> i64 {\n    return n + 1\n}\n";
const PLAIN: &str = "pub fn plain(n: i64) -> i64 {\n    return n + 1\n}\n";
const ALLOCATES: &str =
    "pub fn allocates() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 0\n}\n";

#[test]
fn group_mod_e1_nogc_reaches_an_imported_nogc_function() {
    run_nogc_row(
        "E-1",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.clean(6)\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            ("m.aelys", CLEAN),
        ],
        7,
    );
}

#[test]
fn group_mod_e2_nogc_reaches_an_imported_effect_free_function() {
    run_nogc_row(
        "E-2",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.plain(7)\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            ("m.aelys", PLAIN),
        ],
        8,
    );
}

#[test]
fn group_mod_e6_nogc_reaches_the_trivial_spelling() {
    run_nogc_row(
        "E-6",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    m.tick()\n    return 0\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            ("m.aelys", "pub nogc fn tick() -> i64 {\n    return 0\n}\n"),
        ],
        0,
    );
}

#[test]
fn group_mod_e9_an_unreached_allocator_does_not_taint_its_module() {
    run_nogc_row(
        "E-9",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.plain(4)\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn plain(n: i64) -> i64 {\n    return n + 1\n}\n\npub fn allocates() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 0\n}\n",
            ),
        ],
        5,
    );
}

#[test]
fn group_mod_e3_a_reached_allocator_is_named_in_the_chain() {
    reject_row(
        "E-3",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.allocates()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            ("m.aelys", ALLOCATES),
        ],
        &["[E0727]", "pure -> m.allocates -> Vec::new"],
        &[],
        &["<indirect call>"],
    );
}

#[test]
fn group_mod_e4_a_two_hop_chain_names_its_first_hop() {
    reject_row(
        "E-4",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.middle()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "needs deep\n\npub fn middle() -> i64 {\n    return deep.allocates()\n}\n",
            ),
            ("deep.aelys", ALLOCATES),
        ],
        &["[E0727]", "pure -> m.middle -> deep.allocates -> Vec::new"],
        &[],
        &["<indirect call>"],
    );
}

#[test]
fn group_mod_e5_a_lying_nogc_is_caught_in_its_own_module() {
    reject_row(
        "E-5",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.leaky()\n}\n",
            ),
            (
                "m.aelys",
                "pub nogc fn leaky() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 0\n}\n",
            ),
        ],
        &["[E0727]", "leaky -> Vec::new"],
        &["m.aelys"],
        &[],
    );
}

#[test]
fn group_mod_e7_an_elided_allocation_still_carries_its_effect() {
    let files: Files = &[
        (
            "root.aelys",
            "needs m\n\nnogc fn pure() -> i64 {\n    return m.dead()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
        ),
        (
            "m.aelys",
            "pub fn dead() -> i64 {\n    if false {\n        let v: vec<i64> = Vec::new()\n        return 1\n    }\n    return 0\n}\n",
        ),
    ];
    for (level, opt) in LEVELS {
        let dir = stage(files);
        assert!(
            lower_file_to_air(&dir.path().join("root.aelys"), *opt).is_err(),
            "E-7 at {level}: an allocation the optimizer removes still carries its effect"
        );
    }
}

#[test]
fn group_mod_e8_a_lambda_inside_the_imported_module_still_taints_it() {
    reject_row(
        "E-8",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.outer()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn outer() -> i64 {\n    let f = fn() -> i64 {\n        let v: vec<i64> = Vec::new()\n        return 0\n    }\n    return f()\n}\n",
            ),
        ],
        &["[E0727]", "pure -> m.outer"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_e10_an_imported_nogc_function_is_a_nogc_value() {
    run_nogc_row(
        "E-10",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn call(f: nogc fn() -> i64) -> i64 {\n    return f()\n}\n\nfn main() -> i64 {\n    return call(m.clean)\n}\n",
            ),
            ("m.aelys", "pub nogc fn clean() -> i64 {\n    return 5\n}\n"),
        ],
        5,
    );
}

// non discriminating against modules/base: an unresolvable `m.allocates` was not a direct
#[test]
fn group_mod_e11_an_imported_allocator_is_not_a_nogc_value() {
    reject_row(
        "E-11",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn call(f: nogc fn() -> i64) -> i64 {\n    return f()\n}\n\nfn main() -> i64 {\n    return call(m.allocates)\n}\n",
            ),
            ("m.aelys", ALLOCATES),
        ],
        &["[E0729]"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_e12_an_effect_that_is_not_managed_does_not_close_the_boundary() {
    run_nogc_row(
        "E-12",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.div(6, 2)\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn div(a: i64, b: i64) -> i64 {\n    return a / b\n}\n",
            ),
        ],
        3,
    );
}

#[test]
fn group_mod_e13_an_allocation_spelt_rc_is_still_an_allocation() {
    reject_row(
        "E-13",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.boxed()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn boxed() -> i64 {\n    let r = Rc::new(1)\n    return 0\n}\n",
            ),
        ],
        &["[E0727]", "pure -> m.boxed -> Rc::new"],
        &[],
        &["<indirect call>"],
    );
}

#[test]
fn group_mod_e14_an_allocation_deeper_inside_the_module_still_taints_it() {
    reject_row(
        "E-14",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.feed()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "fn inner() -> i64 {\n    let xs: vec<i64> = Vec::new()\n    return 0\n}\n\npub fn feed() -> i64 {\n    return inner()\n}\n",
            ),
        ],
        &["[E0727]", "pure -> m.feed"],
        &[],
        &["<indirect call>"],
    );
}

#[test]
fn group_mod_e15_an_imported_generic_is_reachable_from_nogc() {
    run_nogc_row(
        "E-15",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.ident(4)\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            ("m.aelys", "pub fn ident<T>(v: T) -> T {\n    return v\n}\n"),
        ],
        4,
    );
}

// callee resolved to its last segment, silently answers with the root's clean function
#[test]
fn group_mod_e16_a_homonym_in_the_root_does_not_clear_the_imported_one() {
    reject_row(
        "E-16",
        &[
            (
                "root.aelys",
                "needs m\n\nfn helper() -> i64 {\n    return 0\n}\n\nnogc fn pure() -> i64 {\n    return m.helper()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn helper() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 0\n}\n",
            ),
        ],
        &["[E0727]", "pure -> m.helper"],
        &[],
        &["<indirect call>"],
    );
}

#[test]
fn group_mod_e17_the_imported_homonym_is_the_one_that_runs() {
    run_nogc_row(
        "E-17",
        &[
            (
                "root.aelys",
                "needs m\n\nfn helper() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 1\n}\n\nnogc fn pure() -> i64 {\n    return m.helper()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            ("m.aelys", "pub fn helper() -> i64 {\n    return 6\n}\n"),
        ],
        6,
    );
}

// the inter-module fixture of e-7 is rejected at its call site today, so it cannot yet fail on
#[test]
fn group_mod_e7b_a_single_file_elided_allocation_still_carries_its_effect() {
    let files: Files = &[(
        "root.aelys",
        "fn dead() -> i64 {\n    if false {\n        let v: vec<i64> = Vec::new()\n        return 1\n    }\n    return 0\n}\n\nnogc fn pure() -> i64 {\n    return dead()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
    )];
    for (level, opt) in LEVELS {
        let dir = stage(files);
        assert!(
            lower_file_to_air(&dir.path().join("root.aelys"), *opt).is_err(),
            "E-7b at {level}: an allocation the optimizer removes still carries its effect"
        );
    }
}


const RESOURCE_MODULE: &str = "pub struct Resource { pub id: i64 }\n\npub fn make(n: i64) -> Resource {\n    return Resource { id: n }\n}\n";

#[test]
fn group_mod_a1_an_imported_affine_type_keeps_its_affineness() {
    reject_row(
        "A-1",
        &[
            (
                "root.aelys",
                "needs m\n\nfn leak() -> i64 {\n    let a = m.make(1)\n    let b = a\n    return a.id\n}\n\nfn main() -> i64 {\n    return leak()\n}\n",
            ),
            ("m.aelys", RESOURCE_MODULE),
        ],
        &["[E0701]"],
        &[],
        &["__q"],
    );
}

#[test]
fn group_mod_a2_the_move_is_caught_inside_the_defining_module() {
    reject_row(
        "A-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.leak()\n}\n",
            ),
            (
                "m.aelys",
                "struct Resource { id: i64 }\n\npub fn leak() -> i64 {\n    let a = Resource { id: 1 }\n    let b = a\n    return a.id\n}\n",
            ),
        ],
        &["[E0701]"],
        &["m.aelys"],
        &[],
    );
}

#[test]
fn group_mod_a3_the_single_file_move_is_still_caught() {
    reject_row(
        "A-3",
        &[(
            "root.aelys",
            "struct Resource { id: i64 }\n\nfn leak() -> i64 {\n    let a = Resource { id: 1 }\n    let b = a\n    return a.id\n}\n\nfn main() -> i64 {\n    return leak()\n}\n",
        )],
        &["[E0701]"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_a4_one_move_of_an_imported_affine_type_is_kept() {
    run_row(
        "A-4",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let a = m.make(9)\n    let b = a\n    return b.id\n}\n",
            ),
            ("m.aelys", RESOURCE_MODULE),
        ],
        9,
    );
}

#[test]
fn group_mod_a5_affineness_survives_two_module_hops() {
    reject_row(
        "A-5",
        &[
            (
                "root.aelys",
                "needs mid\n\nfn main() -> i64 {\n    return mid.leak()\n}\n",
            ),
            (
                "mid.aelys",
                "needs deep\n\npub fn leak() -> i64 {\n    let a = deep.make(1)\n    let b = a\n    return a.id\n}\n",
            ),
            ("deep.aelys", RESOURCE_MODULE),
        ],
        &["[E0701]"],
        &[],
        &["__q"],
    );
}

// affineness must not widen to every imported struct while it is being carried across
#[test]
fn group_mod_a6_a_plain_imported_struct_moves_freely() {
    run_row(
        "A-6",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let a = m.make(3)\n    let b = a\n    let c = b\n    return c.v\n}\n",
            ),
            (
                "m.aelys",
                "pub struct Plain { pub v: i64 }\n\npub fn make(n: i64) -> Plain {\n    return Plain { v: n }\n}\n",
            ),
        ],
        3,
    );
}


#[test]
fn group_mod_b1_a_returned_reference_to_a_dead_local_still_rejects() {
    reject_row(
        "B-1",
        &[
            (
                "root.aelys",
                "needs m\n\nfn make_dangle() -> &i64 {\n    let x: i64 = 7\n    return m.pick(&x)\n}\n\nfn main() -> i64 {\n    return *make_dangle()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn pick(p: &i64) -> &i64 {\n    return p\n}\n",
            ),
        ],
        &["[E0723]"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_b2_a_shared_borrow_under_a_live_mutable_one_still_rejects() {
    reject_row(
        "B-2",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let mut x: i64 = 7\n    let r = &mut x\n    let v = m.read(&x)\n    *r = 1\n    return v\n}\n",
            ),
            (
                "m.aelys",
                "pub fn read(p: &i64) -> i64 {\n    return *p\n}\n",
            ),
        ],
        &["[E0713]"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_b3_references_cross_and_are_used() {
    run_row(
        "B-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let mut x: i64 = 7\n    m.bump(&mut x)\n    return m.read(&x)\n}\n",
            ),
            (
                "m.aelys",
                "pub fn read(p: &i64) -> i64 {\n    return *p\n}\n\npub fn bump(p: &mut i64) -> i64 {\n    *p = *p + 1\n    return 0\n}\n",
            ),
        ],
        8,
    );
}


#[test]
fn group_mod_n2_a_single_file_nogc_call_is_still_accepted() {
    run_row(
        "N-2",
        &[(
            "root.aelys",
            "fn plain(n: i64) -> i64 {\n    return n + 1\n}\n\nnogc fn pure() -> i64 {\n    return plain(2)\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
        )],
        3,
    );
}

#[test]
fn group_mod_n3_the_single_file_chain_does_not_degrade() {
    reject_row(
        "N-3",
        &[(
            "root.aelys",
            "fn allocates() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 0\n}\n\nnogc fn pure() -> i64 {\n    return allocates()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
        )],
        &["[E0727]", "pure -> allocates -> Vec::new"],
        &[],
        &["<indirect call>"],
    );
}


#[test]
fn group_mod_q4_no_stage3_rejection_shows_the_qualification_head() {
    let shapes: &[(&str, Files)] = &[
        (
            "rc-carrier",
            &[
                (
                    "root.aelys",
                    "needs m\n\nfn main() -> i64 {\n    let b: vec<m.Holder> = Vec::new()\n    return 0\n}\n",
                ),
                ("m.aelys", "pub struct Holder { r: Rc<i64> }\n"),
            ],
        ),
        (
            "affine-move",
            &[
                (
                    "root.aelys",
                    "needs m\n\nfn leak() -> i64 {\n    let a = m.make(1)\n    let b = a\n    return a.id\n}\n\nfn main() -> i64 {\n    return leak()\n}\n",
                ),
                ("m.aelys", RESOURCE_MODULE),
            ],
        ),
        (
            "effect-chain",
            &[
                (
                    "root.aelys",
                    "needs m\n\nnogc fn pure() -> i64 {\n    return m.allocates()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
                ),
                ("m.aelys", ALLOCATES),
            ],
        ),
        (
            "type-mismatch",
            &[
                (
                    "root.aelys",
                    "needs m\n\nfn main() -> i64 {\n    let q: i64 = m.make(1)\n    return q\n}\n",
                ),
                ("m.aelys", RESOURCE_MODULE),
            ],
        ),
    ];
    for (shape, files) in shapes {
        let rendered = rejection(&format!("Q-4 {shape}"), files, OptimizationLevel::None);
        assert!(
            !rendered.contains("__q"),
            "Q-4 {shape}: no rejection may show the qualification head\nrendered:\n{rendered}"
        );
    }
}


const NESTED_COLLIDING: Files = &[
    (
        "root.aelys",
        "needs app.util\nneeds app\n\nnogc fn pure() -> i64 {\n    return util.helper()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
    ),
    (
        "util.aelys",
        "pub nogc fn helper() -> i64 {\n    return 1\n}\n",
    ),
    (
        "app.aelys",
        "needs util\n\npub fn go() -> i64 {\n    return util.helper()\n}\n",
    ),
    (
        "app/util.aelys",
        "pub fn helper() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 42\n}\n",
    ),
];

#[test]
fn group_mod_k1_a_nested_path_does_not_borrow_another_modules_summary() {
    reject_row(
        "K-1",
        NESTED_COLLIDING,
        &["[E0727]", "pure -> app.util.helper"],
        &[],
        &["<indirect call>"],
    );
}

#[test]
fn group_mod_k2_the_nested_module_answers_for_itself() {
    run_nogc_row(
        "K-2",
        &[
            (
                "root.aelys",
                "needs app.util\nneeds app\n\nnogc fn pure() -> i64 {\n    return util.helper()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "util.aelys",
                "pub fn helper() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 1\n}\n",
            ),
            (
                "app.aelys",
                "needs util\n\npub fn go() -> i64 {\n    return util.helper()\n}\n",
            ),
            (
                "app/util.aelys",
                "pub nogc fn helper() -> i64 {\n    return 40\n}\n",
            ),
        ],
        40,
    );
}

// a module publishes only what it defines, so one that imports nothing cannot republish
#[test]
fn group_mod_k3_a_module_that_imports_nothing_republishes_nothing() {
    reject_row(
        "K-3",
        &[
            (
                "root.aelys",
                "needs app.util\nneeds app\n\nnogc fn pure() -> i64 {\n    return util.helper()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "util.aelys",
                "pub nogc fn helper() -> i64 {\n    return 1\n}\n",
            ),
            ("app.aelys", "pub fn go() -> i64 {\n    return 0\n}\n"),
            (
                "app/util.aelys",
                "pub fn helper() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 42\n}\n",
            ),
        ],
        &["[E0727]", "pure -> app.util.helper"],
        &[],
        &["<indirect call>"],
    );
}


const RC_RESOURCE: &str = "pub struct Resource { pub id: i64, pub r: Rc<i64> }\n\npub fn make(v: i64) -> Resource {\n    return Resource { id: v, r: Rc::new(1) }\n}\n";

#[test]
fn group_mod_m1_an_imported_affine_name_carrying_an_rc_is_managed() {
    reject_row(
        "M-1",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure(x: m.Resource) -> i64 {\n    return x.id\n}\n\nfn main() -> i64 {\n    let v = m.make(7)\n    return pure(v)\n}\n",
            ),
            ("m.aelys", RC_RESOURCE),
        ],
        &["[E0727]", "the managed parameter"],
        &[],
        &["__q"],
    );
}

#[test]
fn group_mod_m2_the_single_file_twin_is_rejected_alike() {
    reject_row(
        "M-2",
        &[(
            "root.aelys",
            "struct Resource { id: i64, r: Rc<i64> }\n\nfn make(v: i64) -> Resource {\n    return Resource { id: v, r: Rc::new(1) }\n}\n\nnogc fn pure(x: Resource) -> i64 {\n    return x.id\n}\n\nfn main() -> i64 {\n    let v = make(7)\n    return pure(v)\n}\n",
        )],
        &["[E0727]", "the managed parameter"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_m3_a_plain_affine_name_is_still_affine() {
    reject_row(
        "M-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn leak() -> i64 {\n    let a = m.make(1)\n    let b = a\n    return a.id\n}\n\nfn main() -> i64 {\n    return leak()\n}\n",
            ),
            ("m.aelys", RESOURCE_MODULE),
        ],
        &["[E0701]"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_m4_an_affine_name_carrying_an_rc_is_still_affine() {
    reject_row(
        "M-4",
        &[(
            "root.aelys",
            "struct Resource { id: i64, r: Rc<i64> }\n\nfn main() -> i64 {\n    let a = Resource { id: 3, r: Rc::new(1) }\n    let b = a\n    return a.id\n}\n",
        )],
        &["[E0701]"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_m5_the_affine_drop_of_such_a_type_still_runs() {
    run_row_with_stdout(
        "M-5",
        &[(
            "root.aelys",
            "struct Resource { id: i64, r: Rc<i64> }\n\nfn use1() -> i64 {\n    let a = Resource { id: 77, r: Rc::new(1) }\n    return a.id\n}\n\nfn main() -> i64 {\n    return use1()\n}\n",
        )],
        77,
        "77",
        (1, 1),
    );
}

#[test]
fn group_mod_m6_the_imported_twin_keeps_both_axes() {
    reject_row(
        "M-6",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let a = m.make(3)\n    let b = a\n    return a.id\n}\n",
            ),
            ("m.aelys", RC_RESOURCE),
        ],
        &["[E0701]"],
        &[],
        &["__q"],
    );
}


#[test]
fn group_mod_w1_the_chain_is_woven_across_three_modules() {
    reject_row(
        "W-1",
        &[
            (
                "root.aelys",
                "needs mid1\n\nnogc fn pure() -> i64 {\n    return mid1.mid1()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "mid1.aelys",
                "needs mid2\n\npub fn mid1() -> i64 {\n    return mid2.mid2()\n}\n",
            ),
            (
                "mid2.aelys",
                "needs deep\n\npub fn mid2() -> i64 {\n    return deep.allocates()\n}\n",
            ),
            ("deep.aelys", ALLOCATES),
        ],
        &[
            "[E0727]",
            "pure -> mid1.mid1 -> mid2.mid2 -> deep.allocates -> Vec::new",
        ],
        &[],
        &["<indirect call>"],
    );
}

#[test]
fn group_mod_w2_a_real_indirect_call_is_still_named_indirect() {
    reject_row(
        "W-2",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.outer()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn outer() -> i64 {\n    let f = fn() -> i64 {\n        let v: vec<i64> = Vec::new()\n        return 0\n    }\n    return f()\n}\n",
            ),
        ],
        &["[E0727]", "pure -> m.outer -> <indirect call>"],
        &[],
        &[],
    );
}

#[test]
fn group_mod_w3_a_post_merge_diagnostic_names_the_declaring_file() {
    reject_row(
        "W-3",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.make()\n}\n",
            ),
            (
                "m.aelys",
                "pub struct Node { next: Node }\n\npub fn make() -> i64 {\n    return 1\n}\n",
            ),
        ],
        &["[E0904]", "struct `m.Node` has infinite size"],
        &["m.aelys", "pub struct Node"],
        &["root.aelys", "__q"],
    );
}

#[test]
fn group_mod_w4_a_joined_message_anchors_on_its_first_error() {
    reject_row(
        "W-4",
        &[
            (
                "root.aelys",
                "needs a\nneeds bb\n\nfn main() -> i64 {\n    return a.f() + bb.g()\n}\n",
            ),
            (
                "a.aelys",
                "pub struct Node { next: Node }\n\npub fn f() -> i64 {\n    return 1\n}\n",
            ),
            (
                "bb.aelys",
                "pub struct Deep { next: Deep }\n\npub fn g() -> i64 {\n    return 2\n}\n",
            ),
        ],
        &["[E0904]", "struct `a.Node` has infinite size"],
        &["a.aelys", "pub struct Node"],
        &["bb.aelys", "__q"],
    );
}
