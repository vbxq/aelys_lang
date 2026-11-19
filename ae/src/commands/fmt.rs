use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const EXTENSIONS: [&str; 3] = ["aelys", "amod", "case"];

pub fn run(check: bool, paths: &[PathBuf]) -> Result<()> {
    if paths.len() == 1 && paths[0] == Path::new("-") {
        return stdin(check);
    }
    let roots = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };
    let mut files = Vec::new();
    let mut failed = false;
    for root in &roots {
        if root.is_dir() {
            if let Err(err) = walk(root, &mut files) {
                eprintln!("error: {}: {err}", root.display());
                failed = true;
            }
        } else {
            files.push(root.clone());
        }
    }
    let mut unformatted = false;
    for file in &files {
        match format_file(file, check) {
            Ok(changed) => unformatted |= changed,
            Err(message) => {
                eprintln!("error: {message}");
                failed = true;
            }
        }
    }
    if failed {
        exit(2)
    } else if check && unformatted {
        exit(1)
    } else {
        Ok(())
    }
}

fn stdin(check: bool) -> Result<()> {
    let mut src = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut src) {
        eprintln!("error: <stdin>: {err}");
        return exit(2);
    }
    match aelys_fmt::format_source(&src) {
        Ok(out) if check => {
            if out == src {
                Ok(())
            } else {
                print!("{}", diff("<stdin>", &src, &out));
                exit(1)
            }
        }
        Ok(out) => {
            let _ = std::io::stdout().write_all(out.as_bytes());
            Ok(())
        }
        Err(err) => {
            let (line, col) = err.line_col(&src);
            eprintln!("error: <stdin>:{line}:{col}: {err}");
            exit(2)
        }
    }
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_dir() {
            if name != "target" && !name.starts_with('.') {
                walk(&path, files)?;
            }
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|ext| EXTENSIONS.iter().any(|e| ext == *e))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn format_file(path: &Path, check: bool) -> std::result::Result<bool, String> {
    let shown = path.display();
    let src = std::fs::read_to_string(path).map_err(|err| format!("{shown}: {err}"))?;
    let out = aelys_fmt::format_source(&src).map_err(|err| {
        let (line, col) = err.line_col(&src);
        format!("{shown}:{line}:{col}: {err}")
    })?;
    if out == src {
        return Ok(false);
    }
    if check {
        print!("{}", diff(&shown.to_string(), &src, &out));
        return Ok(true);
    }
    write_atomic(path, &out).map_err(|err| format!("{shown}: {err}"))?;
    Ok(true)
}

fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let target = std::fs::canonicalize(path)?;
    let permissions = std::fs::metadata(&target)?.permissions();
    if permissions.readonly() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the file is read-only",
        ));
    }
    let (tmp, mut file) = create_temp(&target)?;
    let written = file
        .set_permissions(permissions)
        .and_then(|()| file.write_all(contents.as_bytes()))
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&tmp, &target));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn create_temp(target: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    let mut n = 0u32;
    loop {
        let mut name = target.as_os_str().to_owned();
        name.push(format!(".{}.{n}.fmt-tmp", std::process::id()));
        let tmp = PathBuf::from(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(err) => return Err(err),
        }
    }
}

const DIFF_BUDGET: usize = 1 << 22;
const DIFF_MAX_EDITS: usize = 2048;

fn diff(name: &str, old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let mut script = vec![' '; prefix];
    script.extend(edit_script(
        &a[prefix..a.len() - suffix],
        &b[prefix..b.len() - suffix],
    ));
    script.extend(std::iter::repeat_n(' ', suffix));
    let mut ops = Vec::with_capacity(script.len());
    let (mut i, mut j) = (0, 0);
    for run in script.chunk_by(|x, y| (*x == ' ') == (*y == ' ')) {
        if run[0] == ' ' {
            for _ in run {
                ops.push((' ', i, j));
                i += 1;
                j += 1;
            }
            continue;
        }
        for &op in run.iter().filter(|op| **op == '-') {
            ops.push((op, i, j));
            i += 1;
        }
        for &op in run.iter().filter(|op| **op == '+') {
            ops.push((op, i, j));
            j += 1;
        }
    }
    let mut out = format!("--- {name}\n+++ {name}\n");
    let mut k = 0;
    while k < ops.len() {
        if ops[k].0 == ' ' {
            k += 1;
            continue;
        }
        let start = k.saturating_sub(3);
        let mut end = k;
        let mut last_change = k;
        while end < ops.len() && end <= last_change + 6 {
            if ops[end].0 != ' ' {
                last_change = end;
            }
            end += 1;
        }
        let end = (last_change + 4).min(ops.len());
        let hunk = &ops[start..end];
        let old_len = hunk.iter().filter(|o| o.0 != '+').count();
        let new_len = hunk.iter().filter(|o| o.0 != '-').count();
        out.push_str(&format!(
            "@@ -{},{old_len} +{},{new_len} @@\n",
            hunk[0].1 + 1,
            hunk[0].2 + 1
        ));
        for &(op, i, j) in hunk {
            let line = if op == '+' { b[j] } else { a[i] };
            out.push_str(&format!("{op}{line}\n"));
        }
        k = end;
    }
    out
}

fn edit_script(a: &[&str], b: &[&str]) -> Vec<char> {
    let (n, m) = (a.len(), b.len());
    let limit = (n + m)
        .min(DIFF_MAX_EDITS)
        .min(DIFF_BUDGET / (n + m).max(1));
    let off = limit + 1;
    let mut v = vec![0usize; 2 * limit + 3];
    let mut trace: Vec<Vec<usize>> = Vec::new();
    for d in 0..=limit {
        for k in (off - d..=off + d).step_by(2) {
            let mut x = if k == off - d || (k != off + d && v[k - 1] < v[k + 1]) {
                v[k + 1]
            } else {
                v[k - 1] + 1
            };
            let mut y = x + off - k;
            while x < n && y < m && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            v[k] = x;
            if x >= n && y >= m {
                trace.push(v[off - d..=off + d].to_vec());
                return backtrack(&trace, n, m);
            }
        }
        trace.push(v[off - d..=off + d].to_vec());
    }
    let mut script = vec!['-'; n];
    script.extend(std::iter::repeat_n('+', m));
    script
}

fn backtrack(trace: &[Vec<usize>], n: usize, m: usize) -> Vec<char> {
    let mut script = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..trace.len()).rev() {
        let prev = &trace[d - 1];
        let k = x + d - y;
        let down = k == 0 || (k != 2 * d && prev[k - 2] < prev[k]);
        let prev_k = if down { k } else { k - 2 };
        let px = prev[prev_k];
        let py = px + d - 1 - prev_k;
        let sx = if down { px } else { px + 1 };
        script.extend(std::iter::repeat_n(' ', x - sx));
        script.push(if down { '+' } else { '-' });
        x = px;
        y = py;
    }
    script.extend(std::iter::repeat_n(' ', x));
    script.reverse();
    script
}

fn exit(code: u8) -> Result<()> {
    Err(Error {
        diagnostics: Vec::new(),
        code,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::diff;

    const BLOCK: &str = "struct Pair@<Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, Cccccccccccccccccccccccccccc> { a: Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, b: Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, c: Cccccccccccccccccccccccccccc,
}
enum Shape@ { Circle(i64), Rect(i64, i64),
}
fn add@( first_number_argument: i64, second_number_argument: i64, third_arg: i64,
) -> i64 { first_number_argument + second_number_argument + third_arg
}
fn main_@() { let total = add@(1000000000000, 2000000000000000, 3000000000000000000); let v: Vec<i64> = vec![     100000000000000,     200000000000000,     300000000000000,     400000000000000,     5, ]; let arr = [     100000000000000,     200000000000000,     300000000000000,     400000000000000,     500000000000, ]; let p = Pair@ {     a: 1000000000000000000,     b: 2000000000000000000,     c: 3000000000000000, }; let t = (     100000000000000000,     200000000000000000,     300000000000000000,     400000000000000000, ); let s = Shape@::Rect(     1000000000000000000,     200000000000000000, ); assert_eq!(     add@(1, 2, 3),     6,     \"sum of one two three should equal six here\" ); println!(\"{} {} {} {} {}\", total, v.len(), arr.len(), p.a, t.0); let m = match s {     Shape@::Circle(r) => r,     Shape@::Rect(w, h) => w + h, }; println!(\"{}\", m > 0);
}

";

    #[test]
    fn diff_lists_removed_lines_before_added_ones() {
        assert_eq!(
            diff("f", "a\nb\nc\nd\n", "a\nx\nc\ny\nz\n"),
            "--- f\n+++ f\n@@ -1,4 +1,5 @@\n a\n-b\n+x\n c\n-d\n+y\n+z\n"
        );
    }

    #[test]
    fn diff_of_a_large_reformat_is_fast() {
        let old: String = (0..1000)
            .map(|i| BLOCK.replace('@', &i.to_string()))
            .collect();
        let new = aelys_fmt::format_source(&old).unwrap();
        let start = Instant::now();
        let out = diff("big", &old, &new);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        assert!(out.lines().filter(|l| l.starts_with('+')).count() > 40_000);
    }
}
