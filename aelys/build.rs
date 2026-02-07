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

            if content != new_content {
                fs::write(&full_path, new_content.as_ref()).unwrap();
                println!("cargo:warning=Updated version in {}", file_path);
            }
        }
    }
}
