use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const CENSUS_REL: &str = "aelys/tests/ignored_census.tsv";

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

fn uncompiled_dirs(tests: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in read_dir_sorted(tests) {
        if path.is_dir() && !path.join("main.rs").is_file() && !rs_files_recursive(&path).is_empty()
        {
            out.push(path);
        }
    }
    out
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
        for orphan in uncompiled_dirs(&tests) {
            problems.push(format!(
                "{} holds *.rs but no main.rs, so cargo makes no target of it and none of those \
                 files is ever compiled",
                orphan.display()
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
         tests/<dir>/main.rs, and of nothing else, only while that member's manifest keeps auto \
         discovery on, so this pairs with G1: G1 forbids the opt out, G2 checks the files that \
         opt out would silence are still there and that no suite sits in a subdirectory cargo \
         never compiles.",
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
