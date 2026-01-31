mod annotation;
mod code;
mod format;
mod kind;
mod message;

pub use format::format_warnings;
pub use kind::WarningKind;

use aelys_syntax::{Source, Span};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Warning {
    pub kind: WarningKind,
    pub span: Span,
    pub source: Option<Arc<Source>>,
    pub context: Option<String>,
}

impl Warning {
    pub fn new(kind: WarningKind, span: Span) -> Self {
        Self { kind, span, source: None, context: None }
    }

    pub fn with_source(mut self, source: Arc<Source>) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    pub fn code(&self) -> u16 {
        self.kind.code()
    }
}

#[derive(Debug, Clone, Default)]
pub struct WarningConfig {
    pub treat_as_error: bool,
    disabled: HashSet<String>,
    enabled: HashSet<String>,
}

impl WarningConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enable_all(&mut self) {
        self.enabled.clear();
        self.enabled.insert("all".into());
    }

    pub fn disable(&mut self, category: &str) {
        self.disabled.insert(category.to_string());
    }

    pub fn enable(&mut self, category: &str) {
        self.enabled.insert(category.to_string());
        self.disabled.remove(category);
    }

    pub fn is_enabled(&self, kind: &WarningKind) -> bool {
        let cat = kind.category();

        if self.disabled.contains("all") || self.disabled.contains(cat) {
            return false;
        }

        self.enabled.contains("all") || self.enabled.contains(cat) || self.enabled.is_empty()
    }

    pub fn parse_flag(&mut self, flag: &str) -> Result<(), String> {
        if flag.starts_with("no-") {
            self.disable(&flag[3..]);
            Ok(())
        } else if flag == "all" {
            self.enable_all();
            Ok(())
        } else if flag == "error" {
            self.treat_as_error = true;
            Ok(())
        } else {
            let valid = ["all", "inline", "unused", "deprecated", "shadow", "type"];
            if valid.contains(&flag) {
                self.enable(flag);
                Ok(())
            } else {
                Err(format!("unknown warning category: {}", flag))
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct WarningCollector {
    warnings: Vec<Warning>,
    config: WarningConfig,
}

impl WarningCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: WarningConfig) -> Self {
        Self { warnings: Vec::new(), config }
    }

    pub fn push(&mut self, warning: Warning) {
        if self.config.is_enabled(&warning.kind) {
            self.warnings.push(warning);
        }
    }

    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    pub fn into_warnings(self) -> Vec<Warning> {
        self.warnings
    }

    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }

    pub fn has_fatal(&self) -> bool {
        self.config.treat_as_error && !self.warnings.is_empty()
    }

    pub fn clear(&mut self) {
        self.warnings.clear();
    }

    pub fn take(&mut self) -> Vec<Warning> {
        std::mem::take(&mut self.warnings)
    }
}
