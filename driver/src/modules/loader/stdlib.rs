use super::types::{LoadResult, ModuleLoader};
use aelys_common::Result;
use aelys_runtime::VM;
use aelys_syntax::NeedsStmt;

impl ModuleLoader {
    pub(crate) fn load_std_module(&mut self, needs: &NeedsStmt, vm: &mut VM) -> Result<LoadResult> {
        let module_path_str = needs.path.join(".");

        if self.loaded_modules.contains_key(&module_path_str) {
            return self.load_loaded_std_module(needs, vm);
        }

        self.load_new_std_module(needs, vm)
    }
}
