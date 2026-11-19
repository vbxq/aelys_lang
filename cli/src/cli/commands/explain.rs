use aelys_common::registry;

pub fn run_explain(code: &str) {
    match registry::lookup(&code.to_ascii_uppercase()) {
        Some(info) => {
            println!("{}: {}\n", info.code, info.title);
            println!("{}", info.explanation);
        }
        None => {
            eprintln!("error: unknown error code: {}", code);
            eprintln!(
                "Use '{} --explain EXXXX' with a valid error code.",
                aelys_common::diagnostic::render::program_name()
            );
            std::process::exit(1);
        }
    }
}
