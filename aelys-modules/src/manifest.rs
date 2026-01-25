// .aelys.toml manifest parsing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Manifest {
    #[serde(default)]
    pub module: HashMap<String, ModulePolicy>,
    #[serde(default)]
    pub build: BuildPolicy,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ModulePolicy {
    #[serde(default)]
    pub capabilities: Vec<String>,   // e.g., ["gpu", "window"]
    pub required_version: Option<String>, // semver constraint
    pub checksum: Option<String>,
    pub kind: Option<String>,        // "script", "native", or "std"
    pub path: Option<String>,        // explicit path override
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BuildPolicy {
    pub bundle_native_modules: Option<bool>, // embed .so/.dll into .avbc
}

#[derive(Debug)]
pub enum ManifestError {
    Io(std::io::Error),
    Parse(toml::de::Error),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(e) => write!(f, "manifest IO error: {}", e),
            ManifestError::Parse(e) => write!(f, "manifest parse error: {}", e),
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<std::io::Error> for ManifestError {
    fn from(e: std::io::Error) -> Self {
        ManifestError::Io(e)
    }
}

impl From<toml::de::Error> for ManifestError {
    fn from(e: toml::de::Error) -> Self {
        ManifestError::Parse(e)
    }
}

impl Manifest {
    pub fn from_str(raw: &str) -> Result<Self, toml::de::Error> { toml::from_str(raw) }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let raw = std::str::from_utf8(bytes).map_err(|e| {
            ManifestError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        Ok(toml::from_str(raw)?)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        toml::to_string_pretty(self).unwrap_or_default().into_bytes()
    }

    pub fn from_file(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    // looks for foo.aelys.toml or aelys.toml in same dir
    pub fn for_source_file(source_path: &Path) -> Option<Self> {
        // Try {filename}.aelys.toml
        let mut manifest_path = source_path.to_path_buf();
        let filename = source_path.file_name()?.to_string_lossy();
        manifest_path.set_file_name(format!("{}.toml", filename));

        if manifest_path.exists() {
            if let Ok(manifest) = Self::from_file(&manifest_path) {
                return Some(manifest);
            }
        }

        // Try aelys.toml in the same directory (project manifest)
        if let Some(parent) = source_path.parent() {
            let project_manifest = parent.join("aelys.toml");
            if project_manifest.exists() {
                if let Ok(manifest) = Self::from_file(&project_manifest) {
                    return Some(manifest);
                }
            }
        }

        None
    }

    pub fn module(&self, name: &str) -> Option<&ModulePolicy> { self.module.get(name) }
    pub fn should_bundle_natives(&self) -> bool { self.build.bundle_native_modules.unwrap_or(false) }
    pub fn module_names(&self) -> impl Iterator<Item = &String> { self.module.keys() }
}

impl ModulePolicy {
    pub fn is_native(&self) -> bool { self.kind.as_deref() == Some("native") }
    pub fn is_script(&self) -> bool { self.kind.as_deref() == Some("script") }
}
