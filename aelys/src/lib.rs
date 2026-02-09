// public facade, re-exports from aelys-driver
// TODO: Test native module bundling and loading on different platforms
// TODO: (CLI) better REPL
// TODO: (VM) Implement JIT compilation for hot functions
// TODO: (VM) Add support for coroutines or async functions
// TODO: (VM) Implement better garbage collection (e.g., Arena GC)
// TODO: (VM) Way better FFI/we should be able to directly import .h files
// TODO: (Lexer/Parser) Revise token definitions and syntax for better clarity
// TODO: (Code Quality) consider Rc<RefCell<_>> for globals/global_indices in compiler
// TODO: (Code Quality) clean up those unsufferable functions in the compiler
// TODO: (Optimization) Improve constant folding to handle more complex expressions
// TODO: (Optimization) Tail call optimization
// TODO: (Optimization) Preallocated register pools
// TODO: (Optimization) Implement loop unrolling optimization pass
// TODO: (Optimization) Implement function inlining optimization pass (and @inline decorator)
// TODO: Custom modules share VM's global namespace, risk of collision if two modules export the same symbol (efor example mod_a::shared overwrites mod_b::shared as both become just shared in VM)
pub mod api;

pub use api::*;
