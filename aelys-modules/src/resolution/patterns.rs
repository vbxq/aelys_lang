// search patterns for module resolution

use super::ModuleKind;
use std::path::{Path, PathBuf};

pub enum ExtensionPattern {
    Direct { ext: &'static str, kind: ModuleKind },      // foo.aelys
    ModFile { ext: &'static str, kind: ModuleKind },     // foo/mod.aelys
    DylibDirect { kind: ModuleKind },                    // foo.so
    DylibModFile { kind: ModuleKind },                   // foo/mod.so
    LibPrefixDylib { kind: ModuleKind },                 // libfoo.so
}

impl ExtensionPattern {
    pub fn to_path(&self, base_path: &Path, module_name: &str) -> PathBuf {
        match self {
            ExtensionPattern::Direct { ext, .. } => base_path.with_extension(ext),
            ExtensionPattern::ModFile { ext, .. } => base_path.join(format!("mod.{}", ext)),
            ExtensionPattern::DylibDirect { .. } => base_path.with_extension(native_dylib_ext()),
            ExtensionPattern::DylibModFile { .. } => {
                base_path.join(format!("mod.{}", native_dylib_ext()))
            }
            ExtensionPattern::LibPrefixDylib { .. } => {
                if let Some(parent) = base_path.parent() {
                    parent.join(format!("lib{}.{}", module_name, native_dylib_ext()))
                } else {
                    // Fallback: just use base_path's directory
                    PathBuf::from(format!("lib{}.{}", module_name, native_dylib_ext()))
                }
            }
        }
    }

    pub fn kind(&self) -> ModuleKind {
        match self {
            ExtensionPattern::Direct { kind, .. }
            | ExtensionPattern::ModFile { kind, .. }
            | ExtensionPattern::DylibDirect { kind }
            | ExtensionPattern::DylibModFile { kind }
            | ExtensionPattern::LibPrefixDylib { kind } => *kind,
        }
    }
}

pub fn full_search_patterns() -> &'static [ExtensionPattern] {
    use ExtensionPattern::*;
    use ModuleKind::*;
    &[
        Direct {
            ext: "aelys",
            kind: Script,
        },
        ModFile {
            ext: "aelys",
            kind: Script,
        },
        Direct {
            ext: "aelys-lib",
            kind: Native,
        },
        ModFile {
            ext: "aelys-lib",
            kind: Native,
        },
        DylibDirect { kind: Native },
        DylibModFile { kind: Native },
        LibPrefixDylib { kind: Native },
    ]
}

pub fn native_only_search_patterns() -> &'static [ExtensionPattern] {
    use ExtensionPattern::*;
    use ModuleKind::*;
    &[
        Direct {
            ext: "aelys-lib",
            kind: Native,
        },
        DylibDirect { kind: Native },
        LibPrefixDylib { kind: Native },
    ]
}

pub fn native_dylib_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}
