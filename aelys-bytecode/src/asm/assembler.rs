//! Assembler: Parses .aasm text and produces bytecode

use super::lexer::{Lexer, Token};
use crate::bytecode::{Function, GlobalLayout, UpvalueDescriptor};
use crate::heap::Heap;
use crate::value::Value;
use std::collections::HashMap;
use thiserror::Error;

/// Assembler error types
#[derive(Debug, Error)]
pub enum AssemblerError {
    #[error("Parse error at line {line}: {message}")]
    ParseError { line: usize, message: String },

    #[error("Unknown opcode: {0}")]
    UnknownOpcode(String),

    #[error("Undefined label: {0}")]
    UndefinedLabel(String),

    #[error("Duplicate label: {0}")]
    DuplicateLabel(String),

    #[error("Invalid register: {0}")]
    InvalidRegister(String),

    #[error("Invalid number: {0}")]
    InvalidNumber(String),

    #[error("Invalid string literal: {0}")]
    InvalidString(String),

    #[error("Expected {expected}, got {got}")]
    Expected { expected: String, got: String },

    #[error("Unexpected end of input")]
    UnexpectedEof,
}

/// Result type for assembler operations
pub type Result<T> = std::result::Result<T, AssemblerError>;

/// Assemble .aasm source into bytecode functions
pub fn assemble(source: &str) -> Result<(Vec<Function>, Heap)> {
    let mut parser = AasmParser::new(source);
    parser.parse()
}

/// Convenience function that takes a string
pub fn assemble_from_string(source: &str) -> Result<(Vec<Function>, Heap)> {
    assemble(source)
}

/// Parser for .aasm files
pub(super) struct AasmParser<'a> {
    pub(super) lexer: Lexer<'a>,
    pub(super) current: Token,
    heap: Heap,
}

impl<'a> AasmParser<'a> {
    fn new(source: &'a str) -> Self {
        let mut lexer = Lexer::new(source);
        let current = lexer.next_token().unwrap_or(Token::Eof);
        Self {
            lexer,
            current,
            heap: Heap::new(),
        }
    }

    pub(super) fn advance(&mut self) -> Result<Token> {
        let prev = std::mem::replace(&mut self.current, self.lexer.next_token()?);
        Ok(prev)
    }

    fn skip_newlines(&mut self) -> Result<()> {
        while self.current == Token::Newline {
            self.advance()?;
        }
        Ok(())
    }

    fn expect(&mut self, expected: Token) -> Result<()> {
        if std::mem::discriminant(&self.current) == std::mem::discriminant(&expected) {
            self.advance()?;
            Ok(())
        } else {
            Err(AssemblerError::Expected {
                expected: format!("{:?}", expected),
                got: format!("{:?}", self.current),
            })
        }
    }

    fn parse(&mut self) -> Result<(Vec<Function>, Heap)> {
        let mut functions = Vec::new();

        self.skip_newlines()?;

        if let Token::Directive(ref d) = self.current
            && d == "version"
        {
            self.advance()?;
            if let Token::Int(_) = self.current {
                self.advance()?;
            }
            self.skip_newlines()?;
        }

        while self.current != Token::Eof {
            self.skip_newlines()?;
            if self.current == Token::Eof {
                break;
            }

            if let Token::Directive(ref d) = self.current.clone() {
                if d == "function" {
                    let func = self.parse_function()?;
                    functions.push(func);
                } else {
                    return Err(AssemblerError::ParseError {
                        line: self.lexer.current_line(),
                        message: format!("Expected .function, got .{}", d),
                    });
                }
            } else {
                self.skip_newlines()?;
                if self.current == Token::Eof {
                    break;
                }
                if let Token::Directive(_) = self.current {
                    continue;
                }
                return Err(AssemblerError::ParseError {
                    line: self.lexer.current_line(),
                    message: format!("Expected directive, got {:?}", self.current),
                });
            }
        }

        let heap = std::mem::take(&mut self.heap);
        Ok((functions, heap))
    }

    fn parse_function(&mut self) -> Result<Function> {
        // .function N
        self.expect(Token::Directive("function".to_string()))?;
        let _func_idx = match self.advance()? {
            Token::Int(n) => n as usize,
            t => {
                return Err(AssemblerError::Expected {
                    expected: "function index".to_string(),
                    got: format!("{:?}", t),
                });
            }
        };
        self.skip_newlines()?;

        let mut name = None;
        let mut arity = 0u8;
        let mut num_registers = 0u8;
        let mut constants = Vec::new();
        let mut bytecode = Vec::new();
        let mut global_names = Vec::new();
        let mut upvalue_descriptors = Vec::new();
        let mut labels: HashMap<String, usize> = HashMap::new();
        let mut label_refs: Vec<(usize, String, bool)> = Vec::new(); // (offset, label, is_conditional)

        loop {
            match &self.current {
                Token::Directive(d) => match d.as_str() {
                    "name" => {
                        self.advance()?;
                        if let Token::String(s) = self.advance()? {
                            name = Some(s);
                        }
                    }
                    "arity" => {
                        self.advance()?;
                        if let Token::Int(n) = self.advance()? {
                            arity = n as u8;
                        }
                    }
                    "registers" => {
                        self.advance()?;
                        if let Token::Int(n) = self.advance()? {
                            num_registers = n as u8;
                        }
                    }
                    "globals" => {
                        self.advance()?;
                        self.skip_newlines()?;
                        global_names = self.parse_globals()?;
                    }
                    "constants" => {
                        self.advance()?;
                        self.skip_newlines()?;
                        constants = self.parse_constants()?;
                    }
                    "upvalues" => {
                        self.advance()?;
                        self.skip_newlines()?;
                        upvalue_descriptors = self.parse_upvalues()?;
                    }
                    "code" => {
                        self.advance()?;
                        self.skip_newlines()?;
                        self.parse_code(&mut bytecode, &mut labels, &mut label_refs)?;
                        break;
                    }
                    "function" => break,
                    _ => {
                        self.advance()?;
                    }
                },
                Token::Newline => {
                    self.advance()?;
                }
                Token::Eof => break,
                _ => break,
            }
        }

        /*
         * This is the label patching phase.
         * For each label reference, we look up the target label's offset,
         * calculate the relative jump distance, and patch the instruction.
         */
        for (offset, label, is_conditional) in label_refs {
            let target = labels
                .get(&label)
                .ok_or_else(|| AssemblerError::UndefinedLabel(label.clone()))?;

            // Calculate relative offset: target - (offset + 1)
            let relative = (*target as i32) - (offset as i32) - 1;
            let relative = relative as i16;

            // Patch the instruction
            let instr = bytecode[offset];
            let patched = if is_conditional {
                // Keep the register in A field
                let a = (instr >> 16) & 0xFF;
                let op = instr >> 24;
                (op << 24) | (a << 16) | ((relative as u16) as u32)
            } else {
                // Jump has no register
                let op = instr >> 24;
                (op << 24) | ((relative as u16) as u32)
            };
            bytecode[offset] = patched;
        }

        let mut func = Function::new(name, arity);
        func.num_registers = num_registers;
        func.set_bytecode(bytecode);
        func.constants = constants;
        func.global_layout = GlobalLayout::new(global_names);
        func.upvalue_descriptors = upvalue_descriptors;
        func.compute_global_layout_hash();

        Ok(func)
    }

    fn parse_constants(&mut self) -> Result<Vec<Value>> {
        let mut constants = Vec::new();

        loop {
            match &self.current {
                Token::Directive(_) => break,
                Token::Eof => break,
                Token::Newline => {
                    self.advance()?;
                    continue;
                }
                Token::Int(idx) => {
                    // Parse constant: INDEX: TYPE VALUE
                    let _idx = *idx;
                    self.advance()?;
                    self.expect(Token::Colon)?;

                    let value = self.parse_constant_value()?;
                    constants.push(value);
                    self.skip_newlines()?;
                }
                _ => break,
            }
        }

        Ok(constants)
    }

    /// Parse global names section: INDEX: "name"
    fn parse_globals(&mut self) -> Result<Vec<String>> {
        let mut globals = Vec::new();

        loop {
            match &self.current {
                Token::Directive(_) => break,
                Token::Eof => break,
                Token::Newline => {
                    self.advance()?;
                    continue;
                }
                Token::Int(idx) => {
                    // Parse global: INDEX: "name"
                    let idx = *idx as usize;
                    self.advance()?;
                    self.expect(Token::Colon)?;

                    if let Token::String(name) = self.advance()? {
                        // Ensure the vec is large enough
                        if globals.len() <= idx {
                            globals.resize(idx + 1, String::new());
                        }
                        globals[idx] = name;
                    }
                    self.skip_newlines()?;
                }
                _ => break,
            }
        }

        Ok(globals)
    }

    /// Parse upvalue descriptors section: INDEX: (local|upvalue) INDEX
    fn parse_upvalues(&mut self) -> Result<Vec<UpvalueDescriptor>> {
        let mut upvalues = Vec::new();

        loop {
            match &self.current {
                Token::Directive(_) => break,
                Token::Eof => break,
                Token::Newline => {
                    self.advance()?;
                    continue;
                }
                Token::Int(idx) => {
                    // Parse upvalue: INDEX: (local|upvalue) INDEX
                    let idx = *idx as usize;
                    self.advance()?;
                    self.expect(Token::Colon)?;

                    // Parse kind: "local" or "upvalue"
                    let is_local = match &self.current {
                        Token::Ident(s) if s == "local" => {
                            self.advance()?;
                            true
                        }
                        Token::Ident(s) if s == "upvalue" => {
                            self.advance()?;
                            false
                        }
                        _ => {
                            return Err(AssemblerError::Expected {
                                expected: "local or upvalue".to_string(),
                                got: format!("{:?}", self.current),
                            });
                        }
                    };

                    // Parse the index
                    let index = self.parse_u8()?;

                    // Ensure the vec is large enough
                    if upvalues.len() <= idx {
                        upvalues.resize(
                            idx + 1,
                            UpvalueDescriptor {
                                is_local: true,
                                index: 0,
                            },
                        );
                    }
                    upvalues[idx] = UpvalueDescriptor { is_local, index };
                    self.skip_newlines()?;
                }
                _ => break,
            }
        }

        Ok(upvalues)
    }

    /// Parse a constant value of the form TYPE VALUE
    fn parse_constant_value(&mut self) -> Result<Value> {
        match self.advance()? {
            Token::Ident(type_name) => {
                match type_name.as_str() {
                    "int" => {
                        if let Token::Int(n) = self.advance()? {
                            Ok(Value::int(n))
                        } else {
                            Err(AssemblerError::Expected {
                                expected: "integer".to_string(),
                                got: format!("{:?}", self.current),
                            })
                        }
                    }
                    "float" => match self.advance()? {
                        Token::Float(f) => Ok(Value::float(f)),
                        Token::Int(n) => Ok(Value::float(n as f64)),
                        Token::Ident(s) if s == "nan" => Ok(Value::float(f64::NAN)),
                        Token::Ident(s) if s == "inf" => Ok(Value::float(f64::INFINITY)),
                        t => Err(AssemblerError::Expected {
                            expected: "float".to_string(),
                            got: format!("{:?}", t),
                        }),
                    },
                    "bool" => match self.advance()? {
                        Token::Bool(b) => Ok(Value::bool(b)),
                        Token::Ident(s) if s == "true" => Ok(Value::bool(true)),
                        Token::Ident(s) if s == "false" => Ok(Value::bool(false)),
                        t => Err(AssemblerError::Expected {
                            expected: "bool".to_string(),
                            got: format!("{:?}", t),
                        }),
                    },
                    "string" => {
                        if let Token::String(s) = self.advance()? {
                            let str_ref = self.heap.intern_string(&s);
                            Ok(Value::ptr(str_ref.index()))
                        } else {
                            Err(AssemblerError::Expected {
                                expected: "string".to_string(),
                                got: format!("{:?}", self.current),
                            })
                        }
                    }
                    "ptr" => {
                        if let Token::Int(n) = self.advance()? {
                            Ok(Value::ptr(n as usize))
                        } else {
                            Err(AssemblerError::Expected {
                                expected: "pointer value".to_string(),
                                got: format!("{:?}", self.current),
                            })
                        }
                    }
                    "func" => {
                        // func @N
                        self.expect(Token::At)?;
                        if let Token::Int(n) = self.advance()? {
                            // Encode as nested function marker (uses dedicated tag)
                            Ok(Value::nested_fn_marker((n - 1) as usize)) // -1 because main is @0
                        } else {
                            Err(AssemblerError::Expected {
                                expected: "function index".to_string(),
                                got: format!("{:?}", self.current),
                            })
                        }
                    }
                    "null" => Ok(Value::null()),
                    "native" => {
                        if let Token::String(_) = self.advance()? {
                            // We can't recreate native functions, return null
                            Ok(Value::null())
                        } else {
                            Ok(Value::null())
                        }
                    }
                    _ => Err(AssemblerError::Expected {
                        expected: "constant type".to_string(),
                        got: type_name,
                    }),
                }
            }
            Token::Null => Ok(Value::null()),
            t => Err(AssemblerError::Expected {
                expected: "constant type".to_string(),
                got: format!("{:?}", t),
            }),
        }
    }

    fn parse_code(
        &mut self,
        bytecode: &mut Vec<u32>,
        labels: &mut HashMap<String, usize>,
        label_refs: &mut Vec<(usize, String, bool)>,
    ) -> Result<()> {
        loop {
            match &self.current {
                Token::Directive(_) => break,
                Token::Eof => break,
                Token::Newline => {
                    self.advance()?;
                    continue;
                }
                Token::LabelRef(name) => {
                    // This is a label definition (L0:)
                    let label_name = name.clone();
                    self.advance()?;
                    if self.current == Token::Colon {
                        self.advance()?;
                        if labels.contains_key(&label_name) {
                            return Err(AssemblerError::DuplicateLabel(label_name));
                        }
                        labels.insert(label_name, bytecode.len());
                    } else {
                        return Err(AssemblerError::Expected {
                            expected: "colon after label".to_string(),
                            got: format!("{:?}", self.current),
                        });
                    }
                }
                Token::Int(_) => {
                    // Skip instruction offset
                    self.advance()?;
                    self.expect(Token::Colon)?;
                    self.parse_instruction(bytecode, label_refs)?;
                }
                Token::Ident(_) => {
                    // Instruction without offset
                    self.parse_instruction(bytecode, label_refs)?;
                }
                _ => {
                    return Err(AssemblerError::ParseError {
                        line: self.lexer.current_line(),
                        message: format!("Unexpected token in code: {:?}", self.current),
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn parse_register(&mut self) -> Result<u8> {
        match self.advance()? {
            Token::Register(r) => Ok(r),
            t => Err(AssemblerError::Expected {
                expected: "register".to_string(),
                got: format!("{:?}", t),
            }),
        }
    }

    pub(super) fn parse_u8(&mut self) -> Result<u8> {
        match self.advance()? {
            Token::Int(n) if (0..=255).contains(&n) => Ok(n as u8),
            Token::Int(n) => Err(AssemblerError::InvalidNumber(format!(
                "{} (must be 0-255)",
                n
            ))),
            t => Err(AssemblerError::Expected {
                expected: "u8".to_string(),
                got: format!("{:?}", t),
            }),
        }
    }

    pub(super) fn parse_i16(&mut self) -> Result<i16> {
        match self.advance()? {
            Token::Int(n) if n >= i16::MIN as i64 && n <= i16::MAX as i64 => Ok(n as i16),
            Token::Int(n) => Err(AssemblerError::InvalidNumber(format!(
                "{} (must fit i16)",
                n
            ))),
            t => Err(AssemblerError::Expected {
                expected: "i16".to_string(),
                got: format!("{:?}", t),
            }),
        }
    }

    pub(super) fn skip_comma(&mut self) -> Result<()> {
        if self.current == Token::Comma {
            self.advance()?;
        }
        Ok(())
    }
}
