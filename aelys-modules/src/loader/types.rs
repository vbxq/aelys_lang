use crate::manifest::Manifest;
use aelys_syntax::Source;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub path: String, // e.g., "utils.helpers"
    pub file_path: PathBuf,
    pub version: Option<String>, // native only
    pub exports: HashMap<String, ExportInfo>,
    pub native_functions: Vec<String>, // for CallGlobalNative opt
}

#[derive(Debug, Clone)]
pub struct ExportInfo {
    pub is_function: bool,
    pub is_mutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFingerprint {
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
}

impl FileFingerprint {
    pub fn from_path(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct LoadedNativeInfo {
    pub file_path: PathBuf,
    pub name: String,
}

pub struct ModuleImports {
    pub module_aliases: HashSet<String>, // "utils" from `needs utils`
    pub known_globals: HashSet<String>,  // wildcard/specific imports
    pub known_native_globals: HashSet<String>, // for CallGlobalNative
    pub symbol_origins: HashMap<String, String>,
}

pub enum LoadResult {
    Module(String),
    Symbol(String),
}

#[allow(dead_code)]
pub struct ModuleLoader {
    pub(crate) base_dir: PathBuf,
    pub(crate) base_root: PathBuf, // canonical, prevents symlink escapes
    pub(crate) loaded_modules: HashMap<String, ModuleInfo>,
    pub(crate) loading_stack: Vec<String>, // circular dep detection
    pub(crate) source: Arc<Source>,
    pub(crate) native_fingerprints: HashMap<String, FileFingerprint>,
    pub(crate) manifest: Option<Manifest>,
    pub(crate) loaded_native_modules: HashMap<String, LoadedNativeInfo>,
}
