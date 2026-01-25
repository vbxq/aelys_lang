#![doc = include_str!("docs.md")]

mod accessors;
mod checks;
mod constructors;
mod fmt;
mod util;

#[derive(Debug, Clone)]
pub struct IntegerOverflowError {
    pub value: i64,
}

// NaN-boxing: all values fit in 64 bits. Floats are stored directly,
// everything else uses the NaN space (quiet NaN has 51 bits of payload).
// This approach is used by LuaJIT, JavaScriptCore, etc. - proven fast.
//
// The 48-bit integer limit is a tradeoff: 64-bit ints would need heap
// allocation or a different encoding. ±140 trillion should be enough...
#[derive(Clone, Copy)]
pub struct Value(u64);

// Tag bits are in bits 48-50, payload in bits 0-47
const QNAN: u64 = 0x7FF8_0000_0000_0000;
const TAG_MASK: u64 = 0x0007_0000_0000_0000;
const TAG_PTR: u64 = 0x0000_0000_0000_0000;
const TAG_INT: u64 = 0x0001_0000_0000_0000;
const TAG_BOOL: u64 = 0x0002_0000_0000_0000;
const TAG_NULL: u64 = 0x0003_0000_0000_0000;
const TAG_NAN: u64 = 0x0004_0000_0000_0000;
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
const CANONICAL_NAN: u64 = QNAN | TAG_NAN | 1;

impl Value {
    pub const INT_MIN: i64 = -(1i64 << 47);
    pub const INT_MAX: i64 = (1i64 << 47) - 1;
}
