use aelys_runtime::Value;

#[test]
fn facade_smoke_compiles_and_runs() {
    let result = aelys::api::run("1 + 2", "<smoke>").expect("run should succeed");
    assert_eq!(result.to_string(), "3");

    let mut vm = aelys::api::new_vm().expect("vm");
    aelys::api::run_with_vm(&mut vm, "fn f(x: int) -> int { x + 1 }", "<def>").unwrap();
    let out = aelys::api::call_function(&mut vm, "f", &[Value::int(41)]).unwrap();
    assert_eq!(out.to_string(), "42");
}
