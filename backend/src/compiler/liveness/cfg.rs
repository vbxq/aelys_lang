use super::{BasicBlock, ControlFlowGraph};
use aelys_sema::{TypedStmt, TypedStmtKind};
use std::collections::HashMap;

pub(super) fn build_cfg(stmts: &[TypedStmt]) -> ControlFlowGraph {
    let mut cfg = ControlFlowGraph {
        blocks: Vec::new(),
        stmt_to_block: HashMap::new(),
        entry: 0,
        exits: Vec::new(),
    };

    if stmts.is_empty() {
        let block = BasicBlock {
            id: 0,
            stmts: Vec::new(),
            successors: Vec::new(),
            predecessors: Vec::new(),
        };
        cfg.blocks.push(block);
        cfg.exits.push(0);
        return cfg;
    }

    let mut current_block_id: usize = 0;
    let mut current_block_stmts: Vec<usize> = Vec::new();
    let mut current_block_predecessors: Vec<usize> = Vec::new();

    for (idx, stmt) in stmts.iter().enumerate() {
        current_block_stmts.push(idx);
        cfg.stmt_to_block.insert(idx, current_block_id);

        let is_terminator = matches!(
            stmt.kind,
            TypedStmtKind::Return(_)
                | TypedStmtKind::Break
                | TypedStmtKind::Continue
                | TypedStmtKind::If { .. }
                | TypedStmtKind::While { .. }
                | TypedStmtKind::For { .. }
                | TypedStmtKind::ForEach { .. }
        );

        if is_terminator || idx == stmts.len() - 1 {
            let block_id = current_block_id;
            let block = BasicBlock {
                id: block_id,
                stmts: std::mem::take(&mut current_block_stmts),
                successors: Vec::new(),
                predecessors: std::mem::take(&mut current_block_predecessors),
            };
            cfg.blocks.push(block);

            if idx < stmts.len() - 1 {
                current_block_id = block_id + 1;

                if !matches!(
                    stmt.kind,
                    TypedStmtKind::Return(_) | TypedStmtKind::Break | TypedStmtKind::Continue
                ) {
                    cfg.blocks[block_id].successors.push(block_id + 1);
                    current_block_predecessors.push(block_id);
                }
            }
        }
    }

    for block in &cfg.blocks {
        if block.successors.is_empty() {
            cfg.exits.push(block.id);
        }
    }

    if cfg.exits.is_empty() && !cfg.blocks.is_empty() {
        cfg.exits.push(cfg.blocks.len() - 1);
    }

    cfg
}
