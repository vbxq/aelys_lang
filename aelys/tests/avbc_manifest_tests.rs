use aelys_bytecode::asm::binary::{deserialize_with_manifest, serialize_with_manifest};
use aelys_runtime::{Function, Heap};

#[test]
fn avbc_round_trip_manifest() {
    let func = Function::new(None, 0);
    let heap = Heap::new();
    let manifest = b"[build]\nbundle_native_modules = true\n".to_vec();

    let bytes = serialize_with_manifest(&func, &heap, Some(&manifest), None);
    let (_func, _heap, decoded_manifest, _bundles) =
        deserialize_with_manifest(&bytes).expect("read");

    assert_eq!(decoded_manifest.unwrap(), manifest);
}
