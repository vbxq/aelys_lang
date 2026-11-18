use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn std_functions_lowered(name: &str, source: &str, level: OptimizationLevel) -> Vec<String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("aelys_driver_{name}_{stamp}.aelys"));
    fs::write(&path, source).unwrap();
    let air = lower_file_to_air(&path, level);
    let _ = fs::remove_file(&path);
    let mut names: Vec<String> = air
        .unwrap()
        .functions
        .iter()
        .map(|f| f.name.clone())
        .filter(|n| n.contains("std."))
        .collect();
    names.sort();
    names
}

#[test]
fn an_imported_std_module_lowers_only_the_functions_the_program_reaches() {
    let source = "use std::str;\nuse std::vec;\n\nfn main() {\n    println!(\"{}\", str::pad_left(\"ab\", 4));\n}\n";
    for level in [OptimizationLevel::None, OptimizationLevel::Standard] {
        assert_eq!(
            std_functions_lowered("std_reach", source, level),
            ["std.str.char_count", "std.str.pad_left", "std.str.repeat"]
        );
    }
}

#[test]
fn a_std_import_nothing_calls_lowers_no_std_function() {
    let source = "use std::io;\nuse std::math;\nuse std::slice;\nuse std::sort;\nuse std::str;\nuse std::vec;\n\nfn main() {\n    println!(\"hello\");\n}\n";
    assert!(std_functions_lowered("std_none", source, OptimizationLevel::None).is_empty());
}
