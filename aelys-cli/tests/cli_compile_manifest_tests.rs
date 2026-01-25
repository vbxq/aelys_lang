use aelys_cli::cli::commands::compile::compile_to_avbc;
use aelys_opt::OptimizationLevel;

#[test]
fn compile_includes_manifest_when_present() {
    let dir = std::env::temp_dir().join("aelys_cli_manifest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let src_path = dir.join("main.aelys");
    std::fs::write(&src_path, "let x = 1\n").unwrap();

    let manifest_path = dir.join("main.aelys.toml");
    std::fs::write(&manifest_path, "[build]\nbundle_native_modules = true\n").unwrap();

    let output = compile_to_avbc(&src_path, OptimizationLevel::None).unwrap();
    let bytes = std::fs::read(output).unwrap();

    let (_func, _heap, manifest_bytes, _bundles) =
        aelys_bytecode::asm::deserialize_with_manifest(&bytes).unwrap();

    assert!(manifest_bytes.is_some());
}
