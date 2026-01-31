use super::analyze::{BlockReason, InlineDecision, ProgramAnalysis};
use super::expand::InlineExpander;
use crate::passes::{OptimizationLevel, OptimizationPass, OptimizationStats};
use aelys_common::{Warning, WarningKind};
use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedProgram, TypedStmt, TypedStmtKind};
use std::collections::{HashMap, HashSet};

const BLOAT_BUDGET: f64 = 0.20;

pub struct FunctionInliner {
    level: OptimizationLevel,
    stats: OptimizationStats,
    warnings: Vec<Warning>,
    warned_functions: HashSet<String>,
    functions: HashMap<String, TypedFunction>,
    expander: InlineExpander,
}

impl FunctionInliner {
    pub fn new(level: OptimizationLevel) -> Self {
        Self {
            level,
            stats: OptimizationStats::new(),
            warnings: Vec::new(),
            warned_functions: HashSet::new(),
            functions: HashMap::new(),
            expander: InlineExpander::new(),
        }
    }

    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    pub fn take_warnings(&mut self) -> Vec<Warning> {
        std::mem::take(&mut self.warnings)
    }

    fn collect_functions(&mut self, program: &TypedProgram) {
        self.functions.clear();
        for stmt in &program.stmts {
            if let TypedStmtKind::Function(f) = &stmt.kind {
                self.functions.insert(f.name.clone(), f.clone());
            }
        }
    }

    fn inline_in_stmt(&mut self, stmt: &mut TypedStmt, analysis: &ProgramAnalysis) {
        match &mut stmt.kind {
            TypedStmtKind::Expression(e) => self.inline_in_expr(e, analysis),

            TypedStmtKind::Let { initializer, .. } => {
                self.inline_in_expr(initializer, analysis);
            }

            TypedStmtKind::Block(stmts) => {
                for s in stmts.iter_mut() {
                    self.inline_in_stmt(s, analysis);
                }
            }

            TypedStmtKind::If { condition, then_branch, else_branch } => {
                self.inline_in_expr(condition, analysis);
                self.inline_in_stmt(then_branch, analysis);
                if let Some(eb) = else_branch {
                    self.inline_in_stmt(eb, analysis);
                }
            }

            TypedStmtKind::While { condition, body } => {
                self.inline_in_expr(condition, analysis);
                self.inline_in_stmt(body, analysis);
            }

            TypedStmtKind::For { start, end, step, body, .. } => {
                self.inline_in_expr(start, analysis);
                self.inline_in_expr(end, analysis);
                if let Some(s) = step {
                    self.inline_in_expr(s, analysis);
                }
                self.inline_in_stmt(body, analysis);
            }

            TypedStmtKind::Return(Some(e)) => self.inline_in_expr(e, analysis),

            TypedStmtKind::Function(f) => {
                for s in f.body.iter_mut() {
                    self.inline_in_stmt(s, analysis);
                }
            }

            _ => {}
        }
    }

    fn inline_in_expr(&mut self, expr: &mut TypedExpr, analysis: &ProgramAnalysis) {
        // recurse first so nested calls get processed
        match &mut expr.kind {
            TypedExprKind::Binary { left, right, .. } => {
                self.inline_in_expr(left, analysis);
                self.inline_in_expr(right, analysis);
            }
            TypedExprKind::Unary { operand, .. } => self.inline_in_expr(operand, analysis),
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.inline_in_expr(left, analysis);
                self.inline_in_expr(right, analysis);
            }
            TypedExprKind::Call { callee, args } => {
                self.inline_in_expr(callee, analysis);
                for arg in args.iter_mut() {
                    self.inline_in_expr(arg, analysis);
                }
            }
            TypedExprKind::Grouping(inner) | TypedExprKind::Lambda(inner) => {
                self.inline_in_expr(inner, analysis);
            }
            TypedExprKind::If { condition, then_branch, else_branch } => {
                self.inline_in_expr(condition, analysis);
                self.inline_in_expr(then_branch, analysis);
                self.inline_in_expr(else_branch, analysis);
            }
            TypedExprKind::Assign { value, .. } => self.inline_in_expr(value, analysis),
            TypedExprKind::Member { object, .. } => self.inline_in_expr(object, analysis),
            TypedExprKind::ArrayLiteral { elements, .. } | TypedExprKind::VecLiteral { elements, .. } => {
                for e in elements.iter_mut() { self.inline_in_expr(e, analysis); }
            }
            TypedExprKind::ArraySized { size, .. } => self.inline_in_expr(size, analysis),
            TypedExprKind::Index { object, index } => {
                self.inline_in_expr(object, analysis);
                self.inline_in_expr(index, analysis);
            }
            TypedExprKind::IndexAssign { object, index, value } => {
                self.inline_in_expr(object, analysis);
                self.inline_in_expr(index, analysis);
                self.inline_in_expr(value, analysis);
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start { self.inline_in_expr(s, analysis); }
                if let Some(e) = end { self.inline_in_expr(e, analysis); }
            }
            TypedExprKind::Slice { object, range } => {
                self.inline_in_expr(object, analysis);
                self.inline_in_expr(range, analysis);
            }
            TypedExprKind::LambdaInner { body, .. } => {
                for s in body.iter_mut() { self.inline_in_stmt(s, analysis); }
            }
            _ => {}
        }

        // now check if this is a call we should inline
        if let TypedExprKind::Call { callee, args } = &expr.kind {
            if let TypedExprKind::Identifier(name) = &callee.kind {
                if let Some(func) = self.functions.get(name).cloned() {
                    let aggressive = self.level == OptimizationLevel::Aggressive;
                    let decision = analysis.should_inline(name, aggressive, BLOAT_BUDGET);

                    match decision {
                        InlineDecision::Inline => {
                            if let Some(inlined) = self.expander.expand_call(&func, args, expr.span) {
                                *expr = inlined;
                                self.stats.functions_inlined += 1;
                            }
                        }
                        InlineDecision::Blocked(reason) => {
                            if let Some(info) = analysis.functions.get(name) {
                                if info.has_inline || info.has_inline_always {
                                    self.emit_warning(name, &func, reason);
                                }
                            }
                        }
                        InlineDecision::Skip => {}
                    }
                }
            }
        }
    }

    fn emit_warning(&mut self, name: &str, func: &TypedFunction, reason: BlockReason) {
        if self.warned_functions.contains(name) {
            return;
        }

        let kind = match reason {
            BlockReason::Recursive => WarningKind::InlineRecursive,
            BlockReason::MutualRecursion(cycle) => WarningKind::InlineMutualRecursion { cycle },
            BlockReason::HasCaptures => WarningKind::InlineHasCaptures,
        };

        let has_always = func.decorators.iter().any(|d| d.name == "inline_always");

        // @inline_always suppresses non-fatal warnings
        let is_fatal = matches!(
            kind,
            WarningKind::InlineRecursive
                | WarningKind::InlineMutualRecursion { .. }
                | WarningKind::InlineNativeFunction
        );
        if !is_fatal && has_always {
            return;
        }

        self.warned_functions.insert(name.to_string());
        let warning = Warning::new(kind, func.span).with_context(name);
        self.warnings.push(warning);
    }
}

impl OptimizationPass for FunctionInliner {
    fn name(&self) -> &'static str { "function_inline" }

    fn run(&mut self, program: &mut TypedProgram) -> OptimizationStats {
        if self.level == OptimizationLevel::None {
            return OptimizationStats::new();
        }

        self.stats = OptimizationStats::new();
        self.warnings.clear();
        self.warned_functions.clear();

        self.collect_functions(program);
        let analysis = ProgramAnalysis::analyze(program);

        for stmt in program.stmts.iter_mut() {
            self.inline_in_stmt(stmt, &analysis);
        }

        self.stats.clone()
    }
}

impl Default for FunctionInliner {
    fn default() -> Self {
        Self::new(OptimizationLevel::Standard)
    }
}
