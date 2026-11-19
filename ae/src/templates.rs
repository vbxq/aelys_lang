pub const BIN_MAIN: &str = "fn main() -> i64 {\n    println(\"Hello, world!\")\n    return 0\n}\n";

pub const GITIGNORE: &str = "/target\n";

pub fn manifest(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n")
}
