#[derive(Debug, Clone)]
pub struct StackFrame {
    pub function_name: Option<String>,
    pub line: u32,
    pub column: u32,
}
