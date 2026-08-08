<p align="center">
  <img src="docs/aelys_banner.png?v=3" alt="aelys" width="1000">
</p>

# Aelys

A programming language with managed memory by default and explicit opt-out for performance-critical code.

Most languages force a single memory model on the entire program. GC'd languages pay for a runtime on every path; systems languages demand manual control everywhere.

Aelys starts from managed memory, reference counting with an optional cycle collector, and lets you leave it behind one function at a time. Default mode gives you heap allocation, type inference, and minimal annotation. `nogc` gives you compiler-enforced zero-allocation with statically checked references, no user-written lifetime annotations.

<!-- ```rust is used for syntax highlighting only, this is Aelys -->
```rust
fn load_mesh(path: str) -> Result<Mesh, IoError> {
    let data = read_file(path) fail |e| IoError::from(e)
    let points = parse_vertices(data) fail IoError::ParseFailed
    Ok(Mesh { vertices: points })
}

nogc fn compute_normals(vertices: &[Vec3], normals: &mut [Vec3]) {
    for i in range(0, vertices.len() - 2, step: 3) {
        let e1 = vertices[i + 1] - vertices[i]
        let e2 = vertices[i + 2] - vertices[i]
        normals[i / 3] = cross(e1, e2).normalize()
    }
}

fn main() {
    let mesh = match load_mesh("model.obj") or load_mesh("default.obj") {
        Ok(m)                     => m,
        Err(IoError::NotFound)    => { println("no mesh found");     return },
        Err(IoError::ParseFailed) => { println("mesh is corrupted"); return }
    }

    let mut normals = Vec<Vec3>::new(mesh.vertices.len() / 3)
    compute_normals(&mesh.vertices, &mut normals)
}
```

`load_mesh` is default mode: GC-backed allocation, Result-based error handling with `?` for propagation and `catch` for recovery. `compute_normals` is `nogc`, meaning no heap allocation, no GC containers, and references checked at compile time. In `main`, `&` at the call site marks the boundary where data is borrowed into `nogc` territory.

Managed code calls `nogc` freely; the reverse is a compile error.

Beyond the memory model: inferred types, pattern matching, compilation to native code via LLVM.

## Memory model

The language provides two levels of control, each narrowing what the runtime provides.

**Default mode.** Reference counting manages the heap, with an optional cycle collector (`--runtime rc+cycles`). Types are inferred and standard collections are managed. Type annotations are optional. `?` propagates errors and `catch` handles them. This is the intended level for most code.

<br>

**`nogc` functions.** A function-level opt-out from managed allocation. Inside a `nogc` function, managed allocation is rejected at compile time, managed containers cannot be created, references are checked for escape and aliasing violations, and calls into managed code are rejected.

The compiler proves that the function satisfies these constraints or rejects it. No warnings, no user-written lifetime annotations. `unsafe {}` is reserved; today it permits only `.unwrap_unchecked()`, and it does not re-enable managed allocation.

```rust
needs "GL/glext.h"

nogc fn upload_normals(buffer_id: u32, normals: &[Vec3]) {
    let byte_size = (normals.len() * size_of(Vec3)) as isize
    unsafe {
        glNamedBufferData(
            buffer_id,
            byte_size,
            normals.as_ptr() as *const void,
            GL_STATIC_DRAW
        )
    }
}
```

<br>

**`#![no_gc]` `#![no_std]` modules.** At the module level, these attributes remove the garbage collector, runtime, and standard library entirely. The language exposes raw pointers, `extern fn`, and inline assembly, while parsing, typing, and semantic analysis still apply. This mode is intended for kernels, boot code, and freestanding targets.

```rust
#![no_gc]
#![no_std]

extern {
    static __bss_start: u8
    static __bss_end:   u8
}

fn zero_bss() {
    let mut ptr = &__bss_start as *const u8 as *mut u8
    let end     = &__bss_end   as *const u8
    while ptr < end {
        *ptr = 0u8
        ptr  = ptr + 1
    }
}

#[no_mangle]
fn kernel_entry() -> ! {
    zero_bss()
    uart::write("boot ok\n")
    loop { asm("wfi") }
}
```

## Status

Aelys is an experimental language and compiler project under active rewrite. It is not ready for use.

```
source → parser → semantic analysis → AIR (Aelys IR) → LLVM IR → native code
```

Parser and semantic analysis are partially implemented. The `nogc` checker rules are under active design. Codegen targets LLVM.

Language semantics, the IR, and parts of the standard library are not stable. Open design questions include the collection strategy, iterator design, the managed / `nogc` boundary rules, and whether a freestanding mode is in scope at all.

## Contributing

Bug reports, design feedback, and discussion around language semantics and compiler behavior are the most useful contributions right now. The codebase changes quickly, so if you want to contribute code, open an issue first so the work can be aligned with the current direction.
