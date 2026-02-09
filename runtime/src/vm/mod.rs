mod alloc;
mod args;
mod builtins;
mod call_api;
mod config;
mod config_access;
mod core;
mod errors;
mod execute;
mod frame;
mod frames;
mod gc;
mod globals;
mod init;
pub mod manual_heap;
mod native;
mod native_registry;
mod repl;
mod resources;

// Implementation modules (extend VM with impl blocks)
mod arithmetic;
mod call_data;
mod calls;
mod closures;
mod comparison;
mod dispatch;
mod helpers;
mod verifier;

pub use aelys_bytecode::{
    AelysClosure, AelysFunction, AelysString, AelysUpvalue, GcObject, GcRef, NativeFunction,
    ObjectKind, UpvalueLocation,
};
pub use aelys_bytecode::{
    BytecodeBuffer, Function, GlobalLayout, Heap, IntegerOverflowError, OpCode, UpvalueDescriptor,
    Value, decode_a, decode_b, decode_c,
};
pub use args::{VmArgsError, VmArgsParsed, parse_vm_args};
pub use builtins::{
    builtin_alloc, builtin_free, builtin_load, builtin_store, builtin_type, register_builtins,
};
pub use config::{VMCapabilities, VmConfig, VmConfigError};
pub use core::{
    CallSiteCacheEntry, MAX_CALL_SITE_SLOTS, MAX_FRAMES, MAX_NO_GC_DEPTH, MAX_REGISTERS,
    StepResult, VM,
};
pub use frame::CallFrame;
pub use manual_heap::{ManualHeap, ManualHeapGuard};
pub use native::{NativeFn, NativeFunctionImpl, build_native_vm_api};
