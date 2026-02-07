// NaN-boxing helpers for native modules
// mirrors aelys-runtime/src/vm/value.rs

use crate::AelysValue;

const QNAN: u64 = 0x7FF8_0000_0000_0000;
const TAG_INT: u64 = 0x0001_0000_0000_0000;
const TAG_BOOL: u64 = 0x0002_0000_0000_0000;
const TAG_NULL: u64 = 0x0003_0000_0000_0000;
const TAG_NAN: u64 = 0x0004_0000_0000_0000;
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

pub fn value_null() -> AelysValue {
    QNAN | TAG_NULL
}

pub fn value_bool(b: bool) -> AelysValue {
    QNAN | TAG_BOOL | (b as u64)
}

pub fn value_int(n: i64) -> AelysValue {
    let payload = (n as u64) & PAYLOAD_MASK;
    QNAN | TAG_INT | payload
}

pub fn value_float(n: f64) -> AelysValue {
    if n.is_nan() {
        QNAN | TAG_NAN | 1
    } else {
        n.to_bits()
    }
}

pub fn value_as_int(v: AelysValue) -> i64 {
    if (v & (QNAN | 0x0007_0000_0000_0000)) == (QNAN | TAG_INT) {
        let payload = v & PAYLOAD_MASK;
        if payload & 0x0000_8000_0000_0000 != 0 {
            (payload | 0xFFFF_0000_0000_0000) as i64
        } else {
            payload as i64
        }
    } else {
        0
    }
}

pub fn value_as_float(v: AelysValue) -> f64 {
    if (v & QNAN) == QNAN {
        if (v & (QNAN | 0x0007_0000_0000_0000)) == (QNAN | TAG_NAN) {
            return f64::NAN;
        }
        return 0.0;
    }
    f64::from_bits(v)
}

pub fn value_as_bool(v: AelysValue) -> bool {
    if (v & (QNAN | 0x0007_0000_0000_0000)) == (QNAN | TAG_BOOL) {
        (v & 1) != 0
    } else {
        false
    }
}

pub fn value_is_null(v: AelysValue) -> bool {
    (v & (QNAN | 0x0007_0000_0000_0000)) == (QNAN | TAG_NULL)
}
pub fn value_is_int(v: AelysValue) -> bool {
    (v & (QNAN | 0x0007_0000_0000_0000)) == (QNAN | TAG_INT)
}
pub fn value_is_float(v: AelysValue) -> bool {
    (v & QNAN) != QNAN || (v & (QNAN | 0x0007_0000_0000_0000)) == (QNAN | TAG_NAN)
}
pub fn value_is_bool(v: AelysValue) -> bool {
    (v & (QNAN | 0x0007_0000_0000_0000)) == (QNAN | TAG_BOOL)
}
