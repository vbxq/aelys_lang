use aelys_sema::TypedProgram;
use aelys_syntax::Span as SyntaxSpan;

use crate::AirFunction;

// the two entry symbols are ABI facts about the emitted module; codegen delegates here so the
// linker-visible names have exactly one definition
pub const USER_MAIN_SYMBOL: &str = "__aelys_main";
pub const NATIVE_ENTRY_SYMBOL: &str = "__aelys_user_main";

// the C runtime exports 33 `__aelys_*` symbols the compiler never sees, so the whole `__` space is
// reserved rather than any list of names the compiler could enumerate
pub const RESERVED_PREFIX: &str = "__";

pub fn function_symbol_name(function: &AirFunction) -> String {
    if !function.is_extern && function.name == "main" {
        USER_MAIN_SYMBOL.to_string()
    } else {
        function.name.clone()
    }
}

pub struct ReservedUserName {
    pub name: String,
    pub span: SyntaxSpan,
}

pub fn reserved_user_names(program: &TypedProgram) -> Vec<ReservedUserName> {
    let mut found = Vec::new();
    crate::bir::build::for_each_fn_decl(&program.stmts, &mut |func, _parent| {
        if func.name.starts_with(RESERVED_PREFIX) {
            found.push(ReservedUserName {
                name: func.name.clone(),
                span: func.span,
            });
        }
    });
    found
}
