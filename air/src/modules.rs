use crate::{
    AirBlock, AirConst, AirEnumDef, AirFunction, AirGlobal, AirProgram, AirStmtKind, AirTerminator,
    AirType, Callee, EnumRef, FunctionId, Operand, Place, Rvalue,
};
use std::collections::HashSet;

const GLOBAL_GET: &str = "__aelys_global_get_";
const MONO_PREFIX: &str = "__mono_";
const GLOBAL_SET: &str = "__aelys_global_set_";

pub struct Qualifier {
    path: String,
    functions: HashSet<String>,
    globals: HashSet<String>,
    types: HashSet<String>,
}

impl Qualifier {
    pub fn new(program: &AirProgram, path: &str) -> Self {
        Self {
            path: path.to_string(),
            functions: program
                .functions
                .iter()
                .filter(|f| !f.is_extern)
                .map(|f| f.name.clone())
                .collect(),
            globals: program.globals.iter().map(|g| g.name.clone()).collect(),
            types: program
                .structs
                .iter()
                .map(|s| s.name.clone())
                .chain(program.enums.iter().map(|e| e.name.clone()))
                .collect(),
        }
    }

    fn value(&self, name: &str) -> String {
        aelys_sema::modules::qualify_value(&self.path, name)
    }

    fn type_name(&self, name: &str) -> String {
        aelys_sema::modules::qualify_type_name(&self.path, name)
    }

    fn symbol_ref(&self, name: &str) -> Option<String> {
        if let Some(rest) = name.strip_prefix(GLOBAL_GET) {
            if self.globals.contains(rest) {
                return Some(format!("{GLOBAL_GET}{}", self.value(rest)));
            }
            return None;
        }
        if let Some(rest) = name.strip_prefix(GLOBAL_SET) {
            if self.globals.contains(rest) {
                return Some(format!("{GLOBAL_SET}{}", self.value(rest)));
            }
            return None;
        }
        if self.functions.contains(name) || self.globals.contains(name) {
            return Some(self.value(name));
        }
        None
    }

    fn type_ref(&self, name: &str) -> Option<String> {
        if self.types.contains(name) {
            return Some(self.type_name(name));
        }
        // lowering pre-mangles a generic enum instance, name and type arguments alike, before
        if !name.starts_with(MONO_PREFIX) {
            return None;
        }
        let rewritten = self.rewrite_mangled(name);
        (rewritten != name).then_some(rewritten)
    }

    // an owned name inside a mangled string sits between delimiters, never inside another name
    fn rewrite_mangled(&self, name: &str) -> String {
        let mut out = String::with_capacity(name.len());
        let mut at = 0;
        while at < name.len() {
            match self.owned_segment_at(name, at) {
                Some(owned) => {
                    out.push_str(&self.type_name(owned));
                    at += owned.len();
                }
                None => {
                    let ch = name[at..].chars().next().expect("in bounds");
                    out.push(ch);
                    at += ch.len_utf8();
                }
            }
        }
        out
    }

    fn owned_segment_at(&self, name: &str, at: usize) -> Option<&String> {
        if at > 0 && !is_mangle_delimiter(name.as_bytes()[at - 1]) {
            return None;
        }
        self.types.iter().find(|owned| {
            name[at..].starts_with(owned.as_str())
                && name
                    .as_bytes()
                    .get(at + owned.len())
                    .is_none_or(|b| is_mangle_delimiter(*b))
        })
    }
}

pub fn qualify(program: &mut AirProgram, path: &str) {
    if path.is_empty() {
        return;
    }
    let q = Qualifier::new(program, path);

    for def in &mut program.structs {
        def.name = q.type_name(&def.name);
        for field in &mut def.fields {
            qualify_type(&mut field.ty, &q);
        }
    }
    for def in &mut program.enums {
        qualify_enum(def, &q);
    }
    for global in &mut program.globals {
        qualify_global(global, &q);
    }
    for function in &mut program.functions {
        qualify_function(function, &q);
    }
}

fn is_mangle_delimiter(b: u8) -> bool {
    b == b'_' || b == b'$'
}

fn qualify_enum(def: &mut AirEnumDef, q: &Qualifier) {
    def.name = q.type_name(&def.name);
    for variant in &mut def.variants {
        for ty in &mut variant.payload {
            qualify_type(ty, q);
        }
    }
}

fn qualify_global(global: &mut AirGlobal, q: &Qualifier) {
    global.name = q.value(&global.name);
    qualify_type(&mut global.ty, q);
    if let Some(init) = &mut global.init {
        qualify_const(init, q);
    }
}

fn qualify_function(function: &mut AirFunction, q: &Qualifier) {
    if !function.is_extern {
        function.name = q.value(&function.name);
    }
    for param in &mut function.params {
        qualify_type(&mut param.ty, q);
    }
    qualify_type(&mut function.ret_ty, q);
    for local in &mut function.locals {
        qualify_type(&mut local.ty, q);
    }
    for block in &mut function.blocks {
        qualify_block(block, q);
    }
}

fn qualify_block(block: &mut AirBlock, q: &Qualifier) {
    for stmt in &mut block.stmts {
        match &mut stmt.kind {
            AirStmtKind::Assign { place, rvalue } => {
                qualify_place(place, q);
                qualify_rvalue(rvalue, q);
            }
            AirStmtKind::GcAlloc { ty, .. }
            | AirStmtKind::Alloc { ty, .. }
            | AirStmtKind::RcAlloc { ty, .. } => qualify_type(ty, q),
            AirStmtKind::CallVoid { func, args } => {
                qualify_callee(func, q);
                for arg in args {
                    qualify_operand(arg, q);
                }
            }
            AirStmtKind::GcDrop(_)
            | AirStmtKind::ArenaCreate(_)
            | AirStmtKind::ArenaDestroy(_)
            | AirStmtKind::Free(_)
            | AirStmtKind::MemoryFence(_) => {}
        }
    }
    match &mut block.terminator {
        AirTerminator::Return(op) => {
            if let Some(op) = op {
                qualify_operand(op, q);
            }
        }
        AirTerminator::Branch { cond, .. } => qualify_operand(cond, q),
        AirTerminator::Switch { discr, targets, .. } => {
            qualify_operand(discr, q);
            for (value, _) in targets {
                qualify_const(value, q);
            }
        }
        AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            qualify_callee(func, q);
            for arg in args {
                qualify_operand(arg, q);
            }
            qualify_place(ret, q);
        }
        AirTerminator::Goto(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => {}
    }
}

fn qualify_rvalue(rvalue: &mut Rvalue, q: &Qualifier) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) | Rvalue::Len(op) => {
            qualify_operand(op, q)
        }
        Rvalue::BinaryOp(_, a, b) => {
            qualify_operand(a, q);
            qualify_operand(b, q);
        }
        Rvalue::Call { func, args } => {
            qualify_callee(func, q);
            for arg in args {
                qualify_operand(arg, q);
            }
        }
        Rvalue::StructInit { name, fields } => {
            if let Some(renamed) = q.type_ref(name) {
                *name = renamed;
            }
            for (_, op) in fields {
                qualify_operand(op, q);
            }
        }
        Rvalue::FieldAccess { base, .. } => qualify_operand(base, q),
        Rvalue::AddressOf(place) => qualify_place(place, q),
        Rvalue::Cast { operand, from, to } => {
            qualify_operand(operand, q);
            qualify_type(from, q);
            qualify_type(to, q);
        }
        Rvalue::Index { base, index } => {
            qualify_operand(base, q);
            qualify_operand(index, q);
        }
        Rvalue::EnumInit {
            enum_ref, payload, ..
        } => {
            qualify_enum_ref(enum_ref, q);
            for op in payload {
                qualify_operand(op, q);
            }
        }
        Rvalue::EnumTag { enum_ref, operand }
        | Rvalue::EnumPayload {
            enum_ref, operand, ..
        } => {
            qualify_enum_ref(enum_ref, q);
            qualify_operand(operand, q);
        }
        Rvalue::ClosureCreate { fn_name, env } => {
            if let Some(renamed) = q.symbol_ref(fn_name) {
                *fn_name = renamed;
            }
            qualify_operand(env, q);
        }
        Rvalue::SliceFromParts { ptr, len } => {
            qualify_operand(ptr, q);
            qualify_operand(len, q);
        }
    }
}

fn qualify_callee(callee: &mut Callee, q: &Qualifier) {
    if let Callee::Named(name) = callee {
        if let Some(renamed) = q.symbol_ref(name) {
            *name = renamed;
        }
    }
}

fn qualify_place(place: &mut Place, q: &Qualifier) {
    match place {
        Place::Global(name) => {
            if q.globals.contains(name.as_str()) {
                *name = q.value(name);
            }
        }
        Place::Index(_, op) => qualify_operand(op, q),
        Place::Local(_) | Place::Field(..) | Place::Deref(_) => {}
    }
}

fn qualify_operand(operand: &mut Operand, q: &Qualifier) {
    if let Operand::Const(value) = operand {
        qualify_const(value, q);
    }
}

fn qualify_const(value: &mut AirConst, q: &Qualifier) {
    match value {
        AirConst::FnRef(name) => {
            if let Some(renamed) = q.symbol_ref(name) {
                *name = renamed;
            }
        }
        AirConst::Enum {
            enum_ref, payload, ..
        } => {
            qualify_enum_ref(enum_ref, q);
            for item in payload {
                qualify_const(item, q);
            }
        }
        AirConst::Struct { name, fields } => {
            if let Some(renamed) = q.type_ref(name) {
                *name = renamed;
            }
            for (_, item) in fields {
                qualify_const(item, q);
            }
        }
        AirConst::Array(items) => {
            for item in items {
                qualify_const(item, q);
            }
        }
        AirConst::ZeroInit(ty) | AirConst::Undef(ty) => qualify_type(ty, q),
        _ => {}
    }
}

fn qualify_enum_ref(r: &mut EnumRef, q: &Qualifier) {
    if let Some(renamed) = q.type_ref(&r.name) {
        r.name = renamed;
    }
    for arg in &mut r.args {
        qualify_type(arg, q);
    }
}

fn qualify_type(ty: &mut AirType, q: &Qualifier) {
    match ty {
        AirType::Struct(name) => {
            if let Some(renamed) = q.type_ref(name) {
                *name = renamed;
            }
        }
        AirType::Enum(r) => qualify_enum_ref(r, q),
        AirType::Ptr(inner)
        | AirType::Array(inner, _)
        | AirType::Slice(inner)
        | AirType::Vec(inner) => qualify_type(inner, q),
        AirType::FnPtr { params, ret, .. } => {
            for param in params {
                qualify_type(param, q);
            }
            qualify_type(ret, q);
        }
        _ => {}
    }
}

pub fn merge(programs: Vec<AirProgram>) -> AirProgram {
    let mut merged = AirProgram {
        functions: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        globals: Vec::new(),
        source_files: Vec::new(),
        mono_instances: Vec::new(),
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: Default::default(),
    };

    let mut base = 0u32;
    for mut program in programs {
        let shift = base;
        for function in &mut program.functions {
            function.id = FunctionId(function.id.0 + shift);
            for block in &mut function.blocks {
                shift_direct_calls(block, shift);
            }
        }
        base += program.functions.len() as u32;

        merged.source_files.extend(program.source_files);
        merged.structs.extend(program.structs);
        merged.enums.extend(program.enums);
        merged.globals.extend(program.globals);
        merged.functions.extend(program.functions);
    }

    merged
}

fn shift_direct_calls(block: &mut AirBlock, shift: u32) {
    for stmt in &mut block.stmts {
        match &mut stmt.kind {
            AirStmtKind::Assign {
                rvalue: Rvalue::Call { func, .. },
                ..
            } => shift_callee(func, shift),
            AirStmtKind::CallVoid { func, .. } => shift_callee(func, shift),
            _ => {}
        }
    }
    if let AirTerminator::Invoke { func, .. } = &mut block.terminator {
        shift_callee(func, shift);
    }
}

fn shift_callee(callee: &mut Callee, shift: u32) {
    if let Callee::Direct(id) = callee {
        *id = FunctionId(id.0 + shift);
    }
}
