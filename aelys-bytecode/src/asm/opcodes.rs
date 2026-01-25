//! Opcode parsing for the assembler

use super::assembler::{AasmParser, AssemblerError, Result};
use super::lexer::Token;
use crate::bytecode::OpCode;

impl<'a> AasmParser<'a> {
    pub(super) fn parse_instruction(
        &mut self,
        bytecode: &mut Vec<u32>,
        label_refs: &mut Vec<(usize, String, bool)>,
    ) -> Result<()> {
        let opcode_name = match self.advance()? {
            Token::Ident(name) => name,
            t => {
                return Err(AssemblerError::Expected {
                    expected: "opcode".to_string(),
                    got: format!("{:?}", t),
                });
            }
        };

        let mut extra_cache_words = 0usize;
        let instr = match opcode_name.as_str() {
            "Move" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                encode_a(OpCode::Move, a, b, 0)
            }
            "LoadI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let imm = self.parse_i16()?;
                encode_b(OpCode::LoadI, a, imm)
            }
            "LoadK" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let k = self.parse_i16()?;
                encode_b(OpCode::LoadK, a, k)
            }
            "LoadNull" => {
                let a = self.parse_register()?;
                encode_a(OpCode::LoadNull, a, 0, 0)
            }
            "LoadBool" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = match self.advance()? {
                    Token::Bool(b) => {
                        if b {
                            1
                        } else {
                            0
                        }
                    }
                    Token::Ident(s) if s == "true" => 1,
                    Token::Ident(s) if s == "false" => 0,
                    t => {
                        return Err(AssemblerError::Expected {
                            expected: "bool".to_string(),
                            got: format!("{:?}", t),
                        });
                    }
                };
                encode_a(OpCode::LoadBool, a, b, 0)
            }
            "Add" => self.parse_ternary_reg(OpCode::Add)?,
            "Sub" => self.parse_ternary_reg(OpCode::Sub)?,
            "Mul" => self.parse_ternary_reg(OpCode::Mul)?,
            "Div" => self.parse_ternary_reg(OpCode::Div)?,
            "Mod" => self.parse_ternary_reg(OpCode::Mod)?,
            "Neg" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                encode_a(OpCode::Neg, a, b, 0)
            }
            "Eq" => self.parse_ternary_reg(OpCode::Eq)?,
            "Ne" => self.parse_ternary_reg(OpCode::Ne)?,
            "Lt" => self.parse_ternary_reg(OpCode::Lt)?,
            "Le" => self.parse_ternary_reg(OpCode::Le)?,
            "Gt" => self.parse_ternary_reg(OpCode::Gt)?,
            "Ge" => self.parse_ternary_reg(OpCode::Ge)?,
            "Not" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                encode_a(OpCode::Not, a, b, 0)
            }
            "Jump" => {
                let (offset, label) = self.parse_jump_target()?;
                let instr = encode_b(OpCode::Jump, 0, offset);
                if let Some(lbl) = label {
                    label_refs.push((bytecode.len(), lbl, false));
                }
                instr
            }
            "JumpIf" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let (offset, label) = self.parse_jump_target()?;
                let instr = encode_b(OpCode::JumpIf, a, offset);
                if let Some(lbl) = label {
                    label_refs.push((bytecode.len(), lbl, true));
                }
                instr
            }
            "JumpIfNot" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let (offset, label) = self.parse_jump_target()?;
                let instr = encode_b(OpCode::JumpIfNot, a, offset);
                if let Some(lbl) = label {
                    label_refs.push((bytecode.len(), lbl, true));
                }
                instr
            }
            "Call" => {
                let dest = self.parse_register()?;
                self.skip_comma()?;
                let func = self.parse_register()?;
                self.skip_comma()?;
                let nargs = self.parse_u8()?;
                encode_a(OpCode::Call, dest, func, nargs)
            }
            "Return" => {
                let a = self.parse_register()?;
                encode_a(OpCode::Return, a, 0, 0)
            }
            "Return0" => encode_a(OpCode::Return0, 0, 0, 0),
            "GetGlobal" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let k = self.parse_u8()?;
                encode_a(OpCode::GetGlobal, a, k, 0)
            }
            "SetGlobal" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let k = self.parse_u8()?;
                encode_a(OpCode::SetGlobal, a, k, 0)
            }
            "GetGlobalIdx" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let idx = self.parse_i16()?;
                encode_b(OpCode::GetGlobalIdx, a, idx)
            }
            "SetGlobalIdx" => {
                let idx = self.parse_i16()?;
                self.skip_comma()?;
                let a = self.parse_register()?;
                encode_b(OpCode::SetGlobalIdx, a, idx)
            }
            "IncGlobalI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let k = self.parse_u8()?;
                self.skip_comma()?;
                let b = self.parse_u8()?;
                encode_a(OpCode::IncGlobalI, a, k, b)
            }
            "EnterNoGc" => encode_a(OpCode::EnterNoGc, 0, 0, 0),
            "ExitNoGc" => encode_a(OpCode::ExitNoGc, 0, 0, 0),
            "Alloc" => {
                let a = self.parse_register()?;
                encode_a(OpCode::Alloc, a, 0, 0)
            }
            "Free" => {
                let a = self.parse_register()?;
                encode_a(OpCode::Free, a, 0, 0)
            }
            "LoadMem" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                encode_a(OpCode::LoadMem, a, b, 0)
            }
            "LoadMemI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::LoadMemI, a, b, c)
            }
            "StoreMem" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                encode_a(OpCode::StoreMem, a, b, 0)
            }
            "StoreMemI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::StoreMemI, a, b, c)
            }
            "Print" => {
                let a = self.parse_register()?;
                encode_a(OpCode::Print, a, 0, 0)
            }
            "MakeClosure" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                // Handle 'kN' format (e.g., k0, k1) where N is the constant index
                let k = if let Token::Ident(s) = &self.current {
                    if s.starts_with('k') {
                        let num_str = &s[1..];
                        let k = num_str.parse::<u8>().map_err(|_| {
                            AssemblerError::InvalidNumber(format!("Invalid constant index: {}", s))
                        })?;
                        self.advance()?;
                        k
                    } else {
                        self.parse_u8()?
                    }
                } else {
                    self.parse_u8()?
                };
                self.skip_comma()?;
                let upval_count = self.parse_u8()?;
                encode_a(OpCode::MakeClosure, a, k, upval_count)
            }
            "GetUpval" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                // Parse 'upval[N]' format
                let idx = self.parse_upval_index()?;
                encode_a(OpCode::GetUpval, a, idx, 0)
            }
            "SetUpval" => {
                // Parse 'upval[N]' format
                let idx = self.parse_upval_index()?;
                self.skip_comma()?;
                let src = self.parse_register()?;
                encode_a(OpCode::SetUpval, idx, src, 0)
            }
            "CloseUpvals" => {
                let a = self.parse_register()?;
                encode_a(OpCode::CloseUpvals, a, 0, 0)
            }
            "ForLoopI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let offset = self.parse_i16()?;
                // Skip any trailing comment (iter+=step; ...)
                while self.current != Token::Newline && self.current != Token::Eof {
                    self.advance()?;
                }
                encode_b(OpCode::ForLoopI, a, offset)
            }
            "ForLoopIInc" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let offset = self.parse_i16()?;
                // Skip any trailing comment
                while self.current != Token::Newline && self.current != Token::Eof {
                    self.advance()?;
                }
                encode_b(OpCode::ForLoopIInc, a, offset)
            }
            // New immediate opcodes
            "AddI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::AddI, a, b, c)
            }
            "SubI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::SubI, a, b, c)
            }
            "LtImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let imm = self.parse_i16()?;
                encode_b(OpCode::LtImm, a, imm)
            }
            "LeImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let imm = self.parse_i16()?;
                encode_b(OpCode::LeImm, a, imm)
            }
            "GtImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let imm = self.parse_i16()?;
                encode_b(OpCode::GtImm, a, imm)
            }
            "GeImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let imm = self.parse_i16()?;
                encode_b(OpCode::GeImm, a, imm)
            }
            "WhileLoopLt" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let offset = self.parse_i16()?;
                // Skip any trailing comment
                while self.current != Token::Newline && self.current != Token::Eof {
                    self.advance()?;
                }
                encode_b(OpCode::WhileLoopLt, a, offset)
            }
            // Type-specialized integer opcodes
            "AddII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::AddII, a, b, c)
            }
            "SubII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::SubII, a, b, c)
            }
            "MulII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::MulII, a, b, c)
            }
            "DivII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::DivII, a, b, c)
            }
            "ModII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::ModII, a, b, c)
            }
            // Type-specialized float opcodes
            "AddFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::AddFF, a, b, c)
            }
            "SubFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::SubFF, a, b, c)
            }
            "MulFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::MulFF, a, b, c)
            }
            "DivFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::DivFF, a, b, c)
            }
            "ModFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::ModFF, a, b, c)
            }
            // Integer comparisons
            "LtII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::LtII, a, b, c)
            }
            "LeII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::LeII, a, b, c)
            }
            "GtII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::GtII, a, b, c)
            }
            "GeII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::GeII, a, b, c)
            }
            "EqII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::EqII, a, b, c)
            }
            "NeII" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::NeII, a, b, c)
            }
            // Float comparisons
            "LtFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::LtFF, a, b, c)
            }
            "LeFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::LeFF, a, b, c)
            }
            "GtFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::GtFF, a, b, c)
            }
            "GeFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::GeFF, a, b, c)
            }
            "EqFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::EqFF, a, b, c)
            }
            "NeFF" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_register()?;
                encode_a(OpCode::NeFF, a, b, c)
            }
            // Integer immediate comparisons
            "LtIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::LtIImm, a, b, c)
            }
            "LeIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::LeIImm, a, b, c)
            }
            "GtIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::GtIImm, a, b, c)
            }
            "GeIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::GeIImm, a, b, c)
            }
            "CallCached" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::CallCached, a, b, c)
            }
            "CallGlobal" => {
                // Format: CallGlobal r<dest>, <global_idx>, <nargs>
                // Followed by 2 cache words (emitted separately)
                let dest = self.parse_register()?;
                self.skip_comma()?;
                let global_idx = self.parse_u8()?;
                self.skip_comma()?;
                let nargs = self.parse_u8()?;
                extra_cache_words = 2;
                encode_a(OpCode::CallGlobal, dest, global_idx, nargs)
            }
            "CallGlobalMono" => {
                // Format: CallGlobalMono r<dest>, <global_idx>, <nargs>
                let dest = self.parse_register()?;
                self.skip_comma()?;
                let global_idx = self.parse_u8()?;
                self.skip_comma()?;
                let nargs = self.parse_u8()?;
                extra_cache_words = 2;
                encode_a(OpCode::CallGlobalMono, dest, global_idx, nargs)
            }
            "CallGlobalNative" => {
                // Format: CallGlobalNative r<dest>, <global_idx>, <nargs>
                let dest = self.parse_register()?;
                self.skip_comma()?;
                let global_idx = self.parse_u8()?;
                self.skip_comma()?;
                let nargs = self.parse_u8()?;
                extra_cache_words = 2;
                encode_a(OpCode::CallGlobalNative, dest, global_idx, nargs)
            }
            "CallUpval" => {
                // Format: CallUpval r<dest>, upval[N], <nargs>
                let dest = self.parse_register()?;
                self.skip_comma()?;
                let upval_idx = self.parse_upval_index()?;
                self.skip_comma()?;
                let nargs = self.parse_u8()?;
                encode_a(OpCode::CallUpval, dest, upval_idx, nargs)
            }
            "TailCallUpval" => {
                // Format: TailCallUpval r<dest>, upval[N], <nargs>
                let dest = self.parse_register()?;
                self.skip_comma()?;
                let upval_idx = self.parse_upval_index()?;
                self.skip_comma()?;
                let nargs = self.parse_u8()?;
                encode_a(OpCode::TailCallUpval, dest, upval_idx, nargs)
            }
            "AddIIG" => self.parse_ternary_reg(OpCode::AddIIG)?,
            "SubIIG" => self.parse_ternary_reg(OpCode::SubIIG)?,
            "MulIIG" => self.parse_ternary_reg(OpCode::MulIIG)?,
            "DivIIG" => self.parse_ternary_reg(OpCode::DivIIG)?,
            "ModIIG" => self.parse_ternary_reg(OpCode::ModIIG)?,
            "AddFFG" => self.parse_ternary_reg(OpCode::AddFFG)?,
            "SubFFG" => self.parse_ternary_reg(OpCode::SubFFG)?,
            "MulFFG" => self.parse_ternary_reg(OpCode::MulFFG)?,
            "DivFFG" => self.parse_ternary_reg(OpCode::DivFFG)?,
            "ModFFG" => self.parse_ternary_reg(OpCode::ModFFG)?,
            "LtIIG" => self.parse_ternary_reg(OpCode::LtIIG)?,
            "LeIIG" => self.parse_ternary_reg(OpCode::LeIIG)?,
            "GtIIG" => self.parse_ternary_reg(OpCode::GtIIG)?,
            "GeIIG" => self.parse_ternary_reg(OpCode::GeIIG)?,
            "EqIIG" => self.parse_ternary_reg(OpCode::EqIIG)?,
            "NeIIG" => self.parse_ternary_reg(OpCode::NeIIG)?,
            "LtFFG" => self.parse_ternary_reg(OpCode::LtFFG)?,
            "LeFFG" => self.parse_ternary_reg(OpCode::LeFFG)?,
            "GtFFG" => self.parse_ternary_reg(OpCode::GtFFG)?,
            "GeFFG" => self.parse_ternary_reg(OpCode::GeFFG)?,
            "EqFFG" => self.parse_ternary_reg(OpCode::EqFFG)?,
            "NeFFG" => self.parse_ternary_reg(OpCode::NeFFG)?,
            "Shl" => self.parse_ternary_reg(OpCode::Shl)?,
            "Shr" => self.parse_ternary_reg(OpCode::Shr)?,
            "BitAnd" => self.parse_ternary_reg(OpCode::BitAnd)?,
            "BitOr" => self.parse_ternary_reg(OpCode::BitOr)?,
            "BitXor" => self.parse_ternary_reg(OpCode::BitXor)?,
            "BitNot" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                encode_a(OpCode::BitNot, a, b, 0)
            }
            "ShlII" => self.parse_ternary_reg(OpCode::ShlII)?,
            "ShrII" => self.parse_ternary_reg(OpCode::ShrII)?,
            "AndII" => self.parse_ternary_reg(OpCode::AndII)?,
            "OrII" => self.parse_ternary_reg(OpCode::OrII)?,
            "XorII" => self.parse_ternary_reg(OpCode::XorII)?,
            "NotI" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                encode_a(OpCode::NotI, a, b, 0)
            }
            "ShlIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::ShlIImm, a, b, c)
            }
            "ShrIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::ShrIImm, a, b, c)
            }
            "AndIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::AndIImm, a, b, c)
            }
            "OrIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::OrIImm, a, b, c)
            }
            "XorIImm" => {
                let a = self.parse_register()?;
                self.skip_comma()?;
                let b = self.parse_register()?;
                self.skip_comma()?;
                let c = self.parse_u8()?;
                encode_a(OpCode::XorIImm, a, b, c)
            }
            _ => return Err(AssemblerError::UnknownOpcode(opcode_name)),
        };

        bytecode.push(instr);
        if extra_cache_words > 0 {
            bytecode.extend(std::iter::repeat(0).take(extra_cache_words));
        }
        Ok(())
    }

    fn parse_ternary_reg(&mut self, op: OpCode) -> Result<u32> {
        let a = self.parse_register()?;
        self.skip_comma()?;
        let b = self.parse_register()?;
        self.skip_comma()?;
        let c = self.parse_register()?;
        Ok(encode_a(op, a, b, c))
    }

    fn parse_jump_target(&mut self) -> Result<(i16, Option<String>)> {
        match &self.current {
            Token::LabelRef(name) => {
                let label = name.clone();
                self.advance()?;
                Ok((0, Some(label)))
            }
            Token::At => {
                self.advance()?;
                if let Token::Int(n) = self.advance()? {
                    Ok((n as i16, None))
                } else {
                    Err(AssemblerError::Expected {
                        expected: "offset".to_string(),
                        got: format!("{:?}", self.current),
                    })
                }
            }
            Token::Int(n) => {
                let offset = *n;
                self.advance()?;
                Ok((offset as i16, None))
            }
            _ => Err(AssemblerError::Expected {
                expected: "label or offset".to_string(),
                got: format!("{:?}", self.current),
            }),
        }
    }

    /// Parse 'upval[N]' format and return N
    pub(super) fn parse_upval_index(&mut self) -> Result<u8> {
        // Expect 'upval' identifier
        if let Token::Ident(s) = &self.current {
            if s != "upval" {
                return Err(AssemblerError::Expected {
                    expected: "upval".to_string(),
                    got: format!("{:?}", self.current),
                });
            }
            self.advance()?;
        } else {
            return Err(AssemblerError::Expected {
                expected: "upval".to_string(),
                got: format!("{:?}", self.current),
            });
        }

        // Expect '['
        if self.current != Token::LBracket {
            return Err(AssemblerError::Expected {
                expected: "[".to_string(),
                got: format!("{:?}", self.current),
            });
        }
        self.advance()?;

        // Parse the index
        let idx = self.parse_u8()?;

        // Expect ']'
        if self.current != Token::RBracket {
            return Err(AssemblerError::Expected {
                expected: "]".to_string(),
                got: format!("{:?}", self.current),
            });
        }
        self.advance()?;

        Ok(idx)
    }
}

/// Encode a Format A instruction
pub(super) fn encode_a(op: OpCode, a: u8, b: u8, c: u8) -> u32 {
    ((op as u32) << 24) | ((a as u32) << 16) | ((b as u32) << 8) | (c as u32)
}

/// Encode a Format B instruction
pub(super) fn encode_b(op: OpCode, a: u8, imm: i16) -> u32 {
    ((op as u32) << 24) | ((a as u32) << 16) | ((imm as u16) as u32)
}
