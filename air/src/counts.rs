use std::collections::{HashMap, HashSet};

use crate::{
    AirBlock, AirConst, AirEnumDef, AirFunction, AirIntSize, AirLocal, AirParam, AirProgram,
    AirStmt, AirStmtKind, AirStructDef, AirTerminator, AirType, BinOp, BlockId, Callee,
    CallingConv, EnumRef, FunctionAttribs, FunctionId, GcMode, InlineHint, LocalId, Operand, Place,
    Rvalue,
};

pub const STR_RETAIN: &str = "__aelys_str_retain";
pub const STR_RELEASE: &str = "__aelys_str_release";
pub const DUP: &str = "__aelys_dup";
pub const DROP: &str = "__aelys_drop";
pub const DUP_PARAM: &str = "__aelys_dup_param";
pub const DROP_PARAM: &str = "__aelys_drop_param";
pub const FN_RETAIN: &str = "__aelys_fn_retain";
pub const FN_RELEASE: &str = "__aelys_fn_release";
pub const RC_RELEASE: &str = "__aelys_rc_release";
pub const RC_RELEASE_SLOT: &str = "__aelys_rc_release_slot";
pub const VEC_RELEASE: &str = "__aelys_vec_release";

const UNROLL_LIMIT: u64 = 8;
const INLINE_LIMIT: u64 = 4;

pub struct Carriers<'a> {
    pub structs: &'a [AirStructDef],
    pub imported: &'a [AirStructDef],
    pub enums: &'a [AirEnumDef],
    pub imported_enums: &'a [AirEnumDef],
}

impl<'a> Carriers<'a> {
    pub fn merged(structs: &'a [AirStructDef], enums: &'a [AirEnumDef]) -> Carriers<'a> {
        Carriers {
            structs,
            imported: &[],
            enums,
            imported_enums: &[],
        }
    }
}

impl Carriers<'_> {
    /// a type holds a string by value through fields and elements, never behind a vec or a pointer
    pub fn carries_string(&self, ty: &AirType) -> bool {
        self.carries(ty, &mut HashSet::new())
    }

    pub fn counted(&self, ty: &AirType) -> bool {
        matches!(ty, AirType::Str | AirType::Param(_)) || self.carries_string(ty)
    }

    fn carries(&self, ty: &AirType, seen: &mut HashSet<String>) -> bool {
        match ty {
            // before mono a type parameter may become a string, so a type that holds one is counted
            AirType::Str | AirType::Param(_) => true,
            AirType::FnPtr { .. } => is_closure(ty),
            AirType::Array(inner, _) => self.carries(inner, seen),
            AirType::Struct(name) => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                self.find(name)
                    .is_some_and(|def| def.fields.iter().any(|f| self.carries(&f.ty, seen)))
            }
            AirType::Enum(r) => {
                let symbol = r.symbol();
                if !seen.insert(symbol.clone()) {
                    return false;
                }
                if let Some(def) = self.enum_def(&symbol) {
                    return def
                        .variants
                        .iter()
                        .flat_map(|v| v.payload.iter())
                        .any(|ty| self.carries(ty, seen));
                }
                // before the mono an instance names its template, whose charge holds the arguments
                let Some(def) = self.enum_def(&r.name) else {
                    return r.args.iter().any(|a| self.carries(a, seen));
                };
                def.variants
                    .iter()
                    .flat_map(|v| v.payload.iter())
                    .any(|ty| match ty {
                        AirType::Param(p) => r
                            .args
                            .get(p.0 as usize)
                            .is_none_or(|arg| self.carries(arg, seen)),
                        other @ (AirType::I8
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
                        | AirType::Array(..)
                        | AirType::Slice(_)
                        | AirType::Vec(_)
                        | AirType::FnPtr { .. }
                        | AirType::Opaque
                        | AirType::Void) => self.carries(other, seen),
                    })
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
            | AirType::Ptr(_)
            | AirType::Rc(_)
            | AirType::Slice(_)
            | AirType::Vec(_)
            | AirType::Opaque
            | AirType::Void => false,
        }
    }

    fn enum_def(&self, name: &str) -> Option<&AirEnumDef> {
        self.enums
            .iter()
            .chain(self.imported_enums.iter())
            .find(|def| def.name == name)
    }

    fn find(&self, name: &str) -> Option<&AirStructDef> {
        self.structs
            .iter()
            .chain(self.imported.iter())
            .find(|def| def.name == name)
    }

    fn leaves(&self, ty: &AirType) -> Vec<Leaf> {
        let mut out = Vec::new();
        match ty {
            AirType::Struct(name) => {
                let Some(def) = self.find(name) else {
                    return out;
                };
                for field in &def.fields {
                    if self.counted(&field.ty) {
                        out.push(Leaf::Field(field.name.clone(), field.ty.clone()));
                    }
                }
            }
            AirType::Array(inner, n) => {
                if self.counted(inner) {
                    out.push(Leaf::Elements((**inner).clone(), *n));
                }
            }
            AirType::Enum(r) => {
                let Some(def) = self.enum_def(&r.symbol()) else {
                    return out;
                };
                let carrying: Vec<(u32, Vec<(u32, AirType)>)> = def
                    .variants
                    .iter()
                    .map(|variant| {
                        let fields: Vec<(u32, AirType)> = variant
                            .payload
                            .iter()
                            .enumerate()
                            .filter(|(_, ty)| self.counted(ty))
                            .map(|(i, ty)| (i as u32, ty.clone()))
                            .collect();
                        (variant.tag, fields)
                    })
                    .filter(|(_, fields)| !fields.is_empty())
                    .collect();
                if !carrying.is_empty() {
                    out.push(Leaf::Variants(r.clone(), carrying));
                }
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
            | AirType::Slice(_)
            | AirType::Vec(_)
            | AirType::FnPtr { .. }
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void => {}
        }
        out
    }
}

enum Leaf {
    Field(String, AirType),
    Elements(AirType, u64),
    Variants(EnumRef, Vec<(u32, Vec<(u32, AirType)>)>),
}

pub fn is_closure(ty: &AirType) -> bool {
    matches!(
        ty,
        AirType::FnPtr {
            conv: CallingConv::Aelys,
            ..
        }
    )
}

pub fn count_callee(slot: &AirType, retain: bool) -> &'static str {
    if mentions_param(slot) {
        return if retain { DUP_PARAM } else { DROP_PARAM };
    }
    if is_closure(slot) {
        return if retain { FN_RETAIN } else { FN_RELEASE };
    }
    match (matches!(slot, AirType::Str), retain) {
        (true, true) => STR_RETAIN,
        (true, false) => STR_RELEASE,
        (false, true) => DUP,
        (false, false) => DROP,
    }
}

pub(crate) fn mentions_param(ty: &AirType) -> bool {
    match ty {
        AirType::Param(_) => true,
        AirType::Ptr(inner)
        | AirType::Rc(inner)
        | AirType::Array(inner, _)
        | AirType::Slice(inner)
        | AirType::Vec(inner) => mentions_param(inner),
        AirType::Enum(r) => r.args.iter().any(mentions_param),
        AirType::FnPtr { params, ret, .. } => {
            params.iter().any(mentions_param) || mentions_param(ret)
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
        | AirType::Opaque
        | AirType::Void => false,
    }
}

pub fn glue_name(ty: &AirType, retain: bool) -> String {
    format!(
        "__glue_{}${}",
        if retain { "dup" } else { "drop" },
        crate::mono::substitute::type_to_string(ty)
    )
}

pub fn is_retain(name: &str) -> bool {
    name == STR_RETAIN || name == DUP || name == DUP_PARAM || name == FN_RETAIN
}

pub fn is_release(name: &str) -> bool {
    name == STR_RELEASE || name == DROP || name == DROP_PARAM || name == FN_RELEASE
}

pub fn rc_drop_glue_name(ty: &AirType) -> String {
    format!(
        "__glue_rcdrop${}",
        crate::mono::substitute::type_to_string(ty)
    )
}

fn local_types(function: &AirFunction) -> HashMap<LocalId, AirType> {
    function
        .locals
        .iter()
        .map(|l| (l.id, l.ty.clone()))
        .chain(function.params.iter().map(|p| (p.id, p.ty.clone())))
        .collect()
}

fn addresses(function: &AirFunction) -> HashMap<LocalId, LocalId> {
    let mut out = HashMap::new();
    for stmt in function.blocks.iter().flat_map(|b| &b.stmts) {
        if let AirStmtKind::Assign {
            place: Place::Local(dst),
            rvalue: Rvalue::AddressOf(Place::Local(base)),
        } = &stmt.kind
        {
            out.insert(*dst, *base);
        }
    }
    out
}

fn count_arg(args: &[Operand]) -> Option<LocalId> {
    match args {
        [Operand::Copy(addr) | Operand::Move(addr)] => Some(*addr),
        _ => None,
    }
}

// the slot a count names: the local its address was taken from, or the pointee it is declared with
fn counted_slot_type(
    arg: LocalId,
    types: &HashMap<LocalId, AirType>,
    addr_of: &HashMap<LocalId, LocalId>,
) -> Option<AirType> {
    if let Some(base) = addr_of.get(&arg) {
        return types.get(base).cloned();
    }
    match types.get(&arg) {
        Some(AirType::Ptr(inner)) => Some((**inner).clone()),
        Some(
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
            | AirType::Rc(_)
            | AirType::Struct(_)
            | AirType::Enum(_)
            | AirType::Array(..)
            | AirType::Slice(_)
            | AirType::Vec(_)
            | AirType::FnPtr { .. }
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void,
        )
        | None => None,
    }
}

/// a string slot keeps its count, a carrier takes its glue, and anything else loses the call
pub fn resolve_counts(program: &mut AirProgram) {
    let structs = program.structs.clone();
    let enums = program.enums.clone();
    let mut glue = GlueSet::new(&structs, &enums, &program.functions);
    // a vec of carriers needs the glue of its element even where no count in the air asks for it
    let mut elements: Vec<AirType> = Vec::new();
    for function in &program.functions {
        let types = function
            .params
            .iter()
            .map(|p| &p.ty)
            .chain(function.locals.iter().map(|l| &l.ty))
            .chain(std::iter::once(&function.ret_ty));
        for ty in types {
            vec_elements(ty, &mut elements);
        }
    }
    for element in elements {
        if element != AirType::Str && glue.carriers().carries_string(&element) {
            glue.glue_for(&element, true);
            glue.glue_for(&element, false);
        }
    }
    for function in &mut program.functions {
        resolve_function(function, &mut glue);
    }
    let mut payloads: Vec<AirType> = Vec::new();
    for stmt in program
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.stmts.iter())
    {
        if let AirStmtKind::RcAlloc { ty, .. } = &stmt.kind
            && !payloads.contains(ty)
        {
            payloads.push(ty.clone());
        }
    }
    for ty in payloads {
        glue.payload_glue_for(&ty);
    }
    program.functions.extend(glue.functions);
}

fn vec_elements(ty: &AirType, out: &mut Vec<AirType>) {
    match ty {
        AirType::Vec(inner) => {
            if !out.contains(inner) {
                out.push((**inner).clone());
            }
            vec_elements(inner, out);
        }
        AirType::Ptr(inner)
        | AirType::Rc(inner)
        | AirType::Array(inner, _)
        | AirType::Slice(inner) => vec_elements(inner, out),
        AirType::Enum(r) => r.args.iter().for_each(|a| vec_elements(a, out)),
        AirType::FnPtr { params, ret, .. } => {
            params.iter().for_each(|p| vec_elements(p, out));
            vec_elements(ret, out);
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
        | AirType::Void => {}
    }
}

fn resolve_function(function: &mut AirFunction, glue: &mut GlueSet<'_>) {
    let types = local_types(function);
    let addr_of = addresses(function);
    let mut orphaned: HashSet<LocalId> = HashSet::new();
    for block in &mut function.blocks {
        block.stmts.retain_mut(|stmt| {
            let AirStmtKind::CallVoid {
                func: Callee::Named(name),
                args,
            } = &mut stmt.kind
            else {
                return true;
            };
            let param_origin = name == DUP_PARAM || name == DROP_PARAM;
            if name != DUP && name != DROP && !param_origin {
                return true;
            }
            let retain = name == DUP || name == DUP_PARAM;
            let Some(arg) = count_arg(args) else {
                return true;
            };
            let Some(slot) = counted_slot_type(arg, &types, &addr_of) else {
                return true;
            };
            if slot == AirType::Str {
                *name = if retain { STR_RETAIN } else { STR_RELEASE }.to_string();
                return true;
            }
            if is_closure(&slot) {
                *name = if retain { FN_RETAIN } else { FN_RELEASE }.to_string();
                return true;
            }
            if param_origin {
                if let Some(callee) = glue.param_glue_for(&slot, retain) {
                    *name = callee;
                    return true;
                }
            } else if glue.carriers().carries_string(&slot) {
                *name = glue.glue_for(&slot, retain);
                return true;
            }
            orphaned.insert(arg);
            false
        });
    }
    if orphaned.is_empty() {
        return;
    }
    let mut referenced = HashSet::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            crate::passes::dead_locals::collect_stmt_locals(&stmt.kind, &mut referenced);
        }
        crate::passes::dead_locals::collect_terminator_locals(&block.terminator, &mut referenced);
    }
    let dead: HashSet<LocalId> = orphaned.difference(&referenced).copied().collect();
    for block in &mut function.blocks {
        block.stmts.retain(|stmt| {
            !matches!(&stmt.kind, AirStmtKind::Assign { place: Place::Local(dst), .. }
                if dead.contains(dst))
        });
    }
    function.locals.retain(|l| !dead.contains(&l.id));
}

struct GlueSet<'a> {
    structs: &'a [AirStructDef],
    enums: &'a [AirEnumDef],
    imported: &'a [AirStructDef],
    made: HashMap<(String, bool), String>,
    functions: Vec<AirFunction>,
    next_function_id: u32,
}

impl<'a> GlueSet<'a> {
    fn new(
        structs: &'a [AirStructDef],
        enums: &'a [AirEnumDef],
        functions: &[AirFunction],
    ) -> Self {
        let next_function_id = functions.iter().map(|f| f.id.0).max().map_or(0, |m| m + 1);
        GlueSet {
            structs,
            enums,
            imported: &[],
            made: HashMap::new(),
            functions: Vec::new(),
            next_function_id,
        }
    }

    fn carriers(&self) -> Carriers<'a> {
        Carriers {
            structs: self.structs,
            imported: self.imported,
            enums: self.enums,
            imported_enums: &[],
        }
    }

    fn glue_for(&mut self, ty: &AirType, retain: bool) -> String {
        let key = (crate::mono::substitute::type_to_string(ty), retain);
        if let Some(name) = self.made.get(&key) {
            return name.clone();
        }
        let name = glue_name(ty, retain);
        self.made.insert(key, name.clone());
        let function = self.build(&name, ty, retain);
        self.functions.push(function);
        name
    }

    fn param_rc_paths(&self, ty: &AirType) -> Vec<crate::rc_paths::RcLeafPath> {
        match ty {
            AirType::Rc(_) => return vec![crate::rc_paths::RcLeafPath { steps: Vec::new() }],
            AirType::Struct(_) | AirType::Enum(_) => {}
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
            | AirType::Array(..)
            | AirType::Slice(_)
            | AirType::Vec(_)
            | AirType::FnPtr { .. }
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void => return Vec::new(),
        }
        match crate::rc_paths::rc_field_paths(ty, self.structs, self.enums) {
            crate::rc_paths::RcScan::Paths(paths) => paths,
            crate::rc_paths::RcScan::None
            | crate::rc_paths::RcScan::Undecidable(_)
            | crate::rc_paths::RcScan::RejectedMultiVariant(_) => Vec::new(),
        }
    }

    fn param_glue_for(&mut self, ty: &AirType, retain: bool) -> Option<String> {
        let strings = self.carriers().carries_string(ty);
        let paths = self.param_rc_paths(ty);
        if !strings && paths.is_empty() {
            return None;
        }
        if paths.is_empty() {
            return Some(self.glue_for(ty, retain));
        }
        let key = (
            format!("param:{}", crate::mono::substitute::type_to_string(ty)),
            retain,
        );
        if let Some(name) = self.made.get(&key) {
            return Some(name.clone());
        }
        let name = format!(
            "__glue_p{}${}",
            if retain { "dup" } else { "drop" },
            crate::mono::substitute::type_to_string(ty)
        );
        self.made.insert(key, name.clone());
        let mut body = GlueBody::new(AirType::Ptr(Box::new(ty.clone())));
        if strings {
            let callee = self.glue_for(ty, retain);
            body.call(&callee, body.param);
        }
        let count = if retain {
            "__aelys_rc_retain"
        } else {
            "__aelys_rc_release"
        };
        for path in &paths {
            let leaf = self.load_rc_leaf(&mut body, ty, path);
            body.emit(AirStmtKind::CallVoid {
                func: Callee::Named(count.to_string()),
                args: vec![Operand::Copy(leaf)],
            });
        }
        let id = FunctionId(self.next_function_id);
        self.next_function_id += 1;
        let calls = paths.len() as u64 + u64::from(strings);
        self.functions
            .push(body.finish(id, &name, calls <= INLINE_LIMIT));
        Some(name)
    }

    fn load_rc_leaf(
        &self,
        body: &mut GlueBody,
        ty: &AirType,
        path: &crate::rc_paths::RcLeafPath,
    ) -> LocalId {
        let param = body.param;
        self.load_rc_leaf_at(body, ty, path, param)
    }

    fn load_rc_leaf_at(
        &self,
        body: &mut GlueBody,
        ty: &AirType,
        path: &crate::rc_paths::RcLeafPath,
        addr: LocalId,
    ) -> LocalId {
        let mut cur = body.temp(ty.clone(), false);
        body.emit(AirStmtKind::Assign {
            place: Place::Local(cur),
            rvalue: Rvalue::Deref(Operand::Copy(addr)),
        });
        let mut cur_ty = ty.clone();
        for step in &path.steps {
            let (next_ty, rvalue) = match step {
                crate::rc_paths::RcPathStep::Field(field) => {
                    let owner = if let AirType::Struct(name) = &cur_ty {
                        self.structs.iter().find(|def| &def.name == name)
                    } else {
                        None
                    };
                    let field_ty = owner
                        .and_then(|def| def.fields.iter().find(|f| &f.name == field))
                        .map_or(AirType::Rc(Box::new(AirType::Void)), |f| f.ty.clone());
                    (
                        field_ty,
                        Rvalue::FieldAccess {
                            base: Operand::Copy(cur),
                            field: field.clone(),
                        },
                    )
                }
                crate::rc_paths::RcPathStep::EnumPayload {
                    enum_ref,
                    tag,
                    field_index,
                } => {
                    let symbol = enum_ref.symbol();
                    let payload_ty = self
                        .enums
                        .iter()
                        .find(|def| def.name == symbol)
                        .and_then(|def| def.variants.iter().find(|v| v.tag == *tag))
                        .and_then(|v| v.payload.get(*field_index as usize).cloned())
                        .unwrap_or(AirType::Rc(Box::new(AirType::Void)));
                    (
                        payload_ty,
                        Rvalue::EnumPayload {
                            enum_ref: enum_ref.clone(),
                            tag: *tag,
                            operand: Operand::Copy(cur),
                            field_index: *field_index,
                        },
                    )
                }
            };
            let next = body.temp(next_ty.clone(), false);
            body.emit(AirStmtKind::Assign {
                place: Place::Local(next),
                rvalue,
            });
            cur = next;
            cur_ty = next_ty;
        }
        cur
    }

    fn payload_glue_for(&mut self, ty: &AirType) {
        let mut body = GlueBody::new(AirType::Ptr(Box::new(ty.clone())));
        let param = body.param;
        let calls = self.release_payload(&mut body, ty, param);
        if calls == 0 {
            return;
        }
        let id = FunctionId(self.next_function_id);
        self.next_function_id += 1;
        self.functions
            .push(body.finish(id, &rc_drop_glue_name(ty), false));
    }

    fn release_payload(&mut self, body: &mut GlueBody, ty: &AirType, addr: LocalId) -> u64 {
        match ty {
            AirType::Rc(_) => {
                body.call(RC_RELEASE_SLOT, addr);
                return 1;
            }
            AirType::Ptr(_) => return 0,
            AirType::Vec(_) => return self.release_vec(body, ty, addr),
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
            | AirType::Enum(_)
            | AirType::Array(..)
            | AirType::Slice(_)
            | AirType::FnPtr { .. }
            | AirType::Param(_)
            | AirType::Opaque
            | AirType::Void => {}
        }
        let mut calls = 0;
        if self.carriers().counted(ty) {
            let callee = self.callee_for(ty, false);
            body.call(&callee, addr);
            calls += 1;
        }
        for path in self.param_rc_paths(ty) {
            let leaf = self.load_rc_leaf_at(body, ty, &path, addr);
            body.call(RC_RELEASE, leaf);
            calls += 1;
        }
        calls + self.release_vec_fields(body, ty, addr)
    }

    fn release_vec(&mut self, body: &mut GlueBody, ty: &AirType, addr: LocalId) -> u64 {
        if let AirType::Vec(elem) = ty
            && **elem != AirType::Str
            && self.carriers().carries_string(elem)
        {
            self.glue_for(elem, true);
            self.glue_for(elem, false);
        }
        body.call(VEC_RELEASE, addr);
        1
    }

    fn release_vec_fields(&mut self, body: &mut GlueBody, ty: &AirType, addr: LocalId) -> u64 {
        let AirType::Struct(name) = ty else {
            return 0;
        };
        let fields: Vec<(String, AirType)> = self
            .carriers()
            .find(name)
            .map(|def| {
                def.fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut calls = 0;
        for (field, field_ty) in fields {
            if !struct_holds_vec(
                &field_ty,
                &self.carriers(),
                &mut HashSet::from([name.clone()]),
            ) {
                continue;
            }
            let field_addr = body.address_of(Place::Field(addr, field), &field_ty);
            calls += if matches!(field_ty, AirType::Vec(_)) {
                self.release_vec(body, &field_ty, field_addr)
            } else {
                self.release_vec_fields(body, &field_ty, field_addr)
            };
        }
        calls
    }

    fn build(&mut self, name: &str, ty: &AirType, retain: bool) -> AirFunction {
        let mut body = GlueBody::new(AirType::Ptr(Box::new(ty.clone())));
        if is_closure(ty) {
            body.call(if retain { FN_RETAIN } else { FN_RELEASE }, body.param);
            let id = FunctionId(self.next_function_id);
            self.next_function_id += 1;
            return body.finish(id, name, true);
        }
        let leaves = self.carriers().leaves(ty);
        let calls: u64 = leaves
            .iter()
            .map(|leaf| match leaf {
                Leaf::Field(_, _) => 1,
                Leaf::Elements(_, n) if *n <= UNROLL_LIMIT => *n,
                Leaf::Elements(_, _) => u64::from(u32::MAX),
                Leaf::Variants(_, variants) => {
                    variants.iter().map(|(_, f)| f.len() as u64).sum::<u64>()
                }
            })
            .sum();
        for leaf in leaves {
            match leaf {
                Leaf::Field(field, field_ty) => {
                    let callee = self.callee_for(&field_ty, retain);
                    let addr = body.address_of(Place::Field(body.param, field), &field_ty);
                    body.call(&callee, addr);
                }
                Leaf::Elements(elem_ty, n) if n <= UNROLL_LIMIT => {
                    let callee = self.callee_for(&elem_ty, retain);
                    for i in 0..n {
                        let index = Operand::Const(AirConst::IntLiteral(i as i64));
                        let addr = body.address_of(Place::Index(body.param, index), &elem_ty);
                        body.call(&callee, addr);
                    }
                }
                Leaf::Elements(elem_ty, n) => {
                    let callee = self.callee_for(&elem_ty, retain);
                    body.count_each(&callee, &elem_ty, n);
                }
                Leaf::Variants(enum_ref, variants) => {
                    let prepared: Vec<(u32, Vec<(u32, AirType, String)>)> = variants
                        .into_iter()
                        .map(|(tag, fields)| {
                            let fields = fields
                                .into_iter()
                                .map(|(index, field_ty)| {
                                    let callee = self.callee_for(&field_ty, retain);
                                    (index, field_ty, callee)
                                })
                                .collect();
                            (tag, fields)
                        })
                        .collect();
                    body.count_variants(&enum_ref, &prepared, ty);
                }
            }
        }
        let id = FunctionId(self.next_function_id);
        self.next_function_id += 1;
        body.finish(id, name, calls <= INLINE_LIMIT)
    }

    fn callee_for(&mut self, ty: &AirType, retain: bool) -> String {
        if ty == &AirType::Str {
            return count_callee(ty, retain).to_string();
        }
        if is_closure(ty) {
            return if retain { FN_RETAIN } else { FN_RELEASE }.to_string();
        }
        self.glue_for(ty, retain)
    }
}

fn struct_holds_vec(ty: &AirType, carriers: &Carriers<'_>, seen: &mut HashSet<String>) -> bool {
    let AirType::Struct(name) = ty else {
        return matches!(ty, AirType::Vec(_));
    };
    if !seen.insert(name.clone()) {
        return false;
    }
    carriers.find(name).is_some_and(|def| {
        def.fields
            .iter()
            .any(|f| struct_holds_vec(&f.ty, carriers, seen))
    })
}

struct GlueBody {
    param: LocalId,
    param_ty: AirType,
    locals: Vec<AirLocal>,
    blocks: Vec<AirBlock>,
    stmts: Vec<AirStmt>,
    current: BlockId,
    next_local: u32,
    next_block: u32,
}

impl GlueBody {
    fn new(param_ty: AirType) -> Self {
        GlueBody {
            param: LocalId(0),
            param_ty,
            locals: Vec::new(),
            blocks: Vec::new(),
            stmts: Vec::new(),
            current: BlockId(0),
            next_local: 1,
            next_block: 1,
        }
    }

    fn temp(&mut self, ty: AirType, is_mut: bool) -> LocalId {
        let id = LocalId(self.next_local);
        self.next_local += 1;
        self.locals.push(AirLocal {
            id,
            ty,
            name: None,
            is_mut,
            span: None,
        });
        id
    }

    fn emit(&mut self, kind: AirStmtKind) {
        self.stmts.push(AirStmt { kind, span: None });
    }

    fn address_of(&mut self, place: Place, pointee: &AirType) -> LocalId {
        let addr = self.temp(AirType::Ptr(Box::new(pointee.clone())), false);
        self.emit(AirStmtKind::Assign {
            place: Place::Local(addr),
            rvalue: Rvalue::AddressOf(place),
        });
        addr
    }

    fn call(&mut self, callee: &str, addr: LocalId) {
        self.emit(AirStmtKind::CallVoid {
            func: Callee::Named(callee.to_string()),
            args: vec![Operand::Copy(addr)],
        });
    }

    fn block_id(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        id
    }

    fn seal(&mut self, terminator: AirTerminator, next: BlockId) {
        let stmts = std::mem::take(&mut self.stmts);
        self.blocks.push(AirBlock {
            id: self.current,
            stmts,
            terminator,
        });
        self.current = next;
    }

    fn count_variants(
        &mut self,
        enum_ref: &EnumRef,
        variants: &[(u32, Vec<(u32, AirType, String)>)],
        enum_ty: &AirType,
    ) {
        let value = self.temp(enum_ty.clone(), false);
        self.emit(AirStmtKind::Assign {
            place: Place::Local(value),
            rvalue: Rvalue::Deref(Operand::Copy(self.param)),
        });
        let tag = self.temp(AirType::I32, false);
        self.emit(AirStmtKind::Assign {
            place: Place::Local(tag),
            rvalue: Rvalue::EnumTag {
                enum_ref: enum_ref.clone(),
                operand: Operand::Copy(value),
            },
        });
        let blocks: Vec<BlockId> = variants.iter().map(|_| self.block_id()).collect();
        let exit = self.block_id();
        let targets = variants
            .iter()
            .zip(blocks.iter())
            .map(|((tag, _), block)| (AirConst::Int(i64::from(*tag), AirIntSize::I32), *block))
            .collect();
        self.seal(
            AirTerminator::Switch {
                discr: Operand::Copy(tag),
                targets,
                default: exit,
            },
            blocks[0],
        );
        for (position, ((variant_tag, fields), block)) in
            variants.iter().zip(blocks.iter()).enumerate()
        {
            debug_assert_eq!(self.current, *block);
            for (index, field_ty, callee) in fields {
                let leaf = self.temp(field_ty.clone(), false);
                self.emit(AirStmtKind::Assign {
                    place: Place::Local(leaf),
                    rvalue: Rvalue::EnumPayload {
                        enum_ref: enum_ref.clone(),
                        tag: *variant_tag,
                        operand: Operand::Copy(value),
                        field_index: *index,
                    },
                });
                let addr = self.address_of(Place::Local(leaf), field_ty);
                self.call(callee, addr);
            }
            let next = blocks.get(position + 1).copied().unwrap_or(exit);
            self.seal(AirTerminator::Goto(exit), next);
        }
    }

    // an array longer than the unroll limit counts in a loop, or the glue is one call per element
    fn count_each(&mut self, callee: &str, elem_ty: &AirType, n: u64) {
        let index = self.temp(AirType::I64, true);
        let cond = self.temp(AirType::Bool, false);
        let next = self.temp(AirType::I64, false);
        let header = self.block_id();
        let body = self.block_id();
        let exit = self.block_id();
        self.emit(AirStmtKind::Assign {
            place: Place::Local(index),
            rvalue: Rvalue::Use(Operand::Const(AirConst::IntLiteral(0))),
        });
        self.seal(AirTerminator::Goto(header), header);
        self.emit(AirStmtKind::Assign {
            place: Place::Local(cond),
            rvalue: Rvalue::BinaryOp(
                BinOp::Lt,
                Operand::Copy(index),
                Operand::Const(AirConst::IntLiteral(n as i64)),
            ),
        });
        self.seal(
            AirTerminator::Branch {
                cond: Operand::Copy(cond),
                then_block: body,
                else_block: exit,
            },
            body,
        );
        let addr = self.address_of(Place::Index(self.param, Operand::Copy(index)), elem_ty);
        self.call(callee, addr);
        self.emit(AirStmtKind::Assign {
            place: Place::Local(next),
            rvalue: Rvalue::BinaryOp(
                BinOp::Add,
                Operand::Copy(index),
                Operand::Const(AirConst::IntLiteral(1)),
            ),
        });
        self.emit(AirStmtKind::Assign {
            place: Place::Local(index),
            rvalue: Rvalue::Use(Operand::Copy(next)),
        });
        self.seal(AirTerminator::Goto(header), exit);
    }

    fn finish(mut self, id: FunctionId, name: &str, inline: bool) -> AirFunction {
        let end = self.block_id();
        self.seal(AirTerminator::Return(None), end);
        AirFunction {
            id,
            name: name.to_string(),
            gc_mode: GcMode::Managed,
            type_params: Vec::new(),
            params: vec![AirParam {
                id: self.param,
                ty: self.param_ty.clone(),
                name: "slot".to_string(),
                span: None,
            }],
            ret_ty: AirType::Void,
            locals: self.locals,
            blocks: self.blocks,
            is_extern: false,
            // the runtime calls the glue through a plain function pointer, so it takes the c abi
            calling_conv: CallingConv::C,
            attributes: FunctionAttribs {
                inline: if inline {
                    InlineHint::Always
                } else {
                    InlineHint::Default
                },
                no_gc: true,
                no_unwind: true,
                cold: false,
            },
            span: None,
        }
    }
}

// after resolution every count names a string slot, so a survivor is a compiler defect
pub fn unresolved_counts(function: &AirFunction) -> Vec<String> {
    let types = local_types(function);
    let addr_of = addresses(function);
    let mut out = Vec::new();
    for stmt in function.blocks.iter().flat_map(|b| &b.stmts) {
        let AirStmtKind::CallVoid {
            func: Callee::Named(name),
            args,
        } = &stmt.kind
        else {
            continue;
        };
        if name == DUP || name == DROP || name == DUP_PARAM || name == DROP_PARAM {
            out.push(format!("`{name}` survived the resolution after mono"));
        } else if name == STR_RETAIN || name == STR_RELEASE {
            let slot = count_arg(args).and_then(|arg| counted_slot_type(arg, &types, &addr_of));
            if slot != Some(AirType::Str) {
                out.push(format!("`{name}` counts a slot that is not a string"));
            }
        } else if name == FN_RETAIN || name == FN_RELEASE {
            let slot = count_arg(args).and_then(|arg| counted_slot_type(arg, &types, &addr_of));
            if !slot.as_ref().is_some_and(is_closure) {
                out.push(format!("`{name}` counts a slot that is not a closure"));
            }
        }
    }
    out
}
