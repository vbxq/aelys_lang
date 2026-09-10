<p align="center">
  <img src="docs/aelys_banner.png?v=3" alt="aelys" width="1000">
</p>

# Aelys

A programming language with managed memory by default and explicit opt-out for performance-critical code.

Most languages force a single memory model on the entire program. GC'd languages pay for a runtime on every path; systems languages demand manual control everywhere.

Aelys starts from managed memory, reference counting with an optional cycle collector, and lets you leave it behind one function at a time. Default mode gives you heap allocation, type inference, and minimal annotation. `nogc` gives you compiler-enforced freedom from *additional* allocation, the caller's buffers are still the caller's, but nothing inside the region allocates, with statically checked references and no user-written lifetime annotations.

The example below is **aspirational**: it shows the language this project is aiming at, and many of the constructs in it do not exist yet, among them `.len()`, `range()`, `Vec<T>::new(n)`, a `Vec` held inside a struct, operator overloading on a user type, the `fail` operator, unqualified `Ok`/`Err`, named call arguments, `or` as a fallback, nested patterns, and user-defined methods. It is kept because it is the target, not because it compiles.

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

`load_mesh` is default mode: GC-backed allocation, Result-based error handling with `?` for propagation and `catch` for recovery. `compute_normals` is `nogc`, meaning no heap allocation, no GC containers, and references checked at compile time. In `main`, slicing a managed container at the call site marks the boundary where data is borrowed into `nogc` territory.

The *boundary* that example illustrates, a `nogc` function taking `&[T]` and `&mut [T]` derived from managed containers, writing through the mutable one, does work today. This compiles and runs:

<!-- ```rust is used for syntax highlighting only, this is Aelys -->
```rust
nogc fn scale(src: &[i64], dst: &mut [i64]) -> i64 {
    let mut i: i64 = 0
    while i < 3 {
        dst[i] = src[i] * 2
        i = i + 1
    }
    return 0
}

fn main() -> i64 {
    let mut input: Vec<i64> = vec[1, 2, 3]
    let mut output: Vec<i64> = vec[0, 0, 0]
    let q = scale(input[0..3], output[0..3])
    println(output[2])
    return 0
}
```

Managed code calls `nogc` freely; the reverse is a compile error only where the call reaches managed allocation.

Beyond the memory model: inferred types, pattern matching, compilation to native code via LLVM.

## Memory model

The language provides two levels of control, each narrowing what the runtime provides.

**Default mode.** Reference counting manages the heap, with an optional cycle collector (`--runtime rc+cycles`). Types are inferred and standard collections are managed. Type annotations are optional. `?` propagates errors and `catch` handles them, over a `Result` the program declares itself, since there is no prelude yet. This is the intended level for most code.

<br>

**`nogc` functions.** A function-level opt-out from managed allocation. Inside a `nogc` function, managed allocation is rejected at compile time, managed containers cannot be created, references are checked for escape and aliasing violations, and calls that reach managed allocation are rejected, naming the path that reaches it.

The compiler proves that the function satisfies these constraints or rejects it. No warnings, no user-written lifetime annotations. `unsafe {}` is narrow: today it permits `.unwrap_unchecked()` and gates every call of an `extern fn`, and it does not re-enable managed allocation.

The FFI example below is **aspirational**: `needs "header.h"`, raw pointer casts, `size_of` and `.as_ptr()` do not exist yet.

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

**`#![no_gc]` `#![no_std]` modules.** At the module level, these attributes remove the garbage collector, runtime, and standard library entirely. This mode is **planned, not implemented**: module attributes, raw pointers and inline assembly are not parsed today. `extern fn` is the exception, and it is real: an `unsafe extern fn` declaration parses, links, and is what `std/io.aelys` is built on. It is intended for kernels, boot code, and freestanding targets, and the module below does not compile.

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

Parser and semantic analysis are partially implemented. There is now a standard library, 586 lines of Aelys across seven modules under `std/`, but **there is still no prelude**: nothing is in scope without a `needs`, `Result` and `Option` included. The `nogc` checker rules are under active design. Codegen targets LLVM.

The library is written in Aelys, not in the compiler. It is reached by sitting next to the root file, because a `needs` resolves to exactly one path and there is no module search path. It cannot print a `string`: `string` is outside the external type surface, so `std/io` writes bytes, and printing a string still goes through the `println` builtin. All of `math`, `slice` and `sort`, and the whole of `str`'s reading surface, are callable from `nogc` with the managed allocator untouched.

Language semantics and the IR are not stable, and neither is the standard library: it is one run old, every container function is fixed to `i64`, `Vec` is append-only because that is the whole builtin surface, and there is no ordering on strings. Open design questions include the collection strategy, iterator design, and whether a freestanding mode is in scope at all. The managed / `nogc` boundary rules are substantially settled: what a `nogc` function may do is decided per operation, and a refusal names the operation and the path that reaches it rather than the type.

## Contributing

Bug reports, design feedback, and discussion around language semantics and compiler behavior are the most useful contributions right now. The codebase changes quickly, so if you want to contribute code, open an issue first so the work can be aligned with the current direction.
