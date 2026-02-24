fn main() {
    cc::Build::new()
        .file("src/aelys_core.c")
        .compile("aelys-core");
}
