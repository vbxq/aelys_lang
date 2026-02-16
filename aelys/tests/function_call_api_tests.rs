use aelys::{call_function, get_function, new_vm, run_with_vm};
use aelys_runtime::Value;

#[test]
fn test_call_function_simple() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn add(a, b) { a + b }", "def").unwrap();

    let result = call_function(&mut vm, "add", &[Value::int(10), Value::int(32)]).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_call_function_no_args() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn get_answer() { 42 }", "def").unwrap();

    let result = call_function(&mut vm, "get_answer", &[]).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_call_function_float() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn mul(a, b) { a * b }", "def").unwrap();

    let result = call_function(&mut vm, "mul", &[Value::float(2.5), Value::float(4.0)]).unwrap();
    assert_eq!(result.as_float(), Some(10.0));
}

#[test]
fn test_call_function_not_found() {
    let mut vm = new_vm().unwrap();

    let err = call_function(&mut vm, "nonexistent", &[]);
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("nonexistent") || msg.contains("not found"));
}

#[test]
fn test_call_function_arity_mismatch() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn need_two(a, b) { a + b }", "def").unwrap();

    let err = call_function(&mut vm, "need_two", &[Value::int(1)]);
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("expected 2") || msg.contains("arity"));
}

#[test]
fn test_get_function_basic() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn double(x) { x * 2 }", "def").unwrap();

    let double = get_function(&vm, "double").unwrap();
    let result = double.call(&mut vm, &[Value::int(21)]).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_get_function_repeated_calls() {
    let mut vm = new_vm().unwrap();
    run_with_vm(
        &mut vm,
        "fn lcg(state) { (state * 1103515245 + 12345) & 0x7FFFFFFF }",
        "def",
    )
    .unwrap();

    let rng = get_function(&vm, "lcg").unwrap();

    let mut state = Value::int(42);
    for _ in 0..1000 {
        state = rng.call(&mut vm, &[state]).unwrap();
    }

    assert!(state.as_int().is_some());
    assert!(state.as_int().unwrap() > 0);
}

#[test]
fn test_get_function_not_found() {
    let vm = new_vm().unwrap();

    let err = get_function(&vm, "nonexistent");
    assert!(err.is_err());
}

#[test]
fn test_call_function_with_globals() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "let mut counter = 0", "init").unwrap();
    run_with_vm(&mut vm, "fn increment(n) { counter += n; counter }", "def").unwrap();

    let result1 = call_function(&mut vm, "increment", &[Value::int(5)]).unwrap();
    assert_eq!(result1.as_int(), Some(5));

    let result2 = call_function(&mut vm, "increment", &[Value::int(3)]).unwrap();
    assert_eq!(result2.as_int(), Some(8));
}

#[test]
fn test_call_function_recursive() {
    let mut vm = new_vm().unwrap();
    run_with_vm(
        &mut vm,
        r#"
fn factorial(n) {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}
"#,
        "def",
    )
    .unwrap();

    let result = call_function(&mut vm, "factorial", &[Value::int(10)]).unwrap();
    assert_eq!(result.as_int(), Some(3628800));
}

#[test]
fn test_call_closure() {
    let mut vm = new_vm().unwrap();
    run_with_vm(
        &mut vm,
        r#"
fn make_adder(n) {
    fn adder(x) { x + n }
    adder
}
let add_10 = make_adder(10)
"#,
        "def",
    )
    .unwrap();

    let result = call_function(&mut vm, "add_10", &[Value::int(32)]).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_callable_function_copy() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn id(x) { x }", "def").unwrap();

    let f1 = get_function(&vm, "id").unwrap();
    let f2 = f1.clone();

    let r1 = f1.call(&mut vm, &[Value::int(1)]).unwrap();
    let r2 = f2.call(&mut vm, &[Value::int(2)]).unwrap();

    assert_eq!(r1.as_int(), Some(1));
    assert_eq!(r2.as_int(), Some(2));
}

#[test]
fn test_call_function_returns_null() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn nothing() { }", "def").unwrap();

    let result = call_function(&mut vm, "nothing", &[]).unwrap();
    assert!(result.is_null());
}

#[test]
fn test_call_function_bool_return() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn is_positive(x) { x > 0 }", "def").unwrap();

    let pos = call_function(&mut vm, "is_positive", &[Value::int(5)]).unwrap();
    let neg = call_function(&mut vm, "is_positive", &[Value::int(-5)]).unwrap();

    assert_eq!(pos.as_bool(), Some(true));
    assert_eq!(neg.as_bool(), Some(false));
}

#[test]
fn test_call_function_mixed_types() {
    let mut vm = new_vm().unwrap();
    run_with_vm(
        &mut vm,
        r#"
fn check(flag, x) {
    if flag { x * 2 } else { x + 1 }
}
"#,
        "def",
    )
    .unwrap();

    let r1 = call_function(&mut vm, "check", &[Value::bool(true), Value::int(10)]).unwrap();
    let r2 = call_function(&mut vm, "check", &[Value::bool(false), Value::int(10)]).unwrap();

    assert_eq!(r1.as_int(), Some(20));
    assert_eq!(r2.as_int(), Some(11));
}

#[test]
fn test_callable_function_introspection() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn id(x) { x }", "def").unwrap();

    let f = get_function(&vm, "id").unwrap();

    assert_eq!(f.arity(), 1);
    assert!(!f.is_native());
    assert!(!f.is_closure());
}

#[test]
fn test_performance_call_function_vs_run() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn inc(x) { x + 1 }", "def").unwrap();

    let inc = get_function(&vm, "inc").unwrap();

    let mut sum = 0i64;
    for i in 0..10000 {
        let result = inc.call(&mut vm, &[Value::int(i)]).unwrap();
        sum += result.as_int().unwrap();
    }

    // sum of (i+1) for i in 0..10000 = sum of 1..10001 = 10000*10001/2 = 50005000
    assert_eq!(sum, 50005000);
}

#[test]
fn test_cached_vs_uncached_performance() {
    let mut vm = new_vm().unwrap();
    run_with_vm(&mut vm, "fn double(x) { x * 2 }", "def").unwrap();

    // Uncached: HashMap lookup each time
    let start = std::time::Instant::now();
    for i in 0..100_000 {
        let _ = call_function(&mut vm, "double", &[Value::int(i)]).unwrap();
    }
    let uncached_time = start.elapsed();

    // Cached: No lookups after get_function
    let double = get_function(&vm, "double").unwrap();
    let start = std::time::Instant::now();
    for i in 0..100_000 {
        let _ = double.call(&mut vm, &[Value::int(i)]).unwrap();
    }
    let cached_time = start.elapsed();

    // Cached should be faster (or at least not slower)
    // We don't assert a specific ratio since it depends on the machine,
    // but print for manual inspection
    eprintln!(
        "100k calls: uncached={:?}, cached={:?}, speedup={:.2}x",
        uncached_time,
        cached_time,
        uncached_time.as_nanos() as f64 / cached_time.as_nanos() as f64
    );
}
