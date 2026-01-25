// .aasm assembly format - text representation of bytecode

pub mod assembler;
pub mod binary;
pub mod disasm;
mod lexer;
mod opcodes;

pub use assembler::{AssemblerError, assemble, assemble_from_string};
pub use binary::{
    BinaryError, NativeBundle, deserialize, deserialize_with_manifest, serialize,
    serialize_with_manifest,
};
pub use disasm::{DisassemblerOptions, disassemble, disassemble_to_string};
