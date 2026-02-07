use aelys_modules::manifest::Manifest;

#[test]
fn parse_manifest_modules_and_build_flags() {
    let raw = r#"
        [module.opengl]
        capabilities = ["gpu", "window"]
        required_version = ">=0.2.0"

        [build]
        bundle_native_modules = true
    "#;

    let manifest = Manifest::parse(raw).expect("parse");
    let opengl = manifest.module("opengl").expect("module");
    assert!(opengl.capabilities.contains(&"gpu".to_string()));
    assert_eq!(manifest.build.bundle_native_modules, Some(true));
}
