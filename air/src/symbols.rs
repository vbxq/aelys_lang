use std::collections::HashMap;

use aelys_sema::{InferType, TypedProgram};
use aelys_syntax::Span as SyntaxSpan;

use crate::{AirFunction, AirProgram, Span};

// the two entry symbols are abi facts about the emitted module; codegen delegates here so the
pub const USER_MAIN_SYMBOL: &str = "__aelys_main";
pub const NATIVE_ENTRY_SYMBOL: &str = "__aelys_user_main";

// whole `__` space is reserved rather than any list of names the compiler could enumerate
pub const RESERVED_PREFIX: &str = "__";

// a program may call these without declaring them, so sema pins their types and codegen must let them through
pub const BOOTSTRAP_BUILTIN_SYMBOLS: &[&str] = &["print", "println", "__aelys_collect"];

pub fn function_symbol_name(function: &AirFunction) -> String {
    if !function.is_extern && function.name == "main" {
        USER_MAIN_SYMBOL.to_string()
    } else {
        function.name.clone()
    }
}

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
    pub has_extern: bool,
}

pub fn duplicate_symbols(air: &AirProgram) -> Vec<DuplicateSymbol> {
    let mut order: Vec<String> = Vec::new();
    let mut by_symbol: HashMap<String, (Vec<Option<Span>>, usize)> = HashMap::new();
    for function in &air.functions {
        let symbol = function_symbol_name(function);
        let entry = by_symbol.entry(symbol.clone()).or_default();
        if entry.0.is_empty() {
            order.push(symbol);
        }
        entry.0.push(function.span);
        if !function.is_extern {
            entry.1 += 1;
        }
    }
    order
        .into_iter()
        .filter_map(|symbol| {
            let (spans, defined) = by_symbol.remove(&symbol)?;
            (spans.len() > 1 && defined > 0).then(|| DuplicateSymbol {
                has_extern: defined < spans.len(),
                symbol,
                spans,
            })
        })
        .collect()
}

pub struct DeclSite {
    pub span: SyntaxSpan,
    pub parent: Option<String>,
}

// a symbol can have fewer typed declarations than air functions: mono synthesises instances and the
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ForeignTypePosition {
    Parameter,
    Return,
}

pub struct ForeignSignatureViolation {
    pub function: String,
    pub what: String,
    pub spelling: String,
    pub reason: &'static str,
    pub span: SyntaxSpan,
}

pub fn foreign_type_rejection(
    ty: &InferType,
    position: ForeignTypePosition,
) -> Option<&'static str> {
    match ty {
        InferType::I8
        | InferType::I16
        | InferType::I32
        | InferType::I64
        | InferType::U8
        | InferType::U16
        | InferType::U32
        | InferType::U64
        | InferType::F32
        | InferType::F64
        | InferType::Bool => None,
        InferType::Null if position == ForeignTypePosition::Return => None,
        // a borrow promises only the referent's abi, so the referent answers in parameter position
        InferType::Ref { referent, .. } if position == ForeignTypePosition::Parameter => {
            foreign_type_rejection(referent, ForeignTypePosition::Parameter)
        }
        InferType::Ref { .. } => {
            Some("a reference the compiler did not prove; a foreign pointer is a `u64` handle")
        }
        InferType::Null => Some("`void` names c's absence of a value, so it is only a return type"),
        InferType::String => Some("an aelys string is a managed value, not a c `char *`"),
        InferType::Rc(_) => {
            Some("an `Rc<T>` lowers to a bare pointer, which would hand c a reference counted cell")
        }
        InferType::Function { .. } => {
            Some("an aelys function value carries an environment pointer, so it is not a c one")
        }
        InferType::Array(..) => Some("an array has no c layout the compiler promises"),
        InferType::Vec(_) => Some("a `vec` is a managed aelys value, not a c array"),
        InferType::Slice { .. } => Some("a slice is a pointer and a length, not a c pointer"),
        InferType::Struct(_) => Some("a struct has no declared c layout"),
        InferType::Enum(..) => Some("an enum has no declared c layout"),
        InferType::Var(_) | InferType::Dynamic => {
            Some("the type is not written, so there is no c type to declare")
        }
        _ => Some("the type is not one the external surface admits"),
    }
}

pub fn foreign_signature_violations(program: &TypedProgram) -> Vec<ForeignSignatureViolation> {
    let mut found = Vec::new();
    crate::bir::build::for_each_fn_decl(&program.stmts, &mut |func, _parent| {
        if func.foreign.is_none() {
            return;
        }
        for param in &func.params {
            if let Some(reason) = foreign_type_rejection(&param.ty, ForeignTypePosition::Parameter)
            {
                found.push(ForeignSignatureViolation {
                    function: func.name.clone(),
                    what: format!("parameter `{}`", param.name),
                    spelling: param.ty.to_string(),
                    reason,
                    span: param.span,
                });
            }
        }
        if let Some(reason) = foreign_type_rejection(&func.return_type, ForeignTypePosition::Return)
        {
            found.push(ForeignSignatureViolation {
                function: func.name.clone(),
                what: "the return type".to_string(),
                spelling: func.return_type.to_string(),
                reason,
                span: func.span,
            });
        }
    });
    found
}

pub const RUNTIME_DEFINED_SYMBOLS: &[&str] = &[
    "aelys_immix_alloc",
    "aelys_immix_block_count",
    "aelys_immix_free",
    "aelys_immix_realloc",
    "main",
];

pub const RUNTIME_IMPORTED_SYMBOLS: &[&str] = &[
    "abort",
    "exit",
    "fflush",
    "fprintf",
    "fputc",
    "free",
    "fwrite",
    "getenv",
    "malloc",
    "memcmp",
    "memcpy",
    "realloc",
    "snprintf",
    "stderr",
    "stdout",
    "strcmp",
];

const RESERVED_UNION_LEN: usize = RUNTIME_DEFINED_SYMBOLS.len() + RUNTIME_IMPORTED_SYMBOLS.len();

const fn reserved_union() -> [&'static str; RESERVED_UNION_LEN] {
    let mut out = [""; RESERVED_UNION_LEN];
    let mut i = 0;
    while i < RUNTIME_DEFINED_SYMBOLS.len() {
        out[i] = RUNTIME_DEFINED_SYMBOLS[i];
        i += 1;
    }
    let mut j = 0;
    while j < RUNTIME_IMPORTED_SYMBOLS.len() {
        out[i + j] = RUNTIME_IMPORTED_SYMBOLS[j];
        j += 1;
    }
    out
}

const RESERVED_UNION: [&str; RESERVED_UNION_LEN] = reserved_union();

pub const RUNTIME_RESERVED_SYMBOLS: &[&str] = &RESERVED_UNION;

pub struct ReservedRuntimeSymbol {
    pub name: String,
    pub symbol: String,
    pub span: Option<Span>,
}

pub fn reserved_runtime_symbols(
    air: &AirProgram,
    reserved_against_declarations: &[&str],
) -> Vec<ReservedRuntimeSymbol> {
    air.functions
        .iter()
        .filter_map(|function| {
            let symbol = function_symbol_name(function);
            // the runtime's own link cannot be diverted by a declaration, and what sits on the other side of the symbol is the binding author's affair
            let claimed = if function.is_extern {
                RUNTIME_DEFINED_SYMBOLS.contains(&symbol.as_str())
                    || reserved_against_declarations.contains(&symbol.as_str())
            } else {
                RUNTIME_RESERVED_SYMBOLS.contains(&symbol.as_str())
            };
            claimed.then(|| ReservedRuntimeSymbol {
                name: function.name.clone(),
                symbol,
                span: function.span,
            })
        })
        .collect()
}
