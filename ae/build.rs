fn main() {
    let dir = std::env::var("DEP_AELYS_CORE_ROOT").expect("aelys-core exports its out dir");
    println!("cargo:rustc-env=AE_CORE_LIB_DIR={dir}");
}
