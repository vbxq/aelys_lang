/// Encode func_ptr into 2 cache words for bytecode patching.
#[inline(always)]
pub(super) fn encode_cache_words(func_ptr: usize, slot_id: u16) -> (u32, u32) {
    let word1 = (func_ptr & 0xFFFFFFFF) as u32;
    let word2 = (((func_ptr >> 32) as u32) << 16) | (slot_id as u32);
    (word1, word2)
}

/// Decode func_ptr and slot_id from 2 cache words.
#[inline(always)]
pub(super) fn decode_cache_words(word1: u32, word2: u32) -> (usize, u16) {
    let func_ptr_low = word1 as usize;
    let func_ptr_high = ((word2 >> 16) as usize) << 32;
    let func_ptr = func_ptr_high | func_ptr_low;
    let slot_id = (word2 & 0xFFFF) as u16;
    (func_ptr, slot_id)
}
