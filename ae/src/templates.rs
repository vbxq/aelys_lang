pub const BIN_MAIN: &str = "fn main() {\n    println!(\"Hello, world!\");\n}\n";

pub const GITIGNORE: &str = "/target\n";

pub fn manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n")
}
