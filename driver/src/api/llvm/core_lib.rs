// both variants land in the same out/ dir, so always match the exact archive name or a
// scan could link `leak` for `--runtime rc`

use std::path::{Path, PathBuf};

use super::runtime::RuntimeVariant;

pub(super) fn resolve_aelys_core_lib(variant: RuntimeVariant) -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("AELYS_CORE_LIB") {
        let path = PathBuf::from(&raw);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "AELYS_CORE_LIB points to a missing file: {}",
            path.display()
        ));
    }

    let roots = candidate_search_roots();

    for root in &roots {
        if let Some(path) = find_aelys_core_lib(root, variant) {
            return Ok(path);
        }
    }

    for root in &roots {
        let _ = build_aelys_core(root);
    }

    for root in &roots {
        if let Some(path) = find_aelys_core_lib(root, variant) {
            return Ok(path);
        }
    }

    let searched = roots
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "could not locate aelys-core-{} static library. Set AELYS_CORE_LIB or build `aelys-core`.\nsearched:\n{}",
        variant.lib_suffix(),
        searched
    ))
}

fn find_aelys_core_lib(root: &Path, variant: RuntimeVariant) -> Option<PathBuf> {
    for profile_dir in core_profile_dirs(root) {
        if let Some(path) = find_core_lib_in_build_out(&profile_dir.join("build"), variant) {
            return Some(path);
        }
    }

    for candidate in exact_core_lib_candidates(root, variant) {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn variant_lib_names(variant: RuntimeVariant) -> [String; 2] {
    let suffix = variant.lib_suffix();
    [
        format!("aelys-core-{}.lib", suffix),
        format!("libaelys-core-{}.a", suffix),
    ]
}

fn exact_core_lib_candidates(root: &Path, variant: RuntimeVariant) -> Vec<PathBuf> {
    let names = variant_lib_names(variant);

    let mut candidates = Vec::with_capacity(names.len() * 2);
    for name in &names {
        candidates.push(root.join(name));
        candidates.push(root.join("deps").join(name));
    }
    candidates
}

fn is_core_lib_name(path: &Path, variant: RuntimeVariant) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    let lowered = file_name.to_ascii_lowercase();
    variant_lib_names(variant)
        .iter()
        .any(|name| lowered == *name)
}

fn find_core_lib_in_dir(dir: &Path, variant: RuntimeVariant) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_core_lib_name(&path, variant) {
            return Some(path);
        }
    }
    None
}

fn find_core_lib_in_build_out(build_dir: &Path, variant: RuntimeVariant) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(build_dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let out_dir = entry.path().join("out");
        if let Some(path) = find_core_lib_in_dir(&out_dir, variant) {
            return Some(path);
        }
    }
    None
}

fn core_profile_dirs(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("target").join("debug"),
        root.join("target").join("release"),
    ]
}

fn candidate_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(raw) = std::env::var("CARGO_MANIFEST_DIR") {
        push_unique(&mut roots, PathBuf::from(raw));
    }

    if let Some(raw) = option_env!("CARGO_MANIFEST_DIR") {
        let manifest = PathBuf::from(raw);
        push_unique(&mut roots, manifest.clone());
        if let Some(parent) = manifest.parent() {
            push_unique(&mut roots, parent.to_path_buf());
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push_unique(&mut roots, dir.to_path_buf());
            if let Some(parent) = dir.parent() {
                push_unique(&mut roots, parent.to_path_buf());
                if let Some(grand_parent) = parent.parent() {
                    push_unique(&mut roots, grand_parent.to_path_buf());
                }
            }
        }
    }

    if let Ok(current) = std::env::current_dir() {
        push_unique(&mut roots, current);
    }

    roots
}

fn push_unique(items: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !items.iter().any(|existing| existing == &candidate) {
        items.push(candidate);
    }
}

fn build_aelys_core(root: &Path) -> Result<(), String> {
    if !root.join("Cargo.toml").is_file() {
        return Err(format!(
            "no Cargo.toml in candidate root {}",
            root.display()
        ));
    }

    let mut args = vec![
        "build".to_string(),
        "-p".to_string(),
        "aelys-core".to_string(),
    ];
    if running_release_binary() {
        args.push("--release".to_string());
    }

    super::run_process_in_dir("cargo", &args, Some(root))
}

fn running_release_binary() -> bool {
    if let Ok(exe) = std::env::current_exe() {
        return exe
            .components()
            .any(|component| component.as_os_str() == "release");
    }
    false
}
