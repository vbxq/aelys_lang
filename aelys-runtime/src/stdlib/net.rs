use crate::stdlib::helpers::{get_handle, get_int, get_string, make_string};
use crate::stdlib::{Resource, StdModuleExports, TcpStreamResource, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

const MAX_BUFFER_SIZE: usize = 16 * 1024 * 1024;
const MAX_RECV_SIZE: usize = 16 * 1024 * 1024;

/// Register all net functions in the VM.
pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut all_exports = Vec::new();
    let mut native_functions = Vec::new();

    macro_rules! reg_fn {
        ($name:expr, $arity:expr, $func:expr) => {{
            register_native(vm, "net", $name, $arity, $func)?;
            all_exports.push($name.to_string());
            native_functions.push(format!("net::{}", $name));
        }};
    }

    reg_fn!("connect", 2, native_connect);
    reg_fn!("send", 2, native_send);
    reg_fn!("recv", 1, native_recv);
    reg_fn!("recv_bytes", 2, native_recv_bytes);
    reg_fn!("recv_line", 1, native_recv_line);
    reg_fn!("close", 1, native_close);
    reg_fn!("listen", 2, native_listen);
    reg_fn!("accept", 1, native_accept);
    reg_fn!("set_timeout", 2, native_set_timeout);
    reg_fn!("set_nodelay", 2, native_set_nodelay);
    reg_fn!("local_addr", 1, native_local_addr);
    reg_fn!("peer_addr", 1, native_peer_addr);
    reg_fn!("shutdown", 2, native_shutdown);

    Ok(StdModuleExports {
        all_exports,
        native_functions,
    })
}

/// Create a network error.
fn net_error(vm: &VM, op: &'static str, msg: String) -> RuntimeError {
    vm.runtime_error(RuntimeErrorKind::TypeError {
        operation: op,
        expected: "valid network operation",
        got: msg,
    })
}

/// connect(host, port) - Connect to a TCP server.
/// Returns a socket handle.
fn native_connect(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let host = get_string(vm, args[0], "net.connect")?;
    let port = get_int(vm, args[1], "net.connect")?;

    if port < 0 || port > 65535 {
        return Err(net_error(
            vm,
            "net.connect",
            format!("invalid port number: {}", port),
        ));
    }

    let addr = format!("{}:{}", host, port);

    // Resolve address
    let addrs: Vec<_> = match addr.to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(_) => return Ok(Value::null()),
    };

    if addrs.is_empty() {
        return Ok(Value::null());
    }

    // Try to connect with timeout
    let stream = match TcpStream::connect_timeout(&addrs[0], Duration::from_secs(30)) {
        Ok(s) => s,
        Err(_) => return Ok(Value::null()),
    };

    let resource = TcpStreamResource {
        stream,
        timeout_ms: None,
    };

    let handle = vm.store_resource(Resource::TcpStream(resource));
    Ok(Value::int(handle as i64))
}

/// send(handle, data) - Send data over connection.
/// Returns number of bytes sent.
fn native_send(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.send")?;
    let data = get_string(vm, args[1], "net.send")?.to_string();

    if let Some(Resource::TcpStream(res)) = vm.get_resource_mut(handle) {
        match res.stream.write_all(data.as_bytes()) {
            Ok(_) => {
                let _ = res.stream.flush();
                Ok(Value::int(data.len() as i64))
            }
            Err(_) => Ok(Value::null()),
        }
    } else {
        Ok(Value::null())
    }
}

/// recv(handle) - Receive all available data from connection.
/// Returns received data as string.
fn native_recv(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.recv")?;

    if let Some(Resource::TcpStream(res)) = vm.get_resource_mut(handle) {
        let mut buffer = vec![0u8; 65536];

        if res.timeout_ms.is_none() {
            let _ = res
                .stream
                .set_read_timeout(Some(Duration::from_millis(100)));
        }

        let mut all_data = Vec::new();
        loop {
            match res.stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if all_data.len() + n > MAX_RECV_SIZE {
                        break;
                    }
                    all_data.extend_from_slice(&buffer[..n]);
                    if n < buffer.len() {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                Err(_) => break,
            }
        }

        if res.timeout_ms.is_none() {
            let _ = res.stream.set_read_timeout(None);
        }

        let s = String::from_utf8_lossy(&all_data);
        Ok(make_string(vm, &s)?)
    } else {
        Ok(Value::null())
    }
}

/// recv_bytes(handle, max) - Receive up to max bytes.
fn native_recv_bytes(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.recv_bytes")?;
    let max = get_int(vm, args[1], "net.recv_bytes")?;

    if max < 0 {
        return Err(net_error(
            vm,
            "net.recv_bytes",
            "max must be non-negative".to_string(),
        ));
    }

    if max as usize > MAX_BUFFER_SIZE {
        return Err(net_error(
            vm,
            "net.recv_bytes",
            format!(
                "max exceeds maximum buffer size of {} bytes",
                MAX_BUFFER_SIZE
            ),
        ));
    }

    if let Some(Resource::TcpStream(res)) = vm.get_resource_mut(handle) {
        let mut buffer = vec![0u8; max as usize];
        match res.stream.read(&mut buffer) {
            Ok(n) => {
                buffer.truncate(n);
                let s = String::from_utf8_lossy(&buffer);
                Ok(make_string(vm, &s)?)
            }
            Err(_) => Ok(Value::null()),
        }
    } else {
        Ok(Value::null())
    }
}

/// recv_line(handle) - Receive a single line (up to newline).
fn native_recv_line(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.recv_line")?;

    if let Some(Resource::TcpStream(res)) = vm.get_resource_mut(handle) {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];

        loop {
            match res.stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    line.push(byte[0]);
                }
                Err(_) => break,
            }
        }

        if line.last() == Some(&b'\r') {
            line.pop();
        }

        let s = String::from_utf8_lossy(&line);
        Ok(make_string(vm, &s)?)
    } else {
        Ok(Value::null())
    }
}

/// close(handle) - Close a socket or listener.
fn native_close(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.close")?;

    match vm.take_resource(handle) {
        Some(Resource::TcpStream(res)) => {
            let _ = res.stream.shutdown(Shutdown::Both);
            Ok(Value::null())
        }
        Some(Resource::TcpListener(_)) => {
            // Listener is closed when dropped
            Ok(Value::null())
        }
        _ => Ok(Value::null()),
    }
}

/// listen(host, port) - Start listening for connections.
/// Returns a listener handle.
fn native_listen(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let host = get_string(vm, args[0], "net.listen")?;
    let port = get_int(vm, args[1], "net.listen")?;

    if port < 0 || port > 65535 {
        return Err(net_error(
            vm,
            "net.listen",
            format!("invalid port number: {}", port),
        ));
    }

    let addr = format!("{}:{}", host, port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(_) => return Ok(Value::null()),
    };

    let handle = vm.store_resource(Resource::TcpListener(listener));
    Ok(Value::int(handle as i64))
}

/// accept(handle) - Accept an incoming connection.
/// Returns a socket handle for the new connection.
fn native_accept(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.accept")?;

    // We need to get the listener, accept, then store the new stream
    let stream = if let Some(Resource::TcpListener(listener)) = vm.get_resource(handle) {
        match listener.accept() {
            Ok(s) => s,
            Err(_) => return Ok(Value::null()),
        }
    } else {
        return Ok(Value::null());
    };

    let resource = TcpStreamResource {
        stream: stream.0,
        timeout_ms: None,
    };

    let new_handle = vm.store_resource(Resource::TcpStream(resource));
    Ok(Value::int(new_handle as i64))
}

/// set_timeout(handle, ms) - Set read/write timeout in milliseconds.
/// Use 0 to disable timeout.
fn native_set_timeout(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.set_timeout")?;
    let ms = get_int(vm, args[1], "net.set_timeout")?;

    if ms < 0 {
        return Err(net_error(
            vm,
            "net.set_timeout",
            "timeout must be non-negative".to_string(),
        ));
    }

    if let Some(Resource::TcpStream(res)) = vm.get_resource_mut(handle) {
        let timeout = if ms == 0 {
            None
        } else {
            Some(Duration::from_millis(ms as u64))
        };

        if res.stream.set_read_timeout(timeout).is_err() {
            return Ok(Value::null());
        }
        if res.stream.set_write_timeout(timeout).is_err() {
            return Ok(Value::null());
        }
        res.timeout_ms = if ms == 0 { None } else { Some(ms as u64) };

        Ok(Value::null())
    } else {
        Ok(Value::null())
    }
}

/// set_nodelay(handle, enabled) - Enable/disable Nagle's algorithm.
fn native_set_nodelay(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.set_nodelay")?;
    let enabled = args[1].is_truthy();

    if let Some(Resource::TcpStream(res)) = vm.get_resource_mut(handle) {
        let _ = res.stream.set_nodelay(enabled);
        Ok(Value::null())
    } else {
        Ok(Value::null())
    }
}

/// local_addr(handle) - Get local address as "host:port".
fn native_local_addr(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.local_addr")?;

    let addr = match vm.get_resource(handle) {
        Some(Resource::TcpStream(res)) => res.stream.local_addr(),
        Some(Resource::TcpListener(listener)) => listener.local_addr(),
        _ => return Ok(Value::null()),
    };

    match addr {
        Ok(a) => Ok(make_string(vm, &a.to_string())?),
        Err(_) => Ok(Value::null()),
    }
}

/// peer_addr(handle) - Get peer address as "host:port".
fn native_peer_addr(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.peer_addr")?;

    if let Some(Resource::TcpStream(res)) = vm.get_resource(handle) {
        match res.stream.peer_addr() {
            Ok(a) => Ok(make_string(vm, &a.to_string())?),
            Err(_) => Ok(Value::null()),
        }
    } else {
        Ok(Value::null())
    }
}

/// shutdown(handle, how) - Shutdown part of a connection.
/// how: "read", "write", or "both"
fn native_shutdown(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = get_handle(vm, args[0], "net.shutdown")?;
    let how_str = get_string(vm, args[1], "net.shutdown")?;

    let how = match how_str {
        "read" => Shutdown::Read,
        "write" => Shutdown::Write,
        "both" => Shutdown::Both,
        _ => {
            return Err(net_error(
                vm,
                "net.shutdown",
                format!(
                    "invalid shutdown mode '{}', use 'read', 'write', or 'both'",
                    how_str
                ),
            ));
        }
    };

    if let Some(Resource::TcpStream(res)) = vm.get_resource(handle) {
        let _ = res.stream.shutdown(how);
        Ok(Value::null())
    } else {
        Ok(Value::null())
    }
}
