use aelys_common::RuntimeErrorKind;
use aelys_runtime::{VM, VmArgsError, VmConfig, VmConfigError, parse_vm_args};
use aelys_syntax::Source;

#[test]
fn parse_vm_args_default() {
    let parsed = parse_vm_args(&[]).expect("should parse defaults");
    assert_eq!(
        parsed.config.max_heap_bytes,
        VmConfig::DEFAULT_MAX_HEAP_BYTES
    );
    assert!(!parsed.config.capabilities.allow_fs);
    assert!(!parsed.config.capabilities.allow_net);
    assert!(!parsed.config.capabilities.allow_exec);
    assert!(parsed.program_args.is_empty());
}

#[test]
fn parse_vm_args_dev_flag_enables_hot_reload() {
    let parsed = parse_vm_args(&["--dev".to_string()]).expect("should parse");
    assert!(parsed.config.allow_hot_reload);
}

#[test]
fn parse_vm_args_allow_deny_caps() {
    let parsed = parse_vm_args(&[
        "--allow-caps=net,gpu".to_string(),
        "--deny-caps=exec".to_string(),
    ])
    .expect("should parse");
    assert!(parsed.config.allowed_caps.contains("net"));
    assert!(parsed.config.allowed_caps.contains("gpu"));
    assert!(parsed.config.denied_caps.contains("exec"));
}

#[test]
fn jvm_style_max_heap_too_small() {
    let err = parse_vm_args(&["-ae.max-heap=4096".to_string()])
        .err()
        .expect("should fail");
    match err {
        VmArgsError::InvalidValue { reason, .. } => {
            assert!(reason.contains("must be >="));
        }
        VmArgsError::InvalidConfig(VmConfigError::MaxHeapTooSmall { .. }) => {}
        _ => panic!("unexpected error: {:?}", err),
    }
}

#[test]
fn gnu_style_max_heap() {
    let parsed = parse_vm_args(&["--ae-max-heap=1G".to_string(), "script.aelys".to_string()])
        .expect("should parse");
    assert_eq!(parsed.config.max_heap_bytes, 1024 * 1024 * 1024);
    assert_eq!(parsed.program_args, vec!["script.aelys".to_string()]);
}

#[test]
fn invalid_vm_arg_value_errors() {
    let err = parse_vm_args(&["-ae.max-heap=not-a-number".to_string()])
        .err()
        .expect("should error");
    match err {
        VmArgsError::InvalidValue { reason, .. } => {
            assert!(reason.contains("invalid integer"));
        }
        _ => panic!("unexpected error"),
    }
}

#[test]
fn parse_vm_args_capabilities_flags() {
    let parsed = parse_vm_args(&[
        "-ae.allow-fs=true".to_string(),
        "-ae.allow-net=false".to_string(),
        "-ae.allow-exec=true".to_string(),
    ])
    .expect("should parse capabilities");
    assert!(parsed.config.capabilities.allow_fs);
    assert!(!parsed.config.capabilities.allow_net);
    assert!(parsed.config.capabilities.allow_exec);
}

#[test]
fn trusted_overrides_capabilities() {
    let parsed = parse_vm_args(&[
        "-ae.trusted=true".to_string(),
        "-ae.allow-fs=false".to_string(),
        "-ae.allow-net=false".to_string(),
        "-ae.allow-exec=false".to_string(),
    ])
    .expect("should parse trusted override");
    assert!(parsed.config.capabilities.allow_fs);
    assert!(parsed.config.capabilities.allow_net);
    assert!(parsed.config.capabilities.allow_exec);
}

#[test]
fn invalid_capability_value_errors() {
    let err = parse_vm_args(&["-ae.allow-fs=maybe".to_string()])
        .err()
        .expect("should error");
    match err {
        VmArgsError::InvalidValue { reason, .. } => {
            assert!(reason.contains("expected true or false"));
        }
        _ => panic!("unexpected error"),
    }
}

#[test]
fn trusted_clears_allowed_caps() {
    // Regression test: trusted should clear allowed_caps, not just denied_caps
    let parsed = parse_vm_args(&[
        "--allow-caps=net".to_string(),
        "-ae.trusted=true".to_string(),
    ])
    .expect("should parse");

    // With trusted=true, allowed_caps should be empty (allowing everything)
    assert!(parsed.config.allowed_caps.is_empty());
    assert!(parsed.config.denied_caps.is_empty());

    // All capabilities should be enabled
    assert!(parsed.config.capabilities.allow_fs);
    assert!(parsed.config.capabilities.allow_net);
    assert!(parsed.config.capabilities.allow_exec);

    // Native caps check should allow any capability
    assert!(parsed.config.check_native_capability("fs").is_ok());
    assert!(parsed.config.check_native_capability("net").is_ok());
    assert!(parsed.config.check_native_capability("gpu").is_ok());
    assert!(parsed.config.check_native_capability("anything").is_ok());
}

#[test]
fn heap_limit_triggers_out_of_memory() {
    let config = VmConfig::new(1024 * 1024).expect("valid config");
    let src = Source::new("<test>", "");
    let mut vm = VM::with_config_and_args(src, config.clone(), Vec::new()).expect("vm init");

    let large = "a".repeat(2 * 1024 * 1024);
    let err = vm.alloc_string(&large).expect_err("should fail");
    match err.kind {
        RuntimeErrorKind::OutOfMemory { .. } => {}
        _ => panic!("expected OutOfMemory"),
    }
}

#[test]
fn manual_heap_respects_limit() {
    let config = VmConfig::new(2 * 1024 * 1024).expect("valid config");
    let src = Source::new("<test>", "");
    let mut vm = VM::with_config_and_args(src, config.clone(), Vec::new()).expect("vm init");

    let slots = (config.max_heap_bytes as usize / std::mem::size_of::<aelys_runtime::Value>()) + 1;
    let err = vm.manual_alloc(slots, 0).expect_err("should OOM");
    match err.kind {
        RuntimeErrorKind::OutOfMemory { .. } => {}
        _ => panic!("expected OutOfMemory"),
    }
}

#[test]
fn merge_heap_rejects_over_limit() {
    let config = VmConfig::new(2 * 1024 * 1024).expect("valid config");
    let src = Source::new("<test>", "");
    let mut vm = VM::with_config_and_args(src, config.clone(), Vec::new()).expect("vm init");

    let mut compile_heap = aelys_runtime::Heap::new();
    let large = "x".repeat(2 * 1024 * 1024);
    compile_heap.alloc_string(&large);

    let err = vm.merge_heap(&mut compile_heap).expect_err("should OOM");
    match err.kind {
        RuntimeErrorKind::OutOfMemory { .. } => {}
        _ => panic!("expected OutOfMemory"),
    }
}
