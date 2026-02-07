use aelys_common::Result;
use aelys_common::error::AelysError;
use aelys_runtime::{VM, VmConfig};
use aelys_syntax::Source;

// REPL VM creation
pub fn new_vm() -> Result<VM> {
    new_vm_with_config(VmConfig::default(), Vec::new())
}

pub fn new_vm_with_config(config: VmConfig, program_args: Vec<String>) -> Result<VM> {
    VM::with_config_and_args(Source::new("<repl>", ""), config, program_args)
        .map_err(AelysError::Runtime)
}
