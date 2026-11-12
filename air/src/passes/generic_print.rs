use crate::passes::instance_surface::InstanceSurfaceError;
use crate::{
    AirConst, AirFunction, AirProgram, AirStmtKind, AirTerminator, AirType, Callee, LocalId,
    Operand, Place, Rvalue, Span,
};
use std::collections::HashSet;

const TO_STRING: &str = "__aelys_to_string";
const STR_CONCAT: &str = "__aelys_str_concat";

fn operand_type<'a>(function: &'a AirFunction, op: &Operand) -> Option<&'a AirType> {
    let id = match op {
        Operand::Copy(id) | Operand::Move(id) => *id,
        Operand::Const(_) => return None,
    };
    function
        .locals
        .iter()
        .find(|l| l.id == id)
        .map(|l| &l.ty)
        .or_else(|| function.params.iter().find(|p| p.id == id).map(|p| &p.ty))
}

// the release pass leaves a to_string temp printed once unreleased, so that one stays a borrow
fn printed_once(function: &AirFunction, target: LocalId) -> bool {
    let mut uses = 0;
    let mut printed = false;
    for block in &function.blocks {
        for stmt in &block.stmts {
            crate::analysis::cfg::for_each_use_stmt(&stmt.kind, |id| {
                if id == target {
                    uses += 1;
                }
            });
            if let AirStmtKind::CallVoid {
                func: Callee::Named(name),
                args,
            } = &stmt.kind
                && (name == "print" || name == "println")
                && matches!(args.as_slice(), [Operand::Copy(id) | Operand::Move(id)] if *id == target)
            {
                printed = true;
            }
        }
        crate::analysis::cfg::for_each_use_term(&block.terminator, |id| {
            if id == target {
                uses += 1;
            }
        });
    }
    printed && uses == 1
}

// a string formatted through a type parameter is copied, so the part stays an owned temp
pub fn format_strings_by_copy(program: &mut AirProgram) {
    for function in &mut program.functions {
        let mut rewrite = Vec::new();
        for (b, block) in function.blocks.iter().enumerate() {
            for (s, stmt) in block.stmts.iter().enumerate() {
                if let AirStmtKind::Assign {
                    place: Place::Local(target),
                    rvalue:
                        Rvalue::Call {
                            func: Callee::Named(name),
                            args,
                        },
                } = &stmt.kind
                    && name == TO_STRING
                    && let [arg] = args.as_slice()
                    && matches!(operand_type(function, arg), Some(AirType::Str))
                {
                    rewrite.push((b, s, printed_once(function, *target)));
                }
            }
        }
        for (b, s, borrowed) in rewrite {
            let AirStmtKind::Assign { rvalue, .. } = &mut function.blocks[b].stmts[s].kind else {
                continue;
            };
            let Rvalue::Call { func, args } = rvalue else {
                continue;
            };
            if borrowed {
                let arg = args.remove(0);
                *rvalue = Rvalue::Use(arg);
            } else {
                *func = Callee::Named(STR_CONCAT.to_string());
                args.insert(0, Operand::Const(AirConst::Str(String::new())));
            }
        }
    }
}

fn source_name(name: &str) -> &str {
    let bare = name.strip_prefix("__q.").unwrap_or(name);
    bare.rsplit('.').next().unwrap_or(bare)
}

fn shown(ty: &AirType) -> String {
    match ty {
        AirType::Struct(name) => source_name(name).to_string(),
        AirType::Enum(r) => source_name(&r.name).to_string(),
        AirType::Array(inner, n) => format!("[{}; {n}]", shown(inner)),
        AirType::Slice(inner) => format!("&[{}]", shown(inner)),
        AirType::Vec(inner) => format!("Vec<{}>", shown(inner)),
        AirType::Rc(inner) => format!("Rc<{}>", shown(inner)),
        AirType::Ptr(inner) => format!("&{}", shown(inner)),
        AirType::FnPtr { .. } => "fn".to_string(),
        AirType::Str => "string".to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn refusal(ty: &AirType, in_format: bool) -> Option<String> {
    let detail = match ty {
        AirType::Struct(_) => "printing a struct is not supported yet; print its fields",
        AirType::Enum(_) if in_format => {
            "formatting an enum inside a string is not supported yet; print it on its own with \
             `println(e)` or match on it"
        }
        AirType::Array(..) => "printing an array is not supported yet; print its elements",
        AirType::Vec(_) => "printing a `Vec` is not supported yet; print its elements",
        AirType::Slice(_) => "printing a slice is not supported yet; print its elements",
        AirType::Rc(_) => "printing an `Rc` is not supported yet; print the value it points to",
        AirType::Ptr(inner) if matches!(inner.as_ref(), AirType::Void) => {
            return Some("printing the unit value is not supported yet".to_string());
        }
        AirType::Ptr(_) => {
            "printing a reference is not supported yet; print the value it points to"
        }
        AirType::FnPtr { .. } => "printing a function or closure is not supported yet",
        AirType::Void => return Some("printing the unit value is not supported yet".to_string()),
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
        | AirType::Enum(_)
        | AirType::Param(_)
        | AirType::Opaque => return None,
    };
    Some(format!("{detail} (found `{}`)", shown(ty)))
}

fn print_refusal(function: &AirFunction) -> Option<(String, Option<Span>)> {
    for block in &function.blocks {
        for stmt in &block.stmts {
            let (name, args) = match &stmt.kind {
                AirStmtKind::Assign {
                    rvalue:
                        Rvalue::Call {
                            func: Callee::Named(name),
                            args,
                        },
                    ..
                }
                | AirStmtKind::CallVoid {
                    func: Callee::Named(name),
                    args,
                } => (name.as_str(), args),
                _ => continue,
            };
            let in_format = match name {
                TO_STRING => true,
                "print" | "println" => false,
                _ => continue,
            };
            let [arg] = args.as_slice() else { continue };
            let Some(ty) = operand_type(function, arg) else {
                continue;
            };
            if let Some(message) = refusal(ty, in_format) {
                return Some((message, stmt.span.or(function.span)));
            }
        }
    }
    None
}

fn call_sites<'a>(program: &'a AirProgram, callee: &str) -> Option<(&'a str, Option<Span>)> {
    program.functions.iter().find_map(|f| {
        f.blocks.iter().find_map(|block| {
            let in_stmts = block.stmts.iter().find_map(|stmt| match &stmt.kind {
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
                } if name == callee => Some(stmt.span.or(f.span)),
                _ => None,
            });
            let in_term = match &block.terminator {
                AirTerminator::Invoke {
                    func: Callee::Named(name),
                    ..
                } if name == callee => Some(f.span),
                _ => None,
            };
            in_stmts.or(in_term).map(|span| (f.name.as_str(), span))
        })
    })
}

pub fn check_generic_print(program: &AirProgram) -> Vec<InstanceSurfaceError> {
    let instances: HashSet<_> = program.mono_instances.iter().map(|i| i.result).collect();
    let mut errors = Vec::new();
    for function in program
        .functions
        .iter()
        .filter(|f| instances.contains(&f.id))
    {
        let Some((message, inside)) = print_refusal(function) else {
            continue;
        };
        let mut target = function.name.as_str();
        let mut site = None;
        for _ in 0..program.functions.len() {
            let Some((caller, span)) = call_sites(program, target) else {
                break;
            };
            site = Some((caller, span));
            let caller_is_instance = program
                .functions
                .iter()
                .any(|f| f.name == caller && instances.contains(&f.id));
            if !caller_is_instance {
                break;
            }
            target = caller;
        }
        let (caller, span) = site.unwrap_or(("", inside));
        let message = if caller.is_empty() {
            message
        } else {
            let caller = if caller.starts_with("__lambda") {
                "a closure".to_string()
            } else {
                format!("`{caller}`")
            };
            format!("{message}; the call in {caller} instantiates the generic printing it")
        };
        errors.push(InstanceSurfaceError {
            code: "E0902",
            message,
            span: span.or(inside),
        });
    }
    errors
}
