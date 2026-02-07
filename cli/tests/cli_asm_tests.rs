use aelys_cli::cli::commands::asm::asm_transform;

#[test]
fn asm_emits_aasm_from_source() {
    let dir = std::env::temp_dir().join("aelys_cli_asm_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("sample.aelys");
    std::fs::write(&src_path, "let x = 1\n").unwrap();

    let output = asm_transform(&src_path).unwrap();

    assert!(output.exists());
    assert_eq!(output.extension().unwrap(), "aasm");
}
