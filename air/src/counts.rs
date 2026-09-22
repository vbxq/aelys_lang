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

const UNROLL_LIMIT: u64 = 8;
const INLINE_LIMIT: u64 = 4;

pub struct Carriers<'a> {
    pub structs: &'a [AirStructDef],
    pub imported: &'a [AirStructDef],
    pub enums: &'a [AirEnumDef],
}

impl<'a> Carriers<'a> {
    pub fn merged(structs: &'a [AirStructDef], enums: &'a [AirEnumDef]) -> Carriers<'a> {
        Carriers {
            structs,
            imported: &[],
            enums,
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
                        other => self.carries(other, seen),
                    })
            }
            _ => false,
        }
    }

    fn enum_def(&self, name: &str) -> Option<&AirEnumDef> {
        self.enums.iter().find(|def| def.name == name)
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
            _ => {}
        }
        out
    }
}

enum Leaf {
    Field(String, AirType),
    Elements(AirType, u64),
    Variants(EnumRef, Vec<(u32, Vec<(u32, AirType)>)>),
}

pub fn count_callee(slot: &AirType, retain: bool) -> &'static str {
    match (matches!(slot, AirType::Str), retain) {
        (true, true) => STR_RETAIN,
        (true, false) => STR_RELEASE,
        (false, true) => DUP,
        (false, false) => DROP,
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
    name == STR_RETAIN || name == DUP
}

pub fn is_release(name: &str) -> bool {
    name == STR_RELEASE || name == DROP
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
        _ => None,
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
        AirType::Ptr(inner) | AirType::Array(inner, _) | AirType::Slice(inner) => {
            vec_elements(inner, out)
        }
        AirType::Enum(r) => r.args.iter().for_each(|a| vec_elements(a, out)),
        AirType::FnPtr { params, ret, .. } => {
            params.iter().for_each(|p| vec_elements(p, out));
            vec_elements(ret, out);
        }
        _ => {}
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
            if name != DUP && name != DROP {
                return true;
            }
            let retain = name == DUP;
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
            if glue.carriers().carries_string(&slot) {
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

    fn build(&mut self, name: &str, ty: &AirType, retain: bool) -> AirFunction {
        let mut body = GlueBody::new(AirType::Ptr(Box::new(ty.clone())));
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
        self.glue_for(ty, retain)
    }
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
                no_gc: false,
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
        if name == DUP || name == DROP {
            out.push(format!("`{name}` survived the resolution after mono"));
        } else if name == STR_RETAIN || name == STR_RELEASE {
            let slot = count_arg(args).and_then(|arg| counted_slot_type(arg, &types, &addr_of));
            if slot != Some(AirType::Str) {
                out.push(format!("`{name}` counts a slot that is not a string"));
            }
        }
    }
    out
}
