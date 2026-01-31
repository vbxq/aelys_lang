use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_sema::{TypedExpr, TypedFmtStringPart};
use aelys_syntax::Span;

impl Compiler {
    pub fn compile_typed_fmt_string(
        &mut self,
        parts: &[TypedFmtStringPart],
        extra_args: &[TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let placeholder_count = parts
            .iter()
            .filter(|p| matches!(p, TypedFmtStringPart::Placeholder))
            .count();

        if placeholder_count != extra_args.len() {
            return Err(CompileError::new(
                CompileErrorKind::TypeInferenceError(format!(
                    "format string has {} placeholder(s) but {} argument(s) provided",
                    placeholder_count,
                    extra_args.len()
                )),
                span,
                self.source.clone(),
            )
            .into());
        }

        if parts.is_empty() {
            return self.compile_literal_string("", dest, span);
        }

        if parts.len() == 1 && extra_args.is_empty() {
            return self.compile_single_typed_fmt_part(&parts[0], dest, span);
        }

        let mut arg_idx = 0;
        let mut result_reg: Option<u8> = None;

        for part in parts {
            let part_reg = self.alloc_register()?;

            match part {
                TypedFmtStringPart::Literal(s) => {
                    self.compile_literal_string(s, part_reg, span)?;
                }
                TypedFmtStringPart::Expr(expr) => {
                    self.compile_typed_expr_to_string(expr, part_reg, span)?;
                }
                TypedFmtStringPart::Placeholder => {
                    let arg = &extra_args[arg_idx];
                    arg_idx += 1;
                    self.compile_typed_expr_to_string(arg, part_reg, span)?;
                }
            }

            match result_reg {
                None => {
                    result_reg = Some(part_reg);
                }
                Some(acc) => {
                    self.emit_a(OpCode::Add, acc, acc, part_reg, span);
                    self.free_register(part_reg);
                }
            }
        }

        if let Some(acc) = result_reg {
            if acc != dest {
                self.emit_a(OpCode::Move, dest, acc, 0, span);
                self.free_register(acc);
            }
        }

        Ok(())
    }

    fn compile_single_typed_fmt_part(
        &mut self,
        part: &TypedFmtStringPart,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        match part {
            TypedFmtStringPart::Literal(s) => self.compile_literal_string(s, dest, span),
            TypedFmtStringPart::Expr(expr) => self.compile_typed_expr_to_string(expr, dest, span),
            TypedFmtStringPart::Placeholder => Err(CompileError::new(
                CompileErrorKind::TypeInferenceError("placeholder without argument".to_string()),
                span,
                self.source.clone(),
            )
            .into()),
        }
    }

    fn compile_typed_expr_to_string(
        &mut self,
        expr: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        self.compile_typed_expr(expr, dest)?;
        self.emit_typed_tostring_call(dest, span)
    }

    fn emit_typed_tostring_call(&mut self, reg: u8, span: Span) -> Result<()> {
        let global_idx = self.get_or_create_global_index("__tostring");
        self.accessed_globals.insert("__tostring".to_string());
        self.emit_call_global_cached(reg, global_idx as u8, 1, "__tostring", span);
        Ok(())
    }
}
