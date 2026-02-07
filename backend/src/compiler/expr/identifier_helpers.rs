use super::Compiler;

impl Compiler {
    // "did you mean" suggestions for typos
    pub(super) fn generate_undefined_variable_hint(&self, var_name: &str) -> Option<String> {
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

        let similar_locals = self.find_similar_locals(var_name);
        if !similar_locals.is_empty() {
            return Some(format!("did you mean '{}'?", similar_locals.join("' or '")));
        }

        let similar_globals = self.find_similar_globals(var_name);
        if !similar_globals.is_empty() {
            return Some(format!(
                "did you mean '{}'?",
                similar_globals.join("' or '")
            ));
        }

        None
    }

    fn find_similar_locals(&self, var_name: &str) -> Vec<String> {
        let mut similar = Vec::new();

        for local in &self.locals {
            if Self::is_similar(var_name, &local.name) {
                similar.push(local.name.clone());
            }
        }

        similar.truncate(3);
        similar
    }

    fn find_similar_globals(&self, var_name: &str) -> Vec<String> {
        let mut similar = Vec::new();

        for global_name in self.globals.keys() {
            if !global_name.contains("::") && Self::is_similar(var_name, global_name) {
                similar.push(global_name.clone());
            }
        }

        similar.truncate(3);
        similar
    }

    fn is_similar(a: &str, b: &str) -> bool {
        if a == b {
            return false;
        }

        if a.len().abs_diff(b.len()) > 2 {
            return false;
        }

        Self::levenshtein_distance(a, b) <= 2
    }

    fn levenshtein_distance(a: &str, b: &str) -> usize {
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
}
