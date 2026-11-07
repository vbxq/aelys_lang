use crate::rc_paths::{RcScan, rc_field_paths};
use crate::{
    AirFunction, AirProgram, AirStmtKind, AirTerminator, AirType, Callee, Operand, Rvalue, Span,
};
use std::collections::{HashMap, HashSet};

pub const REF_CODE: &str = "E0714";
pub const RC_CODE: &str = "E0410";

pub struct InstanceSurfaceError {
    pub code: &'static str,
    pub message: String,
    pub span: Option<Span>,
}

#[derive(Clone)]
enum Finding {
    Reference(AirType),
    UncountedRc(AirType),
    RcInVec(AirType),
    NestedRcRead(AirType),
}

impl Finding {
    fn code(&self) -> &'static str {
        match self {
            Finding::Reference(_) => REF_CODE,
            Finding::UncountedRc(_) | Finding::RcInVec(_) | Finding::NestedRcRead(_) => RC_CODE,
        }
    }
}

#[derive(Clone)]
struct Verdict {
    finding: Finding,
    origin: String,
}

fn calls(function: &AirFunction) -> Vec<(&str, Option<Span>)> {
    let mut out = Vec::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign {
                    rvalue:
                        Rvalue::Call {
                            func: Callee::Named(name),
                            ..
                        },
                    ..
                }
                | AirStmtKind::CallVoid {
                    func: Callee::Named(name),
                    ..
                } => out.push((name.as_str(), stmt.span.or(function.span))),
                _ => {}
            }
        }
        if let AirTerminator::Invoke {
            func: Callee::Named(name),
            ..
        } = &block.terminator
        {
            out.push((name.as_str(), function.span));
        }
    }
    out
}

pub fn check_instance_surface(program: &AirProgram) -> Vec<InstanceSurfaceError> {
    let scan = Scan { program };
    let mut type_args: HashMap<&str, &[AirType]> = HashMap::new();
    let mut verdicts: HashMap<String, Verdict> = HashMap::new();
    for instance in &program.mono_instances {
        let Some(function) = program.functions.iter().find(|f| f.id == instance.result) else {
            continue;
        };
        type_args.insert(function.name.as_str(), instance.type_args.as_slice());
        if let Some(finding) = scan.function(function) {
            verdicts.insert(
                function.name.clone(),
                Verdict {
                    finding,
                    origin: function.name.clone(),
                },
            );
        }
    }
    if verdicts.is_empty() {
        return Vec::new();
    }
    let origins: Vec<String> = verdicts.keys().cloned().collect();
    loop {
        let mut grown = false;
        for function in &program.functions {
            if !type_args.contains_key(function.name.as_str())
                || verdicts.contains_key(&function.name)
            {
                continue;
            }
            let inherited = calls(function)
                .into_iter()
                .find_map(|(callee, _)| verdicts.get(callee).cloned());
            if let Some(verdict) = inherited {
                verdicts.insert(function.name.clone(), verdict);
                grown = true;
            }
        }
        if !grown {
            break;
        }
    }
    let mut errors = Vec::new();
    let mut reported: HashSet<&str> = HashSet::new();
    let mut covered: HashSet<&str> = HashSet::new();
    for caller in &program.functions {
        if type_args.contains_key(caller.name.as_str()) {
            continue;
        }
        for (callee, span) in calls(caller) {
            let Some(verdict) = verdicts.get(callee) else {
                continue;
            };
            if !reported.insert(callee) {
                continue;
            }
            covered.insert(verdict.origin.as_str());
            let args = type_args.get(callee).copied().unwrap_or(&[]);
            errors.push(InstanceSurfaceError {
                code: verdict.finding.code(),
                message: message(verdict, callee, &caller.name, args),
                span,
            });
        }
    }
    for origin in &origins {
        if covered.contains(origin.as_str()) {
            continue;
        }
        let verdict = &verdicts[origin];
        let site = program.functions.iter().find_map(|f| {
            calls(f)
                .into_iter()
                .find(|(callee, _)| callee == origin)
                .map(|(_, span)| (f.name.as_str(), span))
        });
        let (caller, span) = site.unwrap_or((
            "a function value",
            program
                .functions
                .iter()
                .find(|f| &f.name == origin)
                .and_then(|f| f.span),
        ));
        let args = type_args.get(origin.as_str()).copied().unwrap_or(&[]);
        errors.push(InstanceSurfaceError {
            code: verdict.finding.code(),
            message: message(verdict, origin, caller, args),
            span,
        });
    }
    errors
}

fn message(verdict: &Verdict, called: &str, caller: &str, args: &[AirType]) -> String {
    let args = args.iter().map(user_type).collect::<Vec<_>>().join(", ");
    let head = format!(
        "the call to `{}` in `{}` instantiates it with `{args}`",
        source_name(called),
        source_name(caller)
    );
    let which = if verdict.origin == called {
        ", which".to_string()
    } else {
        format!(", which through `{}`", source_name(&verdict.origin))
    };
    match &verdict.finding {
        Finding::Reference(container) => format!(
            "{head}{which} makes a `{}`: a reference or a view held inside an `Rc`, an array, a \
             `Vec`, an enum or a struct can outlive the value it borrows",
            user_type(container)
        ),
        Finding::UncountedRc(container) => format!(
            "{head}{which} makes a `{}`: an `Rc` held inside an array or inside an enum with more \
             than one variant is not counted yet",
            user_type(container)
        ),
        Finding::RcInVec(container) => format!(
            "{head}{which} makes a `{}`: a `Vec` does not release the `Rc` it holds yet",
            user_type(container)
        ),
        Finding::NestedRcRead(inner) => format!(
            "{head}{which} reads an `{}` out of an `Rc<{}>`: a nested `Rc` payload is not \
             supported yet, store one `Rc<T>` layer instead",
            user_type(inner),
            user_type(inner)
        ),
    }
}

fn source_name(symbol: &str) -> &str {
    symbol
        .strip_prefix("__mono_")
        .and_then(|rest| rest.split('$').next())
        .unwrap_or(symbol)
}

pub fn user_type(ty: &AirType) -> String {
    match ty {
        AirType::I8 => "i8".into(),
        AirType::I16 => "i16".into(),
        AirType::I32 => "i32".into(),
        AirType::I64 => "i64".into(),
        AirType::U8 => "u8".into(),
        AirType::U16 => "u16".into(),
        AirType::U32 => "u32".into(),
        AirType::U64 => "u64".into(),
        AirType::F32 => "f32".into(),
        AirType::F64 => "f64".into(),
        AirType::Bool => "bool".into(),
        AirType::Char => "char".into(),
        AirType::Str => "string".into(),
        AirType::Ptr(inner) => format!("&{}", user_type(inner)),
        AirType::Rc(inner) => format!("Rc<{}>", user_type(inner)),
        AirType::Struct(name) => name.clone(),
        AirType::Enum(r) if r.args.is_empty() => r.name.clone(),
        AirType::Enum(r) => format!(
            "{}<{}>",
            r.name,
            r.args.iter().map(user_type).collect::<Vec<_>>().join(", ")
        ),
        AirType::Array(inner, n) => format!("[{}; {n}]", user_type(inner)),
        AirType::Slice(inner) => format!("&[{}]", user_type(inner)),
        AirType::Vec(inner) => format!("Vec<{}>", user_type(inner)),
        AirType::FnPtr { params, ret, .. } => format!(
            "fn({}) -> {}",
            params.iter().map(user_type).collect::<Vec<_>>().join(", "),
            user_type(ret)
        ),
        AirType::Param(id) => format!("T{}", id.0),
        AirType::Opaque => "_".into(),
        AirType::Void => "()".into(),
    }
}

struct Scan<'a> {
    program: &'a AirProgram,
}

impl Scan<'_> {
    fn function(&self, function: &AirFunction) -> Option<Finding> {
        let types = function
            .params
            .iter()
            .map(|p| &p.ty)
            .chain(std::iter::once(&function.ret_ty))
            .chain(function.locals.iter().map(|l| &l.ty));
        let mut rc = None;
        for ty in types {
            if let Some(container) = self.ref_container(ty, &mut HashSet::new()) {
                return Some(Finding::Reference(container));
            }
            if rc.is_none() {
                rc = self.rc_container(ty, &mut HashSet::new());
            }
        }
        rc.or_else(|| Self::nested_rc_read(function))
    }

    fn nested_rc_read(function: &AirFunction) -> Option<Finding> {
        let local_ty = |id| {
            function
                .params
                .iter()
                .find(|p| p.id == id)
                .map(|p| &p.ty)
                .or_else(|| function.locals.iter().find(|l| l.id == id).map(|l| &l.ty))
        };
        function
            .blocks
            .iter()
            .flat_map(|b| &b.stmts)
            .find_map(|stmt| {
                let AirStmtKind::Assign {
                    rvalue: Rvalue::Deref(Operand::Copy(id) | Operand::Move(id)),
                    ..
                } = &stmt.kind
                else {
                    return None;
                };
                let Some(AirType::Rc(inner)) = local_ty(*id) else {
                    return None;
                };
                matches!(inner.as_ref(), AirType::Rc(_))
                    .then(|| Finding::NestedRcRead((**inner).clone()))
            })
    }

    fn payloads(&self, ty: &AirType) -> Vec<AirType> {
        match ty {
            AirType::Struct(name) => self
                .program
                .structs
                .iter()
                .find(|d| &d.name == name && !d.is_closure_env)
                .map(|d| d.fields.iter().map(|f| f.ty.clone()).collect())
                .unwrap_or_default(),
            AirType::Enum(r) => {
                let symbol = r.symbol();
                let mut out: Vec<AirType> = r.args.clone();
                if let Some(def) = self.program.enums.iter().find(|d| d.name == symbol) {
                    out.extend(def.variants.iter().flat_map(|v| v.payload.iter().cloned()));
                }
                out
            }
            AirType::Rc(inner) | AirType::Array(inner, _) | AirType::Vec(inner) => {
                vec![(**inner).clone()]
            }
            AirType::I8
            | AirType::I16
            | AirType::I32
            | AirType::I64
            | AirType::U8
            | AirType::U16
            | AirType::U32
            | AirType::U64
            | AirType::F32
            | AirType::F64
            | AirType::Bool
            | AirType::Char
            | AirType::Str
            | AirType::Ptr(_)
            | AirType::Slice(_)
            | AirType::FnPtr { .. }
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void => Vec::new(),
        }
    }

    fn key(ty: &AirType) -> Option<String> {
        match ty {
            AirType::Struct(name) => Some(name.clone()),
            AirType::Enum(r) => Some(r.symbol()),
            AirType::I8
            | AirType::I16
            | AirType::I32
            | AirType::I64
            | AirType::U8
            | AirType::U16
            | AirType::U32
            | AirType::U64
            | AirType::F32
            | AirType::F64
            | AirType::Bool
            | AirType::Char
            | AirType::Str
            | AirType::Ptr(_)
            | AirType::Rc(_)
            | AirType::Array(..)
            | AirType::Slice(_)
            | AirType::Vec(_)
            | AirType::FnPtr { .. }
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void => None,
        }
    }

    fn holds(&self, ty: &AirType, leaf: fn(&AirType) -> bool, seen: &mut HashSet<String>) -> bool {
        if leaf(ty) {
            return true;
        }
        if let Some(key) = Self::key(ty)
            && !seen.insert(key)
        {
            return false;
        }
        self.payloads(ty)
            .iter()
            .any(|inner| self.holds(inner, leaf, seen))
    }

    fn children(ty: &AirType) -> Vec<&AirType> {
        match ty {
            AirType::Ptr(inner)
            | AirType::Rc(inner)
            | AirType::Array(inner, _)
            | AirType::Slice(inner)
            | AirType::Vec(inner) => vec![inner],
            AirType::Enum(r) => r.args.iter().collect(),
            AirType::FnPtr { params, ret, .. } => {
                params.iter().chain(std::iter::once(ret.as_ref())).collect()
            }
            AirType::I8
            | AirType::I16
            | AirType::I32
            | AirType::I64
            | AirType::U8
            | AirType::U16
            | AirType::U32
            | AirType::U64
            | AirType::F32
            | AirType::F64
            | AirType::Bool
            | AirType::Char
            | AirType::Str
            | AirType::Struct(_)
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void => Vec::new(),
        }
    }

    fn ref_container(&self, ty: &AirType, seen: &mut HashSet<String>) -> Option<AirType> {
        let holds_ref = self.payloads(ty).iter().any(|inner| {
            self.holds(
                inner,
                |t| matches!(t, AirType::Ptr(_) | AirType::Slice(_)),
                seen,
            )
        });
        if holds_ref {
            return Some(ty.clone());
        }
        Self::children(ty)
            .into_iter()
            .find_map(|inner| self.ref_container(inner, &mut HashSet::new()))
    }

    fn rc_container(&self, ty: &AirType, seen: &mut HashSet<String>) -> Option<Finding> {
        if let AirType::Vec(inner) = ty
            && self.holds(inner, |t| matches!(t, AirType::Rc(_)), &mut HashSet::new())
        {
            return Some(Finding::RcInVec(ty.clone()));
        }
        let uncovered = match ty {
            AirType::Array(inner, _) => {
                self.holds(inner, |t| matches!(t, AirType::Rc(_)), &mut HashSet::new())
            }
            AirType::Enum(r) if !r.args.is_empty() => {
                self.holds(ty, |t| matches!(t, AirType::Rc(_)), &mut HashSet::new())
                    && !matches!(
                        rc_field_paths(ty, &self.program.structs, &self.program.enums),
                        RcScan::Paths(_)
                    )
            }
            AirType::I8
            | AirType::I16
            | AirType::I32
            | AirType::I64
            | AirType::U8
            | AirType::U16
            | AirType::U32
            | AirType::U64
            | AirType::F32
            | AirType::F64
            | AirType::Bool
            | AirType::Char
            | AirType::Str
            | AirType::Ptr(_)
            | AirType::Rc(_)
            | AirType::Struct(_)
            | AirType::Enum(_)
            | AirType::Slice(_)
            | AirType::Vec(_)
            | AirType::FnPtr { .. }
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void => false,
        };
        if uncovered {
            return Some(Finding::UncountedRc(ty.clone()));
        }
        if let Some(key) = Self::key(ty)
            && !seen.insert(key)
        {
            return None;
        }
        Self::children(ty)
            .into_iter()
            .chain(
                self.payloads(ty)
                    .iter()
                    .filter(|_| matches!(ty, AirType::Struct(_))),
            )
            .find_map(|inner| self.rc_container(inner, seen))
    }
}
