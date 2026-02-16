use crate::stdlib::helpers::make_string;
use crate::stdlib::{StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::RuntimeError;
use std::io::{self, BufRead, Read, Write};

pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut exports = Vec::new();
    let mut natives = Vec::new();

    macro_rules! reg {
        ($name:expr, $arity:expr, $f:expr) => {{
            register_native(vm, "io", $name, $arity, $f)?;
            exports.push($name.to_string());
            natives.push(format!("io::{}", $name));
        }};
    }

    // core print/read
    reg!("print", 1, native_print);
    reg!("println", 1, native_println);
    reg!("eprint", 1, native_eprint);
    reg!("eprintln", 1, native_eprintln);
    reg!("readline", 0, native_readline);
    reg!("read_char", 0, native_read_char);
    reg!("flush", 0, native_flush);
    reg!("eflush", 0, native_eflush);
    reg!("print_inline", 1, native_print_inline);
    reg!("input", 1, native_input);

    // terminal control (ANSI escapes)
    reg!("clear_screen", 0, native_clear_screen);
    reg!("cursor_home", 0, native_cursor_home);
    reg!("hide_cursor", 0, native_hide_cursor);
    reg!("show_cursor", 0, native_show_cursor);
    reg!("move_cursor", 2, native_move_cursor);

    Ok(StdModuleExports {
        all_exports: exports,
        native_functions: natives,
    })
}

fn native_print(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    print!("{}", vm.value_to_string(args[0]));
    let _ = io::stdout().flush();
    Ok(Value::null())
}

fn native_println(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    println!("{}", vm.value_to_string(args[0]));
    Ok(Value::null())
}

fn native_eprint(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    eprint!("{}", vm.value_to_string(args[0]));
    let _ = io::stderr().flush();
    Ok(Value::null())
}

fn native_eprintln(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    eprintln!("{}", vm.value_to_string(args[0]));
    Ok(Value::null())
}

// returns null on EOF or error (don't crash on stdin issues)
fn native_readline(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let mut buf = String::new();
    match io::stdin().lock().read_line(&mut buf) {
        Ok(0) => Ok(Value::null()),
        Ok(_) => {
            // strip trailing newline (handles both \n and \r\n)
            if buf.ends_with('\n') {
                buf.pop();
            }
            if buf.ends_with('\r') {
                buf.pop();
            }
            make_string(vm, &buf)
        }
        Err(e) => {
            eprintln!("io.readline error: {}", e);
            Ok(Value::null())
        }
    }
}

// read single utf-8 char - bit fiddly because of variable-length encoding
fn native_read_char(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let mut buf = [0u8; 4]; // max utf-8 char
    let mut bytes = io::stdin().lock().bytes();

    let b = match bytes.next() {
        Some(Ok(b)) => b,
        Some(Err(e)) => {
            eprintln!("io.read_char error: {}", e);
            return Ok(Value::null());
        }
        None => return Ok(Value::null()),
    };
    buf[0] = b;

    // figure out how many continuation bytes we need
    let len = if b & 0x80 == 0 {
        1
    } else if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else if b & 0xF8 == 0xF0 {
        4
    } else {
        1
    }; // invalid start byte, treat as single

    for item in buf.iter_mut().take(len).skip(1) {
        if let Some(Ok(c)) = bytes.next() {
            *item = c;
        } else {
            break;
        }
    }

    match std::str::from_utf8(&buf[..len]) {
        Ok(s) => make_string(vm, s),
        Err(_) => make_string(vm, "\u{FFFD}"), // replacement char for invalid utf-8
    }
}

fn native_flush(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = io::stdout().flush();
    Ok(Value::null())
}

fn native_eflush(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let _ = io::stderr().flush();
    Ok(Value::null())
}

fn native_print_inline(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    print!("{}", vm.value_to_string(args[0]));
    let _ = io::stdout().flush();
    Ok(Value::null())
}

// prompt + readline combo, python-style
fn native_input(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    print!("{}", vm.value_to_string(args[0]));
    let _ = io::stdout().flush();

    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) => Ok(Value::null()),
        Ok(_) => {
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            make_string(vm, &line)
        }
        Err(_) => Ok(Value::null()),
    }
}

// ANSI escape helpers - nothing fancy, just the basics
fn native_clear_screen(_: &mut VM, _: &[Value]) -> Result<Value, RuntimeError> {
    print!("\x1b[2J\x1b[H"); // clear + home
    let _ = io::stdout().flush();
    Ok(Value::null())
}

fn native_cursor_home(_: &mut VM, _: &[Value]) -> Result<Value, RuntimeError> {
    print!("\x1b[H");
    let _ = io::stdout().flush();
    Ok(Value::null())
}

fn native_hide_cursor(_: &mut VM, _: &[Value]) -> Result<Value, RuntimeError> {
    print!("\x1b[?25l");
    let _ = io::stdout().flush();
    Ok(Value::null())
}

fn native_show_cursor(_: &mut VM, _: &[Value]) -> Result<Value, RuntimeError> {
    print!("\x1b[?25h");
    let _ = io::stdout().flush();
    Ok(Value::null())
}

fn native_move_cursor(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let x = args[0].as_int().ok_or_else(|| {
        vm.runtime_error(aelys_common::error::RuntimeErrorKind::TypeError {
            operation: "io.move_cursor",
            expected: "int",
            got: vm.value_type_name(args[0]).to_string(),
        })
    })?;
    let y = args[1].as_int().ok_or_else(|| {
        vm.runtime_error(aelys_common::error::RuntimeErrorKind::TypeError {
            operation: "io.move_cursor",
            expected: "int",
            got: vm.value_type_name(args[1]).to_string(),
        })
    })?;
    print!("\x1b[{};{}H", y, x); // row;col format
    let _ = io::stdout().flush();
    Ok(Value::null())
}
