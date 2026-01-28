# Changelog

All notable changes to Aelys, roughly grouped by version. I don't always tag releases perfectly, so this is reconstructed from git history

## 0.19.x - Array and Vec Implementations

**0.19.2-a**
- `needs std.io` now imports symbols directly (`print()` and `io.print()` both work)

**0.19.1-a**
- Added generic type syntax support in function parameters (`fn foo(arr: array<int>)`)

**0.19.0-a**
- Added array and vec implementations to the VM

## 0.18.x - Native Binary Data Manipulation 

This update adds real memory manipulation for @no_gc mode

**0.18.6-a**
- Fixed call site cache using stale entries after global mutation
- Fixed ASM mode ignoring `--allow-caps` flags

**0.18.5-a**
- Fixed mutex poison panic in GlobalLayout cache
- Fixed IncGlobalI silently ignoring out-of-bounds index

**0.18.4-a**
- Fixed auto-semicolon breaking `if { } else { }` on separate lines

**0.18.3-a**
- Fixed register counting and verifier to skip cache words after CallGlobal opcodes
- Removed unsound Send/Sync from BytecodeBuffer (UnsafeCell patching)

**0.18.2-a**
- Fixed assembler/runtime mismatch for memory opcodes (Alloc, LoadMem, StoreMem)

**0.18.1-a**
- Fixed register index overflow in loop instruction verification (ForLoopI, WhileLoopLt)
- Fixed globals sync ignoring null values (caused inconsistency between indexed/named access)

**0.18.0-a**
- New `std.bytes` module for binary data manipulation

## 0.17.x - Performance & Security Hardening

This was a big release because we have split the project into multiple different crates.  
Emphasis also on performance and security.  

**0.17.12-a**
- Fixed nested function marker collision with heap pointers (now uses dedicated NaN tag)

**0.17.11-a**
- Fixed OOB read/write vulnerability in CallGlobal opcodes (verifier now validates cache words)
- Fixed integer truncation vulnerability in call site cache (verifier now validates function size limits)
- Fixed use-after-free vulnerability in call site cache (cache invalidated on global mutation)

**0.17.10-a**
- Fixed use-after-move error in module compiler (`compile.rs`)
- top-level if-expressions with else now return values instead of null
- top-level blocks now return the value of their last expression

**0.17.9-a**
- Fixed critical call site cache corruption in multi-module programs (slot_id collision)
- Global call site slot allocation across module boundaries
- Removed unused `global_cache` field (dead code cleanup)
- Automated version synchronization

**0.17.8-a**
- Fixed `--allow-caps` and `--deny-caps` flags not actually enabling/disabling VM capabilities
- Added comprehensive tests for capability flags

**0.17.7-a**
- Add support for && and || syntax
- Add 360 new test for stdlib

**0.17.6-a**
- Direct opcodes for `alloc`/`free`/`load`/`store` (~45% faster)
- Fixed disassembler output for memory opcodes
- Fixed REPL "forgetting" variables (wrong optimization level used)
- (First public release on github here!)

**0.17.5-a**
- Syntax highlighting fixes for various editors
- Minor bugfixes

**0.17.3-a**
- Added unused variable elimination optimization pass
- Improved for loop handling in optimizer

**0.17.2-a**
- Combined optimization test cases
- Fixed some optimizer edge cases

**0.17.0-a** - Project Restructure
- Major codebase reorganization
- Consolidated dispatch and ops call files
- Fixed module loading in REPL
- Exported VM and Value types in public API for embedding

## 0.16.x

**0.16.0-a** - Bitwise Operators
- Added bitwise operators: `&`, `|`, `^`, `~`, `<<`, `>>`
- Function calling and caching mechanism for embedded usage

## 0.15.x - Inline Caching & Security

This was a big release focused on performance and security.

**0.15.20-a**
- Documentation about safety
- Integer overflow in offsets fix
- Native transmute safety improvements

**0.15.14-a - 0.15.16-a** - Security Hardening
- Register bounds checking
- Bytecode patching bounds validation
- Cache slot DoS prevention
- Module path duplication fix
- Compiler clone epidemic cleanup
- 48-bit integer documentation

**0.15.4-a - 0.15.6-a** - CallGlobalNative
- Added `CallGlobalNative` opcode for optimized native function calls
- Support for known native globals
- Significant performance improvement for stdlib calls

**0.15.0-a - 0.15.2-a** - Monomorphic Inline Cache
- Implemented MIC via bytecode patching
- CallGlobal caching for globals
- Major call overhead reduction

## 0.14.x - Optimizer & Security

**0.14.6-a - 0.14.7-a**
- Fixed func_idx validation during bytecode deserialization
- Fixed parser peek() bounds panic
- Bounded expression recursion
- Bounded block comment nesting

**0.14.3-a - 0.14.5-a** - Buffer Safety
- Fixed unbounded buffer allocation in std
- Path traversal prevention via fs.join
- Bounded network data accumulation
- Register allocation overflow fix
- Constant index truncation fix

**0.14.1-a**
- Critical fix: global constant propagation
- Liveness analysis improvements

## 0.13.x - Bytecode Optimization

**0.13.1-a**
- Working optimization system
- Pipeline integration
- Default optimization level set

**0.13.0-a** - Optimization Module
- Bytecode optimization module
- Constant folding
- Dead code elimination

## 0.12.x - HTTP Server & Native Modules

**0.12.3-a**
- Fixed native system module

**0.12.x**
- Added HTTP server example
- Native module support (alpha)
- Module system improvements

## 0.10.x - Security Overhaul

**0.10.2-a**
- Global layout tests

**0.10.0-a** - Security Update
- Bytecode verifier
- Register/constant out-of-bounds checking
- No more unchecked heap access
- Capability system for stdlib (fs, net require explicit permission)

## 0.9.x

**0.9.0-a** - Compilation Pipeline
- Stage-based compilation pipeline with caching
- Faster incremental builds

## 0.8.x - Type System & Closures

**0.8.10-a - 0.8.11-a**
- Memory safety improvements
- Pre-allocate register vec to prevent pointer invalidation
- Path traversal validation in module loader
- Bounds checking in LoadK opcode

**0.8.7-a**
- Unchecked load/store methods for manual heap
- Globals by index cache
- Direct register access optimization
- Fixed memory leak in @no_gc (ExitNoGc wasn't emitted)

**0.8.1-a - 0.8.2-a** - Typed Pipeline
- Typed compilation pipeline
- Lambda upvalue capture from outer scopes
- Upvalue capture mechanism for nested functions

**0.8.0-a**
- Type inference with guarded operations

## 0.7.x - Standard Library

**0.7.2-a**
- 6.2x faster nested calls
- 6.7x faster function calls

**0.7.0-a** - Standard Library
- Standard library module support
- Global layout hash computation
- Global sync with mapping IDs

## 0.6.x

- Working module system
- Global index allocator
- Import handling

## 0.3.x - Performance

**0.3.3-a**
- Benchmark test files

**0.3.2-a**
- IncGlobalI superinstruction
- Significant performance boost for incrementing globals

**0.3.0-a**
- 50% closure performance improvement via caching

## 0.1.x - Foundations

**0.1.2-a**
- 30% performance improvement
- Garbage collection optimization
- Upvalues support in call frames

**0.1.x**
- Initial closures and upvalues
- Basic GC implementation
- Compiler mutability tracking

## Initial Commit

- Project started
- BSD 3-Clause License
