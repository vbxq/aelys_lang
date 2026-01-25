use aelys_modules::loader::FileFingerprint;
use aelys_modules::manifest::Manifest;
use aelys_syntax::Source;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub path: String,          // e.g. "utils.helpers"
    pub file_path: PathBuf,
    pub version: Option<String>, // native modules only
    pub exports: HashMap<String, ExportInfo>,
    pub native_functions: Vec<String>, // qualified names for CallGlobalNative
}

#[derive(Debug, Clone)]
pub struct ExportInfo {
    pub is_function: bool,
    pub is_mutable: bool,
}

#[derive(Debug, Clone)]
pub struct LoadedNativeInfo {
    pub file_path: PathBuf,
    pub name: String,
}

pub struct ModuleImports {
    pub module_aliases: HashSet<String>,      // "utils" from `needs utils`
    pub known_globals: HashSet<String>,       // direct imports
    pub known_native_globals: HashSet<String>, // native funcs for codegen opt
    pub next_call_site_slot: u16,            // next available call site slot after loading all modules
}

pub enum LoadResult {
    Module(String),  // qualified access: mod.func
    Symbol(String),  // direct access
}

pub struct ModuleLoader {
    pub(crate) base_dir: PathBuf,
    pub(crate) base_root: PathBuf,
    pub(crate) loaded_modules: HashMap<String, ModuleInfo>,
    pub(crate) loading_stack: Vec<String>,
    pub(crate) source: Arc<Source>,
    pub(crate) native_fingerprints: HashMap<String, FileFingerprint>,
    pub(crate) manifest: Option<Manifest>,
    pub(crate) loaded_native_modules: HashMap<String, LoadedNativeInfo>,
    pub(crate) next_call_site_slot: u16,
}
