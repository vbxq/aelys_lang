//! std.string - String manipulation functions
use crate::stdlib::helpers::{get_int, get_string, make_string};
use crate::stdlib::{StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::RuntimeError;

/// Register all string functions in the VM.
pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut all_exports = Vec::new();
    let mut native_functions = Vec::new();

    macro_rules! reg_fn {
        ($name:expr, $arity:expr, $func:expr) => {{
            register_native(vm, "string", $name, $arity, $func)?;
            all_exports.push($name.to_string());
            native_functions.push(format!("string::{}", $name));
        }};
    }

    reg_fn!("len", 1, native_len);
    reg_fn!("char_len", 1, native_char_len);
    reg_fn!("chars", 1, native_chars);
    reg_fn!("bytes", 1, native_bytes);
    reg_fn!("char_at", 2, native_char_at);
    reg_fn!("byte_at", 2, native_byte_at);
    reg_fn!("substr", 3, native_substr);
    reg_fn!("to_upper", 1, native_to_upper);
    reg_fn!("to_lower", 1, native_to_lower);
    reg_fn!("capitalize", 1, native_capitalize);
    reg_fn!("contains", 2, native_contains);
    reg_fn!("starts_with", 2, native_starts_with);
    reg_fn!("ends_with", 2, native_ends_with);
    reg_fn!("find", 2, native_find);
    reg_fn!("rfind", 2, native_rfind);
    reg_fn!("count", 2, native_count);
    reg_fn!("replace", 3, native_replace);
    reg_fn!("replace_first", 3, native_replace_first);
    reg_fn!("split", 2, native_split);
    reg_fn!("join", 2, native_join);
    reg_fn!("repeat", 2, native_repeat);
    reg_fn!("reverse", 1, native_reverse);
    reg_fn!("concat", 2, native_concat);
    reg_fn!("trim", 1, native_trim);
    reg_fn!("trim_start", 1, native_trim_start);
    reg_fn!("trim_end", 1, native_trim_end);
    reg_fn!("pad_left", 3, native_pad_left);
    reg_fn!("pad_right", 3, native_pad_right);
    reg_fn!("is_empty", 1, native_is_empty);
    reg_fn!("is_whitespace", 1, native_is_whitespace);
    reg_fn!("is_numeric", 1, native_is_numeric);
    reg_fn!("is_alphabetic", 1, native_is_alphabetic);
    reg_fn!("is_alphanumeric", 1, native_is_alphanumeric);
    reg_fn!("lines", 1, native_lines);
    reg_fn!("line_count", 1, native_line_count);

    Ok(StdModuleExports {
        all_exports,
        native_functions,
    })
}

/// len(s) - Get string length in bytes.
fn native_len(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.len")?;
    Ok(Value::int(s.len() as i64))
}

/// char_len(s) - Get string length in characters.
fn native_char_len(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.char_len")?;
    Ok(Value::int(s.chars().count() as i64))
}

/// chars(s) - Get characters as newline-separated string.
// TODO: this is awkward, should return an actual array once we have proper iterators
fn native_chars(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.chars")?;
    let chars: Vec<String> = s.chars().map(|c| c.to_string()).collect();
    Ok(make_string(vm, &chars.join("\n"))?)
}

/// bytes(s) - Get bytes as space-separated integers.
fn native_bytes(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.bytes")?;
    let bytes: Vec<String> = s.bytes().map(|b| b.to_string()).collect();
    Ok(make_string(vm, &bytes.join(" "))?)
}

/// char_at(s, index) - Get character at index (0-based).
/// Returns empty string if index is out of bounds.
fn native_char_at(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.char_at")?;
    let index = get_int(vm, args[1], "string.char_at")?;

    if index < 0 {
        return Ok(make_string(vm, "")?);
    }

    match s.chars().nth(index as usize) {
        Some(c) => Ok(make_string(vm, &c.to_string())?),
        None => Ok(make_string(vm, "")?),
    }
}

/// byte_at(s, index) - Get byte at index (0-based).
/// Returns -1 if index is out of bounds.
fn native_byte_at(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.byte_at")?;
    let index = get_int(vm, args[1], "string.byte_at")?;

    if index < 0 || index as usize >= s.len() {
        return Ok(Value::int(-1));
    }

    Ok(Value::int(s.as_bytes()[index as usize] as i64))
}

/// substr(s, start, len) - Extract substring.
/// start is 0-based character index, len is number of characters.
fn native_substr(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.substr")?;
    let start = get_int(vm, args[1], "string.substr")?;
    let len = get_int(vm, args[2], "string.substr")?;

    if start < 0 || len < 0 {
        return Ok(make_string(vm, "")?);
    }

    let result: String = s.chars().skip(start as usize).take(len as usize).collect();

    Ok(make_string(vm, &result)?)
}

fn native_to_upper(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.to_upper")?;
    make_string(vm, &s.to_uppercase())
}

fn native_to_lower(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    make_string(vm, &get_string(vm, args[0], "string.to_lower")?.to_lowercase())
}

fn native_capitalize(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.capitalize")?;
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        make_string(vm, &(first.to_uppercase().to_string() + chars.as_str()))
    } else {
        make_string(vm, "")
    }
}

fn native_contains(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let haystack = get_string(vm, args[0], "string.contains")?;
    let needle = get_string(vm, args[1], "string.contains")?;
    Ok(Value::bool(haystack.contains(needle)))
}

fn native_starts_with(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::bool(
        get_string(vm, args[0], "string.starts_with")?
            .starts_with(get_string(vm, args[1], "string.starts_with")?),
    ))
}

fn native_ends_with(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.ends_with")?;
    let suf = get_string(vm, args[1], "string.ends_with")?;
    Ok(Value::bool(s.ends_with(suf)))
}

// returns byte pos, not char pos - might be confusing w/ unicode
// FIXME: add find_char?
fn native_find(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.find")?;
    let needle = get_string(vm, args[1], "string.find")?;
    Ok(Value::int(s.find(needle).map(|p| p as i64).unwrap_or(-1)))
}

fn native_rfind(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.rfind")?;
    let n = get_string(vm, args[1], "string.rfind")?;
    if let Some(pos) = s.rfind(n) {
        Ok(Value::int(pos as i64))
    } else {
        Ok(Value::int(-1))
    }
}

fn native_count(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.count")?;
    let n = get_string(vm, args[1], "string.count")?;
    Ok(Value::int(s.matches(n).count() as i64))
}

/// replace(s, old, new) - Replace all occurrences.
fn native_replace(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.replace")?;
    let old = get_string(vm, args[1], "string.replace")?;
    let new = get_string(vm, args[2], "string.replace")?;
    Ok(make_string(vm, &s.replace(old, new))?)
}

/// replace_first(s, old, new) - Replace first occurrence.
fn native_replace_first(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.replace_first")?;
    let old = get_string(vm, args[1], "string.replace_first")?;
    let new = get_string(vm, args[2], "string.replace_first")?;
    Ok(make_string(vm, &s.replacen(old, new, 1))?)
}

/// split(s, sep) - Split string by separator.
/// Returns parts separated by newlines.
fn native_split(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.split")?;
    let sep = get_string(vm, args[1], "string.split")?;

    if sep.is_empty() {
        // Split into characters
        let parts: Vec<&str> = s.split("").filter(|x| !x.is_empty()).collect();
        Ok(make_string(vm, &parts.join("\n"))?)
    } else {
        let parts: Vec<&str> = s.split(sep).collect();
        Ok(make_string(vm, &parts.join("\n"))?)
    }
}

/// join(parts, sep) - Join parts by separator.
/// parts are expected to be newline-separated.
fn native_join(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let parts = get_string(vm, args[0], "string.join")?;
    let sep = get_string(vm, args[1], "string.join")?;
    let result = parts.lines().collect::<Vec<&str>>().join(sep);
    Ok(make_string(vm, &result)?)
}

/// repeat(s, n) - Repeat string n times.
fn native_repeat(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.repeat")?;
    let n = get_int(vm, args[1], "string.repeat")?;

    if n <= 0 {
        return Ok(make_string(vm, "")?);
    }

    Ok(make_string(vm, &s.repeat(n as usize))?)
}

/// reverse(s) - Reverse string (by characters).
fn native_reverse(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.reverse")?;
    let reversed: String = s.chars().rev().collect();
    Ok(make_string(vm, &reversed)?)
}

/// concat(a, b) - Concatenate two strings.
fn native_concat(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let a = get_string(vm, args[0], "string.concat")?;
    let b = get_string(vm, args[1], "string.concat")?;
    Ok(make_string(vm, &format!("{}{}", a, b))?)
}

/// trim(s) - Trim whitespace from both ends.
fn native_trim(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.trim")?.to_string();
    let trimmed = s.trim().to_string();
    Ok(make_string(vm, &trimmed)?)
}

/// trim_start(s) - Trim whitespace from start.
fn native_trim_start(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.trim_start")?.to_string();
    let trimmed = s.trim_start().to_string();
    Ok(make_string(vm, &trimmed)?)
}

/// trim_end(s) - Trim whitespace from end.
fn native_trim_end(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.trim_end")?.to_string();
    let trimmed = s.trim_end().to_string();
    Ok(make_string(vm, &trimmed)?)
}

/// pad_left(s, width, char) - Pad start to width with char.
fn native_pad_left(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.pad_left")?.to_string();
    let width = get_int(vm, args[1], "string.pad_left")? as usize;
    let pad_char = get_string(vm, args[2], "string.pad_left")?.to_string();

    let pad_c = pad_char.chars().next().unwrap_or(' ');
    let char_count = s.chars().count();

    if char_count >= width {
        return Ok(make_string(vm, &s)?);
    }

    let padding: String = std::iter::repeat(pad_c).take(width - char_count).collect();
    Ok(make_string(vm, &format!("{}{}", padding, s))?)
}

/// pad_right(s, width, char) - Pad end to width with char.
fn native_pad_right(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.pad_right")?.to_string();
    let width = get_int(vm, args[1], "string.pad_right")? as usize;
    let pad_char = get_string(vm, args[2], "string.pad_right")?.to_string();

    let pad_c = pad_char.chars().next().unwrap_or(' ');
    let char_count = s.chars().count();

    if char_count >= width {
        return Ok(make_string(vm, &s)?);
    }

    let padding: String = std::iter::repeat(pad_c).take(width - char_count).collect();
    Ok(make_string(vm, &format!("{}{}", s, padding))?)
}

/// is_empty(s) - Check if string is empty.
fn native_is_empty(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.is_empty")?;
    Ok(Value::bool(s.is_empty()))
}

/// is_whitespace(s) - Check if string contains only whitespace.
fn native_is_whitespace(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.is_whitespace")?;
    Ok(Value::bool(
        !s.is_empty() && s.chars().all(|c| c.is_whitespace()),
    ))
}

/// is_numeric(s) - Check if string contains only digits.
fn native_is_numeric(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.is_numeric")?;
    Ok(Value::bool(
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()),
    ))
}

/// is_alphabetic(s) - Check if string contains only alphabetic characters.
fn native_is_alphabetic(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.is_alphabetic")?;
    Ok(Value::bool(
        !s.is_empty() && s.chars().all(|c| c.is_alphabetic()),
    ))
}

/// is_alphanumeric(s) - Check if string contains only alphanumeric characters.
fn native_is_alphanumeric(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.is_alphanumeric")?;
    Ok(Value::bool(
        !s.is_empty() && s.chars().all(|c| c.is_alphanumeric()),
    ))
}

/// lines(s) - Split string into lines (returns newline-separated, but normalized).
fn native_lines(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.lines")?;
    // This normalizes line endings
    let result: Vec<&str> = s.lines().collect();
    Ok(make_string(vm, &result.join("\n"))?)
}

/// line_count(s) - Count number of lines.
fn native_line_count(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "string.line_count")?;
    Ok(Value::int(s.lines().count() as i64))
}
