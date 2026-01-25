use super::super::types::ModuleLoader;
use aelys_modules::loader::FileFingerprint;
use std::path::Path;
#[cfg(target_os = "windows")]
use std::time::SystemTime;

// TODO: consider inotify/fsevents for real hot reload instead of polling

impl ModuleLoader {
    pub(crate) fn current_native_fingerprint(&self, file_path: &Path) -> Option<FileFingerprint> {
        let base = FileFingerprint::from_path(file_path);
        #[cfg(target_os = "windows")]
        {
            let instance = Self::find_windows_instance(file_path);
            match (base, instance) {
                (None, Some(instance)) => Some(instance),
                (Some(base), Some(instance)) => {
                    if instance != base {
                        Some(instance)
                    } else {
                        Some(base)
                    }
                }
                (Some(base), None) => Some(base),
                (None, None) => None,
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            base
        }
    }

    #[cfg(target_os = "windows")]
    fn find_windows_instance(path: &Path) -> Option<FileFingerprint> {
        let parent = path.parent()?;
        let stem = path.file_stem()?.to_string_lossy().to_string();
        let ext = path.extension()?.to_string_lossy().to_string();
        let prefix = format!("{}.", stem);
        let suffix = format!(".{}", ext);
        let mut newest: Option<(SystemTime, FileFingerprint)> = None;
        for entry in std::fs::read_dir(parent).ok()? {
            let entry = entry.ok()?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if !name.starts_with(&prefix) || !name.ends_with(&suffix) {
                continue;
            }
            let fingerprint = FileFingerprint::from_path(&entry.path())?;
            let modified = fingerprint.modified.unwrap_or(SystemTime::UNIX_EPOCH);
            let replace = match &newest {
                Some((time, _)) => modified > *time,
                None => true,
            };
            if replace {
                newest = Some((modified, fingerprint));
            }
        }
        newest.map(|(_, fingerprint)| fingerprint)
    }
}
