use crate::stdlib::helpers::{get_handle, get_int, get_string, make_string};
use crate::stdlib::{ByteBuffer, Resource, StdModuleExports, register_native};
use crate::vm::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

const MAX_ALLOC: usize = 256 * 1024 * 1024;

fn err(vm: &VM, op: &'static str, msg: String) -> RuntimeError {
    vm.runtime_error(RuntimeErrorKind::TypeError {
        operation: op,
        expected: "valid byte buffer operation",
        got: msg,
    })
}

fn get_buf_len(vm: &VM, h: usize, op: &'static str) -> Result<usize, RuntimeError> {
    match vm.get_resource(h) {
        Some(Resource::ByteBuffer(buf)) => Ok(buf.data.len()),
        _ => Err(err(vm, op, "invalid handle".into())),
    }
}

fn check_offset(off: i64, op: &'static str) -> Result<usize, String> {
    if off < 0 {
        return Err(format!("{}: negative offset {}", op, off));
    }
    Ok(off as usize)
}

fn bounds_err(size: usize, off: usize, buf_len: usize) -> Option<String> {
    if off.checked_add(size).map_or(true, |end| end > buf_len) {
        Some(format!("{}B at offset {} exceeds buffer size {}", size, off, buf_len))
    } else {
        None
    }
}

macro_rules! impl_read {
    ($name:ident, $op:literal, $size:expr, $ty:ty, $conv:ident, $to_val:expr) => {
        fn $name(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
            let h = get_handle(vm, args[0], $op)?;
            let off = check_offset(get_int(vm, args[1], $op)?, $op).map_err(|e| err(vm, $op, e))?;
            if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h) {
                if let Some(e) = bounds_err($size, off, buf.data.len()) {
                    return Err(err(vm, $op, e));
                }
                let mut arr = [0u8; $size];
                arr.copy_from_slice(&buf.data[off..off + $size]);
                let v = <$ty>::$conv(arr);
                Ok($to_val(v))
            } else {
                Err(err(vm, $op, "invalid handle".into()))
            }
        }
    };
}

macro_rules! impl_write_int {
    ($name:ident, $op:literal, $size:expr, $ty:ty, $conv:ident, $min:expr, $max:expr) => {
        fn $name(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
            let h = get_handle(vm, args[0], $op)?;
            let off = check_offset(get_int(vm, args[1], $op)?, $op).map_err(|e| err(vm, $op, e))?;
            let val = get_int(vm, args[2], $op)?;
            if val < $min || val > $max {
                return Err(err(vm, $op, format!("value {} out of range [{}, {}]", val, $min, $max)));
            }
            let buf_len = get_buf_len(vm, h, $op)?;
            if let Some(e) = bounds_err($size, off, buf_len) {
                return Err(err(vm, $op, e));
            }
            if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
                buf.data[off..off + $size].copy_from_slice(&(val as $ty).$conv());
                Ok(Value::null())
            } else {
                Err(err(vm, $op, "invalid handle".into()))
            }
        }
    };
}

macro_rules! impl_write_float {
    ($name:ident, $op:literal, $size:expr, $ty:ty, $conv:ident) => {
        fn $name(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
            let h = get_handle(vm, args[0], $op)?;
            let off = check_offset(get_int(vm, args[1], $op)?, $op).map_err(|e| err(vm, $op, e))?;
            let val: $ty = if let Some(f) = args[2].as_float() {
                f as $ty
            } else if let Some(i) = args[2].as_int() {
                i as $ty
            } else {
                return Err(err(vm, $op, format!("expected number, got {}", vm.value_type_name(args[2]))));
            };
            let buf_len = get_buf_len(vm, h, $op)?;
            if let Some(e) = bounds_err($size, off, buf_len) {
                return Err(err(vm, $op, e));
            }
            if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
                buf.data[off..off + $size].copy_from_slice(&val.$conv());
                Ok(Value::null())
            } else {
                Err(err(vm, $op, "invalid handle".into()))
            }
        }
    };
}

pub fn register(vm: &mut VM) -> Result<StdModuleExports, RuntimeError> {
    let mut exports = Vec::new();
    let mut natives = Vec::new();

    macro_rules! reg {
        ($n:expr, $a:expr, $f:expr) => {{
            register_native(vm, "bytes", $n, $a, $f)?;
            exports.push($n.to_string());
            natives.push(format!("bytes::{}", $n));
        }};
    }

    reg!("alloc", 1, native_alloc);
    reg!("free", 1, native_free);
    reg!("size", 1, native_size);
    reg!("resize", 2, native_resize);
    reg!("clone", 1, native_clone);
    reg!("equals", 2, native_equals);

    reg!("read_u8", 2, native_read_u8);
    reg!("write_u8", 3, native_write_u8);
    reg!("read_i8", 2, native_read_i8);
    reg!("write_i8", 3, native_write_i8);

    reg!("read_u16", 2, native_read_u16);
    reg!("write_u16", 3, native_write_u16);
    reg!("read_i16", 2, native_read_i16);
    reg!("write_i16", 3, native_write_i16);
    reg!("read_u16_be", 2, native_read_u16_be);
    reg!("write_u16_be", 3, native_write_u16_be);
    reg!("read_i16_be", 2, native_read_i16_be);
    reg!("write_i16_be", 3, native_write_i16_be);

    reg!("read_u32", 2, native_read_u32);
    reg!("write_u32", 3, native_write_u32);
    reg!("read_i32", 2, native_read_i32);
    reg!("write_i32", 3, native_write_i32);
    reg!("read_u32_be", 2, native_read_u32_be);
    reg!("write_u32_be", 3, native_write_u32_be);
    reg!("read_i32_be", 2, native_read_i32_be);
    reg!("write_i32_be", 3, native_write_i32_be);

    reg!("read_u64", 2, native_read_u64);
    reg!("write_u64", 3, native_write_u64);
    reg!("read_i64", 2, native_read_i64);
    reg!("write_i64", 3, native_write_i64);
    reg!("read_u64_be", 2, native_read_u64_be);
    reg!("write_u64_be", 3, native_write_u64_be);
    reg!("read_i64_be", 2, native_read_i64_be);
    reg!("write_i64_be", 3, native_write_i64_be);

    reg!("read_f32", 2, native_read_f32);
    reg!("write_f32", 3, native_write_f32);
    reg!("read_f64", 2, native_read_f64);
    reg!("write_f64", 3, native_write_f64);
    reg!("read_f32_be", 2, native_read_f32_be);
    reg!("write_f32_be", 3, native_write_f32_be);
    reg!("read_f64_be", 2, native_read_f64_be);
    reg!("write_f64_be", 3, native_write_f64_be);

    reg!("copy", 5, native_copy);
    reg!("fill", 4, native_fill);

    reg!("from_string", 1, native_from_string);
    reg!("decode", 3, native_decode);
    reg!("write_string", 3, native_write_string);
    reg!("find", 4, native_find);
    reg!("reverse", 3, native_reverse);
    reg!("swap", 3, native_swap);

    Ok(StdModuleExports { all_exports: exports, native_functions: natives })
}

fn native_alloc(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let size = get_int(vm, args[0], "alloc")?;
    if size <= 0 {
        return Err(err(vm, "alloc", format!("size must be positive, got {}", size)));
    }
    if size as usize > MAX_ALLOC {
        return Err(err(vm, "alloc", format!("size {} exceeds max {}", size, MAX_ALLOC)));
    }
    let handle = vm.store_resource(Resource::ByteBuffer(ByteBuffer { data: vec![0u8; size as usize] }));
    Ok(Value::int(handle as i64))
}

fn native_free(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if args[0].is_null() {
        return Ok(Value::null());
    }
    let h = get_handle(vm, args[0], "free")?;
    match vm.take_resource(h) {
        Some(Resource::ByteBuffer(_)) => Ok(Value::null()),
        Some(_) => Err(err(vm, "free", "not a byte buffer".into())),
        None => Err(err(vm, "free", "invalid handle".into())),
    }
}

fn native_size(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "size")?;
    Ok(Value::int(get_buf_len(vm, h, "size")? as i64))
}

fn native_resize(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "resize")?;
    let new_size = get_int(vm, args[1], "resize")?;
    if new_size <= 0 {
        return Err(err(vm, "resize", format!("size must be positive, got {}", new_size)));
    }
    if new_size as usize > MAX_ALLOC {
        return Err(err(vm, "resize", format!("size {} exceeds max {}", new_size, MAX_ALLOC)));
    }
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
        buf.data.resize(new_size as usize, 0);
        Ok(Value::null())
    } else {
        Err(err(vm, "resize", "invalid handle".into()))
    }
}

fn native_clone(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "clone")?;
    let data = if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h) {
        buf.data.clone()
    } else {
        return Err(err(vm, "clone", "invalid handle".into()));
    };
    let handle = vm.store_resource(Resource::ByteBuffer(ByteBuffer { data }));
    Ok(Value::int(handle as i64))
}

fn native_equals(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h1 = get_handle(vm, args[0], "equals")?;
    let h2 = get_handle(vm, args[1], "equals")?;
    let d1 = if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h1) {
        buf.data.clone()
    } else {
        return Err(err(vm, "equals", "invalid first handle".into()));
    };
    let d2 = if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h2) {
        &buf.data
    } else {
        return Err(err(vm, "equals", "invalid second handle".into()));
    };
    Ok(Value::bool(d1 == *d2))
}

fn native_read_u8(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "read_u8")?;
    let off = check_offset(get_int(vm, args[1], "read_u8")?, "read_u8").map_err(|e| err(vm, "read_u8", e))?;
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h) {
        if off >= buf.data.len() {
            return Err(err(vm, "read_u8", format!("offset {} >= size {}", off, buf.data.len())));
        }
        Ok(Value::int(buf.data[off] as i64))
    } else {
        Err(err(vm, "read_u8", "invalid handle".into()))
    }
}

fn native_write_u8(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "write_u8")?;
    let off = check_offset(get_int(vm, args[1], "write_u8")?, "write_u8").map_err(|e| err(vm, "write_u8", e))?;
    let val = get_int(vm, args[2], "write_u8")?;
    if val < 0 || val > 255 {
        return Err(err(vm, "write_u8", format!("value {} out of range [0, 255]", val)));
    }
    let buf_len = get_buf_len(vm, h, "write_u8")?;
    if off >= buf_len {
        return Err(err(vm, "write_u8", format!("offset {} >= size {}", off, buf_len)));
    }
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
        buf.data[off] = val as u8;
        Ok(Value::null())
    } else {
        Err(err(vm, "write_u8", "invalid handle".into()))
    }
}

fn native_read_i8(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "read_i8")?;
    let off = check_offset(get_int(vm, args[1], "read_i8")?, "read_i8").map_err(|e| err(vm, "read_i8", e))?;
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h) {
        if off >= buf.data.len() {
            return Err(err(vm, "read_i8", format!("offset {} >= size {}", off, buf.data.len())));
        }
        Ok(Value::int(buf.data[off] as i8 as i64))
    } else {
        Err(err(vm, "read_i8", "invalid handle".into()))
    }
}

fn native_write_i8(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "write_i8")?;
    let off = check_offset(get_int(vm, args[1], "write_i8")?, "write_i8").map_err(|e| err(vm, "write_i8", e))?;
    let val = get_int(vm, args[2], "write_i8")?;
    if val < i8::MIN as i64 || val > i8::MAX as i64 {
        return Err(err(vm, "write_i8", format!("value {} out of range [{}, {}]", val, i8::MIN, i8::MAX)));
    }
    let buf_len = get_buf_len(vm, h, "write_i8")?;
    if off >= buf_len {
        return Err(err(vm, "write_i8", format!("offset {} >= size {}", off, buf_len)));
    }
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
        buf.data[off] = val as i8 as u8;
        Ok(Value::null())
    } else {
        Err(err(vm, "write_i8", "invalid handle".into()))
    }
}

impl_read!(native_read_u16, "read_u16", 2, u16, from_le_bytes, |v: u16| Value::int(v as i64));
impl_read!(native_read_i16, "read_i16", 2, i16, from_le_bytes, |v: i16| Value::int(v as i64));
impl_read!(native_read_u16_be, "read_u16_be", 2, u16, from_be_bytes, |v: u16| Value::int(v as i64));
impl_read!(native_read_i16_be, "read_i16_be", 2, i16, from_be_bytes, |v: i16| Value::int(v as i64));

impl_write_int!(native_write_u16, "write_u16", 2, u16, to_le_bytes, 0, u16::MAX as i64);
impl_write_int!(native_write_i16, "write_i16", 2, i16, to_le_bytes, i16::MIN as i64, i16::MAX as i64);
impl_write_int!(native_write_u16_be, "write_u16_be", 2, u16, to_be_bytes, 0, u16::MAX as i64);
impl_write_int!(native_write_i16_be, "write_i16_be", 2, i16, to_be_bytes, i16::MIN as i64, i16::MAX as i64);

impl_read!(native_read_u32, "read_u32", 4, u32, from_le_bytes, |v: u32| Value::int(v as i64));
impl_read!(native_read_i32, "read_i32", 4, i32, from_le_bytes, |v: i32| Value::int(v as i64));
impl_read!(native_read_u32_be, "read_u32_be", 4, u32, from_be_bytes, |v: u32| Value::int(v as i64));
impl_read!(native_read_i32_be, "read_i32_be", 4, i32, from_be_bytes, |v: i32| Value::int(v as i64));

impl_write_int!(native_write_u32, "write_u32", 4, u32, to_le_bytes, 0, u32::MAX as i64);
impl_write_int!(native_write_i32, "write_i32", 4, i32, to_le_bytes, i32::MIN as i64, i32::MAX as i64);
impl_write_int!(native_write_u32_be, "write_u32_be", 4, u32, to_be_bytes, 0, u32::MAX as i64);
impl_write_int!(native_write_i32_be, "write_i32_be", 4, i32, to_be_bytes, i32::MIN as i64, i32::MAX as i64);

impl_read!(native_read_u64, "read_u64", 8, u64, from_le_bytes, |v: u64| Value::int(v as i64));
impl_read!(native_read_i64, "read_i64", 8, i64, from_le_bytes, |v: i64| Value::int(v));
impl_read!(native_read_u64_be, "read_u64_be", 8, u64, from_be_bytes, |v: u64| Value::int(v as i64));
impl_read!(native_read_i64_be, "read_i64_be", 8, i64, from_be_bytes, |v: i64| Value::int(v));

impl_write_int!(native_write_u64, "write_u64", 8, u64, to_le_bytes, i64::MIN, i64::MAX);
impl_write_int!(native_write_i64, "write_i64", 8, i64, to_le_bytes, i64::MIN, i64::MAX);
impl_write_int!(native_write_u64_be, "write_u64_be", 8, u64, to_be_bytes, i64::MIN, i64::MAX);
impl_write_int!(native_write_i64_be, "write_i64_be", 8, i64, to_be_bytes, i64::MIN, i64::MAX);

impl_read!(native_read_f32, "read_f32", 4, f32, from_le_bytes, |v: f32| Value::float(v as f64));
impl_read!(native_read_f64, "read_f64", 8, f64, from_le_bytes, |v: f64| Value::float(v));
impl_read!(native_read_f32_be, "read_f32_be", 4, f32, from_be_bytes, |v: f32| Value::float(v as f64));
impl_read!(native_read_f64_be, "read_f64_be", 8, f64, from_be_bytes, |v: f64| Value::float(v));

impl_write_float!(native_write_f32, "write_f32", 4, f32, to_le_bytes);
impl_write_float!(native_write_f64, "write_f64", 8, f64, to_le_bytes);
impl_write_float!(native_write_f32_be, "write_f32_be", 4, f32, to_be_bytes);
impl_write_float!(native_write_f64_be, "write_f64_be", 8, f64, to_be_bytes);

fn native_copy(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let src_h = get_handle(vm, args[0], "copy")?;
    let src_off = check_offset(get_int(vm, args[1], "copy")?, "copy").map_err(|e| err(vm, "copy", e))?;
    let dst_h = get_handle(vm, args[2], "copy")?;
    let dst_off = check_offset(get_int(vm, args[3], "copy")?, "copy").map_err(|e| err(vm, "copy", e))?;
    let len = get_int(vm, args[4], "copy")?;
    if len < 0 {
        return Err(err(vm, "copy", format!("negative length {}", len)));
    }
    let len = len as usize;
    if len == 0 {
        return Ok(Value::null());
    }

    if src_h == dst_h {
        let buf_len = get_buf_len(vm, src_h, "copy")?;
        if let Some(e) = bounds_err(len, src_off, buf_len) {
            return Err(err(vm, "copy", e));
        }
        if let Some(e) = bounds_err(len, dst_off, buf_len) {
            return Err(err(vm, "copy", e));
        }
        if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(src_h) {
            buf.data.copy_within(src_off..src_off + len, dst_off);
            return Ok(Value::null());
        } else {
            return Err(err(vm, "copy", "invalid handle".into()));
        }
    }

    let src_data = if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(src_h) {
        if let Some(e) = bounds_err(len, src_off, buf.data.len()) {
            return Err(err(vm, "copy", e));
        }
        buf.data[src_off..src_off + len].to_vec()
    } else {
        return Err(err(vm, "copy", "invalid source handle".into()));
    };

    let dst_len = get_buf_len(vm, dst_h, "copy")?;
    if let Some(e) = bounds_err(len, dst_off, dst_len) {
        return Err(err(vm, "copy", e));
    }
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(dst_h) {
        buf.data[dst_off..dst_off + len].copy_from_slice(&src_data);
        Ok(Value::null())
    } else {
        Err(err(vm, "copy", "invalid dest handle".into()))
    }
}

fn native_fill(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "fill")?;
    let off = check_offset(get_int(vm, args[1], "fill")?, "fill").map_err(|e| err(vm, "fill", e))?;
    let len = get_int(vm, args[2], "fill")?;
    let val = get_int(vm, args[3], "fill")?;
    if len < 0 {
        return Err(err(vm, "fill", format!("negative length {}", len)));
    }
    if val < 0 || val > 255 {
        return Err(err(vm, "fill", format!("value {} out of range [0, 255]", val)));
    }
    let len = len as usize;
    if len == 0 {
        return Ok(Value::null());
    }
    let buf_len = get_buf_len(vm, h, "fill")?;
    if let Some(e) = bounds_err(len, off, buf_len) {
        return Err(err(vm, "fill", e));
    }
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
        buf.data[off..off + len].fill(val as u8);
        Ok(Value::null())
    } else {
        Err(err(vm, "fill", "invalid handle".into()))
    }
}

fn native_from_string(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = get_string(vm, args[0], "from_string")?;
    let data = s.as_bytes().to_vec();
    if data.len() > MAX_ALLOC {
        return Err(err(vm, "from_string", format!("string length {} exceeds max {}", data.len(), MAX_ALLOC)));
    }
    let handle = vm.store_resource(Resource::ByteBuffer(ByteBuffer { data }));
    Ok(Value::int(handle as i64))
}

fn native_decode(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "decode")?;
    let off = check_offset(get_int(vm, args[1], "decode")?, "decode").map_err(|e| err(vm, "decode", e))?;
    let len = get_int(vm, args[2], "decode")?;
    if len < 0 {
        return Err(err(vm, "decode", format!("negative length {}", len)));
    }
    let len = len as usize;
    let s = if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h) {
        if let Some(e) = bounds_err(len, off, buf.data.len()) {
            return Err(err(vm, "decode", e));
        }
        match std::str::from_utf8(&buf.data[off..off + len]) {
            Ok(s) => s.to_string(),
            Err(e) => return Err(err(vm, "decode", format!("invalid UTF-8 at byte {}", e.valid_up_to()))),
        }
    } else {
        return Err(err(vm, "decode", "invalid handle".into()));
    };
    make_string(vm, &s)
}

fn native_write_string(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "write_string")?;
    let off = check_offset(get_int(vm, args[1], "write_string")?, "write_string").map_err(|e| err(vm, "write_string", e))?;
    let s = get_string(vm, args[2], "write_string")?.to_string();
    let bytes = s.as_bytes();
    let buf_len = get_buf_len(vm, h, "write_string")?;
    if let Some(e) = bounds_err(bytes.len(), off, buf_len) {
        return Err(err(vm, "write_string", e));
    }
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
        buf.data[off..off + bytes.len()].copy_from_slice(bytes);
        Ok(Value::int(bytes.len() as i64))
    } else {
        Err(err(vm, "write_string", "invalid handle".into()))
    }
}

fn native_find(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "find")?;
    let start = check_offset(get_int(vm, args[1], "find")?, "find").map_err(|e| err(vm, "find", e))?;
    let end = get_int(vm, args[2], "find")?;
    let needle = get_int(vm, args[3], "find")?;
    if needle < 0 || needle > 255 {
        return Err(err(vm, "find", format!("needle {} out of range [0, 255]", needle)));
    }
    let needle = needle as u8;
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource(h) {
        let buf_len = buf.data.len();
        let end = if end < 0 { buf_len } else { (end as usize).min(buf_len) };
        if start >= end {
            return Ok(Value::int(-1));
        }
        match buf.data[start..end].iter().position(|&b| b == needle) {
            Some(pos) => Ok(Value::int((start + pos) as i64)),
            None => Ok(Value::int(-1)),
        }
    } else {
        Err(err(vm, "find", "invalid handle".into()))
    }
}

fn native_reverse(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "reverse")?;
    let off = check_offset(get_int(vm, args[1], "reverse")?, "reverse").map_err(|e| err(vm, "reverse", e))?;
    let len = get_int(vm, args[2], "reverse")?;
    if len < 0 {
        return Err(err(vm, "reverse", format!("negative length {}", len)));
    }
    let len = len as usize;
    if len == 0 {
        return Ok(Value::null());
    }
    let buf_len = get_buf_len(vm, h, "reverse")?;
    if let Some(e) = bounds_err(len, off, buf_len) {
        return Err(err(vm, "reverse", e));
    }
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
        buf.data[off..off + len].reverse();
        Ok(Value::null())
    } else {
        Err(err(vm, "reverse", "invalid handle".into()))
    }
}

fn native_swap(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let h = get_handle(vm, args[0], "swap")?;
    let i = check_offset(get_int(vm, args[1], "swap")?, "swap").map_err(|e| err(vm, "swap", e))?;
    let j = check_offset(get_int(vm, args[2], "swap")?, "swap").map_err(|e| err(vm, "swap", e))?;
    if let Some(Resource::ByteBuffer(buf)) = vm.get_resource_mut(h) {
        let len = buf.data.len();
        if i >= len {
            return Err(err(vm, "swap", format!("index {} >= size {}", i, len)));
        }
        if j >= len {
            return Err(err(vm, "swap", format!("index {} >= size {}", j, len)));
        }
        buf.data.swap(i, j);
        Ok(Value::null())
    } else {
        Err(err(vm, "swap", "invalid handle".into()))
    }
}
