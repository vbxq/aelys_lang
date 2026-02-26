use super::LoweringContext;
use crate::*;
use aelys_sema::{TypedStmt, TypedStmtKind};

impl<'a> LoweringContext<'a> {
    pub(super) fn lower_body(&mut self, stmts: &[TypedStmt]) {
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
    }

    pub(super) fn finalize_function_body(&mut self) {
        if (self.current_stmts.is_empty() && self.current_blocks.is_empty())
            || !self.current_stmts.is_empty()
        {
            self.seal_block(AirTerminator::Return(None));
        }
    }

    pub(super) fn lower_stmt(&mut self, stmt: &TypedStmt) {
        let sp = Some(self.span(&stmt.span));
        match &stmt.kind {
            TypedStmtKind::Expression(expr) => {
                self.lower_expr_discard(expr);
            }
            TypedStmtKind::Let {
                name,
                mutable,
                initializer,
                var_type,
                ..
            } => {
                let ty = self.lower_type_from_infer(var_type);
                let local = self.alloc_named_local(name, ty, *mutable, sp);
                let operand = self.lower_expr(initializer);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(local),
                        rvalue: Rvalue::Use(operand),
                    },
                    sp,
                );
            }
            TypedStmtKind::Block(stmts) => {
                self.lower_body(stmts);
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.lower_if(condition, then_branch, else_branch.as_deref(), sp);
            }
            TypedStmtKind::While { condition, body } => {
                self.lower_while(condition, body, sp);
            }
            TypedStmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => {
                self.lower_for(iterator, start, end, *inclusive, step.as_ref(), body);
            }
            TypedStmtKind::ForEach {
                iterator,
                iterable,
                elem_type,
                body,
            } => {
                self.lower_foreach(iterator, iterable, elem_type, body, sp);
            }
            TypedStmtKind::Return(val) => {
                let operand = val.as_ref().map(|e| self.lower_expr(e));
                self.seal_block(AirTerminator::Return(operand));
            }
            TypedStmtKind::Break => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let exit = loop_ctx.exit;
                    self.seal_block(AirTerminator::Goto(exit));
                }
            }
            TypedStmtKind::Continue => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let header = loop_ctx.header;
                    self.seal_block(AirTerminator::Goto(header));
                }
            }
            TypedStmtKind::Function(func) => {
                self.lower_function(func);
            }
            TypedStmtKind::Needs(_) | TypedStmtKind::StructDecl { .. } => {}
        }
    }

    // control flow desugaring
    pub(super) fn lower_if(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        then_branch: &TypedStmt,
        else_branch: Option<&TypedStmt>,
        _sp: Option<Span>,
    ) {
        let cond = self.lower_expr(condition);
        let then_id = self.alloc_block_id();
        let else_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: then_id,
            else_block: if else_branch.is_some() {
                else_id
            } else {
                merge_id
            },
        });

        self.fixup_block_id_noop(then_id);
        self.lower_stmt(then_branch);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(merge_id));
        }

        if let Some(else_br) = else_branch {
            self.fixup_block_id_noop(else_id);
            self.lower_stmt(else_br);
            if !self.last_block_is_terminated() {
                self.seal_block(AirTerminator::Goto(merge_id));
            }
        }

        self.fixup_block_id_noop(merge_id);
    }

    pub(super) fn lower_while(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        body: &TypedStmt,
        _sp: Option<Span>,
    ) {
        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(header_id);
        let cond = self.lower_expr(condition);
        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: body_id,
            else_block: exit_id,
        });

        self.loop_stack.push(super::LoopBlocks {
            header: header_id,
            exit: exit_id,
        });
        self.fixup_block_id_noop(body_id);
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(header_id));
        }
        self.loop_stack.pop();

        self.fixup_block_id_noop(exit_id);
    }

    // the old fixup_block_id(X) renamed the LAST sealed block to X
    // With nested control flow that creates multiple blocks, the last block is
    // some inner merge, not the branch entry. This nuked entire loop bodies.
    // Now we set the pending id BEFORE lowering so the first seal_block picks it up.
    pub(super) fn fixup_block_id_noop(&mut self, target: BlockId) {
        if let Some(old) = self.pending_block_id {
            if old != target {
                self.block_aliases.push((old.0, target.0));
            }
        }
        self.pending_block_id = Some(target);
    }

    pub(super) fn resolve_block_aliases(&mut self) {
        if self.block_aliases.is_empty() {
            return;
        }
        let resolve = |id: &mut BlockId, aliases: &[(u32, u32)]| {
            let mut current = id.0;
            for _ in 0..aliases.len() {
                if let Some(&(_, to)) = aliases.iter().find(|(from, _)| *from == current) {
                    current = to;
                } else {
                    break;
                }
            }
            *id = BlockId(current);
        };
        for block in &mut self.current_blocks {
            let aliases = &self.block_aliases;
            match &mut block.terminator {
                AirTerminator::Goto(id) => resolve(id, aliases),
                AirTerminator::Branch {
                    then_block,
                    else_block,
                    ..
                } => {
                    resolve(then_block, aliases);
                    resolve(else_block, aliases);
                }
                AirTerminator::Switch {
                    targets, default, ..
                } => {
                    for (_, id) in targets {
                        resolve(id, aliases);
                    }
                    resolve(default, aliases);
                }
                AirTerminator::Invoke { normal, unwind, .. } => {
                    resolve(normal, aliases);
                    resolve(unwind, aliases);
                }
                AirTerminator::Return(_)
                | AirTerminator::Unreachable
                | AirTerminator::Unwind
                | AirTerminator::Panic { .. } => {}
            }
        }
        self.block_aliases.clear();
    }

    pub(super) fn last_block_is_terminated(&self) -> bool {
        // a block is terminated if it has a terminator than Goto
        // Goto is a fallthrough to a merge block, not a definitive exit
        // other terminator return, unreachable, branch etc are definitive exits.
        self.current_stmts.is_empty()
            && self
                .current_blocks
                .last()
                .is_some_and(|b| !matches!(b.terminator, AirTerminator::Goto(_)))
    }
}
