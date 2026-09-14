//! read-only analyses, they never mutate the function; elision policy lives in passes/

pub mod cfg;
pub mod escape;
pub mod liveness;
