use crate::layout;
use crate::*;
use std::fmt::Write;

pub fn fmt_type(ty: &AirType) -> String {
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
        AirType::Str => "str".into(),
        AirType::Void => "void".into(),
        AirType::Ptr(inner) => format!("*{}", fmt_type(inner)),
        AirType::Struct(name) => name.clone(),
        AirType::Array(inner, len) => format!("[{}; {}]", fmt_type(inner), len),
        AirType::Slice(inner) => format!("[{}]", fmt_type(inner)),
        AirType::FnPtr { params, ret, .. } => {
            let ps: Vec<_> = params.iter().map(fmt_type).collect();
            format!("fn({}) -> {}", ps.join(", "), fmt_type(ret))
        }
        AirType::Param(id) => format!("T{}", id.0),
    }
}

fn fmt_int_size(s: &AirIntSize) -> &'static str {
    match s {
        AirIntSize::I8 => "i8",
        AirIntSize::I16 => "i16",
        AirIntSize::I32 => "i32",
        AirIntSize::I64 => "i64",
        AirIntSize::U8 => "u8",
        AirIntSize::U16 => "u16",
        AirIntSize::U32 => "u32",
        AirIntSize::U64 => "u64",
    }
}

fn fmt_float_size(s: &AirFloatSize) -> &'static str {
    match s {
        AirFloatSize::F32 => "f32",
        AirFloatSize::F64 => "f64",
    }
}

pub fn fmt_const(c: &AirConst) -> String {
    match c {
        AirConst::IntLiteral(v) => v.to_string(),
        AirConst::Int(v, size) => format!("{}{}", v, fmt_int_size(size)),
        AirConst::Float(v, size) => {
            let mut s = v.to_string();
            if v.is_finite() && !s.contains('.') {
                s.push_str(".0");
            }
            format!("{}{}", s, fmt_float_size(size))
        }
        AirConst::Bool(b) => b.to_string(),
        AirConst::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        AirConst::Null => "null".into(),
        AirConst::ZeroInit(ty) => format!("zeroinit {}", fmt_type(ty)),
        AirConst::Undef(ty) => format!("undef {}", fmt_type(ty)),
    }
}

fn local_type(func: &AirFunction, id: LocalId) -> &AirType {
    func.params
        .iter()
        .find(|p| p.id == id)
        .map(|p| &p.ty)
        .or_else(|| func.locals.iter().find(|l| l.id == id).map(|l| &l.ty))
        .unwrap_or_else(|| panic!("unknown local %{}", id.0))
}

fn fmt_operand(op: &Operand, func: &AirFunction) -> String {
    match op {
        Operand::Copy(id) | Operand::Move(id) => {
            format!("%{}: {}", id.0, fmt_type(local_type(func, *id)))
        }
        Operand::Const(c) => fmt_const(c),
    }
}

fn fmt_binop(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::Div => "div",
        BinOp::Rem => "rem",
        BinOp::Eq => "eq",
        BinOp::Ne => "ne",
        BinOp::Lt => "lt",
        BinOp::Le => "le",
        BinOp::Gt => "gt",
        BinOp::Ge => "ge",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::BitAnd => "bit_and",
        BinOp::BitOr => "bit_or",
        BinOp::BitXor => "bit_xor",
        BinOp::Shl => "shl",
        BinOp::Shr => "shr",
        BinOp::CheckedAdd => "checked_add",
        BinOp::CheckedSub => "checked_sub",
        BinOp::CheckedMul => "checked_mul",
    }
}

fn fmt_unop(op: &UnOp) -> &'static str {
    match op {
        UnOp::Neg => "neg",
        UnOp::Not => "not",
        UnOp::BitNot => "bit_not",
    }
}

fn func_name(id: FunctionId, program: &AirProgram) -> &str {
    program
        .functions
        .iter()
        .find(|f| f.id == id)
        .map(|f| f.name.as_str())
        .unwrap_or("<unknown>")
}

fn fmt_callee(callee: &Callee, program: &AirProgram) -> String {
    match callee {
        Callee::Direct(id) => func_name(*id, program).into(),
        Callee::Named(name) => name.clone(),
        Callee::FnPtr(id) => format!("*%{}", id.0),
        Callee::Extern(name, _) => name.clone(),
    }
}

fn fmt_args(args: &[Operand], func: &AirFunction) -> String {
    args.iter()
        .map(|a| fmt_operand(a, func))
        .collect::<Vec<_>>()
        .join(", ")
}

fn place_type(place: &Place, func: &AirFunction, program: &AirProgram) -> AirType {
    match place {
        Place::Local(id) => local_type(func, *id).clone(),
        Place::Field(id, name) => {
            if let AirType::Struct(sname) = local_type(func, *id) {
                program
                    .structs
                    .iter()
                    .find(|s| s.name == *sname)
                    .and_then(|s| s.fields.iter().find(|f| f.name == *name))
                    .map(|f| f.ty.clone())
                    .unwrap_or(AirType::Void)
            } else {
                AirType::Void
            }
        }
        Place::Deref(id) => match local_type(func, *id) {
            AirType::Ptr(inner) => (**inner).clone(),
            _ => AirType::Void,
        },
        Place::Index(id, _) => match local_type(func, *id) {
            AirType::Array(inner, _) | AirType::Slice(inner) => (**inner).clone(),
            _ => AirType::Void,
        },
    }
}

fn fmt_place(place: &Place, func: &AirFunction, program: &AirProgram) -> String {
    let ty = place_type(place, func, program);
    match place {
        Place::Local(id) => format!("%{}: {}", id.0, fmt_type(&ty)),
        Place::Field(id, name) => format!("%{}.{}: {}", id.0, name, fmt_type(&ty)),
        Place::Deref(id) => format!("*%{}: {}", id.0, fmt_type(&ty)),
        Place::Index(id, idx) => {
            format!("%{}[{}]: {}", id.0, fmt_operand(idx, func), fmt_type(&ty))
        }
    }
}

fn fmt_rvalue(rv: &Rvalue, func: &AirFunction, program: &AirProgram) -> String {
    match rv {
        Rvalue::Use(op) => format!("use {}", fmt_operand(op, func)),
        Rvalue::BinaryOp(op, l, r) => format!(
            "binop {} {}, {}",
            fmt_binop(op),
            fmt_operand(l, func),
            fmt_operand(r, func)
        ),
        Rvalue::UnaryOp(op, v) => format!("unop {} {}", fmt_unop(op), fmt_operand(v, func)),
        Rvalue::Call { func: callee, args } => format!(
            "call {}({})",
            fmt_callee(callee, program),
            fmt_args(args, func)
        ),
        Rvalue::StructInit { name, fields } => {
            let fs: Vec<_> = fields
                .iter()
                .map(|(n, op)| format!("{}: {}", n, fmt_operand(op, func)))
                .collect();
            format!("struct {} {{ {} }}", name, fs.join(", "))
        }
        Rvalue::FieldAccess { base, field } => {
            format!("field {} . {}", fmt_operand(base, func), field)
        }
        Rvalue::AddressOf(id) => {
            format!("addr %{}: {}", id.0, fmt_type(local_type(func, *id)))
        }
        Rvalue::Deref(op) => format!("deref {}", fmt_operand(op, func)),
        Rvalue::Cast { operand, to, .. } => {
            format!("cast {} -> {}", fmt_operand(operand, func), fmt_type(to))
        }
        Rvalue::Discriminant(op) => format!("discriminant {}", fmt_operand(op, func)),
    }
}

fn fmt_ordering(ord: &Ordering) -> &'static str {
    match ord {
        Ordering::Relaxed => "relaxed",
        Ordering::Acquire => "acquire",
        Ordering::Release => "release",
        Ordering::AcqRel => "acq_rel",
        Ordering::SeqCst => "seq_cst",
    }
}

fn fmt_stmt(stmt: &AirStmtKind, func: &AirFunction, program: &AirProgram) -> String {
    match stmt {
        AirStmtKind::Assign { place, rvalue } => format!(
            "{} = {}",
            fmt_place(place, func, program),
            fmt_rvalue(rvalue, func, program)
        ),
        AirStmtKind::GcAlloc { local, ty, arena } => {
            format!(
                "gc_alloc %{}: {}  [arena={}]",
                local.0,
                fmt_type(ty),
                arena.0
            )
        }
        AirStmtKind::GcDrop(id) => format!("gc_drop %{}", id.0),
        AirStmtKind::ArenaCreate(id) => format!("arena_create {}", id.0),
        AirStmtKind::ArenaDestroy(id) => format!("arena_destroy {}", id.0),
        AirStmtKind::Alloc { local, ty } => format!("alloc %{}: {}", local.0, fmt_type(ty)),
        AirStmtKind::Free(id) => format!("free %{}", id.0),
        AirStmtKind::CallVoid { func: callee, args } => format!(
            "call void {}({})",
            fmt_callee(callee, program),
            fmt_args(args, func)
        ),
        AirStmtKind::MemoryFence(ord) => format!("fence {}", fmt_ordering(ord)),
    }
}

fn fmt_terminator(term: &AirTerminator, func: &AirFunction, program: &AirProgram) -> String {
    match term {
        AirTerminator::Return(Some(op)) => format!("return {}", fmt_operand(op, func)),
        AirTerminator::Return(None) => "return void".into(),
        AirTerminator::Goto(block) => format!("goto block{}", block.0),
        AirTerminator::Branch {
            cond,
            then_block,
            else_block,
        } => format!(
            "branch {} -> block{} else block{}",
            fmt_operand(cond, func),
            then_block.0,
            else_block.0
        ),
        AirTerminator::Switch {
            discr,
            targets,
            default,
        } => {
            let cases: Vec<_> = targets
                .iter()
                .map(|(val, blk)| format!("{} -> block{}", fmt_const(val), blk.0))
                .collect();
            format!(
                "switch {}: {}, default -> block{}",
                fmt_operand(discr, func),
                cases.join(", "),
                default.0
            )
        }
        AirTerminator::Invoke {
            func: callee,
            args,
            ret,
            normal,
            unwind,
        } => format!(
            "invoke {}({}) -> {}  normal block{} unwind block{}",
            fmt_callee(callee, program),
            fmt_args(args, func),
            fmt_place(ret, func, program),
            normal.0,
            unwind.0
        ),
        AirTerminator::Unwind => "unwind".into(),
        AirTerminator::Unreachable => "unreachable".into(),
        AirTerminator::Panic { message, .. } => {
            format!(
                "panic \"{}\"",
                message.replace('\\', "\\\\").replace('"', "\\\"")
            )
        }
    }
}

fn type_layout_for_print(ty: &AirType, program: &AirProgram) -> (u32, u32) {
    match ty {
        AirType::Struct(name) => program
            .structs
            .iter()
            .find(|s| s.name == *name)
            .map(|d| struct_size_align(d, program))
            .unwrap_or((0, 1)),
        _ => {
            let l = layout::layout_of(ty);
            (l.size, l.align)
        }
    }
}

fn struct_size_align(def: &AirStructDef, program: &AirProgram) -> (u32, u32) {
    if def.fields.is_empty() {
        return (0, 1);
    }
    if def.fields.iter().all(|f| f.offset.is_some()) {
        let mut max_align: u32 = 1;
        let mut end: u32 = 0;
        for f in &def.fields {
            let (fs, fa) = type_layout_for_print(&f.ty, program);
            max_align = max_align.max(fa);
            end = end.max(f.offset.unwrap() + fs);
        }
        ((end + max_align - 1) & !(max_align - 1), max_align)
    } else {
        let mut offset: u32 = 0;
        let mut max_align: u32 = 1;
        for f in &def.fields {
            let (fs, fa) = type_layout_for_print(&f.ty, program);
            offset = (offset + fa - 1) & !(fa - 1);
            offset += fs;
            max_align = max_align.max(fa);
        }
        ((offset + max_align - 1) & !(max_align - 1), max_align)
    }
}

fn write_struct(out: &mut String, def: &AirStructDef, program: &AirProgram) {
    let _ = write!(out, "struct {}", def.name);
    if def.is_closure_env {
        out.push_str("  [closure_env]");
    }
    out.push('\n');
    for f in &def.fields {
        let off = match f.offset {
            Some(o) => o.to_string(),
            None => "?".into(),
        };
        let _ = writeln!(out, "  {}: {}  @ {}", f.name, fmt_type(&f.ty), off);
    }
    let (size, align) = struct_size_align(def, program);
    let _ = writeln!(out, "  [size={}, align={}]", size, align);
}

fn write_block(out: &mut String, block: &AirBlock, func: &AirFunction, program: &AirProgram) {
    let _ = writeln!(out, "  block{}:", block.id.0);
    for stmt in &block.stmts {
        let _ = writeln!(out, "    {}", fmt_stmt(&stmt.kind, func, program));
    }
    let _ = writeln!(
        out,
        "    {}",
        fmt_terminator(&block.terminator, func, program)
    );
}

fn write_function(out: &mut String, func: &AirFunction, program: &AirProgram) {
    let _ = write!(out, "fn {}", func.name);
    if !func.type_params.is_empty() {
        let tps: Vec<_> = func
            .type_params
            .iter()
            .map(|p| format!("T{}", p.0))
            .collect();
        let _ = write!(out, "<{}>", tps.join(", "));
    }
    let params: Vec<_> = func
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, fmt_type(&p.ty)))
        .collect();
    let _ = write!(out, "({}) -> {}", params.join(", "), fmt_type(&func.ret_ty));
    if func.is_extern {
        out.push_str("  [extern]");
    } else {
        let gc = match func.gc_mode {
            GcMode::Managed => "managed",
            GcMode::Manual => "manual",
        };
        let conv = match func.calling_conv {
            CallingConv::Aelys => "aelys",
            CallingConv::C => "c",
            CallingConv::Rust => "rust",
        };
        let _ = write!(out, "  [{}, {}]", gc, conv);
    }
    out.push('\n');
    if func.is_extern {
        return;
    }
    if !func.locals.is_empty() {
        out.push_str("  locals:\n");
        for l in &func.locals {
            let _ = writeln!(out, "    %{}: {}", l.id.0, fmt_type(&l.ty));
        }
    }
    for block in &func.blocks {
        write_block(out, block, func, program);
    }
}

pub fn print_program(program: &AirProgram) -> String {
    let mut out = String::new();
    for (i, def) in program.structs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write_struct(&mut out, def, program);
    }
    if !program.structs.is_empty() && !program.functions.is_empty() {
        out.push('\n');
    }
    for (i, func) in program.functions.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write_function(&mut out, func, program);
    }
    if !program.mono_instances.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("# mono instances\n");
        for inst in &program.mono_instances {
            let orig = func_name(inst.original, program);
            let result = func_name(inst.result, program);
            let types: Vec<_> = inst.type_args.iter().map(fmt_type).collect();
            let _ = writeln!(out, "{}[{}] -> {}", orig, types.join(", "), result);
        }
    }
    out
}

pub fn print_function(func: &AirFunction, program: &AirProgram) -> String {
    let mut out = String::new();
    write_function(&mut out, func, program);
    out
}

pub fn print_block(block: &AirBlock, program: &AirProgram) -> String {
    let func = program
        .functions
        .iter()
        .find(|f| f.blocks.iter().any(|b| std::ptr::eq(b, block)))
        .expect("block not found in any function in the program");
    let mut out = String::new();
    write_block(&mut out, block, func, program);
    out
}
