<p align="center">
  <img src="docs/aelys_banner.png?v=3" alt="aelys" width="1000">
</p>

# Aelys

A programming language with managed memory by default and explicit opt-out for performance-critical code.

Most languages force a single memory model on the entire program. GC'd languages pay for a runtime on every path; systems languages demand manual control everywhere.

Aelys starts from managed memory, reference counting with an optional cycle collector, and lets you leave it behind one function at a time. Default mode gives you heap allocation, type inference, and minimal annotation. `nogc` gives you compiler-enforced freedom from *additional* allocation, the caller's buffers are still the caller's, but nothing inside the region allocates, with statically checked references and no user-written lifetime annotations.

The example below is **aspirational**: it shows the language this project is aiming at. Each construct named here was written out and handed to the compiler at the current commit, and every one of them is refused: `.len()` as a call, `range()`, `Vec<T>::new(n)`, a `Vec` held inside a struct (`E0410`), operator overloading on a user type, the `fail` operator, unqualified `Ok`/`Err`, named call arguments, `or` as a fallback, nested patterns, and user-defined methods. Two of those names exist in a narrower form and are easy to misread as missing outright: `.len` is a field rather than a call, and a range is written `0..n` rather than `range(0, n)`. The example is kept because it is the target, not because it compiles.

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

Every fenced block from here on, except the two marked aspirational, was extracted, compiled and run at `-O0` and at `-O2`, and the outputs quoted are the outputs those runs printed. Blocks that name a `std` module are built from the repository root with `aelys compile <file> -I .`, which is what puts `std/` on the module search path; the rest need no flag.

## Memory model

The language provides two levels of control, each narrowing what the runtime provides.

**Default mode.** Reference counting manages the heap, with an optional cycle collector (`--runtime rc+cycles`). Types are inferred and standard collections are managed. Type annotations are optional. `?` propagates errors and `catch` handles them, over the `Result` the prelude puts in scope, or over one the program declares itself. This is the intended level for most code.

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

## Modules and the prelude

A `needs` names a module path. `needs std.io` binds the name `io`, and the module's public functions are reached through it as `io.print_out(...)`; `needs Option, Result from std.result` binds the items themselves.

`a.b.c` is the file `a/b/c.aelys` under a search root. The roots are the directory of the file named on the command line, first, then every `-I <dir>` given, in the order they were written. There is no default root and no environment variable: a module outside the program's own directory is reachable only through `-I`. If one module path resolves under two `-I` roots the compiler stops with `E0622` instead of picking one, because the roots are peers and taking the first would compile a different library than the same invocation with the roots written the other way round. The root file's own directory is not a peer, it outranks every `-I` root, so a copy sitting next to the program shadows the library on the search path and is never ambiguous.

The prelude is one module, `std.prelude`, looked up on that same search path. It re-exports `Option` and `Result`, which is why `?` and `catch` work in a program that imports nothing. A prelude name loses to anything the file binds itself, **silently and with no diagnostic**: define your own `Option` and it is yours, everywhere in that file. `--no-prelude` turns the lookup off, and a prelude that does not resolve is not an error either, so a program compiled without `std/` on a search root simply has no `Result` in scope and says so as an unknown type.

<!-- ```rust is used for syntax highlighting only, this is Aelys -->
```rust
needs std.io
needs std.str

fn main() -> i64 {
    let parts = str.split("a,b,c", ",")
    for p in Vec::as_slice(parts) {
        io.print_out(p)
        io.print_out("\n")
    }
    match str.parse_int("41") {
        Result::Ok(v) => println(v + 1),
        Result::Err(_) => println(-1)
    }
    return 0
}
```

That prints `a`, `b`, `c`, `42`. Nothing in it imports `Result`.

A library can print, and `io` is how. `io.print_out(s)` and `io.print_err(s)` take a `string`; `io.write_out(bytes)` and `io.write_all(fd, bytes)` take a `&[u8]`. All of them are `nogc`, and all of them are Aelys written over a single `unsafe extern nogc fn write`. The `println` builtin is still there and is still the shortest way to print a number.

## Text

`string` is UTF-8. `.len` counts **bytes**, and `s.bytes` is the byte view, a `&[u8]`.

`s[i]` does not exist. It was removed rather than fixed: `.len` counted bytes while `s[i]` counted characters, and the two could not both be right. Character access is `for c in s`, which yields a `char`, or `str.char_at(s, i)`, which yields an `Option<char>` and walks the string to reach it.

A `char` is a Unicode scalar value. Literals are written `'x'`. It carries the six comparison operators, `c as i64` converts out, and `char::from_i64` converts in.

<!-- ```rust is used for syntax highlighting only, this is Aelys -->
```rust
fn main() -> i64 {
    let s: string = "héllo"
    println(s.len)
    println(s.bytes.len)
    let mut chars: i64 = 0
    for c in s {
        chars = chars + 1
        if c == 'é' { println(c as i64) }
    }
    println(chars)
    return 0
}
```

That prints `6`, `6`, `233`, `5`: six bytes, five characters.

A scalar value is not a user-perceived character. One thing a reader sees as a single letter can be several scalar values, and `for c in s` hands back every one of them separately.

<!-- ```rust is used for syntax highlighting only, this is Aelys -->
```rust
fn main() -> i64 {
    let composed: string = "é"
    let decomposed: string = "e" + string::from_char(char::from_i64(769))
    println(composed.len)
    println(decomposed.len)
    let mut a: i64 = 0
    for c in composed { a = a + 1 }
    let mut b: i64 = 0
    for c in decomposed { b = b + 1 }
    println(a)
    println(b)
    return 0
}
```

That prints `2`, `3`, `1`, `2`. The two strings render identically, are not equal, and one of them counts two characters where a reader counts one. There is no normalization and no grapheme segmentation, in the language or in the library. Counting what a user would call characters is not something Aelys does for you today, and a `char` loop is not that count.

## Generics

A function takes a type parameter and unifies it against the element of a slice at the call site.

Bounds are built-in and structural. There are exactly three, `nogc`, `eq` and `ord`, combined with `+`. There is no user-written bound and no user-written impl: **this is not a trait system**, and a name that is not one of the three is refused where it is written (`E0106`). `nogc` says the type is a value a `nogc` function may hold: a primitive, a fixed array, a reference, a `nogc fn`, or a non-generic struct or enum built only from those. `eq` is the numeric types, `char`, `bool` and `string`. `ord` is the numeric types and `char`.

<!-- ```rust is used for syntax highlighting only, this is Aelys -->
```rust
nogc fn largest<T: ord>(xs: &[T]) -> T {
    let mut best: T = xs[0]
    for x in xs {
        if x > best { best = x }
    }
    return best
}

fn main() -> i64 {
    let mut ns: Vec<i64> = vec[3, 9, 4]
    println(largest(ns[0..3]))
    let mut fs: Vec<f64> = vec[1.5, 0.25]
    println(largest(fs[0..2]))
    return 0
}
```

That prints `9` and `1.5`.

`ord` deliberately excludes `string`: calling `largest` on a `&[string]` is `E0732`. Strings order through `str.compare(a, b)`, which answers `-1`, `0` or `1` over the bytes, and `sort.strings` is the entry point built on it.

<!-- ```rust is used for syntax highlighting only, this is Aelys -->
```rust
needs std.sort
needs std.io

fn main() -> i64 {
    let mut ss: Vec<string> = vec["pear", "apple", "fig"]
    sort.strings(ss[0..3])
    for s in Vec::as_slice(ss) {
        io.print_out(s)
        io.print_out("\n")
    }
    return 0
}
```

That prints `apple`, `fig`, `pear`.

## Safety and the optimizer

Whether a program is accepted is a property of the program, not of how hard the compiler was asked to optimize. A refusal raised at `-O0` is raised identically at `-O3`, and nothing the optimizer does may buy a program past a check.

The compiler verifies that rather than assuming it. Every compile above `-O0` runs the front half twice, once with the optimizer off as a reference and once at the level asked for, and compares the two verdicts. If they disagree it refuses with `E0432`, printing the refusing level's diagnostic in full first, because that is the actionable half, and naming the split after it. The comparison runs from type inference to the end of AIR validation; the LLVM pass pipeline, the object writer and the linker run once, below it. Switching to the accepting level is never the answer, since the binary it produces is not covered by the refusal the other level raised.

## Status

Aelys is an experimental language and compiler project under active rewrite. It is not ready for use.

```
source → parser → semantic analysis → AIR (Aelys IR) → LLVM IR → native code
```

Parser and semantic analysis are partially implemented. There is a standard library under `std/`. Codegen targets LLVM.

The library is written in Aelys, not in the compiler. It is reached through the search path above, either by sitting next to the root file or through an `-I` root, and the prelude is found the same way. `math`, `slice`, `sort` and `io` are `nogc` throughout and are callable from a `nogc` function with the managed allocator untouched. `str` is `nogc` except for the six functions that build a new string: `substring`, `repeat`, `trim`, `split`, `join` and `from_int`. `result` and `vec` are not `nogc`.

Language semantics and the IR are not stable, and neither is the standard library. `std.slice` and `std.sort` are generic over their element wherever a bound can say what the operation needs, and `slice.sum` is the one that stays at `i64` because there is no bound for arithmetic yet; `std.vec` is still fixed to `i64` in every function. The whole builtin `Vec` surface is `new`, `push`, `pop`, `len`, `as_slice` and `try_as_unique_mut_slice`, and every container function in the library is built out of those six. Open design questions include the collection strategy, iterator design, whether normalization or grapheme segmentation belong in the library at all, and whether a freestanding mode is in scope. The managed / `nogc` boundary rules are substantially settled: what a `nogc` function may do is decided per operation, and a refusal names the operation and the path that reaches it rather than the type.

## Contributing

Bug reports, design feedback, and discussion around language semantics and compiler behavior are the most useful contributions right now. The codebase changes quickly, so if you want to contribute code, open an issue first so the work can be aligned with the current direction.
