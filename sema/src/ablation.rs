use std::cell::Cell;

thread_local! {
    static BYTES_NO_PLACE_UNCHECKED: Cell<bool> = const { Cell::new(false) };
}

// no cli flag and no env var reaches this: the ablation row drives it to watch the air guard fire
pub fn set_bytes_no_place_unchecked(on: bool) {
    BYTES_NO_PLACE_UNCHECKED.with(|cell| cell.set(on));
}

pub fn bytes_no_place_unchecked() -> bool {
    BYTES_NO_PLACE_UNCHECKED.with(Cell::get)
}
