// module loading and dependency resolution

pub mod loader;
mod needs;

pub use loader::{
    ExportInfo, LoadResult, LoadedNativeInfo, ModuleImports, ModuleInfo, ModuleLoader,
};
pub use needs::{load_modules_for_program, load_modules_with_loader};
