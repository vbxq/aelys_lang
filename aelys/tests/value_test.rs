use aelys_runtime::Value;

#[test]
fn test_int_roundtrip() {
    for n in [
        0i64,
        1,
        -1,
        42,
        -42,
        1000000,
        -1000000,
        i64::MAX >> 16,
        i64::MIN >> 16,
    ] {
        let v = Value::int(n);
        assert!(v.is_int(), "Expected int for {}", n);
        assert_eq!(v.as_int(), Some(n), "Roundtrip failed for {}", n);
    }
}

#[test]
fn test_float_roundtrip() {
    for n in [
        0.0f64,
        1.0,
        -1.0,
        3.14159,
        f64::MAX,
        f64::MIN,
        f64::INFINITY,
    ] {
        let v = Value::float(n);
        assert!(v.is_float(), "Expected float for {}", n);
        assert_eq!(v.as_float(), Some(n), "Roundtrip failed for {}", n);
    }
}

#[test]
fn test_bool_roundtrip() {
    let t = Value::bool(true);
    let f = Value::bool(false);

    assert!(t.is_bool());
    assert!(f.is_bool());
    assert_eq!(t.as_bool(), Some(true));
    assert_eq!(f.as_bool(), Some(false));
}

#[test]
fn test_null() {
    let n = Value::null();
    assert!(n.is_null());
    assert!(!n.is_int());
    assert!(!n.is_float());
    assert!(!n.is_bool());
}

#[test]
fn test_type_discrimination() {
    let int = Value::int(42);
    let float = Value::float(3.14);
    let boolean = Value::bool(true);
    let null = Value::null();

    assert!(int.is_int() && !int.is_float() && !int.is_bool() && !int.is_null());
    assert!(!float.is_int() && float.is_float() && !float.is_bool() && !float.is_null());
    assert!(!boolean.is_int() && !boolean.is_float() && boolean.is_bool() && !boolean.is_null());
    assert!(!null.is_int() && !null.is_float() && !null.is_bool() && null.is_null());
}

#[test]
fn test_is_truthy() {
    assert!(Value::bool(true).is_truthy());
    assert!(!Value::bool(false).is_truthy());
    assert!(!Value::null().is_truthy());
    assert!(Value::int(1).is_truthy());
    assert!(!Value::int(0).is_truthy());
    assert!(Value::float(1.0).is_truthy());
    assert!(!Value::float(0.0).is_truthy());
}
