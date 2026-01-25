// fs module - file system operations (handles, paths, directories)

use crate::stdlib::helpers::{get_handle, get_int, get_string, make_string};
use crate::stdlib::{FileMode, FileResource, Resource, StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const MAX_BUF: usize = 16 * 1024 * 1024; // 16MB should be enough for anyone :)

pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut exports = Vec::new();
    let mut natives = Vec::new();

    macro_rules! reg {
        ($n:expr, $a:expr, $f:expr) => {{
            register_native(vm, "fs", $n, $a, $f)?;
            exports.push($n.to_string());
            natives.push(format!("fs::{}", $n));
        }};
    }

    // handle-based file ops
    reg!("open", 2, native_open);
    reg!("close", 1, native_close);
    reg!("read", 1, native_read);
    reg!("read_line", 1, native_read_line);
    reg!("read_bytes", 2, native_read_bytes);
    reg!("read_all", 1, native_read_all);
    reg!("write", 2, native_write);
    reg!("write_bytes", 2, native_write_bytes);
    reg!("write_line", 2, native_write_line);

    // stat-like queries
    reg!("exists", 1, native_exists);
    reg!("is_file", 1, native_is_file);
    reg!("is_dir", 1, native_is_dir);
    reg!("size", 1, native_size);

    // directories
    reg!("mkdir", 1, native_mkdir);
    reg!("mkdir_all", 1, native_mkdir_all);
    reg!("rmdir", 1, native_rmdir);
    reg!("readdir", 1, native_readdir);

    // file management
    reg!("delete", 1, native_delete);
    reg!("rename", 2, native_rename);
    reg!("copy", 2, native_copy);

    // convenience (no handle needed)
    reg!("read_text", 1, native_read_text);
    reg!("write_text", 2, native_write_text);
    reg!("append_text", 2, native_append_text);

    // path manipulation
    reg!("basename", 1, native_basename);
    reg!("dirname", 1, native_dirname);
    reg!("extension", 1, native_extension);
    reg!("join", 2, native_join);
    reg!("absolute", 1, native_absolute);

    Ok(StdModuleExports { all_exports: exports, native_functions: natives })
}

fn fs_error(vm: &VM, op: &'static str, msg: String) -> RuntimeError {
    vm.runtime_error(RuntimeErrorKind::TypeError { operation: op, expected: "valid file operation", got: msg })
}

// mode: "r", "w", "a", "rw" (or "r+")
fn native_open(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let path = get_string(vm, args[0], "fs.open")?;
    let mode_str = get_string(vm, args[1], "fs.open")?;

    let mode = match mode_str {
        "r" => FileMode::Read,
        "w" => FileMode::Write,
        "a" => FileMode::Append,
        "rw" | "r+" => FileMode::ReadWrite,
        _ => {
            return Err(fs_error(
                vm,
                "fs.open",
                format!("invalid mode '{}', use 'r', 'w', 'a', or 'rw'", mode_str),
            ));
        }
    };

    // Build OpenOptions with O_NOFOLLOW on Unix to prevent symlink attacks (TOCTOU)
    #[cfg(unix)]
    let file = {
        let mut opts = OpenOptions::new();
        // O_NOFOLLOW prevents following symlinks, mitigating TOCTOU race conditions
        opts.custom_flags(libc::O_NOFOLLOW);
        match mode {
            FileMode::Read => opts.read(true).open(path),
            FileMode::Write => opts.write(true).create(true).truncate(true).open(path),
            FileMode::Append => opts.write(true).create(true).append(true).open(path),
            FileMode::ReadWrite => opts.read(true).write(true).create(true).open(path),
        }
    };

    #[cfg(not(unix))]
    let file = match mode {
        FileMode::Read => OpenOptions::new().read(true).open(path),
        FileMode::Write => OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path),
        FileMode::Append => OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(path),
        FileMode::ReadWrite => OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path),
    };

    match file {
        Ok(f) => {
            let resource = match mode {
                FileMode::Read => FileResource {
                    reader: Some(BufReader::new(f)),
                    writer: None,
                    path: path.to_string(),
                    mode,
                },
                FileMode::Write | FileMode::Append => FileResource {
                    reader: None,
                    writer: Some(BufWriter::new(f)),
                    path: path.to_string(),
                    mode,
                },
                FileMode::ReadWrite => {
                    // For read+write, we need to clone the file handle
                    let f2 = f.try_clone().map_err(|e| {
                        fs_error(vm, "fs.open", format!("failed to open file: {}", e))
                    })?;
                    FileResource {
                        reader: Some(BufReader::new(f)),
                        writer: Some(BufWriter::new(f2)),
                        path: path.to_string(),
                        mode,
                    }
                }
            };

            let handle = vm.store_resource(Resource::File(resource));
            Ok(Value::int(handle as i64))
        }
        Err(e) => Err(fs_error(
            vm,
            "fs.open",
            format!("failed to open '{}': {}", path, e),
        )),
    }
}

fn native_close(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "fs.close")?;
    match vm.take_resource(h) {
        Some(Resource::File(mut f)) => {
            if let Some(w) = f.writer.as_mut() { let _ = w.flush(); }
            Ok(Value::null()) // drop closes the file
        }
        _ => Err(fs_error(vm, "fs.close", "invalid file handle".to_string())),
    }
}

fn native_read(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "fs.read")?;

    if let Some(Resource::File(file_res)) = vm.get_resource_mut(handle) {
        if let Some(reader) = file_res.reader.as_mut() {
            let mut content = String::new();
            match reader.get_mut().read_to_string(&mut content) {
                Ok(_) => Ok(make_string(vm, &content)?),
                Err(e) => Err(fs_error(vm, "fs.read", format!("read error: {}", e))),
            }
        } else {
            Err(fs_error(
                vm,
                "fs.read",
                "file not opened for reading".to_string(),
            ))
        }
    } else {
        Err(fs_error(vm, "fs.read", "invalid file handle".to_string()))
    }
}

fn native_read_line(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "fs.read_line")?;
    if let Some(Resource::File(f)) = vm.get_resource_mut(h) {
        if let Some(reader) = f.reader.as_mut() {
            let mut line = String::new();
            return match reader.read_line(&mut line) {
                Ok(0) => Ok(Value::null()),
                Ok(_) => {
                    if line.ends_with('\n') { line.pop(); }
                    if line.ends_with('\r') { line.pop(); }
                    make_string(vm, &line)
                }
                Err(e) => Err(fs_error(vm, "fs.read_line", format!("read: {}", e))),
            };
        }
        return Err(fs_error(vm, "fs.read_line", "not opened for reading".into()));
    }
    Err(fs_error(vm, "fs.read_line", "invalid handle".into()))
}

fn native_read_bytes(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "fs.read_bytes")?;
    let n = get_int(vm, args[1], "fs.read_bytes")?;

    if n < 0 { return Err(fs_error(vm, "fs.read_bytes", "count must be non-negative".into())); }
    if n as usize > MAX_BUF { return Err(fs_error(vm, "fs.read_bytes", format!("count > {} max", MAX_BUF))); }

    if let Some(Resource::File(f)) = vm.get_resource_mut(h) {
        if let Some(reader) = f.reader.as_mut() {
            let mut buf = vec![0u8; n as usize];
            return match reader.get_mut().read(&mut buf) {
                Ok(got) => { buf.truncate(got); make_string(vm, &String::from_utf8_lossy(&buf)) }
                Err(e) => Err(fs_error(vm, "fs.read_bytes", format!("read: {}", e))),
            };
        }
        return Err(fs_error(vm, "fs.read_bytes", "not opened for reading".into()));
    }
    Err(fs_error(vm, "fs.read_bytes", "invalid handle".into()))
}

fn native_read_all(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> { native_read(vm, args) }

fn native_write(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "fs.write")?;
    let data = get_string(vm, args[1], "fs.write")?.to_string();

    if let Some(Resource::File(f)) = vm.get_resource_mut(h) {
        if let Some(w) = f.writer.as_mut() {
            return match w.write_all(data.as_bytes()) {
                Ok(_) => { let _ = w.flush(); Ok(Value::int(data.len() as i64)) }
                Err(e) => Err(fs_error(vm, "fs.write", format!("write: {}", e))),
            };
        }
        return Err(fs_error(vm, "fs.write", "not opened for writing".into()));
    }
    Err(fs_error(vm, "fs.write", "invalid handle".into()))
}

fn native_write_bytes(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    native_write(vm, args) // same thing, we're all strings anyway
}

fn native_write_line(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "fs.write_line")?;
    let txt = get_string(vm, args[1], "fs.write_line")?.to_string();

    if let Some(Resource::File(f)) = vm.get_resource_mut(h) {
        if let Some(w) = f.writer.as_mut() {
            return match writeln!(w, "{}", txt) {
                Ok(_) => { let _ = w.flush(); Ok(Value::int((txt.len() + 1) as i64)) }
                Err(e) => Err(fs_error(vm, "fs.write_line", format!("write: {}", e))),
            };
        }
        return Err(fs_error(vm, "fs.write_line", "not opened for writing".into()));
    }
    Err(fs_error(vm, "fs.write_line", "invalid handle".into()))
}

// --- stat-like queries ---

fn native_exists(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(Path::new(get_string(vm, args[0], "fs.exists")?).exists()))
}

fn native_is_file(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(Path::new(get_string(vm, args[0], "fs.is_file")?).is_file()))
}

fn native_is_dir(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(Path::new(get_string(vm, args[0], "fs.is_dir")?).is_dir()))
}

fn native_size(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.size")?;
    fs::metadata(p)
        .map(|m| Value::int(m.len() as i64))
        .map_err(|e| fs_error(vm, "fs.size", format!("'{}': {}", p, e)))
}

// --- directory ops ---

fn native_mkdir(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.mkdir")?;
    fs::create_dir(p).map(|_| Value::bool(true))
        .map_err(|e| fs_error(vm, "fs.mkdir", format!("'{}': {}", p, e)))
}

fn native_mkdir_all(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.mkdir_all")?;
    fs::create_dir_all(p).map(|_| Value::bool(true))
        .map_err(|e| fs_error(vm, "fs.mkdir_all", format!("'{}': {}", p, e)))
}

fn native_rmdir(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.rmdir")?;
    fs::remove_dir(p).map(|_| Value::bool(true))
        .map_err(|e| fs_error(vm, "fs.rmdir", format!("'{}': {}", p, e)))
}

// returns newline-separated list of entries (sorted)
fn native_readdir(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.readdir")?;
    match fs::read_dir(p) {
        Ok(entries) => {
            let mut names: Vec<_> = entries.flatten()
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .collect();
            names.sort();
            make_string(vm, &names.join("\n"))
        }
        Err(e) => Err(fs_error(vm, "fs.readdir", format!("'{}': {}", p, e))),
    }
}

// --- file management ---

fn native_delete(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.delete")?;
    fs::remove_file(p).map(|_| Value::bool(true))
        .map_err(|e| fs_error(vm, "fs.delete", format!("'{}': {}", p, e)))
}

fn native_rename(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let src = get_string(vm, args[0], "fs.rename")?;
    let dst = get_string(vm, args[1], "fs.rename")?;
    fs::rename(src, dst).map(|_| Value::bool(true))
        .map_err(|e| fs_error(vm, "fs.rename", format!("'{}' -> '{}': {}", src, dst, e)))
}

fn native_copy(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let src = get_string(vm, args[0], "fs.copy")?;
    let dst = get_string(vm, args[1], "fs.copy")?;
    fs::copy(src, dst).map(|n| Value::int(n as i64))
        .map_err(|e| fs_error(vm, "fs.copy", format!("'{}' -> '{}': {}", src, dst, e)))
}

// --- convenience (no handle) ---

fn native_read_text(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.read_text")?;
    fs::read_to_string(p)
        .map_err(|e| fs_error(vm, "fs.read_text", format!("'{}': {}", p, e)))
        .and_then(|s| make_string(vm, &s))
}

fn native_write_text(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.write_text")?;
    let txt = get_string(vm, args[1], "fs.write_text")?;
    fs::write(p, txt).map(|_| Value::int(txt.len() as i64))
        .map_err(|e| fs_error(vm, "fs.write_text", format!("'{}': {}", p, e)))
}

fn native_append_text(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.append_text")?;
    let txt = get_string(vm, args[1], "fs.append_text")?;

    OpenOptions::new().append(true).create(true).open(p)
        .and_then(|mut f| f.write_all(txt.as_bytes()))
        .map(|_| Value::int(txt.len() as i64))
        .map_err(|e| fs_error(vm, "fs.append_text", format!("'{}': {}", p, e)))
}

// --- path manipulation ---

fn native_basename(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.basename")?.to_string();
    let name = Path::new(&p).file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    make_string(vm, &name)
}

fn native_dirname(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.dirname")?.to_string();
    let dir = Path::new(&p).parent().and_then(|p| p.to_str()).unwrap_or("").to_string();
    make_string(vm, &dir)
}

fn native_extension(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.extension")?.to_string();
    let ext = Path::new(&p).extension().and_then(|s| s.to_str()).unwrap_or("").to_string();
    make_string(vm, &ext)
}

// join with path traversal protection - rejects ../ escapes
fn native_join(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    use std::path::Component;

    let base = get_string(vm, args[0], "fs.join")?.to_string();
    let rel = get_string(vm, args[1], "fs.join")?.to_string();

    if Path::new(&rel).is_absolute() {
        return Err(fs_error(vm, "fs.join", "path must not be absolute".into()));
    }

    let joined = Path::new(&base).join(&rel);

    // normalize and detect escape attempts
    let mut parts = Vec::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                if let Some(Component::Normal(_)) = parts.last() {
                    parts.pop();
                } else if matches!(parts.last(), Some(Component::RootDir) | None) {
                    return Err(fs_error(vm, "fs.join", "path escapes base".into()));
                }
            }
            Component::CurDir => {} // skip
            _ => parts.push(c),
        }
    }

    let mut result = std::path::PathBuf::new();
    for c in &parts { result.push(c); }

    // verify still under base
    let base_parts: Vec<_> = Path::new(&base).components().collect();
    if parts.len() < base_parts.len() || parts[..base_parts.len()] != base_parts[..] {
        return Err(fs_error(vm, "fs.join", "path escapes base".into()));
    }

    make_string(vm, &result.to_string_lossy())
}

fn native_absolute(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let p = get_string(vm, args[0], "fs.absolute")?;
    fs::canonicalize(p)
        .map(|abs| abs.to_string_lossy().to_string())
        .map_err(|e| fs_error(vm, "fs.absolute", format!("'{}': {}", p, e)))
        .and_then(|s| make_string(vm, &s))
}
