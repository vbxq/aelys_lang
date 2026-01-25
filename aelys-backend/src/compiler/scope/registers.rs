use super::super::Compiler;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;

impl Compiler {
    pub fn alloc_register(&mut self) -> Result<u8> {
        for (i, used) in self.register_pool.iter_mut().enumerate() {
            if !*used {
                *used = true;
                self.next_register = self.next_register.max(i as u8 + 1);
                return Ok(i as u8);
            }
        }
        Err(CompileError::new(
            CompileErrorKind::TooManyRegisters,
            Span::dummy(),
            self.source.clone(),
        )
        .into())
    }

    pub fn free_register(&mut self, reg: u8) {
        self.register_pool[reg as usize] = false;
    }

    // contiguous block for call args
    pub fn alloc_consecutive_registers_for_call(&self, n: u8, span: Span) -> Result<u8> {
        let n = n as usize;
        let pool_len = self.register_pool.len();

        'outer: for start in 0..pool_len {
            if start + n > pool_len {
                break;
            }
            for offset in 0..n {
                if self.register_pool[start + offset] {
                    continue 'outer;
                }
            }
            return Ok(start as u8);
        }

        Err(CompileError::new(
            CompileErrorKind::TooManyRegisters,
            span,
            self.source.clone(),
        )
        .into())
    }

    pub fn alloc_consecutive_from(&mut self, start: u8, count: u8) -> Result<u8> {
        let start_usize = start as usize;
        let count_usize = count as usize;
        let end_usize = start_usize + count_usize;

        if end_usize > 256 {
            return Err(CompileError::new(
                CompileErrorKind::TooManyRegisters,
                Span::dummy(),
                self.source.clone(),
            )
            .into());
        }

        for i in start_usize..end_usize {
            self.register_pool[i] = true;
        }

        self.next_register = self.next_register.max(end_usize as u8);
        Ok(start)
    }
}
