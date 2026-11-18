use std::sync::OnceLock;
use std::time::Instant;

pub(crate) fn phase<T>(name: &str, run: impl FnOnce() -> T) -> T {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("AELYS_TIME_PASSES").is_some()) {
        return run();
    }
    let start = Instant::now();
    let value = run();
    eprintln!(
        "[time] {name} {:.3}ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    value
}
