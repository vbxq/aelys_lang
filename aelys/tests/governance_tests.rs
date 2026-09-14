use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CENSUS_REL: &str = "aelys/tests/ignored_census.tsv";
const SWEEPS_PIN_REL: &str = "scripts/sweeps_pin.tsv";
const REGISTRY_REL: &str = "common/src/diagnostic/registry.rs";
const COMPILE_CODE_REL: &str = "common/src/error/compile/code.rs";
const FAULT_REL: &str = "common/src/error/fault.rs";
const REGISTERED_CODES: usize = 101;

const UNEMITTED_RESIDUE: &[(&str, &str)] = &[];
const UNEMITTED_RESIDUE_ROWS: usize = 0;

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            let text = fs::read_to_string(&manifest).unwrap_or_default();
            if strip_toml_comments(&text).contains("[workspace]") {
                return dir;
            }
        }
        assert!(
            dir.pop(),
            "governance: no [workspace] manifest above {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

fn strip_toml_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut in_basic = false;
        let mut in_literal = false;
        let mut cut = line.len();
        for (i, c) in line.char_indices() {
            match c {
                '"' if !in_literal => in_basic = !in_basic,
                '\'' if !in_basic => in_literal = !in_literal,
                '#' if !in_basic && !in_literal => {
                    cut = i;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

fn workspace_members(root: &Path) -> Vec<String> {
    let path = root.join("Cargo.toml");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", path.display()));
    let text = strip_toml_comments(&raw);
    let mut span = None;
    let mut offset = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("members") {
            if rest.trim_start().starts_with('=') {
                span = Some(offset + (line.len() - trimmed.len()));
                break;
            }
        }
        offset += line.len() + 1;
    }
    let start = span.unwrap_or_else(|| panic!("governance: no `members` key in {}", path.display()));
    let open = text[start..]
        .find('[')
        .map(|o| start + o)
        .unwrap_or_else(|| panic!("governance: malformed `members` in {}", path.display()));
    let close = text[open..]
        .find(']')
        .map(|o| open + o)
        .unwrap_or_else(|| panic!("governance: unterminated `members` in {}", path.display()));

    let mut out = Vec::new();
    let mut rest = &text[open + 1..close];
    while let Some(q) = rest.find('"') {
        let after = &rest[q + 1..];
        let end = after
            .find('"')
            .unwrap_or_else(|| panic!("governance: unterminated member name in {}", path.display()));
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

const FALSE_KEY_EFFECTS: &[(&str, &str)] = &[
    (
        "doctest",
        "de-registers every ``` example in the lib, so the doc pass compiles and runs none of them",
    ),
    (
        "test",
        "de-registers the target from `cargo test`, its #[cfg(test)] unit tests stop being built \
         and the target itself disappears from any enumeration that selects on `test == true`",
    ),
    (
        "harness",
        "replaces libtest with the target's own main, so the target prints no `test result:` line \
         and every check it holds stops being counted by anything that reads one",
    ),
    (
        "bench",
        "de-registers the target's #[bench] items, which stop being built and stop being run",
    ),
];

fn key_value<'a>(trimmed: &'a str, key: &str) -> Option<&'a str> {
    let rest = trimmed.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    Some(rest.trim())
}

fn deregistration_hits(manifest: &Path) -> Vec<String> {
    let raw = fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", manifest.display()));
    let mut hits = Vec::new();
    for (n, line) in strip_toml_comments(&raw).lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("autotests") {
            if rest.trim_start().starts_with('=') {
                hits.push(format!(
                    "{}:{}: {trimmed}\n    turns off cargo's discovery of tests/*.rs and \
                     tests/<dir>/main.rs, so those suites stop being targets",
                    manifest.display(),
                    n + 1
                ));
            }
        }
        for (key, effect) in FALSE_KEY_EFFECTS {
            if key_value(trimmed, key) == Some("false") {
                hits.push(format!(
                    "{}:{}: {trimmed}\n    {effect}",
                    manifest.display(),
                    n + 1
                ));
            }
        }
        let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        if compact == "[[test]]" {
            hits.push(format!(
                "{}:{}: {trimmed}\n    replaces cargo's discovery with a hand written list, so \
                 every suite absent from that list stops being a target",
                manifest.display(),
                n + 1
            ));
        }
    }
    hits
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("governance: cannot list {}: {e}", dir.display()));
    let mut out: Vec<PathBuf> = entries
        .map(|entry| {
            entry
                .unwrap_or_else(|e| panic!("governance: cannot list {}: {e}", dir.display()))
                .path()
        })
        .collect();
    out.sort();
    out
}

// cargo makes a target of tests/*.rs and of tests/<dir>/main.rs, and of nothing else under tests/
fn target_sources(tests: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in read_dir_sorted(tests) {
        if path.is_dir() {
            let main = path.join("main.rs");
            if main.is_file() {
                out.push(main);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn rs_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in read_dir_sorted(dir) {
        if path.is_dir() {
            out.extend(rs_files_recursive(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn declared_mods(path: &Path) -> Vec<(Vec<String>, String)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    mod_decls(&text).unwrap_or_else(|e| panic!("governance: {}: {e}", path.display()))
}

fn modules_declared_by_suites(tests: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in read_dir_sorted(tests) {
        if !path.extension().is_some_and(|e| e == "rs") {
            continue;
        }
        for (prefix, name) in declared_mods(&path) {
            if prefix.is_empty() {
                out.insert(name);
            }
        }
    }
    out
}

fn walk_modules(file: &Path, children: &Path, out: &mut BTreeSet<PathBuf>) {
    if !out.insert(file.to_path_buf()) {
        return;
    }
    for (prefix, name) in declared_mods(file) {
        let mut base = children.to_path_buf();
        for part in &prefix {
            base.push(part);
        }
        let flat = base.join(format!("{name}.rs"));
        if flat.is_file() {
            walk_modules(&flat, &base.join(&name), out);
        }
        let nested = base.join(&name).join("mod.rs");
        if nested.is_file() {
            walk_modules(&nested, &base.join(&name), out);
        }
    }
}

fn uncompiled_sources(tests: &Path) -> Vec<PathBuf> {
    let declared = modules_declared_by_suites(tests);
    let mut out = Vec::new();
    for path in read_dir_sorted(tests) {
        if !path.is_dir() {
            continue;
        }
        let sources = rs_files_recursive(&path);
        if sources.is_empty() || path.join("main.rs").is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let mut compiled = BTreeSet::new();
        let entry = path.join("mod.rs");
        if entry.is_file() && declared.contains(&name) {
            walk_modules(&entry, &path, &mut compiled);
        }
        out.extend(sources.into_iter().filter(|f| !compiled.contains(f)));
    }
    out
}

// a `mod x;` inside a string or a comment is not a declaration, so this walks tokens instead of lines
fn mod_decls(src: &str) -> Result<Vec<(Vec<String>, String)>, String> {
    let ch: Vec<char> = src.chars().collect();
    let n = ch.len();
    let mut out = Vec::new();
    let mut inline: Vec<(usize, String)> = Vec::new();
    let mut depth = 0usize;
    let mut saw_mod = false;
    let mut name: Option<String> = None;
    let mut i = 0usize;

    while i < n {
        let c = ch[i];

        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            let mut nest = 1usize;
            i += 2;
            while i < n && nest > 0 {
                if ch[i] == '/' && i + 1 < n && ch[i + 1] == '*' {
                    nest += 1;
                    i += 2;
                } else if ch[i] == '*' && i + 1 < n && ch[i + 1] == '/' {
                    nest -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if let Some((end, _)) = read_raw_string(&ch, i)? {
            i = end;
            saw_mod = false;
            name = None;
            continue;
        }
        if c == '"' {
            let (end, _) = read_basic_string(&ch, i)?;
            i = end;
            saw_mod = false;
            name = None;
            continue;
        }
        if c == '\'' {
            i = skip_char_or_lifetime(&ch, i);
            saw_mod = false;
            name = None;
            continue;
        }
        if is_ident(c) && !c.is_ascii_digit() {
            let start = i;
            while i < n && is_ident(ch[i]) {
                i += 1;
            }
            let word: String = ch[start..i].iter().collect();
            if saw_mod && name.is_none() {
                name = Some(word);
            } else {
                saw_mod = word == "mod";
                name = None;
            }
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        match c {
            ';' => {
                if let Some(decl) = name.take() {
                    out.push((inline.iter().map(|(_, m)| m.clone()).collect(), decl));
                }
            }
            '{' => {
                if let Some(decl) = name.take() {
                    inline.push((depth, decl));
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                while inline.last().is_some_and(|(d, _)| *d >= depth) {
                    inline.pop();
                }
                name = None;
            }
            _ => name = None,
        }
        saw_mod = false;
        i += 1;
    }

    Ok(out)
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn g1_no_workspace_manifest_deregisters_a_test_source() {
    let root = repo_root();
    let mut hits = Vec::new();
    for member in workspace_members(&root) {
        hits.extend(deregistration_hits(&root.join(&member).join("Cargo.toml")));
    }
    assert!(
        hits.is_empty(),
        "G1: a workspace manifest de-registers test sources.\n{}\n\n\
         Each hit above lets a check stay in the tree while it stops being a cargo target or \
         stops reporting, so nothing runs it and nothing reports it missing. `test = false` is \
         the quietest of them: the target does not go empty, it vanishes from the enumeration \
         entirely, which is why the whole target set is pinned in scripts/target_pin.tsv as \
         well. This guard fails closed: if such a key is wanted, it has to be re-authorized \
         here on purpose, together with the verification list that will now have to name the \
         surviving checks by hand.",
        hits.join("\n")
    );
}

#[test]
fn g2_every_test_source_is_a_live_target() {
    let root = repo_root();
    let members = workspace_members(&root);
    assert!(
        !members.is_empty(),
        "G2: the root manifest at {} lists no workspace members, so nothing enumerates the tree.",
        root.join("Cargo.toml").display()
    );

    let mut problems = Vec::new();
    let mut total = 0usize;
    for member in &members {
        let dir = root.join(member);
        if !dir.is_dir() {
            problems.push(format!(
                "member `{member}` is listed in the root manifest but {} does not exist",
                dir.display()
            ));
            continue;
        }
        let tests = dir.join("tests");
        if !tests.is_dir() {
            continue;
        }
        let sources = target_sources(&tests);
        if sources.is_empty() {
            problems.push(format!(
                "{} exists but holds no *.rs suite",
                tests.display()
            ));
        }
        total += sources.len();
        for dead in uncompiled_sources(&tests) {
            problems.push(format!(
                "{} is no target and no target reaches it: its directory holds no main.rs, and it \
                 is not a mod.rs a suite declares nor a file that mod.rs reaches through `mod`, \
                 so cargo never compiles it",
                dead.display()
            ));
        }
        problems.extend(deregistration_hits(&dir.join("Cargo.toml")));
    }

    assert!(
        total > 0,
        "G2: no workspace member holds a tests/*.rs or tests/<dir>/main.rs suite, which means \
         the tree this guard is supposed to guard has vanished."
    );
    assert!(
        problems.is_empty(),
        "G2: the test tree and the manifests disagree.\n{}\n\n\
         Cargo makes a target of every *.rs directly under a member's tests/ dir and of every \
         tests/<dir>/main.rs, and compiles tests/<dir>/mod.rs, plus every file that mod.rs \
         reaches through `mod`, only into the suites that declare `mod <dir>;`, and nothing \
         else, only while that member's manifest keeps auto discovery on, so this pairs with \
         G1: G1 forbids the opt out, G2 checks the files that opt out would silence are still \
         there and that no test source sits where cargo never compiles it.",
        problems.join("\n")
    );
}

#[test]
fn g3_the_ignore_census_is_pinned() {
    let root = repo_root();
    let mut rows = Vec::new();
    let mut unreasoned = Vec::new();

    for member in workspace_members(&root) {
        let tests = root.join(&member).join("tests");
        if !tests.is_dir() {
            continue;
        }
        for file in rs_files_recursive(&tests) {
            let name = rel(&root, &file);
            let src = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("governance: cannot read {name}: {e}"));
            let sites = scan_ignores(&src).unwrap_or_else(|e| panic!("governance: {name}: {e}"));
            for site in sites {
                let test = site
                    .test
                    .unwrap_or_else(|| panic!("governance: {name}:{}: an ignore attribute with no fn after it", site.line));
                let condition = match &site.predicate {
                    Some(pred) => format!("cfg({pred})"),
                    None => "always".to_string(),
                };
                match site.reason {
                    Some(reason) if !reason.trim().is_empty() => {
                        assert!(
                            !reason.contains('\t') && !reason.contains('\n'),
                            "G3: {name}:{}: `{test}` has a reason containing a tab or a newline, \
                             which the census file cannot represent",
                            site.line
                        );
                        rows.push(format!("{name}\t{test}\t{condition}\t{reason}"));
                    }
                    _ => unreasoned.push(format!("{name}:{}: {test} [{condition}]", site.line)),
                }
            }
        }
    }

    assert!(
        unreasoned.is_empty(),
        "G3: an ignored test carries no reason.\n{}\n\n\
         An `#[ignore]` with no reason string is a check that exists, is reported as a target \
         that ran, and is never executed. A cfg conditional ignore is the same defect behind a \
         predicate: the bracket above each row is the condition under which the check stops \
         running. Write the real cause into the attribute as `#[ignore = \"why\"]` and pin it \
         in {CENSUS_REL}.",
        unreasoned.join("\n")
    );

    rows.sort();
    let measured = rows.join("\n") + "\n";
    let census_path = root.join(CENSUS_REL);
    let pinned = fs::read_to_string(&census_path).unwrap_or_else(|e| {
        panic!(
            "governance: cannot read {}: {e}\nmeasured census is:\n{measured}",
            census_path.display()
        )
    });

    let measured_set: BTreeSet<&str> = measured.lines().filter(|l| !l.is_empty()).collect();
    let pinned_set: BTreeSet<&str> = pinned.lines().filter(|l| !l.is_empty()).collect();
    if measured_set == pinned_set {
        return;
    }

    let key = |row: &str| -> (String, String) {
        let mut it = row.splitn(4, '\t');
        (
            it.next().unwrap_or_default().to_string(),
            it.next().unwrap_or_default().to_string(),
        )
    };
    let measured_keys: BTreeSet<(String, String)> = measured_set.iter().map(|r| key(r)).collect();
    let pinned_keys: BTreeSet<(String, String)> = pinned_set.iter().map(|r| key(r)).collect();

    let mut moved = Vec::new();
    for row in measured_set.difference(&pinned_set) {
        let k = key(row);
        if pinned_keys.contains(&k) {
            moved.push(format!("CENSUS ROW CHANGED to   {row}"));
        } else {
            moved.push(format!("NEWLY IGNORED           {row}"));
        }
    }
    for row in pinned_set.difference(&measured_set) {
        let k = key(row);
        if measured_keys.contains(&k) {
            moved.push(format!("CENSUS ROW CHANGED from {row}"));
        } else {
            moved.push(format!("NO LONGER IGNORED       {row}"));
        }
    }
    moved.sort();

    panic!(
        "G3: the #[ignore] census moved.\n{}\n\n\
         An ignored test is a check that exists and is never executed, one level below a suite \
         that sits in no verification list, so every entry is a deliberate reviewed act rather \
         than a side effect. Column 3 is the condition: `always` for a bare `#[ignore]`, \
         `cfg(<predicate>)` for an ignore reached through `cfg_attr`, with the predicate \
         whitespace stripped. Only the `always` rows are counted against the harness ignored \
         total. If the move is intended, write this exact content into {CENSUS_REL}:\n\
         ----8<----\n{measured}----8<----",
        moved.join("\n")
    );
}

fn git_index_modes(root: &Path, paths: &[String]) -> BTreeMap<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-s", "--"])
        .args(paths)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "G4: `git ls-files -s` could not be run: {e}\n\n\
                 The tracked mode of a script lives in the git index and nowhere else, so \
                 without git this obligation cannot be evaluated at all. This fails rather \
                 than skips on purpose: a skip here would report a pass for a check that never \
                 ran, which is the same hollowing the exec bits below went missing behind."
            )
        });
    assert!(
        out.status.success(),
        "G4: `git ls-files -s` exited {}: {}\n\n\
         The index mode cannot be read, so the obligation cannot be evaluated, so this fails \
         rather than skips.",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    );

    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut modes = BTreeMap::new();
    for line in text.lines() {
        let (meta, path) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("G4: unparseable `git ls-files -s` row: {line}"));
        let mode = meta
            .split_whitespace()
            .next()
            .unwrap_or_else(|| panic!("G4: unparseable `git ls-files -s` row: {line}"));
        modes.insert(path.to_string(), mode.to_string());
    }
    modes
}

fn shell_scripts(dir: &Path) -> Vec<String> {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("G4: cannot list {}: {e}", dir.display()));
    let mut names = Vec::new();
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("G4: cannot list {}: {e}", dir.display()));
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".sh") && entry.path().is_file() {
            names.push(name);
        }
    }
    names.sort();
    names
}

#[test]
fn g4_every_tracked_shell_script_can_be_invoked() {
    let root = repo_root();
    let scripts_dir = root.join("scripts");

    let scripts = shell_scripts(&scripts_dir);
    assert!(
        !scripts.is_empty(),
        "G4: {} holds no *.sh at all, so this obligation would apply to nothing.",
        scripts_dir.display()
    );

    let rel_paths: Vec<String> = scripts
        .iter()
        .map(|name| format!("scripts/{name}"))
        .collect();
    // core.filemode is false in this repo, so a filesystem exec bit is invisible to git and only the index mode is tracked
    let modes = git_index_modes(&root, &rel_paths);

    let mut script_problems = Vec::new();
    for rel_path in &rel_paths {
        match modes.get(rel_path.as_str()) {
            None => script_problems.push(format!(
                "{rel_path} is not in the git index, so it has no tracked mode and reaches no \
                 checkout but this one"
            )),
            Some(mode) if mode != "100755" => script_problems.push(format!(
                "{rel_path} is {mode} in the index, not 100755, so a fresh checkout cannot \
                 invoke it and `./{rel_path}` exits 126"
            )),
            Some(_) => {}
        }
        let bytes = fs::read(root.join(rel_path))
            .unwrap_or_else(|e| panic!("G4: cannot read {rel_path}: {e}"));
        if !bytes.starts_with(b"#!") {
            script_problems.push(format!(
                "{rel_path} opens with no `#!` line, so which interpreter runs it is whatever \
                 the invoking shell happens to be"
            ));
        }
    }

    assert!(
        script_problems.is_empty(),
        "G4: a shell script under scripts/ cannot be invoked.\n{}\n\n\
         A tracked guard that cannot be invoked is not tracked. This half derives its subjects \
         from the directory listing and consults no status field, because a hand written \
         judgment deciding which files get checked is the mechanism that let this class survive \
         a first round of repair. Every scripts/*.sh has to exist in the index at 100755 and \
         open with a `#!` line, whether some pin calls it a check or a helper. The index is the \
         authority: this repo sets core.fileMode=false, so `chmod +x` alone never reaches git \
         and the bit has to be set with `git update-index --chmod=+x <path>`. The .py helpers \
         beside them are deliberately outside this obligation, not overlooked: their callers \
         run them as `python3 <path>`, they carry no direct invocation contract, and each of \
         their rows in {SWEEPS_PIN_REL} states that.",
        script_problems.join("\n")
    );

    let pin_path = root.join(SWEEPS_PIN_REL);
    let raw = fs::read_to_string(&pin_path).unwrap_or_else(|e| {
        panic!(
            "G4: cannot read {}: {e}\n\n\
             The pin carries the second half of this guard, and a missing or unreadable pin \
             leaves that half with nothing to check. A guard with nothing to check passes, and \
             a guard that passes for that reason is the hole it exists to close, so it fails \
             here.",
            pin_path.display()
        )
    });

    let mut runs = Vec::new();
    let mut pin_problems = Vec::new();
    let mut rows = 0usize;
    for (i, line) in raw.lines().enumerate() {
        let lineno = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert!(
            cols.len() == 3,
            "G4: {SWEEPS_PIN_REL}:{lineno}: {} tab separated fields, expected 3 \
             (name, status, reason): {line}",
            cols.len()
        );
        rows += 1;
        let (name, status, reason) = (cols[0], cols[1], cols[2]);
        match status {
            "run" => {
                runs.push(name.to_string());
                if !scripts_dir.join(name).is_file() {
                    pin_problems.push(format!(
                        "{SWEEPS_PIN_REL}:{lineno}: `{name}` is pinned `run` but \
                         scripts/{name} does not exist"
                    ));
                }
            }
            "excluded" => {
                if reason.trim().is_empty() {
                    pin_problems.push(format!(
                        "{SWEEPS_PIN_REL}:{lineno}: `{name}` is excluded and states no reason"
                    ));
                }
            }
            other => pin_problems.push(format!(
                "{SWEEPS_PIN_REL}:{lineno}: `{name}` has status `{other}`, which is neither \
                 `run` nor `excluded`, so neither half of the pin obligation reaches the row"
            )),
        }
    }

    assert!(
        rows > 0,
        "G4: {} parsed to zero rows, so every obligation derived from it below is vacuous.",
        pin_path.display()
    );
    assert!(
        !runs.is_empty(),
        "G4: {} holds no `run` row, so the pin names no check that has to exist.",
        pin_path.display()
    );
    assert!(
        pin_problems.is_empty(),
        "G4: the sweeps pin does not describe the tree.\n{}\n\n\
         A `run` row names a check whose acceptance includes being executed, so the file it \
         names has to be there. An `excluded` row with no stated reason is a check dropped in \
         silence, which is where this whole class came from. A status that is neither would \
         fall out of both halves and be enforced by nothing, so it fails here rather than \
         passing quietly.",
        pin_problems.join("\n")
    );
}

#[test]
fn g5_every_error_code_is_emitted_and_explained() {
    let root = repo_root();

    let rows = aelys_common::diagnostic::registry::all_codes();
    let registered: BTreeSet<String> = rows.iter().map(|row| row.code.to_string()).collect();
    assert!(
        !registered.is_empty(),
        "G5: {REGISTRY_REL} handed back zero rows, so both halves of this check would compare \
         against nothing and pass. An empty registry is the strongest form of the defect this \
         guard exists to catch, so it fails here rather than reporting green."
    );
    assert_eq!(
        rows.len(),
        registered.len(),
        "G5: {REGISTRY_REL} holds two rows for the same code. `lookup` returns the first, so the \
         second explains nothing and no edit to it can ever be seen."
    );
    for row in rows {
        assert!(
            is_error_code(row.code),
            "G5: {REGISTRY_REL} holds the row `{}`, which is not an `Exxxx` code and can never be \
             reached by `--explain`",
            row.code
        );
        assert!(
            !row.title.trim().is_empty() && !row.explanation.trim().is_empty(),
            "G5: {}'s row carries an empty title or explanation, which answers `--explain` with \
             nothing and is the same hole as having no row at all",
            row.code
        );
    }

    let mut emitted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for source in [COMPILE_CODE_REL, FAULT_REL] {
        let path = root.join(source);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("governance: cannot read {source}: {e}"));
        let stripped =
            strip_rust_literals(&text).unwrap_or_else(|e| panic!("governance: {source}: {e}"));
        let body = fn_body(&stripped, "fn code(")
            .unwrap_or_else(|| panic!("governance: {source}: no `fn code(` body to read"));
        let arms = arm_numbers(body);
        assert!(
            !arms.is_empty(),
            "G5: `fn code` in {source} parsed to zero numeric arms. The map that turns a kind \
             into a code is what makes a code emitted, so a zero here would empty the emitted \
             set and let the whole predicate pass having compared nothing."
        );
        for number in arms {
            emitted
                .entry(format!("E{number:04}"))
                .or_default()
                .insert(source.to_string());
        }
    }

    let mut scanned = 0usize;
    let mut from_literals: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for member in workspace_members(&root) {
        let src = root.join(&member).join("src");
        if !src.is_dir() {
            continue;
        }
        for file in rs_files_recursive(&src) {
            let name = rel(&root, &file);
            if name == REGISTRY_REL {
                continue;
            }
            scanned += 1;
            let text = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("governance: cannot read {name}: {e}"));
            let codes = emitted_codes_in_source(&text)
                .unwrap_or_else(|e| panic!("governance: {name}: {e}"));
            for code in codes {
                from_literals.entry(code).or_default().insert(name.clone());
            }
        }
    }
    assert!(
        scanned > 0,
        "G5: the walk visited no `src` tree at all, so the literal scan measured nothing"
    );
    assert!(
        !from_literals.is_empty(),
        "G5: the scan of {scanned} source files found no `Exxxx` literal. Every code raised \
         outside the two numeric maps is written as a literal at its emission site, so a zero \
         here is a broken scan reporting an empty world rather than a tree that emits nothing."
    );
    for (code, sites) in from_literals {
        emitted.entry(code).or_default().extend(sites);
    }

    let emitted_set: BTreeSet<String> = emitted.keys().cloned().collect();

    let unexplained: Vec<String> = emitted_set
        .difference(&registered)
        .map(|code| {
            let sites: Vec<&str> = emitted[code].iter().map(String::as_str).collect();
            format!("{code}\temitted by {}", sites.join(", "))
        })
        .collect();
    assert!(
        unexplained.is_empty(),
        "G5: a code the compiler can raise has no `--explain` entry.\n{}\n\n\
         Every rendering of these codes ends with `try `aelys --explain <code>``, and that \
         command answers an unregistered code with `unknown error code`. The user is sent to a \
         door that is not there. Add a row to {REGISTRY_REL} for each code above, or stop \
         emitting it.",
        unexplained.join("\n")
    );

    let unemitted: BTreeSet<String> = registered.difference(&emitted_set).cloned().collect();

    assert_eq!(
        UNEMITTED_RESIDUE.len(),
        UNEMITTED_RESIDUE_ROWS,
        "G5: the residue changed size. It is the one hand-written list left in this check, so \
         its length is pinned and every entry added to it is a deliberate edit here."
    );

    let mut residue: BTreeSet<String> = BTreeSet::new();
    for (code, reason) in UNEMITTED_RESIDUE {
        assert!(
            !reason.trim().is_empty(),
            "G5: the residue lists {code} with no reason. An exception with no stated cause is \
             the hand-maintained list this predicate replaces, one entry long."
        );
        assert!(
            residue.insert((*code).to_string()),
            "G5: the residue lists {code} twice, so one of the two reasons is enforced by nothing"
        );
        assert!(
            unemitted.contains(*code),
            "G5: the residue excuses {code}, which is not in the drift: it is either emitted \
             again or gone from {REGISTRY_REL}. A repaired exception that survives its own \
             repair is how the next stale row gets written, so it is removed here.\nreason on \
             file: {reason}"
        );
    }

    let rotting: Vec<String> = unemitted.difference(&residue).cloned().collect();
    assert!(
        rotting.is_empty(),
        "G5: {REGISTRY_REL} explains a code nothing can raise.\n{}\n\n\
         A row for a code no program can reach is documentation that is never read and never \
         contradicted, so it rots undetected and the count above it stops meaning anything. \
         Delete the row, or list the code in UNEMITTED_RESIDUE with the measured reason it is \
         kept.",
        rotting.join("\n")
    );

    assert_eq!(
        registered.len(),
        REGISTERED_CODES,
        "G5: the registry's row count moved. The count is pinned so that deleting rows is a \
         failure rather than a faster pass: with both directions derived, an empty registry and \
         a silent compiler agree with each other."
    );
}

// a26: the row is made to fire, on the construction the review used to walk past it
#[test]
fn g6_the_ablation_puts_a_refusal_in_the_cli_entry_point_and_the_row_goes_red() {
    const ABLATED_HOLDER: &str = "fn compile_file_with_llvm_sources(";
    const ABLATED_REFUSAL: &str = "\n    if opt_level != OptimizationLevel::None {\n                 return Err(backend_diagnostic_error(\n            artifacts.source.clone(),\n                     program_anchor_span(&artifacts.air, artifacts.source.as_ref()),\n                     \"walkpast-probe\",\n            \"ablation\",\n            None,\n                     None,\n            Fault::Unsupported,\n        ));\n    }\n";

    let root = repo_root();
    let llvm = root.join(DRIVER_LLVM_REL);
    let driver_mod = format!("{DRIVER_LLVM_REL}/mod.rs");
    let mut sources: Vec<(String, String)> = Vec::new();
    for path in rs_files_recursive(&llvm) {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", path.display()));
        let name = rel(&root, &path);
        let text = if name == driver_mod {
            let at = text.find(ABLATED_HOLDER).unwrap_or_else(|| {
                panic!("G6-ablation: {driver_mod} has no `{ABLATED_HOLDER}` to ablate")
            });
            let (open, _) = brace_span(&text, at).expect("G6-ablation: the entry point has a body");
            let mut ablated = text.clone();
            ablated.insert_str(open + 1, ABLATED_REFUSAL);
            ablated
        } else {
            text
        };
        sources.push((name, without_test_items(&masked(&text))));
    }

    let (_, mod_src) = sources
        .iter()
        .find(|(name, _)| *name == driver_mod)
        .unwrap_or_else(|| panic!("G6-ablation: {driver_mod} is not in the walk"));
    let stage_at = mod_src
        .find(COMPARED_STAGE)
        .expect("G6-ablation: the compared stage is still there");
    let (compared_open, compared_close) =
        brace_span(mod_src, stage_at).expect("G6-ablation: the compared stage has a body");

    let census = rejection_census(&sources, &driver_mod, compared_open, compared_close);
    let unexcused = unexcused_sites(&census.uncompared);
    let named: Vec<&String> = unexcused
        .iter()
        .filter(|site| site.contains("compile_file_with_llvm_sources -> backend_diagnostic_error"))
        .collect();
    assert_eq!(
        named.len(),
        1,
        "G6-ablation: a user facing refusal built with the repo's own constructor was placed in \
         the function the CLI calls, and the derivation did not report it as unexcused. That is \
         the exact edit that walked past this row before, so a green here is the row not being a \
         guard.\nunexcused: {:?}",
        unexcused
    );
    assert!(
        !census
            .uncompared_keys
            .iter()
            .any(|key| key.ends_with(":compile_file_with_llvm_sources:backend_diagnostic_error")
                && UNCOMPARED_REJECTION_RESIDUE
                    .iter()
                    .any(|(excused, _)| *excused == key.as_str())),
        "G6-ablation: the residue already excuses the ablated site, so the row could never go red \
         on it"
    );
}

struct IgnoreSite {
    line: usize,
    reason: Option<String>,
    predicate: Option<String>,
    test: Option<String>,
}

struct AttrIgnore {
    reason: Option<String>,
    predicate: Option<String>,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// an `#[ignore` inside a string literal or a comment is not an ignore, so this walks tokens instead of lines
fn scan_ignores(src: &str) -> Result<Vec<IgnoreSite>, String> {
    let ch: Vec<char> = src.chars().collect();
    let n = ch.len();
    let newlines: Vec<usize> = ch
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == '\n')
        .map(|(i, _)| i)
        .collect();
    let line_of = |idx: usize| newlines.partition_point(|nl| *nl < idx) + 1;

    let mut sites: Vec<IgnoreSite> = Vec::new();
    let mut pending: Option<usize> = None;
    let mut expect_name = false;
    let mut i = 0usize;

    while i < n {
        let c = ch[i];

        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if ch[i] == '/' && i + 1 < n && ch[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if ch[i] == '*' && i + 1 < n && ch[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if let Some((end, _)) = read_raw_string(&ch, i)? {
            i = end;
            continue;
        }
        if c == '"' {
            let (end, _) = read_basic_string(&ch, i).map_err(|e| format!("line {}: {e}", line_of(i)))?;
            i = end;
            continue;
        }
        if c == '\'' {
            i = skip_char_or_lifetime(&ch, i);
            continue;
        }
        if c == '#' {
            match read_ignore_attr(&ch, i).map_err(|e| format!("line {}: {e}", line_of(i)))? {
                Some((end, found)) => {
                    if pending.is_some() {
                        return Err(format!(
                            "line {}: an ignore attribute follows another with no fn between them",
                            line_of(i)
                        ));
                    }
                    sites.push(IgnoreSite {
                        line: line_of(i),
                        reason: found.reason,
                        predicate: found.predicate,
                        test: None,
                    });
                    pending = Some(sites.len() - 1);
                    expect_name = false;
                    i = end;
                    continue;
                }
                None => {
                    i += 1;
                    continue;
                }
            }
        }
        if is_ident(c) && !c.is_ascii_digit() {
            let start = i;
            while i < n && is_ident(ch[i]) {
                i += 1;
            }
            let word: String = ch[start..i].iter().collect();
            if let Some(idx) = pending {
                if expect_name {
                    sites[idx].test = Some(word);
                    pending = None;
                    expect_name = false;
                } else if word == "fn" {
                    expect_name = true;
                }
            }
            continue;
        }
        i += 1;
    }

    if let Some(idx) = pending {
        return Err(format!(
            "line {}: an ignore attribute with no fn after it",
            sites[idx].line
        ));
    }
    Ok(sites)
}

fn read_raw_string(ch: &[char], i: usize) -> Result<Option<(usize, String)>, String> {
    let n = ch.len();
    let mut j = i;
    if ch[j] == 'b' || ch[j] == 'c' {
        j += 1;
    }
    if j >= n || ch[j] != 'r' {
        return Ok(None);
    }
    if i > 0 && is_ident(ch[i - 1]) {
        return Ok(None);
    }
    j += 1;
    let hash_start = j;
    while j < n && ch[j] == '#' {
        j += 1;
    }
    let hashes = j - hash_start;
    if j >= n || ch[j] != '"' {
        return Ok(None);
    }
    j += 1;
    let body_start = j;
    loop {
        if j >= n {
            return Err("unterminated raw string".to_string());
        }
        if ch[j] == '"' && ch[j + 1..].iter().take(hashes).filter(|c| **c == '#').count() == hashes {
            let body: String = ch[body_start..j].iter().collect();
            return Ok(Some((j + 1 + hashes, body)));
        }
        j += 1;
    }
}

fn read_basic_string(ch: &[char], i: usize) -> Result<(usize, String), String> {
    let n = ch.len();
    let mut j = i + 1;
    let mut out = String::new();
    while j < n {
        match ch[j] {
            '"' => return Ok((j + 1, out)),
            '\\' => {
                j += 1;
                if j >= n {
                    break;
                }
                match ch[j] {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '0' => out.push('\0'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    'x' => {
                        let hex: String = ch[j + 1..(j + 3).min(n)].iter().collect();
                        let code = u32::from_str_radix(&hex, 16)
                            .map_err(|_| format!("bad \\x escape `{hex}`"))?;
                        out.push(char::from_u32(code).unwrap_or('?'));
                        j += 2;
                    }
                    'u' => {
                        let mut k = j + 1;
                        if k >= n || ch[k] != '{' {
                            return Err("bad \\u escape".to_string());
                        }
                        k += 1;
                        let start = k;
                        while k < n && ch[k] != '}' {
                            k += 1;
                        }
                        let hex: String = ch[start..k].iter().collect();
                        let code = u32::from_str_radix(&hex, 16)
                            .map_err(|_| format!("bad \\u escape `{hex}`"))?;
                        out.push(char::from_u32(code).unwrap_or('?'));
                        j = k;
                    }
                    // a backslash before a newline eats the newline and the indentation of the next line
                    '\n' => {
                        j += 1;
                        while j < n && (ch[j] == ' ' || ch[j] == '\t' || ch[j] == '\r' || ch[j] == '\n') {
                            j += 1;
                        }
                        continue;
                    }
                    '\r' => {
                        j += 1;
                        while j < n && (ch[j] == ' ' || ch[j] == '\t' || ch[j] == '\r' || ch[j] == '\n') {
                            j += 1;
                        }
                        continue;
                    }
                    other => return Err(format!("unsupported escape `\\{other}`")),
                }
                j += 1;
            }
            other => {
                out.push(other);
                j += 1;
            }
        }
    }
    Err("unterminated string literal".to_string())
}

// a lone quote is a lifetime or a loop label, only `'x'` and `'\x'` are literals
fn skip_char_or_lifetime(ch: &[char], i: usize) -> usize {
    let n = ch.len();
    // the escaped char comes right after the backslash, so `'\\'` closes at i+3 and the scan starts past it
    if i + 1 < n && ch[i + 1] == '\\' {
        let mut j = i + 3;
        while j < n && ch[j] != '\'' {
            j += 1;
        }
        return (j + 1).min(n);
    }
    if i + 2 < n && ch[i + 2] == '\'' {
        return i + 3;
    }
    i + 1
}

fn read_ignore_attr(ch: &[char], i: usize) -> Result<Option<(usize, AttrIgnore)>, String> {
    let n = ch.len();
    let skip_ws = |mut j: usize| {
        while j < n && ch[j].is_whitespace() {
            j += 1;
        }
        j
    };
    let mut j = i + 1;
    if j < n && ch[j] == '!' {
        j += 1;
    }
    j = skip_ws(j);
    if j >= n || ch[j] != '[' {
        return Ok(None);
    }
    j = skip_ws(j + 1);
    let start = j;
    while j < n && is_ident(ch[j]) {
        j += 1;
    }
    let head: String = ch[start..j].iter().collect();
    if head == "cfg_attr" {
        let open = skip_ws(j);
        if open >= n || ch[open] != '(' {
            return Ok(None);
        }
        let (end, found) = read_cfg_attr_args(ch, open, None)?;
        let Some(found) = found else {
            return Ok(None);
        };
        let end = skip_ws(end);
        if end >= n || ch[end] != ']' {
            return Err("malformed cfg_attr attribute".to_string());
        }
        return Ok(Some((end + 1, found)));
    }
    if head != "ignore" {
        return Ok(None);
    }
    j = skip_ws(j);
    let mut reason = None;
    if j < n && ch[j] == '=' {
        j = skip_ws(j + 1);
        if let Some((end, body)) = read_raw_string(ch, j)? {
            reason = Some(body);
            j = end;
        } else if j < n && ch[j] == '"' {
            let (end, body) = read_basic_string(ch, j)?;
            reason = Some(body);
            j = end;
        } else {
            return Err("an ignore reason must be a string literal".to_string());
        }
        j = skip_ws(j);
    }
    if j >= n || ch[j] != ']' {
        return Err("malformed ignore attribute".to_string());
    }
    Ok(Some((
        j + 1,
        AttrIgnore {
            reason,
            predicate: None,
        },
    )))
}

fn read_attr_string(ch: &[char], j: usize) -> Result<Option<(usize, String)>, String> {
    if let Some(hit) = read_raw_string(ch, j)? {
        return Ok(Some(hit));
    }
    if ch[j] == '"' {
        return read_basic_string(ch, j).map(Some);
    }
    Ok(None)
}

// rustc applies every attribute listed after the predicate, so a nested cfg_attr can hide an ignore one level down
fn read_cfg_attr_args(
    ch: &[char],
    open: usize,
    outer: Option<&str>,
) -> Result<(usize, Option<AttrIgnore>), String> {
    let n = ch.len();
    let skip_ws = |mut j: usize| {
        while j < n && ch[j].is_whitespace() {
            j += 1;
        }
        j
    };

    let mut j = open + 1;
    let pred_start = j;
    let mut depth = 0usize;
    loop {
        if j >= n {
            return Err("unterminated cfg_attr predicate".to_string());
        }
        if let Some((end, _)) = read_attr_string(ch, j)? {
            j = end;
            continue;
        }
        match ch[j] {
            '(' | '[' | '{' => {
                depth += 1;
                j += 1;
            }
            ')' | ']' | '}' => {
                if depth == 0 {
                    return Err("cfg_attr carries no attribute after its predicate".to_string());
                }
                depth -= 1;
                j += 1;
            }
            ',' if depth == 0 => break,
            _ => j += 1,
        }
    }
    let predicate: String = ch[pred_start..j]
        .iter()
        .filter(|c| !c.is_whitespace())
        .collect();
    if predicate.is_empty() {
        return Err("cfg_attr carries an empty predicate".to_string());
    }
    let combined = match outer {
        Some(o) => format!("all({o},{predicate})"),
        None => predicate,
    };

    let mut found: Option<AttrIgnore> = None;
    j += 1;
    loop {
        j = skip_ws(j);
        if j >= n {
            return Err("unterminated cfg_attr".to_string());
        }
        if ch[j] == ')' {
            return Ok((j + 1, found));
        }
        let start = j;
        while j < n && is_ident(ch[j]) {
            j += 1;
        }
        let word: String = ch[start..j].iter().collect();
        if word == "ignore" {
            let mut k = skip_ws(j);
            let mut reason = None;
            if k < n && ch[k] == '=' {
                k = skip_ws(k + 1);
                if k >= n {
                    return Err("unterminated cfg_attr".to_string());
                }
                match read_attr_string(ch, k)? {
                    Some((end, body)) => {
                        reason = Some(body);
                        k = end;
                    }
                    None => return Err("an ignore reason must be a string literal".to_string()),
                }
            }
            found = Some(AttrIgnore {
                reason,
                predicate: Some(combined.clone()),
            });
            j = k;
        } else if word == "cfg_attr" {
            let inner_open = skip_ws(j);
            if inner_open < n && ch[inner_open] == '(' {
                let (end, inner) = read_cfg_attr_args(ch, inner_open, Some(&combined))?;
                if inner.is_some() {
                    found = inner;
                }
                j = end;
            }
        }

        let mut d = 0usize;
        loop {
            if j >= n {
                return Err("unterminated cfg_attr".to_string());
            }
            if let Some((end, _)) = read_attr_string(ch, j)? {
                j = end;
                continue;
            }
            match ch[j] {
                '(' | '[' | '{' => {
                    d += 1;
                    j += 1;
                }
                ')' | ']' | '}' if d > 0 => {
                    d -= 1;
                    j += 1;
                }
                ')' => return Ok((j + 1, found)),
                ',' if d == 0 => {
                    j += 1;
                    break;
                }
                _ => j += 1,
            }
        }
    }
}

fn is_error_code(word: &str) -> bool {
    word.len() == 5 && word.starts_with('E') && word[1..].chars().all(|c| c.is_ascii_digit())
}

fn skip_block_comment(ch: &[char], i: usize) -> usize {
    let n = ch.len();
    let mut depth = 1usize;
    let mut j = i + 2;
    while j < n && depth > 0 {
        if ch[j] == '/' && j + 1 < n && ch[j + 1] == '*' {
            depth += 1;
            j += 2;
        } else if ch[j] == '*' && j + 1 < n && ch[j + 1] == '/' {
            depth -= 1;
            j += 2;
        } else {
            j += 1;
        }
    }
    j
}

fn word_at(ch: &[char], j: usize, word: &str) -> Option<usize> {
    let mut k = j;
    for c in word.chars() {
        if k >= ch.len() || ch[k] != c {
            return None;
        }
        k += 1;
    }
    Some(k)
}

fn cfg_test_attr_end(ch: &[char], i: usize) -> Option<usize> {
    let n = ch.len();
    let skip_ws = |mut j: usize| {
        while j < n && ch[j].is_whitespace() {
            j += 1;
        }
        j
    };
    let mut j = i + 1;
    if j < n && ch[j] == '!' {
        j += 1;
    }
    if j >= n || ch[j] != '[' {
        return None;
    }
    j = skip_ws(j + 1);
    j = word_at(ch, j, "cfg")?;
    if j < n && is_ident(ch[j]) {
        return None;
    }
    j = skip_ws(j);
    if j >= n || ch[j] != '(' {
        return None;
    }
    j = skip_ws(j + 1);
    j = word_at(ch, j, "test")?;
    if j < n && is_ident(ch[j]) {
        return None;
    }
    j = skip_ws(j);
    if j >= n || ch[j] != ')' {
        return None;
    }
    j = skip_ws(j + 1);
    if j >= n || ch[j] != ']' {
        return None;
    }
    Some(j + 1)
}

fn skip_attributed_item(ch: &[char], from: usize) -> Result<usize, String> {
    let n = ch.len();
    let mut nesting = 0usize;
    let mut i = from;
    while i < n {
        let c = ch[i];
        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            i = skip_block_comment(ch, i);
            continue;
        }
        if let Some((end, _)) = read_raw_string(ch, i)? {
            i = end;
            continue;
        }
        if c == '"' {
            let (end, _) = read_basic_string(ch, i)?;
            i = end;
            continue;
        }
        if c == '\'' {
            i = skip_char_or_lifetime(ch, i);
            continue;
        }
        if is_ident(c) && !c.is_ascii_digit() {
            while i < n && is_ident(ch[i]) {
                i += 1;
            }
            continue;
        }
        match c {
            '(' | '[' => {
                nesting += 1;
                i += 1;
            }
            ')' | ']' => {
                nesting = nesting.saturating_sub(1);
                i += 1;
            }
            ';' if nesting == 0 => return Ok(i + 1),
            '{' if nesting == 0 => {
                let mut depth = 0usize;
                let mut j = i;
                while j < n {
                    let d = ch[j];
                    if d == '/' && j + 1 < n && ch[j + 1] == '/' {
                        while j < n && ch[j] != '\n' {
                            j += 1;
                        }
                        continue;
                    }
                    if d == '/' && j + 1 < n && ch[j + 1] == '*' {
                        j = skip_block_comment(ch, j);
                        continue;
                    }
                    if let Some((end, _)) = read_raw_string(ch, j)? {
                        j = end;
                        continue;
                    }
                    if d == '"' {
                        let (end, _) = read_basic_string(ch, j)?;
                        j = end;
                        continue;
                    }
                    if d == '\'' {
                        j = skip_char_or_lifetime(ch, j);
                        continue;
                    }
                    if d == '{' {
                        depth += 1;
                    } else if d == '}' {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(j + 1);
                        }
                    }
                    j += 1;
                }
                return Err("unterminated #[cfg(test)] item".to_string());
            }
            _ => i += 1,
        }
    }
    Err("a #[cfg(test)] attribute with no item after it".to_string())
}

// a code inside a #[cfg(test)] item is an assertion about an emission and not one, so those items are skipped
fn emitted_codes_in_source(src: &str) -> Result<BTreeSet<String>, String> {
    let ch: Vec<char> = src.chars().collect();
    let n = ch.len();
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut i = 0usize;

    while i < n {
        let c = ch[i];

        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            i = skip_block_comment(&ch, i);
            continue;
        }
        if c == '#' {
            if let Some(end) = cfg_test_attr_end(&ch, i) {
                i = skip_attributed_item(&ch, end)?;
                continue;
            }
        }
        if let Some((end, body)) = read_raw_string(&ch, i)? {
            if is_error_code(&body) {
                out.insert(body);
            }
            i = end;
            continue;
        }
        if c == '"' {
            let (end, body) = read_basic_string(&ch, i)?;
            if is_error_code(&body) {
                out.insert(body);
            }
            i = end;
            continue;
        }
        if c == '\'' {
            i = skip_char_or_lifetime(&ch, i);
            continue;
        }
        if is_ident(c) && !c.is_ascii_digit() {
            while i < n && is_ident(ch[i]) {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    Ok(out)
}

fn strip_rust_literals(src: &str) -> Result<String, String> {
    let ch: Vec<char> = src.chars().collect();
    let n = ch.len();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;

    while i < n {
        let c = ch[i];

        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            i = skip_block_comment(&ch, i);
            out.push(' ');
            continue;
        }
        if let Some((end, _)) = read_raw_string(&ch, i)? {
            out.push(' ');
            i = end;
            continue;
        }
        if c == '"' {
            let (end, _) = read_basic_string(&ch, i)?;
            out.push(' ');
            i = end;
            continue;
        }
        if c == '\'' {
            let end = skip_char_or_lifetime(&ch, i);
            out.push(' ');
            i = end;
            continue;
        }
        if is_ident(c) && !c.is_ascii_digit() {
            while i < n && is_ident(ch[i]) {
                out.push(ch[i]);
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    Ok(out)
}

fn fn_body<'a>(stripped: &'a str, needle: &str) -> Option<&'a str> {
    let start = stripped.find(needle)?;
    let open = start + stripped[start..].find('{')?;
    let mut depth = 0usize;
    for (k, b) in stripped.bytes().enumerate().skip(open) {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&stripped[open + 1..k]);
            }
        }
    }
    None
}

fn arm_numbers(body: &str) -> BTreeSet<u16> {
    let ch: Vec<char> = body.chars().collect();
    let n = ch.len();
    let mut out: BTreeSet<u16> = BTreeSet::new();
    let mut i = 0usize;

    while i + 1 < n {
        if ch[i] != '=' || ch[i + 1] != '>' {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        while j < n && ch[j].is_whitespace() {
            j += 1;
        }
        let start = j;
        while j < n && ch[j].is_ascii_digit() {
            j += 1;
        }
        if j > start {
            let mut k = j;
            while k < n && ch[k].is_whitespace() {
                k += 1;
            }
            if k < n && (ch[k] == ',' || ch[k] == '}') {
                let digits: String = ch[start..j].iter().collect();
                if let Ok(value) = digits.parse::<u16>() {
                    out.insert(value);
                }
            }
        }
        i = j.max(i + 2);
    }
    out
}

const DRIVER_LLVM_REL: &str = "driver/src/api/llvm";
const AIR_SRC_REL: &str = "air/src";
const COMPARED_STAGE: &str = "fn air_stage(";
const COMPARED_STAGE_CALLER: &str = "fn lower_file_to_air_with_source(";

const COMPARED_REJECTION_SITES: usize = 12;
const UNCOMPARED_REJECTION_SITES: usize = 22;

// the functions whose refusals the residue excuses for raising before any pass has run
const PRE_OPTIMIZER_HELPERS: &[&str] = &["front_stage", "build_imports"];

// a rejection site the compared stage does not cover, keyed by holder and constructor, and why
const UNCOMPARED_REJECTION_RESIDUE: &[(&str, &str)] = &[
    (
        "driver/src/api/llvm/mod.rs:lower_file_to_air_with_source:optimization_verdict_split",
        "the net's own refusal: it is the comparison, so it cannot be one of the verdicts compared",
    ),
    (
        "driver/src/api/llvm/lower.rs:compile_air_with_llvm_linked:llvm_backend_error_to_diagnostic",
        "llvm module verify, the level-selected pass pipeline, the ir writer and the object \
         writer, all after the air the two runs agreed on; F6 records that the module is verified \
         before the pipeline and never after, and that is Stage 3",
    ),
    (
        "driver/src/api/llvm/lower.rs:compile_air_with_llvm_linked:backend_diagnostic_error",
        "resolving the aelys-core archive and running the linker; the link's undefined symbol set \
         is pinned to the unoptimised air by require_air_externs, and what is left faults Compiler",
    ),
    (
        "driver/src/api/llvm/lower.rs:compile_air_with_llvm_linked:claimed_runtime_symbol_error",
        "E0618, measured gated on has_main_entry and read off the linked executable, which exists \
         once and only on the level actually being compiled",
    ),
    (
        "driver/src/api/llvm/mod.rs:compile_to_typed_ast:sema_errors_to_diagnostics",
        "the typed-ast entry point: it stops at inference, builds no air and constructs no \
         Optimizer, so no level has chosen anything when it raises",
    ),
    (
        "driver/src/api/llvm/mod.rs:build_imports:import_error",
        "raised before the first air_stage call, which PRE_OPTIMIZER_HELPERS re-derives",
    ),
    (
        "driver/src/api/llvm/mod.rs:front_stage:sema_errors_to_diagnostics",
        "raised before the first air_stage call, which PRE_OPTIMIZER_HELPERS re-derives",
    ),
    (
        "driver/src/api/llvm/mod.rs:front_stage:foreign_clash_errors_to_error",
        "raised before the first air_stage call, which PRE_OPTIMIZER_HELPERS re-derives",
    ),
    (
        "driver/src/api/llvm/mod.rs:front_stage:bir_diagnostics_to_error",
        "raised before the first air_stage call, which PRE_OPTIMIZER_HELPERS re-derives",
    ),
    (
        "driver/src/api/llvm/mod.rs:front_stage:reserved_name_errors_to_error",
        "raised before the first air_stage call, which PRE_OPTIMIZER_HELPERS re-derives",
    ),
    (
        "driver/src/api/llvm/mod.rs:front_stage:foreign_signature_errors_to_error",
        "raised before the first air_stage call, which PRE_OPTIMIZER_HELPERS re-derives",
    ),
    (
        "driver/src/api/llvm/mod.rs:front_stage:multiple_diagnostics",
        "raised before the first air_stage call, which PRE_OPTIMIZER_HELPERS re-derives",
    ),
];

// per file, the air lowering sites that turn a program into e0902 or e0904
const AIR_REJECTION_CENSUS: &[(&str, usize)] = &[
    ("air/src/lower/expr.rs", 3),
    ("air/src/lower/mod.rs", 1),
    ("air/src/lower/stmts.rs", 7),
];

// a `#[cfg(test)]` item is not the compiler, so it must not read as a site in it
fn without_test_items(src: &str) -> String {
    let mut out = src.to_string();
    while let Some(at) = out.find("#[cfg(test)]") {
        let Some((_, close)) = brace_span(&out, at) else {
            break;
        };
        let blanked: String = out[at..=close]
            .chars()
            .map(|c| if c == '\n' { '\n' } else { ' ' })
            .collect();
        out.replace_range(at..=close, &blanked);
    }
    out
}

// comments and literals become blanks so a byte offset still names its own line
fn masked(src: &str) -> String {
    let ch: Vec<char> = src.chars().collect();
    let n = ch.len();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    let blank = |out: &mut String, from: usize, to: usize, ch: &[char]| {
        for c in ch.iter().take(to).skip(from) {
            out.push(if *c == '\n' { '\n' } else { ' ' });
        }
    };
    while i < n {
        let c = ch[i];
        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            let mut j = i;
            while j < n && ch[j] != '\n' {
                j += 1;
            }
            blank(&mut out, i, j, &ch);
            i = j;
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            let j = skip_block_comment(&ch, i);
            blank(&mut out, i, j, &ch);
            i = j;
            continue;
        }
        if let Ok(Some((end, _))) = read_raw_string(&ch, i) {
            blank(&mut out, i, end, &ch);
            i = end;
            continue;
        }
        if c == '"' {
            let (end, _) = read_basic_string(&ch, i).unwrap_or((i + 1, String::new()));
            blank(&mut out, i, end, &ch);
            i = end;
            continue;
        }
        if c == '\'' {
            let end = skip_char_or_lifetime(&ch, i);
            blank(&mut out, i, end, &ch);
            i = end;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn line_at(src: &str, at: usize) -> usize {
    src[..at].matches('\n').count() + 1
}

fn brace_span(src: &str, from: usize) -> Option<(usize, usize)> {
    let open = from + src[from..].find('{')?;
    let mut depth = 0usize;
    for (k, b) in src.bytes().enumerate().skip(open) {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some((open, k));
            }
        }
    }
    None
}

// every top level `fn name(..) -> aelyserror`: the whole set of ways this module builds a refusal
fn refusal_constructors(src: &str) -> Vec<(String, usize, usize)> {
    fn_spans(src)
        .into_iter()
        .filter(|(_, returns, _, _)| returns == "-> AelysError")
        .map(|(name, _, open, close)| (name, open, close))
        .collect()
}

fn fn_spans(src: &str) -> Vec<(String, String, usize, usize)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut at = 0usize;
    while let Some(hit) = src[at..].find("fn ") {
        let start = at + hit;
        at = start + 3;
        if start > 0 && is_ident(bytes[start - 1] as char) {
            continue;
        }
        let name_end = match src[at..].find(['(', '<']) {
            Some(k) => at + k,
            None => continue,
        };
        let name = src[at..name_end].trim().to_string();
        if name.is_empty() || !name.chars().all(is_ident) {
            continue;
        }
        let mut paren = name_end;
        while paren < src.len() && bytes[paren] != b'(' {
            paren += 1;
        }
        let mut depth = 0usize;
        let mut j = paren;
        while j < src.len() {
            if bytes[j] == b'(' {
                depth += 1;
            } else if bytes[j] == b')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        let Some((open, close)) = brace_span(src, j) else {
            continue;
        };
        out.push((name, src[j + 1..open].trim().to_string(), open, close));
    }
    out
}

fn call_sites(src: &str, names: &BTreeSet<String>) -> Vec<(String, usize)> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    for name in names {
        let mut at = 0usize;
        while let Some(hit) = src[at..].find(name.as_str()) {
            let start = at + hit;
            at = start + name.len();
            if start > 0 && is_ident(bytes[start - 1] as char) {
                continue;
            }
            let mut j = at;
            while j < src.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j >= src.len() || bytes[j] != b'(' {
                continue;
            }
            if src[..start].trim_end().ends_with("fn") {
                continue;
            }
            out.push((name.clone(), start));
        }
    }
    out.sort_by_key(|(_, at)| *at);
    out
}

fn air_module_file(root: &Path, path: &str) -> Option<PathBuf> {
    let segments: Vec<&str> = path.split("::").collect();
    for take in (1..=segments.len()).rev() {
        let base = root.join(AIR_SRC_REL).join(segments[..take].join("/"));
        if base.is_dir() {
            return Some(base);
        }
        let file = base.with_extension("rs");
        if file.is_file() {
            return Some(file);
        }
    }
    None
}

fn crate_paths(src: &str, head: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut at = 0usize;
    while let Some(hit) = src[at..].find(head) {
        let start = at + hit;
        at = start + head.len();
        let rest = &src[at..];
        let end = rest
            .find(|c: char| !(is_ident(c) || c == ':'))
            .unwrap_or(rest.len());
        let path = rest[..end].trim_end_matches(':');
        if !path.is_empty() && path.chars().next().is_some_and(|c| c.is_lowercase()) {
            out.insert(path.to_string());
        }
    }
    out
}

struct RejectionCensus {
    compared: Vec<String>,
    uncompared: Vec<(String, String)>,
    uncompared_keys: BTreeSet<String>,
}

// every call of a refusal constructor, in exactly one of two buckets and never in neither
fn rejection_census(
    sources: &[(String, String)],
    driver_mod: &str,
    compared_open: usize,
    compared_close: usize,
) -> RejectionCensus {
    let mut constructors: BTreeSet<String> = BTreeSet::new();
    let mut constructor_bodies: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (name, text) in sources {
        for (ctor, open, close) in refusal_constructors(text) {
            constructors.insert(ctor);
            constructor_bodies
                .entry(name.clone())
                .or_default()
                .push((open, close));
        }
    }

    let mut census = RejectionCensus {
        compared: Vec::new(),
        uncompared: Vec::new(),
        uncompared_keys: BTreeSet::new(),
    };
    for (name, text) in sources {
        let empty = Vec::new();
        let bodies = constructor_bodies.get(name).unwrap_or(&empty);
        let enclosing = fn_spans(text);
        for (ctor, at) in call_sites(text, &constructors) {
            if bodies
                .iter()
                .any(|(open, close)| *open <= at && at <= *close)
            {
                continue;
            }
            if name == driver_mod && compared_open <= at && at <= compared_close {
                census
                    .compared
                    .push(format!("{name}:{} {ctor}", line_at(text, at)));
                continue;
            }
            // no third arm: a site that reached here was never classified, and that is how g6 was walked past
            let holder = enclosing
                .iter()
                .filter(|(_, _, open, close)| *open <= at && at <= *close)
                .min_by_key(|(_, _, open, close)| close - open)
                .map(|(holder, _, _, _)| holder.clone())
                .unwrap_or_else(|| "<file scope>".to_string());
            let key = format!("{name}:{holder}:{ctor}");
            census.uncompared.push((
                format!("{name}:{} {holder} -> {ctor}", line_at(text, at)),
                key.clone(),
            ));
            census.uncompared_keys.insert(key);
        }
    }
    census
}

fn unexcused_sites(uncompared: &[(String, String)]) -> Vec<String> {
    let excused: BTreeSet<String> = UNCOMPARED_REJECTION_RESIDUE
        .iter()
        .map(|(key, _)| (*key).to_string())
        .collect();
    uncompared
        .iter()
        .filter(|(_, key)| !excused.contains(key))
        .map(|(site, _)| site.clone())
        .collect()
}

#[test]
fn g6_every_rejecting_check_the_optimizer_can_reach_is_on_the_compared_path() {
    let root = repo_root();
    let llvm = root.join(DRIVER_LLVM_REL);
    let mut sources: Vec<(String, String)> = Vec::new();
    for path in rs_files_recursive(&llvm) {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", path.display()));
        sources.push((rel(&root, &path), without_test_items(&masked(&text))));
    }
    assert!(
        !sources.is_empty(),
        "G6: {DRIVER_LLVM_REL} handed back no source at all, so every set derived below would be \
         empty and the whole row would pass having read nothing."
    );

    let constructors: BTreeSet<String> = sources
        .iter()
        .flat_map(|(_, text)| refusal_constructors(text))
        .map(|(ctor, _, _)| ctor)
        .collect();
    assert!(
        !constructors.is_empty(),
        "G6: no `fn .. -> AelysError` was found in {DRIVER_LLVM_REL}. Every refusal this stage \
         raises is built by one of those, so a zero here is a broken parse reporting a compiler \
         that cannot refuse anything."
    );

    let driver_mod = format!("{DRIVER_LLVM_REL}/mod.rs");
    let (_, mod_src) = sources
        .iter()
        .find(|(name, _)| *name == driver_mod)
        .unwrap_or_else(|| panic!("G6: {driver_mod} is not in the walk"));

    let stage_at = mod_src
        .find(COMPARED_STAGE)
        .unwrap_or_else(|| panic!("G6: {driver_mod} declares no `{COMPARED_STAGE}`. The compared stage is what runs twice, so without it nothing is compared and this row cannot be evaluated."));
    let (compared_open, compared_close) =
        brace_span(mod_src, stage_at).expect("G6: the compared stage has a body");

    let caller_at = mod_src
        .find(COMPARED_STAGE_CALLER)
        .unwrap_or_else(|| panic!("G6: {driver_mod} declares no `{COMPARED_STAGE_CALLER}`"));
    let (caller_open, caller_close) =
        brace_span(mod_src, caller_at).expect("G6: the caller has a body");

    let calls: Vec<usize> = mod_src[caller_open..caller_close]
        .match_indices("air_stage(")
        .map(|(k, _)| caller_open + k)
        .collect();
    assert_eq!(
        calls.len(),
        2,
        "G6: the compared stage is called {} time(s) from {COMPARED_STAGE_CALLER}, and the net is \
         two runs compared: one on the program the user wrote and one on the program the \
         optimizer left. One call is a compiler that takes a single verdict on trust.",
        calls.len()
    );
    let first_call = calls[0];
    let guard = "(opt_level != OptimizationLevel::None).then(";
    assert!(
        mod_src[caller_open..first_call].contains(guard),
        "G6: the reference run is not guarded by `{guard}`. At -O0 the optimizer installs no pass, \
         so that guard is both what makes the comparison run once there and the single place the \
         whole net can be switched off without moving one site; anything else in it has to be \
         read here rather than trusted."
    );
    assert!(
        mod_src[caller_open..first_call].contains("Optimizer::new")
            || mod_src[first_call..caller_close].contains("optimizer.optimize"),
        "G6: the optimizer is not run inside {COMPARED_STAGE_CALLER}, so the region this row \
         calls post-optimizer is not derived from anything."
    );
    assert!(
        !mod_src[compared_open..compared_close].contains("Optimizer::new"),
        "G6: the compared stage constructs an Optimizer of its own. It has to receive the program \
         it is given, or the reference run optimizes too and both sides say the same thing by \
         construction."
    );

    let census = rejection_census(&sources, &driver_mod, compared_open, compared_close);
    let RejectionCensus {
        compared,
        uncompared,
        uncompared_keys,
    } = census;

    // the residue excuses these for running early, so where they are called is derived and not trusted
    for helper in PRE_OPTIMIZER_HELPERS {
        let head = format!("{helper}(");
        let mut seen = 0usize;
        for (name, text) in &sources {
            for (from, _) in text.match_indices(&head) {
                if text[..from].trim_end().ends_with("fn") {
                    continue;
                }
                seen += 1;
                assert!(
                    *name == driver_mod && caller_open <= from && from < first_call,
                    "G6: {helper} is called at {name}:{}, outside the region of \
                     {COMPARED_STAGE_CALLER} that runs before the first `{COMPARED_STAGE}` call. \
                     The residue excuses its refusals for raising before any pass has run, so a \
                     call from anywhere else makes that reason false.",
                    line_at(text, from)
                );
            }
        }
        assert!(
            seen > 0,
            "G6: {helper} is named in PRE_OPTIMIZER_HELPERS and called nowhere in \
             {DRIVER_LLVM_REL}, so the residue entries that lean on it are excusing sites this \
             derivation can no longer see"
        );
    }

    let unexcused = unexcused_sites(&uncompared);
    assert!(
        unexcused.is_empty(),
        "G6: a check that can refuse a program runs after the optimizer and outside the compared \
         stage.\n{}\n\n\
         Everything downstream of `optimizer.optimize` reads a program the `-O` level chose, so a \
         refusal raised there is a verdict on the optimizer's output rather than on the source. \
         Move it inside `{COMPARED_STAGE}` so both runs see it, or list it in \
         UNCOMPARED_REJECTION_RESIDUE with the measured reason it cannot be compared.",
        unexcused.join("\n")
    );

    for (key, reason) in UNCOMPARED_REJECTION_RESIDUE {
        assert!(
            !reason.trim().is_empty(),
            "G6: the residue lists {key} with no reason, which is the hand written list this \
             derivation replaces, one entry long"
        );
        assert!(
            uncompared_keys.contains(*key),
            "G6: the residue excuses {key}, which the derivation no longer finds. An exception \
             that survives its own repair is how the next stale row gets written.\nreason on \
             file: {reason}"
        );
    }
    assert_eq!(
        UNCOMPARED_REJECTION_RESIDUE.len(),
        uncompared_keys.len(),
        "G6: the residue holds {} keys and the derivation finds {}: {:?}",
        UNCOMPARED_REJECTION_RESIDUE.len(),
        uncompared_keys.len(),
        uncompared_keys
    );
    assert_eq!(
        compared.len(),
        COMPARED_REJECTION_SITES,
        "G6: the compared stage now holds {} rejection sites, the census pinned {COMPARED_REJECTION_SITES}. \
         Re-derive it by reading the list rather than editing the number:\n{}",
        compared.len(),
        compared.join("\n")
    );
    assert_eq!(
        uncompared.len(),
        UNCOMPARED_REJECTION_SITES,
        "G6: {} rejection sites now run outside the compared stage, the census pinned \
         {UNCOMPARED_REJECTION_SITES}:\n{}",
        uncompared.len(),
        uncompared
            .iter()
            .map(|(site, _)| site.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );

    // the refusals the compared stage raises inside air are the ones no enumerated row named
    let mut frontier: BTreeSet<PathBuf> = BTreeSet::new();
    for path in crate_paths(&mod_src[compared_open..compared_close], "aelys_air::") {
        if let Some(file) = air_module_file(&root, &path) {
            frontier.insert(file);
        }
    }
    assert!(
        !frontier.is_empty(),
        "G6: the compared stage names no `aelys_air::` module, so the closure below would be \
         empty and every air side refusal would read as unreachable."
    );
    let mut closure: BTreeSet<PathBuf> = BTreeSet::new();
    while let Some(entry) = frontier.iter().next().cloned() {
        frontier.remove(&entry);
        let files = if entry.is_dir() {
            rs_files_recursive(&entry)
        } else {
            vec![entry.clone()]
        };
        for file in files {
            if !closure.insert(file.clone()) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            for path in crate_paths(&without_test_items(&masked(&text)), "crate::") {
                if let Some(next) = air_module_file(&root, &path) {
                    if !next.is_dir() && closure.contains(&next) {
                        continue;
                    }
                    frontier.insert(next);
                }
            }
        }
    }

    let mut air_sites: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut unreachable: Vec<String> = Vec::new();
    for file in rs_files_recursive(&root.join(AIR_SRC_REL)) {
        let text = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", file.display()));
        let text = without_test_items(&masked(&text));
        let name = rel(&root, &file);
        for pattern in ["self.report_unsupported(", "self.report_program("] {
            for (at, _) in text.match_indices(pattern) {
                let line = line_at(&text, at);
                if closure.contains(&file) {
                    air_sites.entry(name.clone()).or_default().push(line);
                } else {
                    unreachable.push(format!("{name}:{line} {pattern}"));
                }
            }
        }
    }
    assert!(
        unreachable.is_empty(),
        "G6: an air lowering refusal lives in a module the compared stage cannot reach.\n{}\n\n\
         Either it is dead and nothing raises it, or the compared stage is narrower than the \
         lowering that actually runs.",
        unreachable.join("\n")
    );

    let census: Vec<(String, usize)> = air_sites
        .iter()
        .map(|(name, lines)| (name.clone(), lines.len()))
        .collect();
    let pinned: Vec<(String, usize)> = AIR_REJECTION_CENSUS
        .iter()
        .map(|(name, count)| ((*name).to_string(), *count))
        .collect();
    assert_eq!(
        census,
        pinned,
        "G6: the air lowering refusal census moved. Every one of these is a check that can refuse \
         a program after the optimizer has rewritten it, and none of them carries an enumerated \
         row, so the census is what tracks them. Re-derive it from the sites below rather than \
         editing the counts:\n{}",
        air_sites
            .iter()
            .map(|(name, lines)| format!(
                "  {name}: {}",
                lines
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

const TRIAGE_REL: &str = "aelys/tests/wildcard_triage.tsv";

const TRIAGE_TYPES: &[&str] = &["InferType", "AirType", "ResolvedType", "AirIntSize"];

const TRIAGE_CLASSES: &[&str] = &["correct-as-is", "must-decide", "must-error"];

const TRIAGE_REASONS: &[(&str, &str)] = &[
    (
        "carrier-walk",
        "the leaf of a recursive walk asking whether a type carries an Rc, a Vec, a type \
         variable, a struct or an opaque; a scalar carries none of them",
    ),
    (
        "aggregate-only",
        "the arms name aggregate, pointer or collection shapes, and a scalar never reaches the \
         path they take",
    ),
    (
        "refuses-adaptation",
        "the arm answers no to a numeric rank, an integer literal fit or a float fit, which is \
         the refusal the primitive wants",
    ),
    (
        "delegates-to-a-decided-arm",
        "the fallback forwards to a function that carries an explicit arm for the primitive",
    ),
];

struct TriageRow {
    key: String,
    class: String,
    reason: String,
}

struct WildcardArm {
    file: String,
    function: String,
    ordinal: usize,
    ty: &'static str,
    pattern: String,
    rhs: String,
    decided: bool,
}

fn source_files_for_triage(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for member in workspace_members(root) {
        let src = root.join(&member).join("src");
        if src.is_dir() {
            out.extend(rs_files_recursive(&src));
        }
    }
    out.sort();
    out
}

fn enclosing_fn(src: &str, at: usize) -> String {
    let bytes = src.as_bytes();
    let mut last = String::from("<top>");
    let mut i = 0usize;
    while let Some(hit) = src[i..at].find("fn ") {
        let start = i + hit;
        let before_ok = start == 0 || !is_ident(src[..start].chars().next_back().unwrap_or(' '));
        i = start + 3;
        if !before_ok {
            continue;
        }
        let mut j = i;
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        let mut k = j;
        while k < bytes.len() && is_ident(bytes[k] as char) {
            k += 1;
        }
        if k > j {
            last = src[j..k].to_string();
        }
    }
    last
}

// the body braces of the match that starts at `from`, or none when it is not a match expression
fn match_body_span(src: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = src.as_bytes();
    let mut i = from;
    let (mut paren, mut bracket) = (0i32, 0i32);
    let open = loop {
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] as char {
            '(' => paren += 1,
            ')' => paren -= 1,
            '[' => bracket += 1,
            ']' => bracket -= 1,
            '{' if paren == 0 && bracket == 0 => break i,
            ';' | '}' => return None,
            _ => {}
        }
        i += 1;
    };
    let mut depth = 0i32;
    let mut j = open;
    while j < bytes.len() {
        match bytes[j] as char {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, j));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

// (pattern, right hand side) for each arm at the match's own level
fn top_level_arms(body: &str) -> Vec<(String, String)> {
    let bytes = body.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let (mut brace, mut paren, mut bracket) = (0i32, 0i32, 0i32);
    let mut head_start = 0usize;
    let mut i = 0usize;
    while i < n {
        let c = bytes[i] as char;
        if brace == 0
            && paren == 0
            && bracket == 0
            && c == '='
            && i + 1 < n
            && bytes[i + 1] as char == '>'
        {
            let head = body[head_start..i].to_string();
            i += 2;
            while i < n && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
            let rhs_start = i;
            if i < n && bytes[i] as char == '{' {
                let mut d = 0i32;
                while i < n {
                    match bytes[i] as char {
                        '{' => d += 1,
                        '}' => {
                            d -= 1;
                            if d == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let rhs_end = i;
                while i < n && matches!(bytes[i] as char, ' ' | '\t' | '\r' | '\n') {
                    i += 1;
                }
                if i < n && bytes[i] as char == ',' {
                    i += 1;
                }
                out.push((head, body[rhs_start..rhs_end].to_string()));
            } else {
                let (mut d, mut p, mut b) = (0i32, 0i32, 0i32);
                while i < n {
                    match bytes[i] as char {
                        '(' => p += 1,
                        ')' => p -= 1,
                        '[' => b += 1,
                        ']' => b -= 1,
                        '{' => d += 1,
                        '}' => d -= 1,
                        ',' if p == 0 && b == 0 && d == 0 => break,
                        _ => {}
                    }
                    i += 1;
                }
                let rhs_end = i;
                if i < n {
                    i += 1;
                }
                out.push((head, body[rhs_start..rhs_end].to_string()));
            }
            head_start = i;
            continue;
        }
        match c {
            '(' => paren += 1,
            ')' => paren -= 1,
            '[' => bracket += 1,
            ']' => bracket -= 1,
            '{' => brace += 1,
            '}' => brace -= 1,
            _ => {}
        }
        i += 1;
    }
    out
}

// a block body is blanked so a nested construction never makes the arm look like it names the type
fn without_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '{' => {
                depth += 1;
                out.push(' ');
            }
            '}' => {
                depth = (depth - 1).max(0);
                out.push(' ');
            }
            _ => out.push(if depth > 0 { ' ' } else { c }),
        }
    }
    out
}

fn is_wildcard_pattern(head: &str) -> bool {
    let head = head.trim().trim_start_matches('|');
    let head = match head.find(" if ") {
        Some(at) => &head[..at],
        None => head,
    };
    head.split('|').any(|alt| {
        let a = alt.trim();
        !a.is_empty()
            && a.chars().next().is_some_and(|c| c == '_' || c.is_lowercase())
            && a.chars().all(is_ident)
    })
}

fn one_line(text: &str, cap: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > cap {
        flat.chars().take(cap).collect()
    } else {
        flat
    }
}

fn rhs_digest(rhs: &str) -> String {
    let t = rhs.trim();
    if let Some(inner) = t.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        format!("{{ {} }}", one_line(inner, 44))
    } else {
        one_line(t.split(',').next().unwrap_or(t), 48)
    }
}

fn wildcard_arms(root: &Path) -> Vec<WildcardArm> {
    let mut out = Vec::new();
    for file in source_files_for_triage(root) {
        let name = rel(root, &file);
        let raw = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("governance: cannot read {name}: {e}"));
        let src = masked(&raw);
        let mut per_fn: BTreeMap<String, usize> = BTreeMap::new();
        let mut at = 0usize;
        while let Some(hit) = src[at..].find("match") {
            let start = at + hit;
            at = start + 5;
            let before = src[..start].chars().next_back().unwrap_or(' ');
            let after = src[start + 5..].chars().next().unwrap_or(' ');
            if is_ident(before) || is_ident(after) {
                continue;
            }
            let Some((open, close)) = match_body_span(&src, start + 5) else {
                continue;
            };
            let arms = top_level_arms(&src[open + 1..close]);
            if arms.is_empty() {
                continue;
            }
            let surface: String = arms
                .iter()
                .map(|(h, r)| format!("{h}\n{}", without_blocks(r)))
                .collect::<Vec<_>>()
                .join("\n");
            let Some(ty) = TRIAGE_TYPES
                .iter()
                .find(|t| surface.contains(&format!("{t}::")))
            else {
                continue;
            };
            let Some((head, rhs)) = arms.iter().rev().find(|(h, _)| is_wildcard_pattern(h)) else {
                continue;
            };
            let function = enclosing_fn(&src, start);
            let ordinal = per_fn.entry(function.clone()).or_insert(0);
            *ordinal += 1;
            out.push(WildcardArm {
                file: name.clone(),
                function: function.clone(),
                ordinal: *ordinal,
                ty,
                pattern: one_line(head, 48),
                rhs: rhs_digest(rhs),
                decided: surface.contains(&format!("{ty}::Char")),
            });
        }
    }
    out
}

#[test]
fn g7_every_wildcard_arm_a_new_primitive_reaches_is_triaged() {
    let root = repo_root();
    let arms = wildcard_arms(&root);
    assert!(
        arms.len() > 100,
        "G7: the derivation found {} wildcard arms over {:?}. The census is the deliverable, so \
         a count this low is a broken scan reporting an empty world rather than a tree with no \
         wildcards.",
        arms.len(),
        TRIAGE_TYPES
    );

    let decided: Vec<&WildcardArm> = arms.iter().filter(|a| a.decided).collect();
    assert!(
        !decided.is_empty(),
        "G7: not one of the {} wildcard arms sits in a match that names `Char`. Adding a \
         primitive means writing explicit arms, so a zero here means the derivation cannot see \
         them and the whole split is meaningless.",
        arms.len()
    );

    let measured: BTreeMap<String, &WildcardArm> = arms
        .iter()
        .filter(|a| !a.decided)
        .map(|a| {
            (
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    a.file, a.function, a.ordinal, a.ty, a.pattern, a.rhs
                ),
                a,
            )
        })
        .collect();

    let path = root.join(TRIAGE_REL);
    let pinned_text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "governance: cannot read {}: {e}\nmeasured rows are:\n{}",
            path.display(),
            measured.keys().cloned().collect::<Vec<_>>().join("\n")
        )
    });

    let mut pinned: BTreeMap<String, TriageRow> = BTreeMap::new();
    for (n, line) in pinned_text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            cols.len(),
            8,
            "G7: {TRIAGE_REL}:{}: a row has {} tab separated columns, not 8",
            n + 1,
            cols.len()
        );
        let key = cols[..6].join("\t");
        let class = cols[6].to_string();
        let reason = cols[7].to_string();
        assert!(
            TRIAGE_CLASSES.contains(&class.as_str()),
            "G7: {TRIAGE_REL}:{}: `{class}` is not one of {:?}",
            n + 1,
            TRIAGE_CLASSES
        );
        assert!(
            TRIAGE_REASONS.iter().any(|(r, _)| *r == reason),
            "G7: {TRIAGE_REL}:{}: `{reason}` is not one of the reasons this file admits: {:?}. \
             A free text reason is a sentence nobody can check, so the vocabulary is closed and \
             every entry is defined at the top of this test.",
            n + 1,
            TRIAGE_REASONS.iter().map(|(r, _)| *r).collect::<Vec<_>>()
        );
        assert!(
            pinned
                .insert(key.clone(), TriageRow { key, class, reason })
                .is_none(),
            "G7: {TRIAGE_REL}:{}: the same arm is pinned twice",
            n + 1
        );
    }

    let unclassified: Vec<String> = measured
        .keys()
        .filter(|k| !pinned.contains_key(*k))
        .cloned()
        .collect();
    let stale: Vec<String> = pinned
        .keys()
        .filter(|k| !measured.contains_key(*k))
        .cloned()
        .collect();

    assert!(
        unclassified.is_empty(),
        "G7: a wildcard arm a new primitive can reach carries no triage row.\n{}\n\n\
         Adding a primitive to `InferType` / `AirType` / `ResolvedType` / `AirIntSize` compiles \
         through every one of these arms without a word, so the arm decides the new type's \
         behaviour by accident. Classify each row in {TRIAGE_REL} as `correct-as-is` when the \
         wildcard's value is the right answer for a new scalar, or as `must-decide` / \
         `must-error` when it is not, and give the arm an explicit arm instead.",
        unclassified.join("\n")
    );

    assert!(
        stale.is_empty(),
        "G7: {TRIAGE_REL} classifies an arm the derivation no longer finds.\n{}\n\n\
         Either the arm gained an explicit `Char` arm and is decided, or it moved. A row that \
         survives the code it describes is how the census stops meaning anything.",
        stale.join("\n")
    );

    let open: Vec<String> = pinned
        .values()
        .filter(|row| row.class != "correct-as-is")
        .map(|row| format!("{}\t{}\t{}", row.class, row.reason, row.key))
        .collect();
    assert!(
        open.is_empty(),
        "G7: a triaged arm is still open.\n{}\n\n\
         `must-decide` and `must-error` name arms that answer wrongly for the new primitive. \
         They are red on purpose: write the explicit arm, and the derivation will move the row \
         to the decided side on its own.",
        open.join("\n")
    );
}

const TARGET_PIN_REL: &str = "scripts/target_pin.tsv";
const NAME_PIN_REL: &str = "scripts/test_name_pin.tsv";
const FEATURES_PIN_REL: &str = "scripts/features_pin.tsv";

fn package_name(root: &Path, member: &str) -> String {
    let path = root.join(member).join("Cargo.toml");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", path.display()));
    let text = strip_toml_comments(&raw);
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package {
            if let Some(value) = key_value(trimmed, "name") {
                return value.trim().trim_matches('"').to_string();
            }
        }
    }
    panic!("governance: no [package] name in {}", path.display())
}

fn declared_features(root: &Path, member: &str) -> Vec<String> {
    let path = root.join(member).join("Cargo.toml");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", path.display()));
    let text = strip_toml_comments(&raw);
    let mut in_features = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if in_features && !trimmed.is_empty() {
            if let Some((name, _)) = trimmed.split_once('=') {
                out.push(name.trim().trim_matches('"').to_string());
            }
        }
    }
    out
}

fn tsv_rows(path: &Path, want: usize) -> Vec<Vec<String>> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", path.display()));
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<String> = line.split('\t').map(|f| f.to_string()).collect();
        if fields.len() >= want {
            out.push(fields);
        }
    }
    out
}

fn read_attr(ch: &[char], i: usize) -> Option<(usize, String)> {
    let n = ch.len();
    let mut j = i + 1;
    if j < n && ch[j] == '!' {
        j += 1;
    }
    if j >= n || ch[j] != '[' {
        return None;
    }
    let start = j + 1;
    let mut depth = 0usize;
    while j < n {
        match ch[j] {
            '"' => {
                let (end, _) = read_basic_string(ch, j).ok()?;
                j = end;
                continue;
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((j + 1, ch[start..j].iter().collect()));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

fn call_args(text: &str, name: &str) -> Option<String> {
    let rest = text.trim().strip_prefix(name)?.trim_start();
    let rest = rest.strip_prefix('(')?;
    let rest = rest.strip_suffix(')')?;
    Some(rest.to_string())
}

fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    let mut in_string = false;
    for c in text.chars() {
        match c {
            '"' => {
                in_string = !in_string;
                cur.push(c);
            }
            '(' if !in_string => {
                depth += 1;
                cur.push(c);
            }
            ')' if !in_string => {
                depth -= 1;
                cur.push(c);
            }
            ',' if !in_string && depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn cfg_holds(pred: &str, feature: Option<&str>) -> bool {
    let p = pred.trim();
    if let Some(inner) = call_args(p, "not") {
        return !cfg_holds(&inner, feature);
    }
    if let Some(inner) = call_args(p, "any") {
        return split_top(&inner).iter().any(|a| cfg_holds(a, feature));
    }
    if let Some(inner) = call_args(p, "all") {
        return split_top(&inner).iter().all(|a| cfg_holds(a, feature));
    }
    if let Some(value) = key_value(p, "feature") {
        return feature == Some(value.trim().trim_matches('"'));
    }
    match p {
        "unix" => cfg!(unix),
        "windows" => cfg!(windows),
        _ => true,
    }
}

fn attrs_hold(attrs: &[String], feature: Option<&str>) -> bool {
    attrs.iter().all(|attr| match call_args(attr, "cfg") {
        Some(pred) => cfg_holds(&pred, feature),
        None => true,
    })
}

fn attrs_register(attrs: &[String]) -> bool {
    attrs
        .iter()
        .any(|attr| matches!(attr.trim(), "test" | "bench"))
}

fn registered_tests(src: &str, feature: Option<&str>) -> Result<Vec<String>, String> {
    let ch: Vec<char> = src.chars().collect();
    let n = ch.len();
    let mut out = Vec::new();
    let mut attrs: Vec<String> = Vec::new();
    let mut mods: Vec<String> = Vec::new();
    let mut frames: Vec<Option<String>> = Vec::new();
    let mut suppressed: Vec<usize> = Vec::new();
    let mut pending_mod: Option<(String, bool)> = None;
    let mut expect_fn_name = false;
    let mut expect_mod_name = false;
    let mut i = 0usize;

    while i < n {
        let c = ch[i];
        if c == '/' && i + 1 < n && ch[i + 1] == '/' {
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && ch[i + 1] == '*' {
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if ch[i] == '/' && i + 1 < n && ch[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if ch[i] == '*' && i + 1 < n && ch[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if let Some((end, _)) = read_raw_string(&ch, i)? {
            i = end;
            continue;
        }
        if c == '"' {
            let (end, _) = read_basic_string(&ch, i)?;
            i = end;
            continue;
        }
        if c == '\'' {
            i = skip_char_or_lifetime(&ch, i);
            continue;
        }
        if c == '#' {
            if let Some((end, body)) = read_attr(&ch, i) {
                attrs.push(body);
                i = end;
                continue;
            }
            i += 1;
            continue;
        }
        if c == '{' {
            match pending_mod.take() {
                Some((name, live)) => {
                    frames.push(Some(name.clone()));
                    mods.push(name);
                    if !live {
                        suppressed.push(frames.len());
                    }
                }
                None => frames.push(None),
            }
            attrs.clear();
            i += 1;
            continue;
        }
        if c == '}' {
            let depth = frames.len();
            if matches!(frames.pop(), Some(Some(_))) {
                mods.pop();
            }
            if suppressed.last() == Some(&depth) {
                suppressed.pop();
            }
            attrs.clear();
            i += 1;
            continue;
        }
        if c == ';' {
            pending_mod = None;
            expect_fn_name = false;
            expect_mod_name = false;
            attrs.clear();
            i += 1;
            continue;
        }
        if is_ident(c) && !c.is_ascii_digit() {
            let start = i;
            while i < n && is_ident(ch[i]) {
                i += 1;
            }
            let word: String = ch[start..i].iter().collect();
            if expect_fn_name {
                expect_fn_name = false;
                if suppressed.is_empty() && attrs_register(&attrs) && attrs_hold(&attrs, feature) {
                    let mut full = mods.clone();
                    full.push(word);
                    out.push(full.join("::"));
                }
                attrs.clear();
            } else if expect_mod_name {
                expect_mod_name = false;
                pending_mod = Some((word, attrs_hold(&attrs, feature)));
                attrs.clear();
            } else if word == "fn" {
                expect_fn_name = true;
            } else if word == "mod" {
                expect_mod_name = true;
            }
            continue;
        }
        i += 1;
    }
    Ok(out)
}

// rustc names a unit test by the module path of its file, so the walk carries that path with the file
fn walk_lib_modules(
    file: &Path,
    dir: &Path,
    path: &[String],
    seen: &mut BTreeSet<PathBuf>,
    out: &mut Vec<(Vec<String>, PathBuf)>,
) {
    if !seen.insert(file.to_path_buf()) {
        return;
    }
    out.push((path.to_vec(), file.to_path_buf()));
    for (prefix, name) in declared_mods(file) {
        let mut base = dir.to_path_buf();
        let mut modpath = path.to_vec();
        for part in &prefix {
            base.push(part);
            modpath.push(part.clone());
        }
        modpath.push(name.clone());
        let flat = base.join(format!("{name}.rs"));
        if flat.is_file() {
            walk_lib_modules(&flat, &base.join(&name), &modpath, seen, out);
            continue;
        }
        let nested = base.join(&name).join("mod.rs");
        if nested.is_file() {
            walk_lib_modules(&nested, &base.join(&name), &modpath, seen, out);
        }
    }
}

fn lib_sources(root: &Path, member: &str) -> Vec<(Vec<String>, PathBuf)> {
    let entry = root.join(member).join("src").join("lib.rs");
    if !entry.is_file() {
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    walk_lib_modules(
        &entry,
        &root.join(member).join("src"),
        &[],
        &mut seen,
        &mut out,
    );
    out
}

#[test]
fn g8_the_verification_inventories_still_describe_the_tree() {
    let root = repo_root();
    let target_pin = root.join(TARGET_PIN_REL);
    let name_pin = root.join(NAME_PIN_REL);
    let features_pin = root.join(FEATURES_PIN_REL);

    let target_rows = tsv_rows(&target_pin, 3);
    let name_rows = tsv_rows(&name_pin, 4);
    let feature_rows = tsv_rows(&features_pin, 4);
    assert!(
        !target_rows.is_empty() && !name_rows.is_empty(),
        "G8: {TARGET_PIN_REL} or {NAME_PIN_REL} holds no row, and a pin that holds no row gates \
         nothing."
    );

    let mut feature_runs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &feature_rows {
        if row[2] == "run" {
            feature_runs
                .entry(row[0].clone())
                .or_default()
                .push(row[1].clone());
        }
    }

    let mut measured_targets: BTreeSet<(String, String)> = BTreeSet::new();
    let mut measured_names: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for member in workspace_members(&root) {
        let pkg = package_name(&root, &member);
        // cargo names the default lib target after the package, with the dashes turned into underscores
        let lib_target = pkg.replace('-', "_");
        for (modpath, source) in lib_sources(&root, &member) {
            let src = fs::read_to_string(&source)
                .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", source.display()));
            let mut passes = vec![("lib".to_string(), None)];
            for feat in feature_runs.get(&pkg).into_iter().flatten() {
                passes.push((format!("lib+{feat}"), Some(feat.clone())));
            }
            for (kind, feature) in passes {
                let names = registered_tests(&src, feature.as_deref())
                    .unwrap_or_else(|e| panic!("governance: {}: {e}", rel(&root, &source)));
                for name in names {
                    let mut full = modpath.clone();
                    full.push(name);
                    measured_names.insert((
                        pkg.clone(),
                        lib_target.clone(),
                        kind.clone(),
                        full.join("::"),
                    ));
                }
            }
        }
        let tests = root.join(&member).join("tests");
        if !tests.is_dir() {
            continue;
        }
        for source in target_sources(&tests) {
            let target = if source.file_name().is_some_and(|n| n == "main.rs") {
                source.parent().and_then(|p| p.file_name())
            } else {
                source.file_stem()
            }
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| panic!("governance: unnamed target at {}", source.display()))
            .to_string();
            measured_targets.insert((pkg.clone(), target.clone()));
            let src = fs::read_to_string(&source)
                .unwrap_or_else(|e| panic!("governance: cannot read {}: {e}", source.display()));
            let mut passes = vec![("test".to_string(), None)];
            for feat in feature_runs.get(&pkg).into_iter().flatten() {
                passes.push((format!("test+{feat}"), Some(feat.clone())));
            }
            for (kind, feature) in passes {
                let names = registered_tests(&src, feature.as_deref())
                    .unwrap_or_else(|e| panic!("governance: {}: {e}", rel(&root, &source)));
                for name in names {
                    measured_names.insert((pkg.clone(), target.clone(), kind.clone(), name));
                }
            }
        }
    }

    let pinned_targets: BTreeSet<(String, String)> = target_rows
        .iter()
        .filter(|row| row[2] == "test")
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();
    let mut target_moves = Vec::new();
    for t in measured_targets.difference(&pinned_targets) {
        target_moves.push(format!(
            "SUITE NOT IN {TARGET_PIN_REL}   {}\t{}\ttest",
            t.0, t.1
        ));
    }
    for t in pinned_targets.difference(&measured_targets) {
        target_moves.push(format!(
            "PINNED SUITE IS GONE         {}\t{}\ttest",
            t.0, t.1
        ));
    }

    assert!(
        !measured_names.is_empty(),
        "G8: the source derivation registered no check at all, so the comparison below would \
         hold whatever the pins say."
    );

    let pinned_features: BTreeSet<(String, String)> = feature_rows
        .iter()
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();
    for member in workspace_members(&root) {
        let pkg = package_name(&root, &member);
        for feat in declared_features(&root, &member) {
            assert!(
                pinned_features.contains(&(pkg.clone(), feat.clone())),
                "G8: `{pkg}` declares the feature `{feat}` and {FEATURES_PIN_REL} does not \
                 decide it. A feature nothing names is a configuration the verification never \
                 builds and never runs."
            );
        }
    }

    let kinds: BTreeSet<String> = measured_names.iter().map(|r| r.2.clone()).collect();
    let pinned_names: BTreeSet<(String, String, String, String)> = name_rows
        .iter()
        .filter(|row| kinds.contains(&row[2]))
        .map(|row| {
            (
                row[0].clone(),
                row[1].clone(),
                row[2].clone(),
                row[3].clone(),
            )
        })
        .collect();
    let mut name_moves = Vec::new();
    for r in measured_names.difference(&pinned_names) {
        name_moves.push(format!(
            "CHECK NOT IN {NAME_PIN_REL}  {}\t{}\t{}\t{}",
            r.0, r.1, r.2, r.3
        ));
    }
    for r in pinned_names.difference(&measured_names) {
        name_moves.push(format!(
            "PINNED CHECK IS GONE             {}\t{}\t{}\t{}",
            r.0, r.1, r.2, r.3
        ));
    }
    target_moves.sort();
    name_moves.sort();
    let shown: Vec<String> = target_moves
        .iter()
        .chain(name_moves.iter())
        .take(40)
        .cloned()
        .collect();

    assert!(
        target_moves.is_empty() && name_moves.is_empty(),
        "G8: the verification inventories and the test tree disagree, {} target move(s) and {} \
         check move(s), first 40 shown.\n{}\n\n\
         {TARGET_PIN_REL} and {NAME_PIN_REL} are what scripts/verify_suites.sh compares the \
         machine against, and nothing in `cargo test` used to read either, so a suite could be \
         added, renamed or deleted and stay outside the compared set until someone happened to \
         run the script. This derives the integration suites and the crate unit tests from the \
         sources the way cargo registers them and fails on the difference; the bin and doc ranks \
         are outside the derivation and outside this comparison. Refresh the name pin with \
         `scripts/verify_suites.sh --update-name-pin` and edit {TARGET_PIN_REL} by hand.",
        target_moves.len(),
        name_moves.len(),
        shown.join("\n")
    );
}
