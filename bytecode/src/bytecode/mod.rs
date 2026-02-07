// bytecode format and instruction encoding

mod buffer;
mod decode;
mod function;
mod global_layout;
mod opcode;
mod upvalue;

pub use buffer::BytecodeBuffer;
pub use decode::{decode_a, decode_b, decode_c};
pub use function::Function;
pub use global_layout::GlobalLayout;
pub use opcode::OpCode;
pub use upvalue::UpvalueDescriptor;
