// heap object types

mod closure;
mod function;
mod gc_object;
mod gc_ref;
mod kinds;
mod native;
mod string;
mod upvalue;

pub use closure::AelysClosure;
pub use function::AelysFunction;
pub use gc_object::GcObject;
pub use gc_ref::GcRef;
pub use kinds::ObjectKind;
pub use native::NativeFunction;
pub use string::AelysString;
pub use upvalue::{AelysUpvalue, UpvalueLocation};
