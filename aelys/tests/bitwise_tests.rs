use aelys::run_with_config_and_opt;
use aelys_opt::OptimizationLevel;
use aelys_runtime::{Value, VmConfig};

fn run(code: &str) -> Value {
    run_with_config_and_opt(
        code,
        "<test>",
        VmConfig::default(),
        Vec::new(),
        OptimizationLevel::None,
    )
    .expect("Code should execute successfully")
}

fn run_with_opt(code: &str, level: OptimizationLevel) -> Value {
    run_with_config_and_opt(code, "<test>", VmConfig::default(), Vec::new(), level)
        .expect("Code should execute successfully")
}

fn run_fails(code: &str) -> bool {
    run_with_config_and_opt(
        code,
        "<test>",
        VmConfig::default(),
        Vec::new(),
        OptimizationLevel::None,
    )
    .is_err()
}

#[test]
fn test_left_shift() {
    let result = run("5 << 2");
    assert_eq!(result.as_int(), Some(20)); // 5 * 4 = 20
}

#[test]
fn test_right_shift() {
    let result = run("20 >> 2");
    assert_eq!(result.as_int(), Some(5)); // 20 / 4 = 5
}

#[test]
fn test_right_shift_negative() {
    // Arithmetic right shift preserves sign
    let result = run("-8 >> 2");
    assert_eq!(result.as_int(), Some(-2));
}

#[test]
fn test_bitwise_and() {
    let result = run("12 & 10");
    // 12 = 1100, 10 = 1010, AND = 1000 = 8
    assert_eq!(result.as_int(), Some(8));
}

#[test]
fn test_bitwise_or() {
    let result = run("12 | 10");
    // 12 = 1100, 10 = 1010, OR = 1110 = 14
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn test_bitwise_xor() {
    let result = run("12 ^ 10");
    // 12 = 1100, 10 = 1010, XOR = 0110 = 6
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn test_bitwise_not() {
    let result = run("~5");
    // Two's complement: ~5 = -6
    assert_eq!(result.as_int(), Some(-6));
}

#[test]
fn test_bitwise_not_negative() {
    let result = run("~(-6)");
    // ~(-6) = 5
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_shift_by_zero() {
    let result = run("42 << 0");
    assert_eq!(result.as_int(), Some(42));

    let result = run("42 >> 0");
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_bitwise_with_zero() {
    let result = run("0 & 255");
    assert_eq!(result.as_int(), Some(0));

    let result = run("0 | 255");
    assert_eq!(result.as_int(), Some(255));

    let result = run("0 ^ 255");
    assert_eq!(result.as_int(), Some(255));
}

#[test]
fn test_bitwise_identity() {
    let result = run("255 & 255");
    assert_eq!(result.as_int(), Some(255));

    let result = run("255 | 255");
    assert_eq!(result.as_int(), Some(255));

    let result = run("255 ^ 255");
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn test_not_not_identity() {
    let result = run("~~42");
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_precedence_shift_over_and() {
    // << and >> have higher precedence than &
    let result = run("1 << 4 & 16");
    // Should be (1 << 4) & 16 = 16 & 16 = 16
    assert_eq!(result.as_int(), Some(16));
}

#[test]
fn test_precedence_and_over_xor() {
    // & has higher precedence than ^
    let result = run("15 & 7 ^ 3");
    // Should be (15 & 7) ^ 3 = 7 ^ 3 = 4
    assert_eq!(result.as_int(), Some(4));
}

#[test]
fn test_precedence_xor_over_or() {
    // ^ has higher precedence than |
    let result = run("8 ^ 4 | 2");
    // Should be (8 ^ 4) | 2 = 12 | 2 = 14
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn test_precedence_not_highest() {
    // ~ has highest precedence
    let result = run("~0 & 255");
    // Should be (~0) & 255 = -1 & 255 = 255
    assert_eq!(result.as_int(), Some(255));
}

#[test]
fn test_compound_bit_flags() {
    // Setting multiple bits
    let result = run("(1 << 0) | (1 << 2) | (1 << 4)");
    // 1 + 4 + 16 = 21
    assert_eq!(result.as_int(), Some(21));
}

#[test]
fn test_mask_and_shift() {
    // Extract middle byte
    let result = run("(0xFF00 >> 8) & 0xFF");
    assert_eq!(result.as_int(), Some(255));
}

#[test]
fn test_toggle_bits() {
    // Toggle bits using XOR
    let result = run("0b1010 ^ 0b1111");
    // 10 ^ 15 = 5
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_bitwise_with_variables() {
    let code = r#"
        let x = 5
        let shift = 2
        x << shift
    "#;
    let result = run(code);
    assert_eq!(result.as_int(), Some(20));
}

#[test]
fn test_bitwise_mask_with_variables() {
    let code = r#"
        let value = 0xFF
        let mask = 0x0F
        value & mask
    "#;
    let result = run(code);
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_constant_fold_left_shift() {
    let result = run_with_opt("5 << 2", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(20));
}

#[test]
fn test_constant_fold_right_shift() {
    let result = run_with_opt("20 >> 2", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_constant_fold_and() {
    let result = run_with_opt("12 & 10", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(8));
}

#[test]
fn test_constant_fold_or() {
    let result = run_with_opt("12 | 10", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn test_constant_fold_xor() {
    let result = run_with_opt("12 ^ 10", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn test_constant_fold_not() {
    let result = run_with_opt("~5", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(-6));
}

#[test]
fn test_constant_fold_complex() {
    // Complex expression that should be fully folded
    let result = run_with_opt("(1 << 4) | (1 << 2) | (1 << 0)", OptimizationLevel::Basic);
    // 16 + 4 + 1 = 21
    assert_eq!(result.as_int(), Some(21));
}

#[test]
fn test_constant_fold_nested() {
    let result = run_with_opt("~(~5)", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_optimization_preserves_bitwise_semantics() {
    let code = "5 << 2";

    let result_o0 = run_with_opt(code, OptimizationLevel::None);
    let result_o1 = run_with_opt(code, OptimizationLevel::Basic);
    let result_o2 = run_with_opt(code, OptimizationLevel::Standard);
    let result_o3 = run_with_opt(code, OptimizationLevel::Aggressive);

    assert_eq!(result_o0.as_int(), Some(20));
    assert_eq!(result_o1.as_int(), Some(20));
    assert_eq!(result_o2.as_int(), Some(20));
    assert_eq!(result_o3.as_int(), Some(20));
}

#[test]
fn test_optimization_preserves_complex_bitwise() {
    let code = "(255 & 15) | (240 & 255)";

    let result_o0 = run_with_opt(code, OptimizationLevel::None);
    let result_o2 = run_with_opt(code, OptimizationLevel::Standard);

    // 15 | 240 = 255
    assert_eq!(result_o0.as_int(), Some(255));
    assert_eq!(result_o2.as_int(), Some(255));
}

#[test]
fn test_float_left_shift_fails() {
    assert!(run_fails("1.5 << 2"));
}

#[test]
fn test_float_right_shift_fails() {
    assert!(run_fails("1.5 >> 2"));
}

#[test]
fn test_float_bitwise_and_fails() {
    assert!(run_fails("1.5 & 2"));
}

#[test]
fn test_float_bitwise_or_fails() {
    assert!(run_fails("1.5 | 2"));
}

#[test]
fn test_float_bitwise_xor_fails() {
    assert!(run_fails("1.5 ^ 2"));
}

#[test]
fn test_float_bitwise_not_fails() {
    assert!(run_fails("~1.5"));
}

#[test]
fn test_float_on_right_side_fails() {
    assert!(run_fails("5 << 2.0"));
}

#[test]
fn test_both_floats_fail() {
    assert!(run_fails("1.5 & 2.5"));
}

#[test]
fn test_bitwise_in_function() {
    let code = r#"
        fn set_bit(value: int, bit: int) -> int {
            value | (1 << bit)
        }
        set_bit(0, 3)
    "#;
    let result = run(code);
    assert_eq!(result.as_int(), Some(8)); // 1 << 3 = 8
}

#[test]
fn test_bitwise_in_function_clear_bit() {
    let code = r#"
        fn clear_bit(value: int, bit: int) -> int {
            value & ~(1 << bit)
        }
        clear_bit(15, 1)
    "#;
    let result = run(code);
    // 15 = 1111, clear bit 1 -> 1101 = 13
    assert_eq!(result.as_int(), Some(13));
}

#[test]
fn test_bitwise_in_function_toggle_bit() {
    let code = r#"
        fn toggle_bit(value: int, bit: int) -> int {
            value ^ (1 << bit)
        }
        toggle_bit(10, 2)
    "#;
    let result = run(code);
    // 10 = 1010, toggle bit 2 -> 1110 = 14
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn test_bitwise_in_function_check_bit() {
    let code = r#"
        fn has_bit(value: int, bit: int) -> bool {
            (value & (1 << bit)) != 0
        }
        has_bit(10, 1)
    "#;
    let result = run(code);
    // 10 = 1010, bit 1 is set
    assert_eq!(result.as_bool(), Some(true));
}

// =============================================================================
// Edge Cases - Shift Amount Wrap-Around (branchless behavior with & 63)
// =============================================================================

#[test]
fn test_negative_shift_wraps() {
    // Negative shift amount wraps around (& 63)
    // -1 & 63 = 63 (in two's complement, -1 has all bits set)
    let code = r#"
        let n = -1
        1 << n
    "#;
    let result = run(code);
    // Should be 1 << 63 (wrapped) - this overflows to negative in i64
    assert!(result.as_int().is_some());
}

#[test]
fn test_large_shift_wraps() {
    // Shift by 64 wraps to 0: 64 & 63 = 0
    let code = r#"
        let n = 64
        1 << n
    "#;
    let result = run(code);
    assert_eq!(result.as_int(), Some(1)); // 64 & 63 = 0, so 1 << 0 = 1
}

#[test]
fn test_large_right_shift_wraps() {
    // 66 & 63 = 2, so 8 >> 66 = 8 >> 2 = 2
    let code = r#"
        let n = 66
        8 >> n
    "#;
    let result = run(code);
    assert_eq!(result.as_int(), Some(2)); // 66 & 63 = 2, so 8 >> 2 = 2
}

#[test]
fn test_shift_by_63_succeeds() {
    // Shift by 63 (max before wrap) should succeed
    let code = r#"
        let n = 63
        1 << n
    "#;
    let result = run(code);
    // 1 << 63 = i64::MIN (sign bit set)
    assert!(result.as_int().is_some());
}

#[test]
fn test_shift_by_46_succeeds() {
    // Shift by 46 should succeed and give a positive result
    let code = r#"
        let n = 46
        1 << n
    "#;
    let result = run(code);
    assert_eq!(result.as_int(), Some(1_i64 << 46));
}

// =============================================================================
// Edge Cases - String Type Errors (compile-time)
// =============================================================================

#[test]
fn test_string_bitwise_and_fails() {
    assert!(run_fails("\"hello\" & 5"));
}

#[test]
fn test_string_bitwise_or_fails() {
    assert!(run_fails("\"hello\" | 5"));
}

#[test]
fn test_string_bitwise_xor_fails() {
    assert!(run_fails("\"hello\" ^ 5"));
}

#[test]
fn test_string_left_shift_fails() {
    assert!(run_fails("\"hello\" << 2"));
}

#[test]
fn test_string_right_shift_fails() {
    assert!(run_fails("\"hello\" >> 2"));
}

#[test]
fn test_string_bitwise_not_fails() {
    assert!(run_fails("~\"hello\""));
}

// =============================================================================
// Edge Cases - Boolean Type Errors (compile-time)
// =============================================================================

#[test]
fn test_bool_bitwise_and_fails() {
    assert!(run_fails("true & 5"));
}

#[test]
fn test_bool_bitwise_or_fails() {
    assert!(run_fails("true | 5"));
}

#[test]
fn test_bool_left_shift_fails() {
    assert!(run_fails("true << 2"));
}

#[test]
fn test_bool_bitwise_not_fails() {
    assert!(run_fails("~true"));
}
