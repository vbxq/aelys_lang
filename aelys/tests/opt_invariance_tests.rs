use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const UNCODED: &str = "UNCODED";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the aelys crate has a parent workspace dir")
        .to_path_buf()
}

fn codes(rendered: &str) -> BTreeSet<String> {
    let bytes = rendered.as_bytes();
    let mut found = BTreeSet::new();
    for i in 0..bytes.len().saturating_sub(6) {
        if bytes[i] != b'[' || bytes[i + 1] != b'E' || bytes[i + 6] != b']' {
            continue;
        }
        if bytes[i + 2..i + 6].iter().all(u8::is_ascii_digit) {
            found.insert(String::from_utf8_lossy(&bytes[i + 1..i + 6]).into_owned());
        }
    }
    found
}

// the backend family reports on the optimized program, so it is outside the invariant
fn is_backend(code: &str) -> bool {
    code.starts_with("E09")
}

fn verdict(path: &Path, level: OptimizationLevel) -> BTreeSet<String> {
    match lower_file_to_air(path, level) {
        Ok(_) => BTreeSet::new(),
        Err(rendered) => {
            let found = codes(&rendered);
            if found.is_empty() {
                return BTreeSet::from([UNCODED.to_string()]);
            }
            found.into_iter().filter(|code| !is_backend(code)).collect()
        }
    }
}

fn render(verdict: &BTreeSet<String>) -> String {
    if verdict.is_empty() {
        "accepted".to_string()
    } else {
        verdict.iter().cloned().collect::<Vec<_>>().join(",")
    }
}

fn invariant_verdict(path: &Path) -> Result<BTreeSet<String>, String> {
    let verdicts: Vec<(&str, BTreeSet<String>)> = LEVELS
        .iter()
        .map(|(name, level)| (*name, verdict(path, *level)))
        .collect();
    let baseline = &verdicts[0].1;
    if verdicts.iter().all(|(_, v)| v == baseline) {
        return Ok(baseline.clone());
    }
    Err(verdicts
        .iter()
        .map(|(name, v)| format!("{name}={}", render(v)))
        .collect::<Vec<_>>()
        .join(" | "))
}

fn write_fixture(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(format!("{name}.aelys"));
    std::fs::write(&path, source).expect("write fixture");
    path
}

fn report(failures: Vec<String>, corpus: &str, checked: usize) {
    if failures.is_empty() {
        return;
    }
    panic!(
        "the safety diagnostic code set must not depend on the -O level, but {} of {checked} \
         {corpus} entries disagree across -O0/-O1/-O2/-O3:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

const REJECT_FIXTURES: &[(&str, &str, &str)] = &[
    (
        "reject_e0701_use_after_move",
        "E0701",
        "struct Resource { id: i64 }
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    println(a.id)
    return 0
}
",
    ),
    (
        "reject_e0702_double_move",
        "E0702",
        "struct Resource { id: i64 }
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    let c = a
    return 0
}
",
    ),
    (
        "reject_e0703_maybe_moved_use",
        "E0703",
        "struct Resource { id: i64 }
fn main() -> i64 {
    let cond = true
    let a = Resource{id: 1}
    if cond {
        let b = a
    }
    println(a.id)
    return 0
}
",
    ),
    (
        "reject_e0704_maybe_moved_on_drop",
        "E0704",
        "struct Resource { id: i64 }
fn f(c: bool) {
    let a = Resource{id: 1}
    if c {
        let b = a
    }
}
fn main() -> i64 {
    f(true)
    return 0
}
",
    ),
    (
        "reject_e0711_write_while_shared_borrow",
        "E0711",
        "fn main() -> i64 {
    let mut x = 3
    let r = &x
    x = 5
    return *r
}
",
    ),
    (
        "reject_e0711_push_while_element_borrowed",
        "E0711",
        "fn main() -> i64 {
    let v = vec[10, 20, 30]
    let r = &v[0]
    Vec::push(v, 40)
    return *r
}
",
    ),
    (
        "reject_e0711_write_while_borrow_used_in_folded_branch",
        "E0711",
        "fn main() -> i64 {
    let mut x = 3
    let r = &x
    x = 5
    if false {
        return *r
    }
    return 0
}
",
    ),
    (
        "reject_e0713_overlapping_mut_used_in_folded_branch",
        "E0713",
        "fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    let r2 = &mut x
    if false {
        *r1 = 5
        *r2 = 6
    }
    return x
}
",
    ),
    (
        "reject_e0702_double_move_in_folded_branch",
        "E0702",
        "struct Resource { id: i64 }
fn main() -> i64 {
    let a = Resource{id: 1}
    if false {
        let b = a
        let c = a
    }
    return 0
}
",
    ),
    (
        "reject_e0712_move_while_borrowed",
        "E0712",
        "struct Resource { id: i64 }
fn touch(r: &Resource) {
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = &a
    let b = a
    touch(r)
    return 0
}
",
    ),
    (
        "reject_e0712_move_while_borrowed_through_call",
        "E0712",
        "struct Resource { id: i64 }
fn touch(r: &Resource) {
}
fn get(r: &Resource) -> &Resource {
    return r
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = get(&a)
    let b = a
    touch(r)
    return 0
}
",
    ),
    (
        "reject_e0713_overlapping_mut_borrow",
        "E0713",
        "fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    let r2 = &mut x
    *r1 = 5
    *r2 = 6
    return x
}
",
    ),
    (
        "reject_e0714_ref_in_array_aggregate",
        "E0714",
        "fn main() -> i64 {
    let v = vec[10, 20, 30]
    let a = &v[0]
    let arr = [a]
    return 0
}
",
    ),
    (
        "reject_e0714_ref_in_enum_payload",
        "E0714",
        "enum Box { Of(&i64), Empty }
fn main() -> i64 {
    let mut x = 5
    let o = Box::Of(&x)
    return 0
}
",
    ),
    (
        "reject_e0723_origin_through_call",
        "E0723",
        "fn id(r: &i64) -> &i64 {
    return r
}
fn via(r: &i64) -> &i64 {
    return id(r)
}
fn main() -> i64 {
    return 0
}
",
    ),
    (
        "reject_e0725_ref_captured_by_closure",
        "E0725",
        "fn main() -> i64 {
    let a = 10
    let r = &a
    let f = fn() -> i64 {
        return *r
    }
    return 0
}
",
    ),
    (
        "reject_e0726_nested_ref",
        "E0726",
        "fn main() -> i64 {
    let x = 5
    let r = &x
    let pp = &r
    return 0
}
",
    ),
    (
        "reject_e0727_nogc_allocates_rc",
        "E0727",
        "nogc fn f() -> i64 {
    let r = Rc::new(5)
    return 0
}
fn main() -> i64 { return f() }
",
    ),
    (
        "reject_e0727_nogc_allocates_vec",
        "E0727",
        "nogc fn f() -> i64 {
    let mut v = Vec::new()
    return 0
}
fn main() -> i64 { return f() }
",
    ),
    (
        "reject_e0727_nogc_through_call_chain",
        "E0727",
        "fn leaf() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}
fn helper() -> i64 {
    return leaf()
}
nogc fn f() -> i64 {
    return helper()
}
fn main() -> i64 { return f() }
",
    ),
    (
        // this source once exploited the nested-shadow miscompile to route the
        // nogc call at `f` into the nested vec body . that shadow is now rejected e0418 in
        "reject_e0418_nested_fn_shadows_global",
        "E0418",
        "fn dup() -> i64 { return 0 }
fn holder() -> i64 {
    fn dup() -> i64 {
        let mut v = Vec::new()
        Vec::push(v, 1)
        return 0
    }
    return 0
}
nogc fn f() -> i64 { return dup() }
fn main() -> i64 { return f() + holder() }
",
    ),
    // appear at -o0 and vanish at -o2. these pin every enforcement point across all four levels.
    (
        "reject_e0412_vec_producing_form_grouping",
        "E0412",
        "fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    let w = (v)
    return 0
}
",
    ),
    (
        "reject_e0412_nested_vec_push",
        "E0412",
        "fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let mut vv = Vec::new()
    Vec::push(vv, inner)
    return 0
}
",
    ),
    (
        "reject_e0412_array_of_vec",
        "E0412",
        "fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let arr = [inner]
    return 0
}
",
    ),
    (
        "reject_e0412_generic_fn_with_vec_arg",
        "E0412",
        "fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return keep(v) }
",
    ),
    (
        // invariant. a live dangerous construct must reject at every level, and this fixture pins that.
        "reject_e0412_generic_enum_with_vec_payload",
        "E0412",
        "enum Opt<T> { Some(T), Nil }
fn main() -> i64 {
    let mut i1 = Vec::new()
    Vec::push(i1, 1)
    let a = Opt::Some(i1)
    let r: i64 = match a { Opt::Some(w) => w[0], Opt::Nil => 0 }
    return r
}
",
    ),
];

const ACCEPT_FIXTURES: &[(&str, &str)] = &[
    (
        "accept_sequential_mut_borrows",
        "fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    *r1 = 5
    let r2 = &mut x
    *r2 = 6
    return x
}
",
    ),
    (
        "accept_move_after_borrow_dies",
        "struct Resource { id: i64 }
fn touch(r: &Resource) {
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = &a
    touch(r)
    let b = a
    return 0
}
",
    ),
    (
        "accept_nogc_compute_only",
        "nogc fn f(x: i64) -> i64 {
    let y = x * 2
    return y + 1
}
fn main() -> i64 { return f(20) }
",
    ),
    (
        "accept_managed_outside_nogc",
        "fn g() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}
fn main() -> i64 { return g() }
",
    ),
    (
        "accept_rc_carrier_moved",
        "struct Node { next: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let n: Node = Node { next: a }
    let m: Node = n
    return Rc::get(m.next)
}
",
    ),
    (
        "accept_rc_returned_then_vec",
        "struct Node { next: Rc<i64> }
fn make(v: i64) -> Node {
    return Node { next: Rc::new(v) }
}
fn main() -> i64 {
    let n = make(7)
    let mut v = Vec::new()
    Vec::push(v, Rc::get(n.next))
    return v[0]
}
",
    ),
];

#[test]
fn reject_fixtures_report_the_same_code_set_at_every_opt_level() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    for (name, required, source) in REJECT_FIXTURES {
        let path = write_fixture(dir.path(), name, source);
        match invariant_verdict(&path) {
            Err(breakdown) => failures.push(format!("  {name}: {breakdown}")),
            Ok(set) => {
                if !set.contains(*required) {
                    failures.push(format!(
                        "  {name}: invariant but no longer reports {required} (got {})",
                        render(&set)
                    ));
                }
            }
        }
    }
    report(failures, "compile-FAIL fixture", REJECT_FIXTURES.len());
}

#[test]
fn accept_fixtures_stay_accepted_at_every_opt_level() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    for (name, source) in ACCEPT_FIXTURES {
        let path = write_fixture(dir.path(), name, source);
        match invariant_verdict(&path) {
            Err(breakdown) => failures.push(format!("  {name}: {breakdown}")),
            Ok(set) if !set.is_empty() => failures.push(format!(
                "  {name}: rejected at every level with {}",
                render(&set)
            )),
            Ok(_) => {}
        }
    }
    report(failures, "compile-PASS fixture", ACCEPT_FIXTURES.len());
}

fn collect_aelys(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_aelys(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("aelys") {
            out.push(path);
        }
    }
}

#[test]
fn the_aelys_corpus_reports_the_same_code_set_at_every_opt_level() {
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in [
        "tests_e2e",
        "torture",
        "examples",
        "aelys/tests/exploration",
    ] {
        collect_aelys(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(
        files.len() > 300,
        "the corpus should be the whole repo's .aelys files, found {}",
        files.len()
    );

    let mut failures = Vec::new();
    for path in &files {
        if let Err(breakdown) = invariant_verdict(path) {
            let name = path.strip_prefix(&root).unwrap_or(path).display();
            failures.push(format!("  {name}: {breakdown}"));
        }
    }
    report(failures, "corpus file", files.len());
}
