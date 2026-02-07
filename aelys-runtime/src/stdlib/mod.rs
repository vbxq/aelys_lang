pub mod bytes;
pub mod convert;
pub mod fs;
pub mod io;
pub mod math;
pub mod net;
pub mod string;
pub mod sys;
pub mod time;

use crate::vm::{GcRef, ObjectKind, VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::time::Instant;

#[derive(Debug)]
pub enum Resource {
    File(FileResource),
    TcpStream(TcpStreamResource),
    TcpListener(TcpListener),
    UdpSocket(UdpSocketResource),
    Timer(Instant),
    ByteBuffer(ByteBuffer),
}

#[derive(Debug)]
pub struct FileResource {
    pub reader: Option<BufReader<File>>,
    pub writer: Option<BufWriter<File>>,
    pub path: String,
    pub mode: FileMode,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileMode {
    Read,
    Write,
    Append,
    ReadWrite,
}

#[derive(Debug)]
pub struct TcpStreamResource {
    pub stream: TcpStream,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug)]
pub struct UdpSocketResource {
    pub socket: UdpSocket,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug)]
pub struct ByteBuffer {
    pub data: Vec<u8>,
}

pub const STD_MODULES: &[&str] = &[
    "std.math",
    "std.io",
    "std.fs",
    "std.net",
    "std.time",
    "std.string",
    "std.convert",
    "std.sys",
    "std.bytes",
];

pub fn is_std_module(path: &[String]) -> bool {
    if path.is_empty() || path[0] != "std" {
        return false;
    }
    if path.len() == 1 {
        // Just "std" alone is not valid
        return false;
    }
    let full_path = path.join(".");
    STD_MODULES.contains(&full_path.as_str())
}

pub fn get_std_module_name(path: &[String]) -> Option<&str> {
    if path.len() >= 2 && path[0] == "std" {
        Some(&path[1])
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct StdModuleExports {
    pub all_exports: Vec<String>,
    pub native_functions: Vec<String>, // for CallGlobalNative opt
}

pub fn register_std_module(
    vm: &mut VM,
    module_name: &str,
) -> Result<StdModuleExports, RuntimeError> {
    match module_name {
        "math" => math::register(vm),
        "io" => io::register(vm),
        "fs" => {
            if !vm.capabilities().allow_fs {
                return Err(vm.runtime_error(RuntimeErrorKind::CapabilityDenied {
                    operation: "std.fs",
                }));
            }
            fs::register(vm)
        }
        "net" => {
            if !vm.capabilities().allow_net {
                return Err(vm.runtime_error(RuntimeErrorKind::CapabilityDenied {
                    operation: "std.net",
                }));
            }
            net::register(vm)
        }
        "time" => time::register(vm),
        "string" => string::register(vm),
        "convert" => convert::register(vm),
        "sys" => sys::register(vm),
        "bytes" => bytes::register(vm),
        _ => Err(
            vm.runtime_error(RuntimeErrorKind::UndefinedVariable(format!(
                "std.{}",
                module_name
            ))),
        ),
    }
}

pub trait VmResources {
    fn store_resource(&mut self, resource: Resource) -> usize;
    fn get_resource(&self, handle: usize) -> Option<&Resource>;
    fn get_resource_mut(&mut self, handle: usize) -> Option<&mut Resource>;
    fn take_resource(&mut self, handle: usize) -> Option<Resource>;
    fn is_valid_handle(&self, handle: usize) -> bool;
}

pub mod helpers {
    use super::*;

    pub fn get_number(vm: &VM, value: Value, op: &'static str) -> Result<f64, RuntimeError> {
        if let Some(f) = value.as_float() {
            Ok(f)
        } else if let Some(i) = value.as_int() {
            Ok(i as f64)
        } else {
            Err(vm.runtime_error(RuntimeErrorKind::TypeError {
                operation: op,
                expected: "number",
                got: vm.value_type_name(value).to_string(),
            }))
        }
    }

    pub fn get_int(vm: &VM, value: Value, op: &'static str) -> Result<i64, RuntimeError> {
        value.as_int().ok_or_else(|| {
            vm.runtime_error(RuntimeErrorKind::TypeError {
                operation: op,
                expected: "int",
                got: vm.value_type_name(value).to_string(),
            })
        })
    }

    pub fn get_string<'a>(
        vm: &'a VM,
        value: Value,
        op: &'static str,
    ) -> Result<&'a str, RuntimeError> {
        if let Some(ptr) = value.as_ptr()
            && let Some(obj) = vm.heap().get(GcRef::new(ptr))
            && let ObjectKind::String(s) = &obj.kind
        {
            return Ok(s.as_str());
        }
        Err(vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: op,
            expected: "string",
            got: vm.value_type_name(value).to_string(),
        }))
    }

    pub fn get_bool(vm: &VM, value: Value, op: &'static str) -> Result<bool, RuntimeError> {
        value.as_bool().ok_or_else(|| {
            vm.runtime_error(RuntimeErrorKind::TypeError {
                operation: op,
                expected: "bool",
                got: vm.value_type_name(value).to_string(),
            })
        })
    }

    pub fn make_string(vm: &mut VM, s: &str) -> Result<Value, RuntimeError> {
        let str_ref = vm.intern_string(s)?;
        Ok(Value::ptr(str_ref.index()))
    }

    pub fn get_handle(vm: &VM, value: Value, op: &'static str) -> Result<usize, RuntimeError> {
        let handle = value.as_int().ok_or_else(|| {
            vm.runtime_error(RuntimeErrorKind::TypeError {
                operation: op,
                expected: "handle (int)",
                got: vm.value_type_name(value).to_string(),
            })
        })?;
        if handle < 0 {
            return Err(vm.runtime_error(RuntimeErrorKind::TypeError {
                operation: op,
                expected: "non-negative handle",
                got: format!("negative value {}", handle),
            }));
        }
        Ok(handle as usize)
    }

    pub fn make_int_checked(vm: &VM, n: i64, op: &'static str) -> Result<Value, RuntimeError> {
        Value::int_checked(n).map_err(|_| {
            vm.runtime_error(RuntimeErrorKind::TypeError {
                operation: op,
                expected: "integer in 48-bit range",
                got: format!(
                    "value {} exceeds range ({} to {})",
                    n,
                    Value::INT_MIN,
                    Value::INT_MAX
                ),
            })
        })
    }
}

pub fn register_native(
    vm: &mut VM,
    module_alias: &str,
    name: &str,
    arity: u8,
    func: crate::vm::NativeFn,
) -> Result<(), RuntimeError> {
    let qualified_name = format!("{}::{}", module_alias, name);
    let func_ref = vm.alloc_native(&qualified_name, arity, func)?;
    vm.set_global(qualified_name, Value::ptr(func_ref.index()));
    Ok(())
}

pub fn register_constant(vm: &mut VM, module_alias: &str, name: &str, value: Value) {
    let qualified_name = format!("{}::{}", module_alias, name);
    vm.set_global(qualified_name, value);
}
