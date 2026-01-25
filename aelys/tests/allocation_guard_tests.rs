use std::fs;
use std::path::Path;

/// Ensure no runtime modules bypass VM allocation guards by calling Heap::alloc* directly.
#[test]
fn vm_modules_do_not_bypass_allocation_guards() {
    let vm_dir = Path::new("../aelys-runtime/src/vm");
    let allowed = [
        "vm.rs",
        "heap.rs",
        "manual_heap.rs",
        "config.rs",
        "args.rs",
        "mod.rs",
        "alloc.rs",
    ];

    fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).expect("dir exists") {
            let entry = entry.expect("read entry");
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(vm_dir, &mut files);

    for file in files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if allowed.contains(&name) {
            continue;
        }

        let content = fs::read_to_string(&file).expect("read file");
        let mut bad = Vec::new();
        if content.contains("heap.alloc(") || content.contains("heap_mut().alloc") {
            bad.push("heap.alloc");
        }
        if content.contains("heap.alloc_function") || content.contains("heap.alloc_native") {
            bad.push("heap.alloc_function/native");
        }

        if !bad.is_empty() {
            panic!(
                "direct heap allocation {:?} found in {}",
                bad,
                file.display()
            );
        }
    }
}
