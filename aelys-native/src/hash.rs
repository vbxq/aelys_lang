// FNV-1a hash for export validation

use crate::{AelysExport, AelysModuleDescriptor};
use core::ffi::CStr;

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

pub unsafe fn compute_exports_hash(exports: *const AelysExport, export_count: u32) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_u64(&mut hash, export_count as u64);
    if export_count == 0 || exports.is_null() {
        return hash;
    }

    let slice = unsafe { core::slice::from_raw_parts(exports, export_count as usize) };
    for export in slice {
        let name_bytes = if export.name.is_null() {
            &[][..]
        } else {
            unsafe { CStr::from_ptr(export.name) }.to_bytes()
        };
        hash_bytes(&mut hash, name_bytes);
        hash_u64(&mut hash, export.kind as u32 as u64);
        hash_bytes(&mut hash, &[export.arity]);
        hash_u64(&mut hash, export.value as usize as u64);
    }

    hash
}

pub unsafe fn init_descriptor_exports_hash(descriptor: *mut AelysModuleDescriptor) {
    if descriptor.is_null() {
        return;
    }
    let desc = unsafe { &mut *descriptor };
    desc.exports_hash = unsafe { compute_exports_hash(desc.exports, desc.export_count) };
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn hash_u64(hash: &mut u64, value: u64) {
    hash_bytes(hash, &value.to_le_bytes());
}
