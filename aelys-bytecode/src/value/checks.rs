use super::{QNAN, TAG_BOOL, TAG_INT, TAG_MASK, TAG_NESTED_FN, TAG_NULL, TAG_PTR, Value};

impl Value {
    // floats are the only values that don't have QNAN set (except actual NaN which we canonicalize)
    // Canonical NaN (TAG_NAN) is also a float. TAG_NESTED_FN is not a float.
    pub fn is_float(&self) -> bool {
        if (self.0 & QNAN) != QNAN {
            return true;
        }
        let tag = self.0 & TAG_MASK;
        tag != TAG_PTR
            && tag != TAG_INT
            && tag != TAG_BOOL
            && tag != TAG_NULL
            && tag != TAG_NESTED_FN
    }

    pub fn is_int(&self) -> bool {
        (self.0 & (QNAN | TAG_MASK)) == (QNAN | TAG_INT)
    }
    pub fn is_bool(&self) -> bool {
        (self.0 & (QNAN | TAG_MASK)) == (QNAN | TAG_BOOL)
    }
    pub fn is_null(&self) -> bool {
        (self.0 & (QNAN | TAG_MASK)) == (QNAN | TAG_NULL)
    }
    pub fn is_ptr(&self) -> bool {
        (self.0 & (QNAN | TAG_MASK)) == (QNAN | TAG_PTR)
    }
}
