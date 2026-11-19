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

fn diff(name: &str, old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            ops.push((' ', i, j));
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ops.push(('+', i, j));
            j += 1;
        } else {
            ops.push(('-', i, j));
            i += 1;
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

fn exit(code: u8) -> Result<()> {
    Err(Error {
        diagnostics: Vec::new(),
        code,
    })
}
