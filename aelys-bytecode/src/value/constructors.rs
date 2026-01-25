use super::{
    CANONICAL_NAN, IntegerOverflowError, PAYLOAD_MASK, QNAN, TAG_BOOL, TAG_INT, TAG_NULL, TAG_PTR,
    Value,
};

impl Value {
    // wraps silently on overflow - caller should use int_checked if they care
    #[inline(always)]
    pub fn int(n: i64) -> Self {
        Self(QNAN | TAG_INT | ((n as u64) & PAYLOAD_MASK))
    }

    #[inline(always)]
    pub fn int_checked(n: i64) -> Result<Self, IntegerOverflowError> {
        // sign-extend from 48 bits and check if it matches
        if n == (n << 16) >> 16 { Ok(Self::int(n)) }
        else { Err(IntegerOverflowError { value: n }) }
    }

    pub fn float(n: f64) -> Self {
        if n.is_nan() { Self(CANONICAL_NAN) } else { Self(n.to_bits()) }
    }

    pub fn bool(b: bool) -> Self { Self(QNAN | TAG_BOOL | (b as u64)) }

    pub fn null() -> Self { Self(QNAN | TAG_NULL) }

    pub fn ptr(p: usize) -> Self {
        debug_assert!(p <= PAYLOAD_MASK as usize, "ptr too big for NaN boxing");
        Self(QNAN | TAG_PTR | (p as u64))
    }
}
