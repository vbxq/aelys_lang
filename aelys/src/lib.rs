// public facade, re-exports from aelys-driver
// TODO: Test native module bundling and loading on different platforms
// TODO: (VM) Implement JIT compilation for hot functions
// TODO: (VM) Add support for coroutines or async functions
// TODO: (VM) Implement better garbage collection (e.g., Arena GC)
// TODO: (Lexer/Parser) Revise token definitions and syntax for better clarity
// TODO: (Code Quality) consider Rc<RefCell<_>> for globals/global_indices in compiler
// TODO: (Code Quality) clean up those unsufferable functions in the compiler
// TODO: (Optimization) Improve constant folding to handle more complex expressions
// TODO: (Optimization) Tail call optimization
// TODO: (Optimization) Preallocated register pools
// TODO: (Optimization) Implement loop unrolling optimization pass
// TODO: (Optimization) Implement function inlining optimization pass (and @inline decorator)
// TODO: (Documentation) Add more examples and actually write the documentation!
// TODO: the vm args parsing is kinda broken

pub mod api;

pub use api::*;
