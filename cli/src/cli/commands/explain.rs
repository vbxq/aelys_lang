use aelys_common::registry;

pub fn run_explain(code: &str) {
    match registry::lookup(code) {
        Some(info) => {
            eprintln!("{}: {}\n", info.code, info.title);
            eprintln!("{}", info.explanation);
        }
        None => {
            eprintln!("error: unknown error code: {}", code);
            eprintln!("Use 'aelys --explain EXXXX' with a valid error code.");
        }
    }
}
