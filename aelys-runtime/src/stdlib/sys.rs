use crate::stdlib::helpers::{get_int, get_string, make_string};
use crate::stdlib::{StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use std::env;
use std::process::Command;

pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut all_exports = Vec::new();
    let mut native_functions = Vec::new();

    macro_rules! reg_fn {
        ($name:expr, $arity:expr, $func:expr) => {{
            register_native(vm, "sys", $name, $arity, $func)?;
            all_exports.push($name.to_string());
            native_functions.push(format!("sys::{}", $name));
        }};
    }

    reg_fn!("args", 0, native_args);
    reg_fn!("arg", 1, native_arg);
    reg_fn!("arg_count", 0, native_arg_count);
    reg_fn!("script_path", 0, native_script_path);
    reg_fn!("script_dir", 0, native_script_dir);
    reg_fn!("env", 1, native_env);
    reg_fn!("set_env", 2, native_set_env);
    reg_fn!("unset_env", 1, native_unset_env);
    reg_fn!("env_vars", 0, native_env_vars);
    reg_fn!("exit", 1, native_exit);
    reg_fn!("pid", 0, native_pid);
    reg_fn!("cwd", 0, native_cwd);
    reg_fn!("set_cwd", 1, native_set_cwd);
    reg_fn!("home", 0, native_home);
    reg_fn!("platform", 0, native_platform);
    reg_fn!("arch", 0, native_arch);
    reg_fn!("os", 0, native_os);
    reg_fn!("hostname", 0, native_hostname);
    reg_fn!("cpu_count", 0, native_cpu_count);
    reg_fn!("exec", 1, native_exec);
    reg_fn!("exec_output", 1, native_exec_output);
    reg_fn!("exec_args", 2, native_exec_args);
    reg_fn!("exec_args_output", 2, native_exec_args_output);
    reg_fn!("random", 0, native_random);
    reg_fn!("random_int", 2, native_random_int);

    Ok(StdModuleExports {
        all_exports,
        native_functions,
    })
}

/// Create a sys error.
fn sys_error(vm: &VM, op: &'static str, msg: String) -> RuntimeError {
    vm.runtime_error(RuntimeErrorKind::TypeError {
        operation: op,
        expected: "valid system operation",
        got: msg,
    })
}

/// args() - Get command line arguments as newline-separated string.
fn native_args(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let joined = {
        let args = vm.program_args();
        args.join("\n")
    };
    Ok(make_string(vm, &joined)?)
}

/// arg(index) - Get specific argument by index (0-based).
/// Returns null if index is out of bounds.
fn native_arg(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let index = get_int(vm, args[0], "sys.arg")?;

    if index < 0 {
        return Ok(Value::null());
    }

    let arg = vm.program_args().get(index as usize).cloned();

    match arg {
        Some(arg) => Ok(make_string(vm, &arg)?),
        None => Ok(Value::null()),
    }
}

/// arg_count() - Get number of command line arguments.
fn native_arg_count(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::int(vm.program_args().len() as i64))
}

/// script_path() - Get the absolute path to the currently executing script.
/// Returns null if not available.
fn native_script_path(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    match vm.script_path().map(|s| s.to_string()) {
        Some(path) => Ok(make_string(vm, &path)?),
        None => Ok(Value::null()),
    }
}

/// script_dir() - Get the directory containing the currently executing script.
/// Returns null if not available.
fn native_script_dir(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    use std::path::Path;
    match vm.script_path().map(|s| s.to_string()) {
        Some(path) => {
            let path = Path::new(&path);
            match path.parent() {
                Some(dir) => Ok(make_string(vm, &dir.to_string_lossy())?),
                None => Ok(Value::null()),
            }
        }
        None => Ok(Value::null()),
    }
}

/// env(name) - Get environment variable.
/// Returns null if not set.
fn native_env(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let name = get_string(vm, args[0], "sys.env")?;
    match env::var(name) {
        Ok(value) => Ok(make_string(vm, &value)?),
        Err(_) => Ok(Value::null()),
    }
}

/// set_env(name, value) - Set environment variable.
fn native_set_env(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let name = get_string(vm, args[0], "sys.set_env")?.to_string();
    let value = get_string(vm, args[1], "sys.set_env")?.to_string();
    // SAFETY: We're setting environment variables with valid UTF-8 strings.
    // This is safe as long as no other threads are reading env vars concurrently.
    unsafe { env::set_var(&name, &value) };
    Ok(Value::null())
}

/// unset_env(name) - Unset environment variable.
fn native_unset_env(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let name = get_string(vm, args[0], "sys.unset_env")?.to_string();
    // SAFETY: We're removing environment variables.
    // This is safe as long as no other threads are reading env vars concurrently.
    unsafe { env::remove_var(&name) };
    Ok(Value::null())
}

/// env_vars() - Get all environment variables as NAME=VALUE lines.
fn native_env_vars(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let vars: Vec<String> = env::vars().map(|(k, v)| format!("{}={}", k, v)).collect();
    Ok(make_string(vm, &vars.join("\n"))?)
}

/// exit(code) - Exit with status code.
fn native_exit(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let code = get_int(vm, args[0], "sys.exit")?;
    std::process::exit(code as i32);
}

/// pid() - Get current process ID.
fn native_pid(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::int(std::process::id() as i64))
}

/// cwd() - Get current working directory.
fn native_cwd(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    match env::current_dir() {
        Ok(path) => Ok(make_string(vm, &path.to_string_lossy())?),
        Err(e) => Err(sys_error(vm, "sys.cwd", format!("cannot get cwd: {}", e))),
    }
}

/// set_cwd(path) - Set current working directory.
fn native_set_cwd(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let path = get_string(vm, args[0], "sys.set_cwd")?;
    match env::set_current_dir(path) {
        Ok(_) => Ok(Value::null()),
        Err(e) => Err(sys_error(
            vm,
            "sys.set_cwd",
            format!("cannot change to '{}': {}", path, e),
        )),
    }
}

/// home() - Get home directory.
fn native_home(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    match home_dir() {
        Some(path) => Ok(make_string(vm, &path)?),
        None => Ok(Value::null()),
    }
}

/// Get home directory in a cross-platform way.
fn home_dir() -> Option<String> {
    // Try HOME first (Unix)
    if let Ok(home) = env::var("HOME") {
        return Some(home);
    }

    // Try USERPROFILE (Windows)
    if let Ok(home) = env::var("USERPROFILE") {
        return Some(home);
    }

    // Try HOMEDRIVE + HOMEPATH (Windows fallback)
    if let (Ok(drive), Ok(path)) = (env::var("HOMEDRIVE"), env::var("HOMEPATH")) {
        return Some(format!("{}{}", drive, path));
    }

    None
}

/// platform() - Get platform name.
fn native_platform(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "freebsd") {
        "freebsd"
    } else if cfg!(target_os = "openbsd") {
        "openbsd"
    } else if cfg!(target_os = "netbsd") {
        "netbsd"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "unknown"
    };
    Ok(make_string(vm, platform)?)
}

/// arch() - Get architecture.
fn native_arch(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "arm") {
        "arm"
    } else if cfg!(target_arch = "riscv64") {
        "riscv64"
    } else if cfg!(target_arch = "riscv32") {
        "riscv32"
    } else if cfg!(target_arch = "powerpc64") {
        "powerpc64"
    } else if cfg!(target_arch = "powerpc") {
        "powerpc"
    } else if cfg!(target_arch = "mips64") {
        "mips64"
    } else if cfg!(target_arch = "mips") {
        "mips"
    } else {
        "unknown"
    };
    Ok(make_string(vm, arch)?)
}

/// os() - Get OS name (more detailed than platform).
fn native_os(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(make_string(vm, std::env::consts::OS)?)
}

/// hostname() - Get hostname.
fn native_hostname(vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    // Try to get hostname from environment
    if let Ok(hostname) = env::var("HOSTNAME") {
        return Ok(make_string(vm, &hostname)?);
    }

    // Fall back to reading /etc/hostname on Unix
    #[cfg(unix)]
    {
        if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
            return Ok(make_string(vm, hostname.trim())?);
        }

        // Try reading from /proc/sys/kernel/hostname
        if let Ok(hostname) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
            return Ok(make_string(vm, hostname.trim())?);
        }
    }

    // Windows: use COMPUTERNAME
    if let Ok(hostname) = env::var("COMPUTERNAME") {
        return Ok(make_string(vm, &hostname)?);
    }

    Ok(make_string(vm, "unknown")?)
}

/// cpu_count() - Get number of CPUs.
fn native_cpu_count(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    Ok(Value::int(num_cpus() as i64))
}

/// Get number of CPUs (portable implementation).
fn num_cpus() -> usize {
    // Try to get from environment (for containers/cgroups)
    if let Ok(cpus) = env::var("AELYS_CPU_COUNT") {
        if let Ok(n) = cpus.parse::<usize>() {
            return n;
        }
    }

    // Use std::thread::available_parallelism if available
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1)
}

/// exec(command) - Execute shell command, returns exit code.
fn native_exec(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if !vm.capabilities().allow_exec {
        return Err(vm.runtime_error(RuntimeErrorKind::CapabilityDenied {
            operation: "sys.exec",
        }));
    }
    let command = get_string(vm, args[0], "sys.exec")?;

    #[cfg(unix)]
    let status = Command::new("sh").arg("-c").arg(command).status();

    #[cfg(windows)]
    let status = Command::new("cmd").arg("/C").arg(command).status();

    match status {
        Ok(status) => Ok(Value::int(status.code().unwrap_or(-1) as i64)),
        Err(e) => Err(sys_error(
            vm,
            "sys.exec",
            format!("failed to execute '{}': {}", command, e),
        )),
    }
}

/// exec_output(command) - Execute command and capture output.
/// Returns stdout as string, or null on error.
fn native_exec_output(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if !vm.capabilities().allow_exec {
        return Err(vm.runtime_error(RuntimeErrorKind::CapabilityDenied {
            operation: "sys.exec_output",
        }));
    }
    let command = get_string(vm, args[0], "sys.exec_output")?;

    #[cfg(unix)]
    let output = Command::new("sh").arg("-c").arg(command).output();

    #[cfg(windows)]
    let output = Command::new("cmd").arg("/C").arg(command).output();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(make_string(vm, &stdout)?)
        }
        Err(e) => Err(sys_error(
            vm,
            "sys.exec_output",
            format!("failed to execute '{}': {}", command, e),
        )),
    }
}

/// exec_args(program, args_string) - Execute program with arguments directly (no shell).
/// Arguments are passed as a newline-separated string.
/// This is safer than exec() as it prevents shell injection attacks.
/// Returns exit code.
fn native_exec_args(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if !vm.capabilities().allow_exec {
        return Err(vm.runtime_error(RuntimeErrorKind::CapabilityDenied {
            operation: "sys.exec_args",
        }));
    }
    let program = get_string(vm, args[0], "sys.exec_args")?;
    let args_str = get_string(vm, args[1], "sys.exec_args")?;

    // Split arguments by newline
    let cmd_args: Vec<&str> = args_str.lines().collect();

    let status = Command::new(program).args(&cmd_args).status();

    match status {
        Ok(status) => Ok(Value::int(status.code().unwrap_or(-1) as i64)),
        Err(e) => Err(sys_error(
            vm,
            "sys.exec_args",
            format!("failed to execute '{}': {}", program, e),
        )),
    }
}

/// exec_args_output(program, args_string) - Execute program with arguments and capture output.
/// Arguments are passed as a newline-separated string.
/// This is safer than exec_output() as it prevents shell injection attacks.
/// Returns stdout as string.
fn native_exec_args_output(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if !vm.capabilities().allow_exec {
        return Err(vm.runtime_error(RuntimeErrorKind::CapabilityDenied {
            operation: "sys.exec_args_output",
        }));
    }
    let program = get_string(vm, args[0], "sys.exec_args_output")?;
    let args_str = get_string(vm, args[1], "sys.exec_args_output")?;

    // Split arguments by newline
    let cmd_args: Vec<&str> = args_str.lines().collect();

    let output = Command::new(program).args(&cmd_args).output();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(make_string(vm, &stdout)?)
        }
        Err(e) => Err(sys_error(
            vm,
            "sys.exec_args_output",
            format!("failed to execute '{}': {}", program, e),
        )),
    }
}

/// random() - Get a random float between 0 and 1.
// TODO: Imagine making prng3d and not using a proper PRNG here :)
fn native_random(_vm: &mut VM, _args: &[Value]) -> Result<Value, RuntimeError> {
    // Simple PRNG using system time as seed
    use std::time::SystemTime;

    let seed = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    // Simple xorshift
    let mut x = seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;

    // Convert to float in [0, 1)
    let result = (x as f64) / (u64::MAX as f64);
    Ok(Value::float(result))
}

/// random_int(min, max) - Get a random integer in range [min, max].
fn native_random_int(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let min = get_int(vm, args[0], "sys.random_int")?;
    let max = get_int(vm, args[1], "sys.random_int")?;

    if min > max {
        return Err(sys_error(
            vm,
            "sys.random_int",
            format!("min ({}) > max ({})", min, max),
        ));
    }

    use std::time::SystemTime;

    let seed = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    // Simple xorshift
    let mut x = seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;

    // Map to range
    let range = (max - min + 1) as u64;
    let result = min + (x % range) as i64;
    Ok(Value::int(result))
}
