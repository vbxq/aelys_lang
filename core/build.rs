fn main() {
    println!("cargo:rerun-if-changed=src/aelys_core_common.c");
    println!("cargo:rerun-if-changed=src/aelys_rc_leak.c");
    println!("cargo:rerun-if-changed=src/aelys_rc_real.c");
    println!("cargo:rerun-if-changed=src/aelys_rc_cycles.c");
    println!("cargo:rerun-if-changed=src/aelys_rc.h");
    println!("cargo:rerun-if-changed=src/aelys_alloc_immix.c");
    println!("cargo:rerun-if-changed=src/aelys_alloc_immix.h");
    // the allocator's poison calls are dead unless the archive itself is instrumented,
    // so set AELYS_CORE_ASAN=1 to actually arm the intra-region UAF net
    println!("cargo:rerun-if-env-changed=AELYS_CORE_ASAN");
    let asan = std::env::var_os("AELYS_CORE_ASAN").is_some_and(|v| v == "1");
    let build = || {
        let mut b = cc::Build::new();
        if asan {
            b.flag("-fsanitize=address");
        }
        b
    };

    build()
        .file("src/aelys_core_common.c")
        .file("src/aelys_alloc_immix.c")
        .file("src/aelys_rc_leak.c")
        .compile("aelys-core-leak");

    build()
        .file("src/aelys_core_common.c")
        .file("src/aelys_alloc_immix.c")
        .file("src/aelys_rc_real.c")
        .compile("aelys-core-rc");

    build()
        .file("src/aelys_core_common.c")
        .file("src/aelys_alloc_immix.c")
        .file("src/aelys_rc_cycles.c")
        .compile("aelys-core-rc-cycles");
}
