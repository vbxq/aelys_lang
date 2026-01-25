// disassembler: bytecode -> .aasm text

use crate::bytecode::{Function, OpCode, decode_a, decode_b, decode_c};
use crate::heap::Heap;
use crate::object::{GcRef, ObjectKind};
use crate::value::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

macro_rules! writeln_ignore {
    ($dst:expr) => { let _ = writeln!($dst); };
    ($dst:expr, $($arg:tt)*) => { let _ = writeln!($dst, $($arg)*); };
}

#[derive(Debug, Clone, Default)]
pub struct DisassemblerOptions {
    pub include_line_info: bool,
}

pub fn disassemble(func: &Function, heap: Option<&Heap>) -> String {
    disassemble_with_options(func, heap, &DisassemblerOptions::default())
}

pub fn disassemble_with_options(
    func: &Function,
    heap: Option<&Heap>,
    options: &DisassemblerOptions,
) -> String {
    let mut output = String::new();
    let ctx = DisasmContext::new(heap, options);

    writeln_ignore!(output, "; Aelys Assembly (.aasm)");
    writeln_ignore!(output, "; Disassembled from bytecode");
    writeln_ignore!(output);
    writeln_ignore!(output, ".version 1");
    writeln_ignore!(output);

    let mut all_functions = Vec::new();
    collect_functions(func, &mut all_functions);

    for (idx, f) in all_functions.iter().enumerate() {
        if idx > 0 {
            writeln_ignore!(output);
        }
        ctx.disassemble_function(&mut output, f, idx);
    }

    output
}

pub fn disassemble_to_string(func: &Function, heap: Option<&Heap>) -> String { disassemble(func, heap) }

fn collect_functions<'a>(func: &'a Function, out: &mut Vec<&'a Function>) {
    out.push(func);
    for f in &func.nested_functions { collect_functions(f, out); }
}

struct DisasmContext<'a> {
    heap: Option<&'a Heap>,
    options: &'a DisassemblerOptions,
}

impl<'a> DisasmContext<'a> {
    fn new(heap: Option<&'a Heap>, options: &'a DisassemblerOptions) -> Self { Self { heap, options } }

    fn disassemble_function(&self, output: &mut String, func: &Function, func_idx: usize) {
        writeln_ignore!(output, "; !== FUN {}", func_idx);
        writeln_ignore!(output, ".function {}", func_idx);

        if let Some(name) = &func.name {
            writeln_ignore!(output, "  .name \"{}\"", escape_string(name));
        }
        writeln_ignore!(output, "  .arity {}", func.arity);
        writeln_ignore!(output, "  .registers {}", func.num_registers);

        // Output global names for ALL functions (needed for indexed global access and module loading)
        if !func.global_layout.names().is_empty() {
            writeln_ignore!(output);
            writeln_ignore!(output, "  .globals");
            for (idx, name) in func.global_layout.names().iter().enumerate() {
                writeln_ignore!(output, "    {}: \"{}\"", idx, escape_string(name));
            }
        }
        writeln_ignore!(output);

        if !func.constants.is_empty() {
            writeln_ignore!(output, "  .constants");
            for (idx, constant) in func.constants.iter().enumerate() {
                let const_str = self.format_constant(constant, &func.nested_functions);
                writeln_ignore!(output, "    {}: {}", idx, const_str);
            }
            writeln_ignore!(output);
        }

        // Output upvalue descriptors if present
        if !func.upvalue_descriptors.is_empty() {
            writeln_ignore!(output, "  .upvalues");
            for (idx, desc) in func.upvalue_descriptors.iter().enumerate() {
                let kind = if desc.is_local { "local" } else { "upvalue" };
                writeln_ignore!(output, "    {}: {} {}", idx, kind, desc.index);
            }
            writeln_ignore!(output);
        }

        if func.bytecode.is_empty() {
            writeln_ignore!(output, "  .code");
            writeln_ignore!(output, "    ; (empty)");
            return;
        }

        // two-pass: collect jump targets first, then disasm with labels
        let labels = self.collect_jump_targets(func.bytecode.as_slice());

        writeln_ignore!(output, "  .code");
        let mut skip_cache_words = 0usize;
        for (offset, &instr) in func.bytecode.iter().enumerate() {
            // Skip cache words (they follow CallGlobal, CallGlobalMono, CallGlobalNative)
            if skip_cache_words > 0 {
                skip_cache_words -= 1;
                continue;
            }

            // Emit label if this is a jump target
            if let Some(label) = labels.get(&offset) {
                writeln_ignore!(output, "  {}:", label);
            }

            let disasm = self.disassemble_instruction(instr, offset, &labels);

            // Check if this instruction has cache words following it
            let opcode = OpCode::from_u8((instr >> 24) as u8);
            if let Some(op) = opcode {
                match op {
                    OpCode::CallGlobal | OpCode::CallGlobalMono | OpCode::CallGlobalNative => {
                        skip_cache_words = 2; // Skip the 2 cache words
                    }
                    _ => {}
                }
            }

            if self.options.include_line_info {
                let line = func.get_line(offset);
                if line > 0 {
                    writeln_ignore!(output, "    {:04}: {:30} ; line {}", offset, disasm, line);
                } else {
                    writeln_ignore!(output, "    {:04}: {}", offset, disasm);
                }
            } else {
                writeln_ignore!(output, "    {:04}: {}", offset, disasm);
            }
        }
    }

    fn collect_jump_targets(&self, bytecode: &[u32]) -> HashMap<usize, String> {
        let mut targets = HashSet::new();

        for (offset, &instr) in bytecode.iter().enumerate() {
            let opcode = OpCode::from_u8((instr >> 24) as u8);

            if let Some(op) = opcode {
                match op {
                    OpCode::Jump | OpCode::JumpIf | OpCode::JumpIfNot => {
                        let (_, _, imm) = decode_b(instr);
                        let target = if imm >= 0 {
                            offset.wrapping_add(1).wrapping_add(imm as usize)
                        } else {
                            offset.wrapping_add(1).wrapping_sub((-imm) as usize)
                        };
                        targets.insert(target);
                    }
                    _ => {}
                }
            }
        }

        let mut sorted_targets: Vec<_> = targets.into_iter().collect();
        sorted_targets.sort();

        let mut labels = HashMap::new();
        for (idx, target) in sorted_targets.into_iter().enumerate() {
            labels.insert(target, format!("L{}", idx));
        }

        labels
    }

    fn disassemble_instruction(
        &self,
        instr: u32,
        offset: usize,
        labels: &HashMap<usize, String>,
    ) -> String {
        let opcode = match OpCode::from_u8((instr >> 24) as u8) {
            Some(op) => op,
            None => return format!(".word 0x{:08x}", instr),
        };

        match opcode {
            // Format A: 3 registers
            OpCode::Move => {
                let (_, a, b, _) = decode_a(instr);
                format!("Move      r{}, r{}", a, b)
            }
            OpCode::LoadNull => {
                let (_, a, _, _) = decode_a(instr);
                format!("LoadNull  r{}", a)
            }
            OpCode::LoadBool => {
                let (_, a, b, _) = decode_a(instr);
                format!("LoadBool  r{}, {}", a, b != 0)
            }
            OpCode::Add => {
                let (_, a, b, c) = decode_a(instr);
                format!("Add       r{}, r{}, r{}", a, b, c)
            }
            OpCode::Sub => {
                let (_, a, b, c) = decode_a(instr);
                format!("Sub       r{}, r{}, r{}", a, b, c)
            }
            OpCode::Mul => {
                let (_, a, b, c) = decode_a(instr);
                format!("Mul       r{}, r{}, r{}", a, b, c)
            }
            OpCode::Div => {
                let (_, a, b, c) = decode_a(instr);
                format!("Div       r{}, r{}, r{}", a, b, c)
            }
            OpCode::Mod => {
                let (_, a, b, c) = decode_a(instr);
                format!("Mod       r{}, r{}, r{}", a, b, c)
            }
            OpCode::Neg => {
                let (_, a, b, _) = decode_a(instr);
                format!("Neg       r{}, r{}", a, b)
            }
            OpCode::Eq => {
                let (_, a, b, c) = decode_a(instr);
                format!("Eq        r{}, r{}, r{}", a, b, c)
            }
            OpCode::Ne => {
                let (_, a, b, c) = decode_a(instr);
                format!("Ne        r{}, r{}, r{}", a, b, c)
            }
            OpCode::Lt => {
                let (_, a, b, c) = decode_a(instr);
                format!("Lt        r{}, r{}, r{}", a, b, c)
            }
            OpCode::Le => {
                let (_, a, b, c) = decode_a(instr);
                format!("Le        r{}, r{}, r{}", a, b, c)
            }
            OpCode::Gt => {
                let (_, a, b, c) = decode_a(instr);
                format!("Gt        r{}, r{}, r{}", a, b, c)
            }
            OpCode::Ge => {
                let (_, a, b, c) = decode_a(instr);
                format!("Ge        r{}, r{}, r{}", a, b, c)
            }
            OpCode::Not => {
                let (_, a, b, _) = decode_a(instr);
                format!("Not       r{}, r{}", a, b)
            }
            OpCode::Call => {
                let (_, dest, func, nargs) = decode_a(instr);
                format!("Call      r{}, r{}, {}", dest, func, nargs)
            }
            OpCode::Return => {
                let (_, a, _, _) = decode_a(instr);
                format!("Return    r{}", a)
            }
            OpCode::Return0 => "Return0".to_string(),
            OpCode::GetGlobal => {
                let (_, a, k, _) = decode_a(instr);
                format!("GetGlobal r{}, {}", a, k)
            }
            OpCode::SetGlobal => {
                let (_, a, k, _) = decode_a(instr);
                format!("SetGlobal r{}, {}", a, k)
            }
            OpCode::IncGlobalI => {
                let (_, a, k, b) = decode_c(instr);
                format!("IncGlobalI r{}, {}, {}", a, k, b)
            }
            OpCode::EnterNoGc => "EnterNoGc".to_string(),
            OpCode::ExitNoGc => "ExitNoGc".to_string(),
            OpCode::Alloc => {
                let (_, a, b, _) = decode_a(instr);
                format!("Alloc     r{}, r{}", a, b)
            }
            OpCode::Free => {
                let (_, a, _, _) = decode_a(instr);
                format!("Free      r{}", a)
            }
            OpCode::LoadMem => {
                let (_, a, b, c) = decode_a(instr);
                format!("LoadMem   r{}, r{}, r{}", a, b, c)
            }
            OpCode::LoadMemI => {
                let (_, a, b, c) = decode_a(instr);
                format!("LoadMemI  r{}, r{}, {}", a, b, c)
            }
            OpCode::StoreMem => {
                let (_, a, b, c) = decode_a(instr);
                format!("StoreMem  r{}, r{}, r{}", a, b, c)
            }
            OpCode::StoreMemI => {
                let (_, a, b, c) = decode_a(instr);
                format!("StoreMemI r{}, {}, r{}", a, b, c)
            }
            OpCode::Print => {
                let (_, a, _, _) = decode_a(instr);
                format!("Print     r{}", a)
            }

            // Format B: register + immediate
            OpCode::LoadI => {
                let (_, a, imm) = decode_b(instr);
                format!("LoadI     r{}, {}", a, imm)
            }
            OpCode::LoadK => {
                let (_, a, k) = decode_b(instr);
                format!("LoadK     r{}, {}", a, k)
            }
            OpCode::Jump => {
                let (_, _, imm) = decode_b(instr);
                let target = if imm >= 0 {
                    offset.wrapping_add(1).wrapping_add(imm as usize)
                } else {
                    offset.wrapping_add(1).wrapping_sub((-imm) as usize)
                };
                if let Some(label) = labels.get(&target) {
                    format!("Jump      {}", label)
                } else {
                    format!("Jump      @{}", target)
                }
            }
            OpCode::JumpIf => {
                let (_, a, imm) = decode_b(instr);
                let target = if imm >= 0 {
                    offset.wrapping_add(1).wrapping_add(imm as usize)
                } else {
                    offset.wrapping_add(1).wrapping_sub((-imm) as usize)
                };
                if let Some(label) = labels.get(&target) {
                    format!("JumpIf    r{}, {}", a, label)
                } else {
                    format!("JumpIf    r{}, @{}", a, target)
                }
            }
            OpCode::JumpIfNot => {
                let (_, a, imm) = decode_b(instr);
                let target = if imm >= 0 {
                    offset.wrapping_add(1).wrapping_add(imm as usize)
                } else {
                    offset.wrapping_add(1).wrapping_sub((-imm) as usize)
                };
                if let Some(label) = labels.get(&target) {
                    format!("JumpIfNot r{}, {}", a, label)
                } else {
                    format!("JumpIfNot r{}, @{}", a, target)
                }
            }

            // Closure opcodes
            OpCode::MakeClosure => {
                let (_, a, k, upval_count) = decode_a(instr);
                format!("MakeClosure r{}, k{}, {}", a, k, upval_count)
            }
            OpCode::GetUpval => {
                let (_, a, upval_idx, _) = decode_a(instr);
                format!("GetUpval  r{}, upval[{}]", a, upval_idx)
            }
            OpCode::SetUpval => {
                let (_, upval_idx, src, _) = decode_a(instr);
                format!("SetUpval  upval[{}], r{}", upval_idx, src)
            }
            OpCode::CloseUpvals => {
                let (_, from_reg, _, _) = decode_a(instr);
                format!("CloseUpvals r{}", from_reg)
            }
            OpCode::ForLoopI => {
                let (_, a, offset) = decode_b(instr);
                format!("ForLoopI  r{}, {}", a, offset)
            }
            OpCode::ForLoopIInc => {
                let (_, a, offset) = decode_b(instr);
                format!("ForLoopIInc r{}, {}", a, offset)
            }
            // Immediate arithmetic
            OpCode::AddI => {
                let (_, a, b, c) = decode_a(instr);
                format!("AddI      r{}, r{}, {}", a, b, c)
            }
            OpCode::SubI => {
                let (_, a, b, c) = decode_a(instr);
                format!("SubI      r{}, r{}, {}", a, b, c)
            }
            // Immediate comparison
            OpCode::LtImm => {
                let (_, a, imm) = decode_b(instr);
                format!("LtImm     r{}, {}", a, imm)
            }
            OpCode::LeImm => {
                let (_, a, imm) = decode_b(instr);
                format!("LeImm     r{}, {}", a, imm)
            }
            OpCode::GtImm => {
                let (_, a, imm) = decode_b(instr);
                format!("GtImm     r{}, {}", a, imm)
            }
            OpCode::GeImm => {
                let (_, a, imm) = decode_b(instr);
                format!("GeImm     r{}, {}", a, imm)
            }
            // While loop superinstruction
            OpCode::WhileLoopLt => {
                let (_, a, offset) = decode_b(instr);
                format!("WhileLoopLt r{}, {}", a, offset)
            }
            // Type-specialized integer arithmetic
            OpCode::AddII => {
                let (_, a, b, c) = decode_a(instr);
                format!("AddII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::SubII => {
                let (_, a, b, c) = decode_a(instr);
                format!("SubII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::MulII => {
                let (_, a, b, c) = decode_a(instr);
                format!("MulII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::DivII => {
                let (_, a, b, c) = decode_a(instr);
                format!("DivII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::ModII => {
                let (_, a, b, c) = decode_a(instr);
                format!("ModII     r{}, r{}, r{}", a, b, c)
            }
            // Type-specialized float arithmetic
            OpCode::AddFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("AddFF     r{}, r{}, r{}", a, b, c)
            }
            OpCode::SubFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("SubFF     r{}, r{}, r{}", a, b, c)
            }
            OpCode::MulFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("MulFF     r{}, r{}, r{}", a, b, c)
            }
            OpCode::DivFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("DivFF     r{}, r{}, r{}", a, b, c)
            }
            OpCode::ModFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("ModFF     r{}, r{}, r{}", a, b, c)
            }
            // Type-specialized integer comparisons
            OpCode::LtII => {
                let (_, a, b, c) = decode_a(instr);
                format!("LtII      r{}, r{}, r{}", a, b, c)
            }
            OpCode::LeII => {
                let (_, a, b, c) = decode_a(instr);
                format!("LeII      r{}, r{}, r{}", a, b, c)
            }
            OpCode::GtII => {
                let (_, a, b, c) = decode_a(instr);
                format!("GtII      r{}, r{}, r{}", a, b, c)
            }
            OpCode::GeII => {
                let (_, a, b, c) = decode_a(instr);
                format!("GeII      r{}, r{}, r{}", a, b, c)
            }
            OpCode::EqII => {
                let (_, a, b, c) = decode_a(instr);
                format!("EqII      r{}, r{}, r{}", a, b, c)
            }
            OpCode::NeII => {
                let (_, a, b, c) = decode_a(instr);
                format!("NeII      r{}, r{}, r{}", a, b, c)
            }
            // Type-specialized float comparisons
            OpCode::LtFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("LtFF      r{}, r{}, r{}", a, b, c)
            }
            OpCode::LeFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("LeFF      r{}, r{}, r{}", a, b, c)
            }
            OpCode::GtFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("GtFF      r{}, r{}, r{}", a, b, c)
            }
            OpCode::GeFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("GeFF      r{}, r{}, r{}", a, b, c)
            }
            OpCode::EqFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("EqFF      r{}, r{}, r{}", a, b, c)
            }
            OpCode::NeFF => {
                let (_, a, b, c) = decode_a(instr);
                format!("NeFF      r{}, r{}, r{}", a, b, c)
            }
            // Integer comparison with immediate
            OpCode::LtIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("LtIImm    r{}, r{}, {}", a, b, c)
            }
            OpCode::LeIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("LeIImm    r{}, r{}, {}", a, b, c)
            }
            OpCode::GtIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("GtIImm    r{}, r{}, {}", a, b, c)
            }
            OpCode::GeIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("GeIImm    r{}, r{}, {}", a, b, c)
            }
            // Global by index
            OpCode::GetGlobalIdx => {
                let (_, a, imm) = decode_b(instr);
                format!("GetGlobalIdx r{}, {}", a, imm)
            }
            OpCode::SetGlobalIdx => {
                let (_, a, imm) = decode_b(instr);
                format!("SetGlobalIdx {}, r{}", imm, a)
            }
            // Cached call
            OpCode::CallCached => {
                let (_, a, b, c) = decode_a(instr);
                format!("CallCached r{}, r{}, {}", a, b, c)
            }
            // CallGlobal - combined GetGlobalIdx + Call with inline caching
            OpCode::CallGlobal => {
                let (_, dest, global_idx, nargs) = decode_a(instr);
                format!("CallGlobal r{}, {}, {}", dest, global_idx, nargs)
            }
            // CallGlobalMono - fast path for cached global calls
            OpCode::CallGlobalMono => {
                let (_, dest, global_idx, nargs) = decode_a(instr);
                format!("CallGlobalMono r{}, {}, {}", dest, global_idx, nargs)
            }
            // CallGlobalNative - fast path for native function calls
            OpCode::CallGlobalNative => {
                let (_, dest, global_idx, nargs) = decode_a(instr);
                format!("CallGlobalNative r{}, {}, {}", dest, global_idx, nargs)
            }
            // CallUpval - combined GetUpval + Call (for recursive closures)
            OpCode::CallUpval => {
                let (_, dest, upval_idx, nargs) = decode_a(instr);
                format!("CallUpval r{}, upval[{}], {}", dest, upval_idx, nargs)
            }
            // TailCallUpval - tail call via upvalue (reuses stack frame)
            OpCode::TailCallUpval => {
                let (_, dest, upval_idx, nargs) = decode_a(instr);
                format!("TailCallUpval r{}, upval[{}], {}", dest, upval_idx, nargs)
            }

            // Guarded integer arithmetic
            OpCode::AddIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("AddIIG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::SubIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("SubIIG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::MulIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("MulIIG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::DivIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("DivIIG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::ModIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("ModIIG    r{}, r{}, r{}", a, b, c)
            }

            // Guarded float arithmetic
            OpCode::AddFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("AddFFG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::SubFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("SubFFG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::MulFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("MulFFG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::DivFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("DivFFG    r{}, r{}, r{}", a, b, c)
            }
            OpCode::ModFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("ModFFG    r{}, r{}, r{}", a, b, c)
            }

            // Guarded integer comparisons
            OpCode::LtIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("LtIIG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::LeIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("LeIIG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::GtIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("GtIIG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::GeIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("GeIIG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::EqIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("EqIIG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::NeIIG => {
                let (_, a, b, c) = decode_a(instr);
                format!("NeIIG     r{}, r{}, r{}", a, b, c)
            }

            // Guarded float comparisons
            OpCode::LtFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("LtFFG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::LeFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("LeFFG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::GtFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("GtFFG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::GeFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("GeFFG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::EqFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("EqFFG     r{}, r{}, r{}", a, b, c)
            }
            OpCode::NeFFG => {
                let (_, a, b, c) = decode_a(instr);
                format!("NeFFG     r{}, r{}, r{}", a, b, c)
            }

            // Generic bitwise operations
            OpCode::Shl => {
                let (_, a, b, c) = decode_a(instr);
                format!("Shl       r{}, r{}, r{}", a, b, c)
            }
            OpCode::Shr => {
                let (_, a, b, c) = decode_a(instr);
                format!("Shr       r{}, r{}, r{}", a, b, c)
            }
            OpCode::BitAnd => {
                let (_, a, b, c) = decode_a(instr);
                format!("BitAnd    r{}, r{}, r{}", a, b, c)
            }
            OpCode::BitOr => {
                let (_, a, b, c) = decode_a(instr);
                format!("BitOr     r{}, r{}, r{}", a, b, c)
            }
            OpCode::BitXor => {
                let (_, a, b, c) = decode_a(instr);
                format!("BitXor    r{}, r{}, r{}", a, b, c)
            }
            OpCode::BitNot => {
                let (_, a, b, _) = decode_a(instr);
                format!("BitNot    r{}, r{}", a, b)
            }

            // Type-specialized integer bitwise
            OpCode::ShlII => {
                let (_, a, b, c) = decode_a(instr);
                format!("ShlII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::ShrII => {
                let (_, a, b, c) = decode_a(instr);
                format!("ShrII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::AndII => {
                let (_, a, b, c) = decode_a(instr);
                format!("AndII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::OrII => {
                let (_, a, b, c) = decode_a(instr);
                format!("OrII      r{}, r{}, r{}", a, b, c)
            }
            OpCode::XorII => {
                let (_, a, b, c) = decode_a(instr);
                format!("XorII     r{}, r{}, r{}", a, b, c)
            }
            OpCode::NotI => {
                let (_, a, b, _) = decode_a(instr);
                format!("NotI      r{}, r{}", a, b)
            }

            // Bitwise with immediate
            OpCode::ShlIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("ShlIImm   r{}, r{}, {}", a, b, c)
            }
            OpCode::ShrIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("ShrIImm   r{}, r{}, {}", a, b, c)
            }
            OpCode::AndIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("AndIImm   r{}, r{}, {}", a, b, c)
            }
            OpCode::OrIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("OrIImm    r{}, r{}, {}", a, b, c)
            }
            OpCode::XorIImm => {
                let (_, a, b, c) = decode_a(instr);
                format!("XorIImm   r{}, r{}, {}", a, b, c)
            }
        }
    }

    fn format_constant(&self, value: &Value, _nested_functions: &[Function]) -> String {
        if value.is_null() {
            "null".to_string()
        } else if let Some(b) = value.as_bool() {
            format!("bool {}", b)
        } else if let Some(n) = value.as_int() {
            format!("int {}", n)
        } else if let Some(f) = value.as_float() {
            // Handle special float values
            if f.is_nan() {
                "float nan".to_string()
            } else if f.is_infinite() {
                if f.is_sign_positive() {
                    "float inf".to_string()
                } else {
                    "float -inf".to_string()
                }
            } else {
                format!("float {}", f)
            }
        } else if let Some(ptr) = value.as_ptr() {
            // Check if it's a nested function reference (marker pattern)
            if (ptr & (1 << 20)) != 0 {
                let func_idx = ptr & 0xFFFFF;
                format!("func @{}", func_idx + 1) // +1 because main is @0
            } else {
                if let Some(heap) = self.heap {
                    if let Some(obj) = heap.get(GcRef::new(ptr)) {
                        match &obj.kind {
                            ObjectKind::String(s) => {
                                format!("string \"{}\"", escape_string(s.as_str()))
                            }
                            ObjectKind::Function(f) => {
                                if let Some(name) = f.name() {
                                    format!("func \"{}\"", escape_string(name))
                                } else {
                                    "func <anonymous>".to_string()
                                }
                            }
                            ObjectKind::Native(n) => {
                                format!("native \"{}\"", escape_string(&n.name))
                            }
                            ObjectKind::Upvalue(_) => "upvalue".to_string(),
                            ObjectKind::Closure(_) => "closure".to_string(),
                        }
                    } else {
                        format!("ptr {}", ptr)
                    }
                } else {
                    format!("ptr {}", ptr)
                }
            }
        } else {
            format!("unknown 0x{:016x}", value.as_int().unwrap_or(0) as u64)
        }
    }
}

pub fn escape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\0' => result.push_str("\\0"),
            c if c.is_control() => {
                // Use \xNN for other control characters
                for byte in c.to_string().bytes() {
                    result.push_str(&format!("\\x{:02x}", byte));
                }
            }
            c => result.push(c),
        }
    }
    result
}
