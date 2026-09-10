use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CENSUS_REL: &str = "aelys/tests/ignored_census.tsv";
const SWEEPS_PIN_REL: &str = "scripts/sweeps_pin.tsv";
const REGISTRY_REL: &str = "common/src/diagnostic/registry.rs";
const COMPILE_CODE_REL: &str = "common/src/error/compile/code.rs";
const FAULT_REL: &str = "common/src/error/fault.rs";
const REGISTERED_CODES: usize = 95;

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
