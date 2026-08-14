use std::collections::HashMap;

use aelys_sema::TypedProgram;
use aelys_syntax::Span as SyntaxSpan;

use crate::{AirFunction, AirProgram, Span};

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

// the source name a user writes for a symbol codegen will emit under a different one
pub fn symbol_for_source_name(name: &str) -> String {
    if name == "main" {
        USER_MAIN_SYMBOL.to_string()
    } else {
        name.to_string()
    }
}

pub struct DuplicateSymbol {
    pub symbol: String,
    pub spans: Vec<Option<Span>>,
}

// keyed on the emitted symbol, not the air name: `main` and `__aelys_main` share an image and a
// name-keyed check misses exactly that pair
pub fn duplicate_symbols(air: &AirProgram) -> Vec<DuplicateSymbol> {
    let mut order: Vec<String> = Vec::new();
    let mut by_symbol: HashMap<String, Vec<Option<Span>>> = HashMap::new();
    for function in &air.functions {
        if function.is_extern {
            continue;
        }
        let symbol = function_symbol_name(function);
        let entry = by_symbol.entry(symbol.clone()).or_default();
        if entry.is_empty() {
            order.push(symbol);
        }
        entry.push(function.span);
    }
    order
        .into_iter()
        .filter_map(|symbol| {
            let spans = by_symbol.remove(&symbol)?;
            (spans.len() > 1).then(|| DuplicateSymbol { symbol, spans })
        })
        .collect()
}

pub struct DeclSite {
    pub span: SyntaxSpan,
    pub parent: Option<String>,
}

// a symbol can have fewer typed declarations than air functions: mono synthesises instances and the
// compiler's own lambdas have no `fn` to point at, so the caller must index this with `.get`
pub fn decl_sites_by_symbol(program: &TypedProgram) -> HashMap<String, Vec<DeclSite>> {
    let mut sites: HashMap<String, Vec<DeclSite>> = HashMap::new();
    crate::bir::build::for_each_fn_decl(&program.stmts, &mut |func, parent| {
        sites
            .entry(symbol_for_source_name(&func.name))
            .or_default()
            .push(DeclSite {
                span: func.span,
                parent: parent.map(|p| p.to_string()),
            });
    });
    sites
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
