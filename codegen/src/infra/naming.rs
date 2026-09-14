pub(crate) fn is_reserved_bootstrap_builtin(name: &str) -> bool {
    matches!(name, "print" | "println")
}

pub(crate) fn reserved_bootstrap_builtin_message(name: &str) -> String {
    format!("reserved builtin during bootstrap: {}", name)
}
