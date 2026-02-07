use aelys_cli::cli::vm_config::parse_vm_args_or_error;

#[test]
fn vm_args_parse_trusted_and_caps() {
    let args = vec![
        "-ae.trusted=true".to_string(),
        "--allow-caps=fs,net".to_string(),
    ];

    let parsed = parse_vm_args_or_error(&args).unwrap();

    assert!(parsed.config.capabilities.allow_fs);
    assert!(parsed.config.capabilities.allow_net);
    assert!(parsed.config.capabilities.allow_exec);
}

#[test]
fn vm_args_allow_caps_enables_capabilities() {
    let args = vec!["--allow-caps=fs".to_string()];
    let parsed = parse_vm_args_or_error(&args).unwrap();

    assert!(parsed.config.capabilities.allow_fs);
    assert!(!parsed.config.capabilities.allow_net);
    assert!(!parsed.config.capabilities.allow_exec);
    assert!(parsed.config.allowed_caps.contains("fs"));
}

#[test]
fn vm_args_allow_multiple_caps() {
    let args = vec!["--allow-caps=fs,net,exec".to_string()];
    let parsed = parse_vm_args_or_error(&args).unwrap();

    assert!(parsed.config.capabilities.allow_fs);
    assert!(parsed.config.capabilities.allow_net);
    assert!(parsed.config.capabilities.allow_exec);
}

#[test]
fn vm_args_deny_caps_disables_capabilities() {
    let args = vec![
        "--allow-caps=fs,net,exec".to_string(),
        "--deny-caps=fs".to_string(),
    ];
    let parsed = parse_vm_args_or_error(&args).unwrap();

    // deny overrides allow
    assert!(!parsed.config.capabilities.allow_fs);
    assert!(parsed.config.capabilities.allow_net);
    assert!(parsed.config.capabilities.allow_exec);
}
