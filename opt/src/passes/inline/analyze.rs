use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedProgram, TypedStmt, TypedStmtKind};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub body_size: usize,
    pub call_count: usize,
    pub calls: HashSet<String>,
    pub has_captures: bool,
    pub is_recursive: bool,
    pub has_inline: bool,
    pub has_inline_always: bool,
}

pub struct ProgramAnalysis {
    pub functions: HashMap<String, FunctionInfo>,
    pub recursion_cycles: Vec<Vec<String>>,
    pub total_size: usize,
}

impl ProgramAnalysis {
    pub fn analyze(program: &TypedProgram) -> Self {
        let mut functions = HashMap::new();
        let mut total_size = 0;

        for stmt in &program.stmts {
            total_size += count_stmt_size(stmt);
            if let TypedStmtKind::Function(func) = &stmt.kind {
                let info = analyze_function(func);
                functions.insert(func.name.clone(), info);
            }
        }

        // count call sites
        let mut call_counts: HashMap<String, usize> = HashMap::new();
        for stmt in &program.stmts {
            count_calls_in_stmt(stmt, &mut call_counts);
        }
        for (name, count) in call_counts {
            if let Some(info) = functions.get_mut(&name) {
                info.call_count = count;
            }
        }

        // detect direct recursion
        for (name, info) in functions.iter_mut() {
            if info.calls.contains(name) {
                info.is_recursive = true;
            }
        }

        let recursion_cycles = find_mutual_recursion(&functions);

        Self {
            functions,
            recursion_cycles,
            total_size,
        }
    }

    pub fn should_inline(&self, name: &str, aggressive: bool, bloat_budget: f64) -> InlineDecision {
        let Some(info) = self.functions.get(name) else {
            return InlineDecision::Skip;
        };

        if info.is_recursive {
            return InlineDecision::Blocked(BlockReason::Recursive);
        }

        for cycle in &self.recursion_cycles {
            if cycle.contains(&name.to_string()) {
                return InlineDecision::Blocked(BlockReason::MutualRecursion(cycle.clone()));
            }
        }

        // @inline_always forces it (except for truly impossible cases already checked)
        if info.has_inline_always {
            return InlineDecision::Inline;
        }

        if info.has_captures {
            return InlineDecision::Blocked(BlockReason::HasCaptures);
        }

        // @inline decorator
        if info.has_inline {
            return InlineDecision::Inline;
        }

        // trivial functions: always inline regardless of mode
        if info.body_size <= 3 {
            return InlineDecision::Inline;
        }

        // single call site: inline to eliminate the function entirely
        if info.call_count == 1 {
            return InlineDecision::Inline;
        }

        if aggressive {
            let inline_cost = info.body_size * info.call_count;
            let budget = (self.total_size as f64 * bloat_budget).max(10.0) as usize;
            if inline_cost <= budget {
                return InlineDecision::Inline;
            }
        }

        InlineDecision::Skip
    }
}

#[derive(Debug, Clone)]
pub enum InlineDecision {
    Inline,
    Skip,
    Blocked(BlockReason),
}

#[derive(Debug, Clone)]
pub enum BlockReason {
    Recursive,
    MutualRecursion(Vec<String>),
    HasCaptures,
}

fn analyze_function(func: &TypedFunction) -> FunctionInfo {
    let mut calls = HashSet::new();
    for stmt in &func.body {
        collect_calls_in_stmt(stmt, &mut calls);
    }

    let has_inline = func.decorators.iter().any(|d| d.name == "inline");
    let has_inline_always = func.decorators.iter().any(|d| d.name == "inline_always");

    FunctionInfo {
        body_size: func.body.iter().map(count_stmt_size).sum(),
        call_count: 0,
        calls,
        has_captures: !func.captures.is_empty(),
        is_recursive: false,
        has_inline,
        has_inline_always,
    }
}

fn collect_calls_in_stmt(stmt: &TypedStmt, calls: &mut HashSet<String>) {
    match &stmt.kind {
        TypedStmtKind::Expression(e) => collect_calls_in_expr(e, calls),
        TypedStmtKind::Let { initializer, .. } => collect_calls_in_expr(initializer, calls),
        TypedStmtKind::Block(stmts) => {
            for s in stmts {
                collect_calls_in_stmt(s, calls);
            }
        }
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_calls_in_expr(condition, calls);
            collect_calls_in_stmt(then_branch, calls);
            if let Some(eb) = else_branch {
                collect_calls_in_stmt(eb, calls);
            }
        }
        TypedStmtKind::While { condition, body } => {
            collect_calls_in_expr(condition, calls);
            collect_calls_in_stmt(body, calls);
        }
        TypedStmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_calls_in_expr(start, calls);
            collect_calls_in_expr(end, calls);
            if let Some(s) = &**step {
                collect_calls_in_expr(s, calls);
            }
            collect_calls_in_stmt(body, calls);
        }
        TypedStmtKind::Return(Some(e)) => collect_calls_in_expr(e, calls),
        TypedStmtKind::Function(f) => {
            for s in &f.body {
                collect_calls_in_stmt(s, calls);
            }
        }
        _ => {}
    }
}

fn collect_calls_in_expr(expr: &TypedExpr, calls: &mut HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Call { callee, args } => {
            if let TypedExprKind::Identifier(name) = &callee.kind {
                calls.insert(name.clone());
            }
            collect_calls_in_expr(callee, calls);
            for arg in args {
                collect_calls_in_expr(arg, calls);
            }
        }
        TypedExprKind::Binary { left, right, .. } => {
            collect_calls_in_expr(left, calls);
            collect_calls_in_expr(right, calls);
        }
        TypedExprKind::Unary { operand, .. } => collect_calls_in_expr(operand, calls),
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            collect_calls_in_expr(left, calls);
            collect_calls_in_expr(right, calls);
        }
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_calls_in_expr(condition, calls);
            collect_calls_in_expr(then_branch, calls);
            collect_calls_in_expr(else_branch, calls);
        }
        TypedExprKind::Grouping(inner) | TypedExprKind::Lambda(inner) => {
            collect_calls_in_expr(inner, calls);
        }
        TypedExprKind::LambdaInner { body, .. } => {
            for s in body {
                collect_calls_in_stmt(s, calls);
            }
        }
        TypedExprKind::Assign { value, .. } => collect_calls_in_expr(value, calls),
        TypedExprKind::Member { object, .. } => collect_calls_in_expr(object, calls),
        TypedExprKind::ArrayLiteral { elements, .. }
        | TypedExprKind::VecLiteral { elements, .. } => {
            for e in elements {
                collect_calls_in_expr(e, calls);
            }
        }
        TypedExprKind::ArraySized { size, .. } => collect_calls_in_expr(size, calls),
        TypedExprKind::Index { object, index } => {
            collect_calls_in_expr(object, calls);
            collect_calls_in_expr(index, calls);
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            collect_calls_in_expr(object, calls);
            collect_calls_in_expr(index, calls);
            collect_calls_in_expr(value, calls);
        }
        TypedExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                collect_calls_in_expr(s, calls);
            }
            if let Some(e) = end {
                collect_calls_in_expr(e, calls);
            }
        }
        TypedExprKind::Slice { object, range } => {
            collect_calls_in_expr(object, calls);
            collect_calls_in_expr(range, calls);
        }
        _ => {}
    }
}

fn count_calls_in_stmt(stmt: &TypedStmt, counts: &mut HashMap<String, usize>) {
    match &stmt.kind {
        TypedStmtKind::Expression(e) => count_calls_in_expr(e, counts),
        TypedStmtKind::Let { initializer, .. } => count_calls_in_expr(initializer, counts),
        TypedStmtKind::Block(stmts) => {
            for s in stmts {
                count_calls_in_stmt(s, counts);
            }
        }
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            count_calls_in_expr(condition, counts);
            count_calls_in_stmt(then_branch, counts);
            if let Some(eb) = else_branch {
                count_calls_in_stmt(eb, counts);
            }
        }
        TypedStmtKind::While { condition, body } => {
            count_calls_in_expr(condition, counts);
            count_calls_in_stmt(body, counts);
        }
        TypedStmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            count_calls_in_expr(start, counts);
            count_calls_in_expr(end, counts);
            if let Some(s) = &**step {
                count_calls_in_expr(s, counts);
            }
            count_calls_in_stmt(body, counts);
        }
        TypedStmtKind::Return(Some(e)) => count_calls_in_expr(e, counts),
        TypedStmtKind::Function(f) => {
            for s in &f.body {
                count_calls_in_stmt(s, counts);
            }
        }
        _ => {}
    }
}

fn count_calls_in_expr(expr: &TypedExpr, counts: &mut HashMap<String, usize>) {
    match &expr.kind {
        TypedExprKind::Call { callee, args } => {
            if let TypedExprKind::Identifier(name) = &callee.kind {
                *counts.entry(name.clone()).or_insert(0) += 1;
            }
            count_calls_in_expr(callee, counts);
            for arg in args {
                count_calls_in_expr(arg, counts);
            }
        }
        TypedExprKind::Binary { left, right, .. } => {
            count_calls_in_expr(left, counts);
            count_calls_in_expr(right, counts);
        }
        TypedExprKind::Unary { operand, .. } => count_calls_in_expr(operand, counts),
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            count_calls_in_expr(left, counts);
            count_calls_in_expr(right, counts);
        }
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            count_calls_in_expr(condition, counts);
            count_calls_in_expr(then_branch, counts);
            count_calls_in_expr(else_branch, counts);
        }
        TypedExprKind::Grouping(inner) | TypedExprKind::Lambda(inner) => {
            count_calls_in_expr(inner, counts);
        }
        TypedExprKind::LambdaInner { body, .. } => {
            for s in body {
                count_calls_in_stmt(s, counts);
            }
        }
        TypedExprKind::Assign { value, .. } => count_calls_in_expr(value, counts),
        TypedExprKind::Member { object, .. } => count_calls_in_expr(object, counts),
        TypedExprKind::ArrayLiteral { elements, .. }
        | TypedExprKind::VecLiteral { elements, .. } => {
            for e in elements {
                count_calls_in_expr(e, counts);
            }
        }
        TypedExprKind::ArraySized { size, .. } => count_calls_in_expr(size, counts),
        TypedExprKind::Index { object, index } => {
            count_calls_in_expr(object, counts);
            count_calls_in_expr(index, counts);
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            count_calls_in_expr(object, counts);
            count_calls_in_expr(index, counts);
            count_calls_in_expr(value, counts);
        }
        TypedExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                count_calls_in_expr(s, counts);
            }
            if let Some(e) = end {
                count_calls_in_expr(e, counts);
            }
        }
        TypedExprKind::Slice { object, range } => {
            count_calls_in_expr(object, counts);
            count_calls_in_expr(range, counts);
        }
        _ => {}
    }
}

fn count_stmt_size(stmt: &TypedStmt) -> usize {
    match &stmt.kind {
        TypedStmtKind::Block(stmts) => stmts.iter().map(count_stmt_size).sum(),
        TypedStmtKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            1 + count_stmt_size(then_branch)
                + else_branch.as_ref().map_or(0, |e| count_stmt_size(e))
        }
        TypedStmtKind::While { body, .. } => 1 + count_stmt_size(body),
        TypedStmtKind::For { body, .. } => 1 + count_stmt_size(body),
        TypedStmtKind::Function(f) => f.body.iter().map(count_stmt_size).sum(),
        _ => 1,
    }
}

fn find_mutual_recursion(functions: &HashMap<String, FunctionInfo>) -> Vec<Vec<String>> {
    let mut cycles = Vec::new();
    let mut visited = HashSet::new();

    for name in functions.keys() {
        if visited.contains(name) {
            continue;
        }

        let mut path = Vec::new();
        let mut path_set = HashSet::new();
        find_cycles_dfs(
            name,
            functions,
            &mut path,
            &mut path_set,
            &mut visited,
            &mut cycles,
        );
    }

    cycles
}

fn find_cycles_dfs(
    current: &str,
    functions: &HashMap<String, FunctionInfo>,
    path: &mut Vec<String>,
    path_set: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    cycles: &mut Vec<Vec<String>>,
) {
    if path_set.contains(current) {
        let cycle_start = path.iter().position(|n| n == current).unwrap();
        let mut cycle: Vec<_> = path[cycle_start..].to_vec();
        cycle.push(current.to_string());
        if cycle.len() > 2 {
            cycles.push(cycle);
        }
        return;
    }

    if visited.contains(current) {
        return;
    }

    path.push(current.to_string());
    path_set.insert(current.to_string());

    if let Some(info) = functions.get(current) {
        for callee in &info.calls {
            if functions.contains_key(callee) {
                find_cycles_dfs(callee, functions, path, path_set, visited, cycles);
            }
        }
    }

    path.pop();
    path_set.remove(current);
    visited.insert(current.to_string());
}
