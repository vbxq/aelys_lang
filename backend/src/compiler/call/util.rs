pub(super) fn arg_range_available(register_pool: &[bool; 256], start: u8, args_len: usize) -> bool {
    for i in 0..args_len {
        let arg_reg = match start.checked_add(i as u8) {
            Some(r) => r,
            None => return false,
        };
        if (arg_reg as usize) >= register_pool.len() || register_pool[arg_reg as usize] {
            return false;
        }
    }
    true
}
