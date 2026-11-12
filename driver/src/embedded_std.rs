pub const EMBEDDED_STD_PREFIX: &str = "<embedded>/std";

pub const EMBEDDED_STD: &[(&str, &str)] = &[
    ("io", include_str!("../../std/io.aelys")),
    ("math", include_str!("../../std/math.aelys")),
    ("prelude", include_str!("../../std/prelude.aelys")),
    ("result", include_str!("../../std/result.aelys")),
    ("slice", include_str!("../../std/slice.aelys")),
    ("sort", include_str!("../../std/sort.aelys")),
    ("str", include_str!("../../std/str.aelys")),
    ("vec", include_str!("../../std/vec.aelys")),
];

pub fn embedded_std_path(name: &str) -> String {
    format!("{EMBEDDED_STD_PREFIX}/{name}.aelys")
}

pub fn embedded_std_hash() -> String {
    std_hash(EMBEDDED_STD)
}

pub fn std_hash(modules: &[(&str, &str)]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (name, content) in modules {
        feed(name.as_bytes());
        feed(&[0]);
        feed(content.as_bytes());
        feed(&[0]);
    }
    format!("{hash:016x}")
}
