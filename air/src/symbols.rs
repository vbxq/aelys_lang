// the two entry symbols are ABI facts about the emitted module; codegen delegates here so the
// linker-visible names have exactly one definition
pub const USER_MAIN_SYMBOL: &str = "__aelys_main";
pub const NATIVE_ENTRY_SYMBOL: &str = "__aelys_user_main";

use crate::AirFunction;

pub fn function_symbol_name(function: &AirFunction) -> String {
    if !function.is_extern && function.name == "main" {
        USER_MAIN_SYMBOL.to_string()
    } else {
        function.name.clone()
    }
}
