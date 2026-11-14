// scan could link `leak` for `--runtime rc`

use std::path::{Path, PathBuf};

use super::runtime::RuntimeVariant;

pub fn resolve_aelys_core_lib(variant: RuntimeVariant) -> Result<PathBuf, String> {
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
    let prefer_release = running_release_binary();

    resolve_from_roots(
        &roots,
        variant,
        &|root, variant| find_aelys_core_lib(root, variant, prefer_release),
        &|root, archive| core_sources_are_newer(root, archive),
        &mut |root| {
            let _ = build_aelys_core(root);
        },
    )
}

fn resolve_from_roots(
    roots: &[PathBuf],
    variant: RuntimeVariant,
    find: &dyn Fn(&Path, RuntimeVariant) -> Option<PathBuf>,
    stale: &dyn Fn(&Path, &Path) -> bool,
    build: &mut dyn FnMut(&Path),
) -> Result<PathBuf, String> {
    for root in roots {
        if let Some(path) = find(root, variant) {
            if !stale(root, &path) {
                return Ok(path);
            }
            build(root);
            if let Some(path) = find(root, variant)
                && !stale(root, &path)
            {
                return Ok(path);
            }
        }
    }

    for root in roots {
        build(root);
    }

    let mut stale_archive = None;
    for root in roots {
        if let Some(path) = find(root, variant) {
            if !stale(root, &path) {
                return Ok(path);
            }
            stale_archive.get_or_insert(path);
        }
    }

    let searched = roots
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    // returning a stale archive links a runtime that does not match core/src, silently
    if let Some(path) = stale_archive {
        return Err(format!(
            "aelys-core-{} static library is older than core/src and could not be rebuilt: {}\nrebuild `aelys-core` or set AELYS_CORE_LIB.",
            variant.lib_suffix(),
            path.display()
        ));
    }
    Err(format!(
        "could not locate aelys-core-{} static library. Set AELYS_CORE_LIB or build `aelys-core`.\nsearched:\n{}",
        variant.lib_suffix(),
        searched
    ))
}

fn find_aelys_core_lib(
    root: &Path,
    variant: RuntimeVariant,
    prefer_release: bool,
) -> Option<PathBuf> {
    for profile_dir in core_profile_dirs(root, prefer_release) {
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

fn core_profile_dirs(root: &Path, prefer_release: bool) -> Vec<PathBuf> {
    let debug = root.join("target").join("debug");
    let release = root.join("target").join("release");
    if prefer_release {
        vec![release, debug]
    } else {
        vec![debug, release]
    }
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

fn core_sources_are_newer(root: &Path, archive: &Path) -> bool {
    let Ok(archive_time) = archive.metadata().and_then(|m| m.modified()) else {
        return true;
    };
    // a source we cannot stat is not evidence of staleness, or a root without core/src rejects every archive
    [
        "core/build.rs",
        "core/src/aelys_core_common.c",
        "core/src/aelys_alloc_immix.c",
        "core/src/aelys_alloc_immix.h",
        "core/src/aelys_rc.h",
        "core/src/aelys_rc_leak.c",
        "core/src/aelys_rc_real.c",
        "core/src/aelys_rc_cycles.c",
    ]
    .iter()
    .map(|path| root.join(path))
    .any(|path| {
        path.metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|source_time| source_time > archive_time)
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn changed_core_source_requires_archive_rebuild() {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("aelys-core-lib-{id}"));
        let source = root.join("core/src/aelys_core_common.c");
        let archive = root.join("target/debug/build/aelys-core/out/libaelys-core-rc.a");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(&archive, b"old").unwrap();
        thread::sleep(Duration::from_millis(10));
        fs::write(&source, b"new").unwrap();
        assert!(core_sources_are_newer(&root, &archive));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn untouched_core_source_leaves_archive_fresh() {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("aelys-core-lib-fresh-{id}"));
        let source = root.join("core/src/aelys_core_common.c");
        let archive = root.join("target/debug/build/aelys-core/out/libaelys-core-rc.a");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(&source, b"new").unwrap();
        thread::sleep(Duration::from_millis(10));
        fs::write(&archive, b"old").unwrap();
        assert!(!core_sources_are_newer(&root, &archive));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_dirs_search_the_running_profile_first() {
        let root = Path::new("/nowhere");
        let release_first = core_profile_dirs(root, true);
        let debug_first = core_profile_dirs(root, false);
        assert!(release_first[0].ends_with("release"), "{release_first:?}");
        assert!(debug_first[0].ends_with("debug"), "{debug_first:?}");
    }

    const CORE_SOURCES: [&str; 8] = [
        "core/build.rs",
        "core/src/aelys_core_common.c",
        "core/src/aelys_alloc_immix.c",
        "core/src/aelys_alloc_immix.h",
        "core/src/aelys_rc.h",
        "core/src/aelys_rc_leak.c",
        "core/src/aelys_rc_real.c",
        "core/src/aelys_rc_cycles.c",
    ];

    fn archive_in(root: &Path, profile: &str) -> PathBuf {
        root.join("target")
            .join(profile)
            .join("build/aelys-core-abc/out/libaelys-core-rc.a")
    }

    #[test]
    fn resolve_never_returns_an_archive_the_rebuild_left_stale() {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("aelys-core-lib-stale-{id}"));
        let debug = archive_in(&root, "debug");
        let release = archive_in(&root, "release");
        for archive in [&debug, &release] {
            fs::create_dir_all(archive.parent().unwrap()).unwrap();
        }
        fs::write(&debug, b"stale").unwrap();
        thread::sleep(Duration::from_millis(10));
        for source in CORE_SOURCES {
            let path = root.join(source);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"src").unwrap();
        }
        thread::sleep(Duration::from_millis(10));
        fs::write(&release, b"fresh").unwrap();

        let resolved = resolve_from_roots(
            std::slice::from_ref(&root),
            RuntimeVariant::Rc,
            &|root, variant| find_aelys_core_lib(root, variant, true),
            &core_sources_are_newer,
            &mut |_| {},
        );

        assert_eq!(resolved.as_deref(), Ok(release.as_path()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolve_fails_closed_when_no_archive_is_ever_fresh() {
        let root = PathBuf::from("/root");
        let debug = root.join("target/debug/libaelys-core-rc.a");

        let resolved = resolve_from_roots(
            std::slice::from_ref(&root),
            RuntimeVariant::Rc,
            &|_, _| Some(debug.clone()),
            &|_, _| true,
            &mut |_| {},
        );

        assert!(
            resolved.is_err(),
            "resolved to a stale archive: {resolved:?}"
        );
    }
}
