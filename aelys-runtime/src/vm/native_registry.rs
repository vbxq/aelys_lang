use super::VM;
use crate::native::NativeModule;

impl VM {
    pub fn register_native_module(&mut self, name: String, module: NativeModule) {
        self.native_modules.insert(name, module);
    }
}
