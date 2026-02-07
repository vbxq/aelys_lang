// flat switch dispatch, hot state in locals
// FIXME: computed goto would be faster but Rust doesn't support it

use super::cache::{decode_cache_words, encode_cache_words};
use super::decode::{decode_abc, decode_aimm};
use crate::vm::{AelysClosure, CallFrame, GcObject, GcRef, ObjectKind, VM, Value};
use aelys_bytecode::object::{AelysArray, AelysVec};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

impl VM {
    #[allow(unused_unsafe)]
    pub fn run_fast(&mut self) -> Result<Value, RuntimeError> {
        if self.frames.is_empty() {
            return Ok(Value::null());
        }

        // 16k registers = 128KB at 8 bytes per Value. More than enough for any
        // sane call depth. Pre-allocating avoids reallocation during execution.
        const REGISTER_STACK_SIZE: usize = 16384;
        if self.registers.capacity() < REGISTER_STACK_SIZE {
            self.registers
                .reserve(REGISTER_STACK_SIZE - self.registers.len());
        }
        if self.registers.len() < REGISTER_STACK_SIZE {
            self.registers.resize(REGISTER_STACK_SIZE, Value::null());
        }
        // SAFETY: Register stack is now fully allocated, pointers are stable

        // Load frame state into local variables for faster access
        let frame_idx = self.frames.len() - 1;
        let frame = &self.frames[frame_idx];
        let mut ip = frame.ip;
        let mut base = frame.base;
        let mut func_ref = frame.function;
        let mut bytecode_ptr = frame.bytecode_ptr;
        let mut bytecode_len = frame.bytecode_len;
        let mut constants_ptr = frame.constants_ptr;
        let mut constants_len = frame.constants_len;
        let mut upvalues_ptr = frame.upvalues_ptr;
        let mut upvalues_len = frame.upvalues_len;
        let mut current_frame_idx = frame_idx;
        let mut global_mapping_id = frame.global_mapping_id;

        loop {
            // Check end of bytecode
            if ip >= bytecode_len {
                self.frames.pop();
                if self.frames.is_empty() {
                    return Ok(Value::null());
                }
                // Reload frame state
                current_frame_idx = self.frames.len() - 1;
                let frame = &self.frames[current_frame_idx];
                ip = frame.ip;
                base = frame.base;
                func_ref = frame.function;
                bytecode_ptr = frame.bytecode_ptr;
                bytecode_len = frame.bytecode_len;
                constants_ptr = frame.constants_ptr;
                constants_len = frame.constants_len;
                upvalues_ptr = frame.upvalues_ptr;
                upvalues_len = frame.upvalues_len;
                let new_gmap = frame.global_mapping_id;
                if new_gmap != 0 && new_gmap != global_mapping_id {
                    global_mapping_id = self.prepare_globals_for_function(func_ref);
                }
                continue;
            }

            // Fetch instruction
            let instr = unsafe { *bytecode_ptr.add(ip) };
            ip += 1;

            let opcode_byte = (instr >> 24) as u8;

            // Get registers pointer (may change after resize, but we refresh it for calls)
            let mut regs_ptr = self.registers.as_mut_ptr();
            let regs_len = self.registers.len();

            // Bounds-checking macro - returns RuntimeError for out-of-bounds access
            macro_rules! check_reg {
                ($idx:expr) => {{
                    let idx = $idx;
                    if idx >= regs_len {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::InvalidRegister {
                            reg: idx,
                            max: regs_len,
                        }));
                    }
                }};
            }

            #[allow(unused_unsafe)]
            macro_rules! reg_get {
                ($idx:expr) => {{
                    let idx = $idx;
                    check_reg!(idx);
                    // SAFETY: bounds checked above
                    unsafe { *regs_ptr.add(idx) }
                }};
            }
            #[allow(unused_unsafe)]
            macro_rules! reg_ref {
                ($idx:expr) => {{
                    let idx = $idx;
                    check_reg!(idx);
                    // SAFETY: bounds checked above
                    unsafe { &*regs_ptr.add(idx) }
                }};
            }
            #[allow(unused_unsafe)]
            macro_rules! reg_set {
                ($idx:expr, $val:expr) => {{
                    let idx = $idx;
                    check_reg!(idx);
                    // SAFETY: bounds checked above
                    unsafe {
                        *regs_ptr.add(idx) = $val;
                    }
                }};
            }

            // Semantic dispatch - opcodes grouped by functionality
            match opcode_byte {
                // Load/Store operations: Move(0), LoadI(1), LoadK(2), LoadNull(3), LoadBool(4),
                // GetGlobalIdx(75), SetGlobalIdx(76)
                0..=4 | 75..=76 => {
                    include!("ops/load_store.inc");
                }

                // Arithmetic operations: Add(5), Sub(6), Mul(7), Div(8), Mod(9), Neg(10),
                // AddI(42), SubI(43), AddII(49)-ModII(53), AddFF(54)-ModFF(58),
                // AddIIG(82)-ModIIG(86), AddFFG(87)-ModFFG(91)
                5..=10 | 42..=43 | 49..=58 | 82..=91 => {
                    include!("ops/arithmetic.inc");
                }

                // Comparison operations: Eq(11), Ne(12), Lt(13), Le(14), Gt(15), Ge(16),
                // LtII(59)-NeII(64), LtFF(65)-NeFF(70), LtIImm(71)-GeIImm(74),
                // LtIIG(92)-NeIIG(97), LtFFG(98)-NeFFG(103)
                11..=16 | 59..=74 | 92..=103 => {
                    include!("ops/comparison.inc");
                }

                // Control flow operations: Not(17), Jump(18), JumpIf(19), JumpIfNot(20),
                // ForLoopI(40), ForLoopIInc(41), LtImm(44)-GeImm(47), WhileLoopLt(48)
                17..=20 | 40..=41 | 44..=48 => {
                    include!("ops/control_flow.inc");
                }

                // Call operations: Call(21), Return(22), Return0(23),
                // CallGlobal(77), CallGlobalMono(78), CallCached(79),
                // CallUpval(80), TailCallUpval(81), CallGlobalNative(104)
                21..=23 | 77..=81 | 104 => {
                    include!("ops/calls.inc");
                }

                // Global variable operations: GetGlobal(24), SetGlobal(25), IncGlobalI(39)
                24..=25 | 39 => {
                    include!("ops/globals.inc");
                }

                // Memory operations: EnterNoGc(26), ExitNoGc(27), Alloc(28), Free(29),
                // LoadMem(30), LoadMemI(31), StoreMem(32), StoreMemI(33)
                26..=33 => {
                    include!("ops/memory.inc");
                }

                // Closure operations: Print(34), MakeClosure(35), GetUpval(36),
                // SetUpval(37), CloseUpvals(38)
                34..=38 => {
                    include!("ops/closures.inc");
                }

                // Bitwise operations: Shl(105), Shr(106), BitAnd(107), BitOr(108), BitXor(109),
                // BitNot(110), ShlII(111)-XorII(115), NotI(116), ShlIImm(117)-XorIImm(121)
                105..=121 => {
                    include!("ops/bitwise.inc");
                }

                // Array and Vec operations: 130-175
                130..=175 => {
                    include!("ops/arrays.inc");
                }

                _ => {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidOpcode {
                        opcode: opcode_byte,
                    }));
                }
            }
        }
    }
}
