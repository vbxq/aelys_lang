use super::{PAYLOAD_MASK, Value};

impl Value {
    pub fn as_int(&self) -> Option<i64> {
        if !self.is_int() { return None; }
        let payload = self.0 & PAYLOAD_MASK;
        Some(((payload << 16) as i64) >> 16) // sign-extend from 48 bits
    }

    pub fn as_float(&self) -> Option<f64> {
        self.is_float().then(|| f64::from_bits(self.0))
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.is_bool().then(|| (self.0 & 1) != 0)
    }

    pub fn as_ptr(&self) -> Option<usize> {
        self.is_ptr().then(|| (self.0 & PAYLOAD_MASK) as usize)
    }

    #[inline(always)]
    pub fn raw_bits(&self) -> u64 { self.0 }

    #[inline(always)]
    pub fn from_raw(bits: u64) -> Self { Self(bits) }

    // unchecked variants for type-specialized opcodes (hot paths)
    #[inline(always)]
    pub fn as_int_unchecked(&self) -> i64 {
        debug_assert!(self.is_int(), "type confusion: not an int");
        ((self.0 & PAYLOAD_MASK) << 16) as i64 >> 16
    }

    #[inline(always)]
    pub fn as_float_unchecked(&self) -> f64 {
        debug_assert!(self.is_float(), "type confusion: not a float");
        f64::from_bits(self.0)
    }
}
