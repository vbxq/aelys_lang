//! Binary serialization for .avbc format

use crate::bytecode::{Function, GlobalLayout, UpvalueDescriptor};
use crate::heap::Heap;
use crate::object::{GcRef, ObjectKind};
use crate::value::Value;
use std::io::{self, Cursor, Read};
use thiserror::Error;

/// Magic bytes for .avbc files
pub const MAGIC: &[u8; 4] = b"VBXQ";

/// Current format version
pub const VERSION: u16 = 1;

const MAX_BYTECODE_LEN: usize = 1_000_000;
const MAX_CONSTANTS: usize = 65_535;
const MAX_NESTED_FUNCTIONS: usize = 4_096;
const MAX_UPVALUE_DESCRIPTORS: usize = 256;
const MAX_LINES: usize = 1_000_000;
const MAX_GLOBAL_NAMES: usize = 65_535;
const MAX_STRING_LEN: usize = 1_000_000;
const MAX_NESTING_DEPTH: usize = 64;
const MAX_SECTION_LEN: usize = 256 * 1024 * 1024;

const SECTION_MANIFEST: u32 = u32::from_le_bytes(*b"MANF");
const SECTION_BUNDLES: u32 = u32::from_le_bytes(*b"NBND");

/// Result type for deserialization with manifest and bundles
pub type DeserializeResult = Result<(Function, Heap, Option<Vec<u8>>, Vec<NativeBundle>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBundle {
    pub name: String,
    pub target: String,
    pub checksum: String,
    pub bytes: Vec<u8>,
}

/// Binary format errors
#[derive(Debug, Error)]
pub enum BinaryError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Invalid magic number")]
    InvalidMagic,

    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u16),

    #[error("Invalid constant type: {0}")]
    InvalidConstantType(u8),

    #[error("Invalid nested function index: {index} (max: {max})")]
    InvalidNestedFunctionIndex { index: usize, max: usize },

    #[error("Invalid UTF-8 in string")]
    InvalidUtf8,

    #[error("Unexpected end of file")]
    UnexpectedEof,

    #[error("Limit exceeded: {what} (max {limit})")]
    LimitExceeded { what: &'static str, limit: usize },
}

pub type Result<T> = std::result::Result<T, BinaryError>;

/// Serialize a function and its heap to .avbc binary format
pub fn serialize(func: &Function, heap: &Heap) -> Vec<u8> {
    let mut writer = BinaryWriter::new();
    writer.write_program(func, heap);
    writer.into_bytes()
}

/// Serialize a function and heap to .avbc with optional manifest and bundles.
pub fn serialize_with_manifest(
    func: &Function,
    heap: &Heap,
    manifest: Option<&[u8]>,
    bundles: Option<&[NativeBundle]>,
) -> Vec<u8> {
    let mut writer = BinaryWriter::new();
    writer.write_program(func, heap);
    if let Some(manifest_bytes) = manifest {
        writer.write_section(SECTION_MANIFEST, manifest_bytes);
    }
    if let Some(bundles) = bundles {
        let data = build_bundles_section(bundles);
        writer.write_section(SECTION_BUNDLES, &data);
    }
    writer.into_bytes()
}

/// Deserialize .avbc binary format to a function and heap
pub fn deserialize(data: &[u8]) -> Result<(Function, Heap)> {
    let reader = BinaryReader::new(data);
    reader.read_program()
}

/// Deserialize .avbc binary format to a function, heap, optional manifest, and bundles.
pub fn deserialize_with_manifest(data: &[u8]) -> DeserializeResult {
    let reader = BinaryReader::new(data);
    reader.read_program_with_sections()
}

/// Binary writer for .avbc format
struct BinaryWriter {
    buffer: Vec<u8>,
}

impl BinaryWriter {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    fn write_program(&mut self, func: &Function, heap: &Heap) {
        self.write_bytes(MAGIC);
        self.write_u16(VERSION);
        self.write_u16(0); // Flags (reserved)

        let func_count = count_functions(func);
        self.write_u32(func_count as u32);
        self.write_u32(0); // Reserved

        self.write_function(func, heap);
    }

    fn write_section(&mut self, tag: u32, data: &[u8]) {
        assert!(data.len() <= u32::MAX as usize, "section data too large");
        self.write_u32(tag);
        self.write_u32(data.len() as u32);
        self.write_bytes(data);
    }

    fn write_function(&mut self, func: &Function, heap: &Heap) {
        // Name
        if let Some(name) = &func.name {
            self.write_u16(name.len() as u16);
            self.write_bytes(name.as_bytes());
        } else {
            self.write_u16(0);
        }

        // Metadata
        self.write_u8(func.arity);
        self.write_u8(func.num_registers);

        // Constants
        self.write_u16(func.constants.len() as u16);
        for constant in &func.constants {
            self.write_constant(constant, heap);
        }

        // Bytecode - strip cache state during serialization
        self.write_u32(func.bytecode.len() as u32);
        let mut skip_cache_words = 0u32;
        for &instr in func.bytecode.iter() {
            if skip_cache_words > 0 {
                // Reset cache words to 0
                self.write_u32(0);
                skip_cache_words -= 1;
            } else {
                let opcode = (instr >> 24) as u8;
                // CallGlobalMono (78) -> CallGlobal (77) and skip next 2 words
                // CallGlobal (77) -> keep as-is but still reset cache words
                if opcode == 78 {
                    // Rewrite opcode from 78 to 77
                    let new_instr = (instr & 0x00FFFFFF) | (77 << 24);
                    self.write_u32(new_instr);
                    skip_cache_words = 2;
                } else if opcode == 77 {
                    // CallGlobal - write as-is but reset cache words
                    self.write_u32(instr);
                    skip_cache_words = 2;
                } else {
                    self.write_u32(instr);
                }
            }
        }

        // Nested functions
        self.write_u16(func.nested_functions.len() as u16);
        for nested in &func.nested_functions {
            self.write_function(nested, heap);
        }

        // Upvalue descriptors (needed for closures)
        self.write_u16(func.upvalue_descriptors.len() as u16);
        for desc in &func.upvalue_descriptors {
            self.write_u8(if desc.is_local { 1 } else { 0 });
            self.write_u8(desc.index);
        }

        // Line info (RLE)
        self.write_u16(func.lines.len() as u16);
        for &(count, line) in &func.lines {
            self.write_u16(count);
            self.write_u32(line);
        }

        // Global names (for indexed global access)
        self.write_u16(func.global_layout.names().len() as u16);
        for name in func.global_layout.names() {
            self.write_u16(name.len() as u16);
            self.write_bytes(name.as_bytes());
        }
    }

    fn write_constant(&mut self, value: &Value, heap: &Heap) {
        if value.is_null() {
            self.write_u8(0); // TAG_NULL
        } else if let Some(b) = value.as_bool() {
            self.write_u8(1); // TAG_BOOL
            self.write_u8(if b { 1 } else { 0 });
        } else if let Some(n) = value.as_int() {
            self.write_u8(2); // TAG_INT
            self.write_i64(n);
        } else if value.is_float() {
            if let Some(f) = value.as_float() {
                self.write_u8(3); // TAG_FLOAT
                self.write_f64(f);
            }
        } else if let Some(func_idx) = value.as_nested_fn_marker() {
            // Nested function marker (uses dedicated tag)
            self.write_u8(5); // TAG_FUNC
            self.write_u32(func_idx as u32);
        } else if let Some(ptr) = value.as_ptr() {
            // Try to resolve from heap
            if let Some(obj) = heap.get(GcRef::new(ptr)) {
                match &obj.kind {
                    ObjectKind::String(s) => {
                        self.write_u8(4); // TAG_STRING
                        let bytes = s.as_bytes();
                        self.write_u32(bytes.len() as u32);
                        self.write_bytes(bytes);
                    }
                    _ => {
                        // Other object types: store as ptr
                        self.write_u8(6); // TAG_PTR
                        self.write_u64(ptr as u64);
                    }
                }
            } else {
                self.write_u8(6); // TAG_PTR
                self.write_u64(ptr as u64);
            }
        } else {
            // Unknown: store as null
            self.write_u8(0);
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    fn write_u8(&mut self, v: u8) {
        self.buffer.push(v);
    }

    fn write_u16(&mut self, v: u16) {
        self.buffer.extend_from_slice(&v.to_le_bytes());
    }

    fn write_u32(&mut self, v: u32) {
        self.buffer.extend_from_slice(&v.to_le_bytes());
    }

    fn write_u64(&mut self, v: u64) {
        self.buffer.extend_from_slice(&v.to_le_bytes());
    }

    fn write_i64(&mut self, v: i64) {
        self.buffer.extend_from_slice(&v.to_le_bytes());
    }

    fn write_f64(&mut self, v: f64) {
        self.buffer.extend_from_slice(&v.to_le_bytes());
    }
}

fn build_bundles_section(bundles: &[NativeBundle]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_u32_to(&mut buf, bundles.len() as u32);
    for bundle in bundles {
        write_string_to(&mut buf, &bundle.name);
        write_string_to(&mut buf, &bundle.target);
        write_string_to(&mut buf, &bundle.checksum);
        write_bytes_to(&mut buf, &bundle.bytes);
    }
    assert!(
        buf.len() <= MAX_SECTION_LEN,
        "native bundle section too large"
    );
    buf
}

fn write_u32_to(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_string_to(buf: &mut Vec<u8>, s: &str) {
    assert!(s.len() <= u32::MAX as usize, "string too large for section");
    write_u32_to(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

fn write_bytes_to(buf: &mut Vec<u8>, bytes: &[u8]) {
    assert!(
        bytes.len() <= u32::MAX as usize,
        "bytes too large for section"
    );
    write_u32_to(buf, bytes.len() as u32);
    buf.extend_from_slice(bytes);
}

/// Binary reader for .avbc format
struct BinaryReader<'a> {
    cursor: Cursor<&'a [u8]>,
    heap: Heap,
}

impl<'a> BinaryReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            cursor: Cursor::new(data),
            heap: Heap::new(),
        }
    }

    fn read_program(mut self) -> Result<(Function, Heap)> {
        // Header
        let mut magic = [0u8; 4];
        self.cursor.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(BinaryError::InvalidMagic);
        }

        let version = self.read_u16()?;
        if version != VERSION {
            return Err(BinaryError::UnsupportedVersion(version));
        }

        let _flags = self.read_u16()?;
        let _func_count = self.read_u32()?;
        let _reserved = self.read_u32()?;

        // Read main function (which includes nested functions)
        let func = self.read_function(0)?;

        Ok((func, self.heap))
    }

    fn read_program_with_sections(mut self) -> DeserializeResult {
        // Header
        let mut magic = [0u8; 4];
        self.cursor.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(BinaryError::InvalidMagic);
        }

        let version = self.read_u16()?;
        if version != VERSION {
            return Err(BinaryError::UnsupportedVersion(version));
        }

        let _flags = self.read_u16()?;
        let _func_count = self.read_u32()?;
        let _reserved = self.read_u32()?;

        let func = self.read_function(0)?;
        let (manifest, bundles) = self.read_sections()?;

        Ok((func, self.heap, manifest, bundles))
    }

    fn read_function(&mut self, depth: usize) -> Result<Function> {
        if depth > MAX_NESTING_DEPTH {
            return Err(BinaryError::LimitExceeded {
                what: "function nesting depth",
                limit: MAX_NESTING_DEPTH,
            });
        }
        // Name
        let name_len = self.read_u16()? as usize;
        if name_len > MAX_STRING_LEN {
            return Err(BinaryError::LimitExceeded {
                what: "function name length",
                limit: MAX_STRING_LEN,
            });
        }
        let name = if name_len > 0 {
            let mut bytes = vec![0u8; name_len];
            self.cursor.read_exact(&mut bytes)?;
            Some(String::from_utf8(bytes).map_err(|_| BinaryError::InvalidUtf8)?)
        } else {
            None
        };

        // Metadata
        let arity = self.read_u8()?;
        let num_registers = self.read_u8()?;

        // Constants
        let const_count = self.read_u16()? as usize;
        if const_count > MAX_CONSTANTS {
            return Err(BinaryError::LimitExceeded {
                what: "constants",
                limit: MAX_CONSTANTS,
            });
        }
        let mut constants = Vec::with_capacity(const_count);
        for _ in 0..const_count {
            let value = self.read_constant()?;
            constants.push(value);
        }

        // Bytecode
        let bc_len = self.read_u32()? as usize;
        if bc_len > MAX_BYTECODE_LEN {
            return Err(BinaryError::LimitExceeded {
                what: "bytecode length",
                limit: MAX_BYTECODE_LEN,
            });
        }
        let mut bytecode = Vec::with_capacity(bc_len);
        for _ in 0..bc_len {
            bytecode.push(self.read_u32()?);
        }

        // Nested functions
        let nested_count = self.read_u16()? as usize;
        if nested_count > MAX_NESTED_FUNCTIONS {
            return Err(BinaryError::LimitExceeded {
                what: "nested functions",
                limit: MAX_NESTED_FUNCTIONS,
            });
        }

        Self::validate_func_markers(&constants, nested_count)?;

        let mut nested_functions = Vec::with_capacity(nested_count);
        for _ in 0..nested_count {
            nested_functions.push(self.read_function(depth + 1)?);
        }

        // Upvalue descriptors (needed for closures)
        let upvalue_count = self.read_u16()? as usize;
        if upvalue_count > MAX_UPVALUE_DESCRIPTORS {
            return Err(BinaryError::LimitExceeded {
                what: "upvalue descriptors",
                limit: MAX_UPVALUE_DESCRIPTORS,
            });
        }
        let mut upvalue_descriptors = Vec::with_capacity(upvalue_count);
        for _ in 0..upvalue_count {
            let is_local = self.read_u8()? != 0;
            let index = self.read_u8()?;
            upvalue_descriptors.push(UpvalueDescriptor { is_local, index });
        }

        // Line info
        let lines_count = self.read_u16()? as usize;
        if lines_count > MAX_LINES {
            return Err(BinaryError::LimitExceeded {
                what: "line info entries",
                limit: MAX_LINES,
            });
        }
        let mut lines = Vec::with_capacity(lines_count);
        for _ in 0..lines_count {
            let count = self.read_u16()?;
            let line = self.read_u32()?;
            lines.push((count, line));
        }

        // Global names (for indexed global access)
        let global_names_count = self.read_u16()? as usize;
        if global_names_count > MAX_GLOBAL_NAMES {
            return Err(BinaryError::LimitExceeded {
                what: "global names",
                limit: MAX_GLOBAL_NAMES,
            });
        }
        let mut global_names = Vec::with_capacity(global_names_count);
        for _ in 0..global_names_count {
            let name_len = self.read_u16()? as usize;
            if name_len > MAX_STRING_LEN {
                return Err(BinaryError::LimitExceeded {
                    what: "global name length",
                    limit: MAX_STRING_LEN,
                });
            }
            let name = if name_len > 0 {
                let mut bytes = vec![0u8; name_len];
                self.cursor.read_exact(&mut bytes)?;
                String::from_utf8(bytes).map_err(|_| {
                    BinaryError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid UTF-8 in global name",
                    ))
                })?
            } else {
                String::new()
            };
            global_names.push(name);
        }

        // Compute global_layout_hash from global layout names
        let mut func = Function::new(name, arity);
        func.num_registers = num_registers;
        func.set_bytecode(bytecode);
        func.constants = constants;
        func.nested_functions = nested_functions;
        func.upvalue_descriptors = upvalue_descriptors;
        func.lines = lines;
        func.global_layout = GlobalLayout::new(global_names);
        func.compute_global_layout_hash();

        Ok(func)
    }

    fn validate_func_markers(constants: &[Value], nested_count: usize) -> Result<()> {
        for constant in constants {
            if let Some(func_idx) = constant.as_nested_fn_marker()
                && func_idx >= nested_count
            {
                return Err(BinaryError::InvalidNestedFunctionIndex {
                    index: func_idx,
                    max: nested_count.saturating_sub(1),
                });
            }
        }
        Ok(())
    }

    fn read_constant(&mut self) -> Result<Value> {
        let tag = self.read_u8()?;
        match tag {
            0 => Ok(Value::null()), // TAG_NULL
            1 => {
                // TAG_BOOL
                let b = self.read_u8()? != 0;
                Ok(Value::bool(b))
            }
            2 => {
                // TAG_INT
                let n = self.read_i64()?;
                Ok(Value::int(n))
            }
            3 => {
                // TAG_FLOAT
                let f = self.read_f64()?;
                Ok(Value::float(f))
            }
            4 => {
                // TAG_STRING
                let len = self.read_u32()? as usize;
                if len > MAX_STRING_LEN {
                    return Err(BinaryError::LimitExceeded {
                        what: "string length",
                        limit: MAX_STRING_LEN,
                    });
                }
                let mut bytes = vec![0u8; len];
                self.cursor.read_exact(&mut bytes)?;
                let s = String::from_utf8(bytes).map_err(|_| BinaryError::InvalidUtf8)?;
                let str_ref = self.heap.intern_string(&s);
                Ok(Value::ptr(str_ref.index()))
            }
            5 => {
                // TAG_FUNC (nested function marker with dedicated tag)
                let func_idx = self.read_u32()? as usize;
                Ok(Value::nested_fn_marker(func_idx))
            }
            6 => {
                // TAG_PTR
                let ptr = self.read_u64()? as usize;
                Ok(Value::ptr(ptr))
            }
            _ => Err(BinaryError::InvalidConstantType(tag)),
        }
    }

    fn read_sections(&mut self) -> Result<(Option<Vec<u8>>, Vec<NativeBundle>)> {
        let mut manifest = None;
        let mut bundles = Vec::new();
        while self.remaining() > 0 {
            let tag = self.read_u32()?;
            let len = self.read_u32()? as usize;
            if len > MAX_SECTION_LEN {
                return Err(BinaryError::LimitExceeded {
                    what: "section length",
                    limit: MAX_SECTION_LEN,
                });
            }
            let data = self.read_bytes(len)?;
            match tag {
                SECTION_MANIFEST => {
                    manifest = Some(data);
                }
                SECTION_BUNDLES => {
                    let mut parsed = parse_bundles_section(&data)?;
                    bundles.append(&mut parsed);
                }
                _ => {}
            }
        }
        Ok((manifest, bundles))
    }

    fn remaining(&self) -> usize {
        let pos = self.cursor.position() as usize;
        let len = self.cursor.get_ref().len();
        len.saturating_sub(pos)
    }

    fn read_bytes(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.cursor.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn read_u8(&mut self) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.cursor.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_u16(&mut self) -> Result<u16> {
        let mut buf = [0u8; 2];
        self.cursor.read_exact(&mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let mut buf = [0u8; 4];
        self.cursor.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let mut buf = [0u8; 8];
        self.cursor.read_exact(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn read_i64(&mut self) -> Result<i64> {
        let mut buf = [0u8; 8];
        self.cursor.read_exact(&mut buf)?;
        Ok(i64::from_le_bytes(buf))
    }

    fn read_f64(&mut self) -> Result<f64> {
        let mut buf = [0u8; 8];
        self.cursor.read_exact(&mut buf)?;
        Ok(f64::from_le_bytes(buf))
    }
}

fn parse_bundles_section(data: &[u8]) -> Result<Vec<NativeBundle>> {
    let mut cursor = Cursor::new(data);
    let count = read_u32_from(&mut cursor)? as usize;
    let mut bundles = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_string_from(&mut cursor, "bundle name")?;
        let target = read_string_from(&mut cursor, "bundle target")?;
        let checksum = read_string_from(&mut cursor, "bundle checksum")?;
        let bytes = read_bytes_from(&mut cursor, "bundle bytes")?;
        bundles.push(NativeBundle {
            name,
            target,
            checksum,
            bytes,
        });
    }
    Ok(bundles)
}

fn read_u32_from(cursor: &mut Cursor<&[u8]>) -> Result<u32> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_string_from(cursor: &mut Cursor<&[u8]>, what: &'static str) -> Result<String> {
    let len = read_u32_from(cursor)? as usize;
    if len > MAX_STRING_LEN {
        return Err(BinaryError::LimitExceeded {
            what,
            limit: MAX_STRING_LEN,
        });
    }
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|_| BinaryError::InvalidUtf8)
}

fn read_bytes_from(cursor: &mut Cursor<&[u8]>, what: &'static str) -> Result<Vec<u8>> {
    let len = read_u32_from(cursor)? as usize;
    if len > MAX_SECTION_LEN {
        return Err(BinaryError::LimitExceeded {
            what,
            limit: MAX_SECTION_LEN,
        });
    }
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf)?;
    Ok(buf)
}

/// Count total number of functions (including nested)
fn count_functions(func: &Function) -> usize {
    let mut count = 1;
    for nested in &func.nested_functions {
        count += count_functions(nested);
    }
    count
}
