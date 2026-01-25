use super::{AelysClosure, AelysFunction, AelysString, AelysUpvalue, NativeFunction};

/// The different types of GC-managed objects.
#[derive(Debug)]
pub enum ObjectKind {
    String(AelysString),
    Function(AelysFunction),
    Native(NativeFunction),
    Upvalue(AelysUpvalue),
    Closure(AelysClosure),
}
