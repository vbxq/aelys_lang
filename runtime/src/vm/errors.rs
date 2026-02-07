use super::{ObjectKind, VM};
use aelys_common::error::{RuntimeError, RuntimeErrorKind, StackFrame};
use std::sync::Arc;

impl VM {
    pub fn runtime_error(&self, kind: RuntimeErrorKind) -> RuntimeError {
        RuntimeError::new(kind, self.build_stack_trace(), Arc::clone(&self.source))
    }

    pub fn runtime_error_with_hint(&self, kind: RuntimeErrorKind, var_name: &str) -> RuntimeError {
        let mut error = self.runtime_error(kind);

        if let RuntimeErrorKind::UndefinedVariable(_) = &error.kind {
            let hint = self.generate_undefined_variable_hint(var_name);
            if let Some(hint_msg) = hint {
                error.kind = RuntimeErrorKind::UndefinedVariable(format!(
                    "{}\n\nhint: {}",
                    var_name, hint_msg
                ));
            }
        }

        error
    }

    pub fn current_line(&self) -> u32 {
        if let Some(frame) = self.frames.last()
            && let Some(obj) = self.heap.get(frame.function())
            && let ObjectKind::Function(func) = &obj.kind
        {
            return func.function.get_line(frame.ip().saturating_sub(1));
        }
        0
    }

    fn generate_undefined_variable_hint(&self, var_name: &str) -> Option<String> {
        let mut found_modules = Vec::new();

        for global_name in self.globals.keys() {
            if let Some((module, func)) = global_name.split_once("::")
                && func == var_name
                && !found_modules.contains(&module)
            {
                found_modules.push(module);
            }
        }

        if !found_modules.is_empty() {
            let best_module = found_modules
                .iter()
                .find(|m| m.starts_with("std."))
                .or_else(|| found_modules.first())?;

            let display_module = if !best_module.contains('.') && !best_module.contains('/') {
                format!("std.{}", best_module)
            } else {
                best_module.to_string()
            };

            return Some(format!(
                "'{}' is available in {}, try: needs {} from {}",
                var_name, display_module, var_name, display_module
            ));
        }

        let similar_vars = self.find_similar_variables(var_name);
        if !similar_vars.is_empty() {
            return Some(format!("did you mean '{}'?", similar_vars.join("' or '")));
        }

        None
    }

    fn find_similar_variables(&self, var_name: &str) -> Vec<String> {
        let mut similar = Vec::new();

        for name in self.globals.keys() {
            if self.is_similar(var_name, name) {
                similar.push(name.clone());
            }
        }

        similar.truncate(3);
        similar
    }

    fn is_similar(&self, a: &str, b: &str) -> bool {
        if a == b {
            return false;
        }

        if a.len().abs_diff(b.len()) > 2 {
            return false;
        }

        self.levenshtein_distance(a, b) <= 2
    }

    fn levenshtein_distance(&self, a: &str, b: &str) -> usize {
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let a_len = a_chars.len();
        let b_len = b_chars.len();

        if a_len == 0 {
            return b_len;
        }
        if b_len == 0 {
            return a_len;
        }

        let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];

        for (i, row) in matrix.iter_mut().enumerate().take(a_len + 1) {
            row[0] = i;
        }
        for (j, val) in matrix[0].iter_mut().enumerate().take(b_len + 1) {
            *val = j;
        }

        for i in 1..=a_len {
            for j in 1..=b_len {
                let cost = if a_chars[i - 1] == b_chars[j - 1] {
                    0
                } else {
                    1
                };
                matrix[i][j] = std::cmp::min(
                    std::cmp::min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1),
                    matrix[i - 1][j - 1] + cost,
                );
            }
        }

        matrix[a_len][b_len]
    }

    fn build_stack_trace(&self) -> Vec<StackFrame> {
        let mut stack_trace = Vec::new();
        for frame in self.frames.iter().rev() {
            if let Some(obj) = self.heap.get(frame.function())
                && let ObjectKind::Function(func) = &obj.kind
            {
                let function_name = func.name().map(String::from);
                let line = if frame.ip() > 0 {
                    func.function.get_line(frame.ip() - 1)
                } else {
                    func.function.get_line(0)
                };
                stack_trace.push(StackFrame {
                    function_name,
                    line,
                    column: 1,
                });
            }
        }
        stack_trace
    }
}
