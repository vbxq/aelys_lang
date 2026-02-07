mod patterns;

pub use patterns::{
    ExtensionPattern, full_search_patterns, native_dylib_ext, native_only_search_patterns,
};

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    Script,
    Native,
}

#[derive(Debug, Clone)]
pub struct ModuleResolution {
    pub path: PathBuf,
    pub kind: ModuleKind,
}
