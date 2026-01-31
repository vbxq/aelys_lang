use super::stack::StackFrame;
use aelys_syntax::Source;
use std::fmt;
use std::sync::Arc;

#[derive(Debug)]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    pub stack_trace: Vec<StackFrame>,
    pub source: Arc<Source>,
}

#[derive(Debug)]
pub enum RuntimeErrorKind {
    TypeError {
        operation: &'static str,
        expected: &'static str,
        got: String,
    },
    DivisionByZero,
    UndefinedVariable(String),
    NotCallable(String),
    ArityMismatch {
        expected: u8,
        got: u8,
    },
    StackOverflow,
    InvalidAllocationSize {
        size: i64,
    },
    OutOfMemory {
        requested: u64,
        max: u64,
    },
    InvalidMemoryHandle,
    DoubleFree,
    UseAfterFree,
    MemoryOutOfBounds {
        offset: usize,
        size: usize,
    },
    NegativeMemoryIndex {
        value: i64,
    },
    InvalidConstantIndex {
        index: usize,
        max: usize,
    },
    InvalidOpcode {
        opcode: u8,
    },
    InvalidRegister {
        reg: usize,
        max: usize,
    },
    InvalidBytecode(String),
    CapabilityDenied {
        operation: &'static str,
    },
    NativeError {
        code: i32,
    },
    IndexOutOfBounds {
        index: i64,
        length: i64,
    },
}

impl RuntimeError {
    pub fn new(kind: RuntimeErrorKind, stack_trace: Vec<StackFrame>, source: Arc<Source>) -> Self {
        Self { kind, stack_trace, source }
    }
}

impl RuntimeErrorKind {
    pub fn message(&self) -> String {
        match self {
            Self::TypeError {
                operation,
                expected,
                got,
            } => {
                format!(
                    "type error in '{}': expected {}, got {}",
                    operation, expected, got
                )
            }
            Self::DivisionByZero => "division by zero".to_string(),
            Self::UndefinedVariable(name) => format!("undefined variable '{}'", name),
            Self::NotCallable(ty) => format!("'{}' is not callable", ty),
            Self::ArityMismatch { expected, got } => {
                format!("expected {} arguments, got {}", expected, got)
            }
            Self::StackOverflow => "stack overflow".to_string(),
            Self::InvalidAllocationSize { size } => {
                format!("invalid allocation size: {} (must be > 0)", size)
            }
            Self::OutOfMemory { requested, max } => {
                format!(
                    "out of memory: requested {} bytes (max {} bytes)",
                    requested, max
                )
            }
            Self::InvalidMemoryHandle => "invalid memory handle".to_string(),
            Self::DoubleFree => "double free: pointer was already freed".to_string(),
            Self::UseAfterFree => "use after free: pointer was already freed".to_string(),
            Self::MemoryOutOfBounds { offset, size } => format!(
                "memory access out of bounds: offset {} exceeds size {}",
                offset, size
            ),
            Self::NegativeMemoryIndex { value } => {
                format!("negative memory index: {}", value)
            }
            Self::InvalidConstantIndex { index, max } => {
                format!("invalid constant index: {} (max: {})", index, max)
            }
            Self::InvalidOpcode { opcode } => format!("invalid opcode: {}", opcode),
            Self::InvalidRegister { reg, max } => {
                format!("invalid register index: {} (max: {})", reg, max)
            }
            Self::InvalidBytecode(message) => format!("invalid bytecode: {}", message),
            Self::CapabilityDenied { operation } => format!("capability denied: {}", operation),
            Self::NativeError { code } => format!("native error: code {}", code),
            Self::IndexOutOfBounds { index, length } => {
                format!("index out of bounds: index {} is out of bounds for length {}", index, length)
            }
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "error: {}", self.kind.message())?;

        if let Some(frame) = self.stack_trace.first() {
            writeln!(
                f,
                "  --> {}:{}:{}",
                self.source.name, frame.line, frame.column
            )?;

            let line_content = self.source.get_line(frame.line);
            let line_num_width = frame.line.to_string().len().max(2);

            writeln!(f, "{:width$} |", "", width = line_num_width)?;
            writeln!(
                f,
                "{:>width$} | {}",
                frame.line,
                line_content,
                width = line_num_width
            )?;
        }

        if !self.stack_trace.is_empty() {
            const MAX_FRAMES: usize = 50;
            writeln!(f)?;
            writeln!(f, "stack trace (most recent call first):")?;
            let display_count = self.stack_trace.len().min(MAX_FRAMES);
            for frame in &self.stack_trace[..display_count] {
                let name = frame.function_name.as_deref().unwrap_or("<script>");
                writeln!(f, "  {} ({}:{})", name, self.source.name, frame.line)?;
            }
            if self.stack_trace.len() > MAX_FRAMES {
                writeln!(
                    f,
                    "  ... {} more frames",
                    self.stack_trace.len() - MAX_FRAMES
                )?;
            }
        }

        Ok(())
    }
}
