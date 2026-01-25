use super::types::{LoadedNativeInfo, ModuleLoader};
use aelys_modules::manifest::Manifest;
use aelys_syntax::Source;
use std::path::Path;
use std::sync::Arc;

impl ModuleLoader {
    pub fn new(entry_file: &Path, source: Arc<Source>) -> Self {
        let base_dir = entry_file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| ".".into());
        let base_root = base_dir.canonicalize().unwrap_or_else(|_| base_dir.clone());
        Self {
            base_dir, base_root,
            loaded_modules: std::collections::HashMap::new(),
            loading_stack: Vec::new(),
            source,
            native_fingerprints: std::collections::HashMap::new(),
            manifest: Manifest::for_source_file(entry_file),
            loaded_native_modules: std::collections::HashMap::new(),
        }
    }

    pub fn with_manifest(entry_file: &Path, source: Arc<Source>, manifest: Option<Manifest>) -> Self {
        let base_dir = entry_file.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| ".".into());
        let base_root = base_dir.canonicalize().unwrap_or_else(|_| base_dir.clone());
        Self {
            base_dir, base_root,
            loaded_modules: std::collections::HashMap::new(),
            loading_stack: Vec::new(),
            source,
            native_fingerprints: std::collections::HashMap::new(),
            manifest,
            loaded_native_modules: std::collections::HashMap::new(),
        }
    }

    pub fn manifest(&self) -> Option<&Manifest> { self.manifest.as_ref() }
    pub fn loaded_native_modules(&self) -> &std::collections::HashMap<String, LoadedNativeInfo> { &self.loaded_native_modules }
}
