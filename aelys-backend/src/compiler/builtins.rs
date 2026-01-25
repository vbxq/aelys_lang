use super::Compiler;

impl Compiler {
    // VM intrinsics
    pub const BUILTINS: &'static [&'static str] = &["alloc", "free", "load", "store", "type"];
    pub fn is_builtin(name: &str) -> bool { Self::BUILTINS.contains(&name) }
}
