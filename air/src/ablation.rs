use std::cell::Cell;

thread_local! {
    static ARRAY_LENGTH_FROM_SIZE_EXPR: Cell<bool> = const { Cell::new(false) };
}

// no cli flag and no env var reaches this: the ablation row drives it to watch the net fire
pub fn set_array_length_from_size_expr(on: bool) {
    ARRAY_LENGTH_FROM_SIZE_EXPR.with(|cell| cell.set(on));
}

pub fn array_length_from_size_expr() -> bool {
    ARRAY_LENGTH_FROM_SIZE_EXPR.with(Cell::get)
}
