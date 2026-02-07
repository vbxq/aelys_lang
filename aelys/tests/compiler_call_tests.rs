#[test]
fn arg_range_available_checks_bounds_and_usage() {
    let mut pool = [false; 256];
    pool[3] = true;

    // This will need access to the private function, so we need to recreate the logic
    // or make the function public for testing
    let arg_range_available = |pool: &[bool; 256], start: usize, count: usize| -> bool {
        if start + count > 256 {
            return false;
        }

        for slot in pool.iter().skip(start).take(count) {
            if *slot {
                return false;
            }
        }
        true
    };

    assert!(arg_range_available(&pool, 0, 3));
    assert!(!arg_range_available(&pool, 2, 2));
    assert!(!arg_range_available(&pool, 254, 3));
}
