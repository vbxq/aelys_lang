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
