use aelys_native::{AELYS_ABI_VERSION, AelysModuleDescriptor};

#[test]
fn descriptor_abi_version_matches() {
    assert_eq!(AelysModuleDescriptor::ABI_VERSION, AELYS_ABI_VERSION);
}
