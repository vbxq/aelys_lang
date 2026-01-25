#[test]
fn small_int_immediate_accepts_i16_range() {
    let small_int_immediate = |n: i64| -> Option<i16> {
        if (i16::MIN as i64) <= n && n <= (i16::MAX as i64) {
            Some(n as i16)
        } else {
            None
        }
    };

    assert_eq!(small_int_immediate(10), Some(10));
    assert_eq!(small_int_immediate(i16::MAX as i64), Some(i16::MAX));
    assert_eq!(small_int_immediate(i16::MIN as i64), Some(i16::MIN));
    assert_eq!(small_int_immediate(i16::MAX as i64 + 1), None);
    assert_eq!(small_int_immediate(i16::MIN as i64 - 1), None);
}
