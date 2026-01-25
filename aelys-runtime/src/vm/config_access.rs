use super::VM;
use super::config::{VMCapabilities, VmConfig};
use aelys_syntax::Source;
use std::sync::Arc;

impl VM {
    pub fn capabilities(&self) -> &VMCapabilities {
        &self.config.capabilities
    }

    pub fn set_capabilities(&mut self, capabilities: VMCapabilities) {
        self.config.capabilities = capabilities;
    }

    pub fn config(&self) -> &VmConfig {
        &self.config
    }

    pub fn program_args(&self) -> &[String] {
        &self.program_args
    }

    pub fn set_script_path(&mut self, path: String) {
        self.script_path = Some(path);
    }

    pub fn script_path(&self) -> Option<&str> {
        self.script_path.as_deref()
    }

    pub fn source(&self) -> &Arc<Source> {
        &self.source
    }
}
