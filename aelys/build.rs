use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../Cargo.toml");

    let version = env!("CARGO_PKG_VERSION");
    let workspace_root = Path::new("..").canonicalize().unwrap();

    let updates = vec![
        (
            "README.md",
            r"# aelys \d+\.\d+\.\d+-[a-z]",
            format!("# aelys {}", version),
        ),
        (
            "examples/hello.aelys",
            r"Aelys v\d+\.\d+\.\d+-[a-z]",
            format!("Aelys v{}", version),
        ),
    ];

    for (file_path, pattern, replacement) in updates {
        let full_path = workspace_root.join(file_path);
        if full_path.exists() {
            let content = fs::read_to_string(&full_path).unwrap();
            let re = regex::Regex::new(pattern).unwrap();
            let new_content = re.replace_all(&content, replacement.as_str());

            // a build that rewrites a tracked file makes every clean tree a lie, so it only says so
            if content != new_content {
                println!(
                    "cargo:warning={file_path} carries a stale version stamp; it should read \
                     `{replacement}`"
                );
            }
        }
    }
}
