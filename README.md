<p align="center">
  <img src="docs/aelys_banner.png?v=3" alt="aelys" width="1000">
</p>

# Aelys

A programming language with managed memory by default and explicit opt-out for performance-critical code.

Most languages force a single memory model on the entire program. GC'd languages pay for a runtime on every path; systems languages demand manual control everywhere.

Aelys starts from managed memory, reference counting with an optional cycle collector, and lets you leave it behind one function at a time. Default mode gives you heap allocation, type inference, and minimal annotation. `nogc` gives you compiler-enforced freedom from *additional* allocation: the caller's buffers are still the caller's, but nothing inside the function allocates, with statically checked references and no user-written lifetime annotations.

```rust run
fn half(n: i64) -> Result<i64, string> {
    if n % 2 != 0 { return Err("odd") }
    return Ok(n / 2)
}

fn quarter(n: i64) -> Result<i64, string> {
    let h = half(n)?
    return half(h)
}

nogc fn scale(src: &[i64], dst: &mut [i64]) -> i64 {
    let mut i: i64 = 0
    while i < src.len {
        dst[i] = src[i] * 2
        i = i + 1
    }
    return src.len
}

fn main() -> i64 {
    match quarter(12) {
        Ok(v) => println(v),
        Err(e) => println(e)
    }
    match quarter(6) {
        Ok(v) => println(v),
        Err(e) => println(e)
    }
    let q = quarter(10) catch |e| -1
    println(q)

    let mut input: Vec<i64> = vec[1, 2, 3]
    let mut output: Vec<i64> = vec[0, 0, 0]
    let n = scale(input[0..3], output[0..3])
    println(output[2])
    return 0
}
```

```output
3
odd
-1
6
```

`half` and `quarter` are default mode: `?` propagates an error and `catch` recovers from one, over the `Result` the prelude puts in scope. `scale` is `nogc`: it allocates nothing, and it reads and writes through slices of two managed `Vec`s that `main` hands it. Managed code calls `nogc` freely; the reverse is refused where the call reaches managed allocation, and the refusal names the path (see [How a refusal reads](#how-a-refusal-reads)).

Beyond the memory model: inferred types, pattern matching, compilation to native code via LLVM.

## How the examples are checked

Every fenced block in this file is extracted and checked by `aelys/tests/readme_examples_tests.rs`. Blocks are marked `rust` for syntax highlighting only; they are Aelys.

- `rust run` is a whole program. It is compiled at `-O0` and at `-O2` and run. Its standard output must equal the `output` block after it, its standard error must equal the `stderr` block if there is one and be empty otherwise, and its exit status must be 0 unless the block says `exit=N`. A block that names `rc+cycles` is also compiled and run under that runtime, and `stats` runs it with `AELYS_RC_STATS=1`.
- `rust refused E....` must be refused at both levels with exactly the listed error codes, in order. When a `diagnostic` block follows, it is the compiler's output, with the temporary directory cut out of the paths and without the empty line the CLI prints last.
- `rust module file=P` is a library file written at `P` beside the next program.
- An unknown word after `rust run` or `rust refused`, a `stderr` block for a runtime the program does not run under, a `~~~` fence and an indented code block all fail the test.
- `sh` blocks are run as written, and the `output` block after them is what they print.
- The one `text` block is a diagram and is not checked.

The test counts the blocks of each kind, so a block cannot be added without being checked.

## Building and running

`cargo build --release` builds the compiler as `target/release/aelys-cli`, and this file calls it `aelys-cli`. Its messages call it `aelys` ("try `aelys --explain E0433`"); the command to type is `aelys-cli --explain E0433`. It links programs with the system `cc`.

```rust run file=hello.aelys
fn main() -> i64 {
    println("hello, world")
    return 0
}
```

```output
hello, world
```

```sh
aelys-cli compile hello.aelys
./hello
aelys-cli compile hello.aelys -O0 -o greet
./greet
ls
```

```output
hello, world
hello, world
greet
hello
hello.aelys
hello.o
```

Without `-o` the executable is written beside the source, named after it without the extension; `-o` names it. The object file `hello.o` is left beside the source either way; this is a known defect. The standard library is built into the compiler, so a program that uses `std` compiles from any directory with no flag, as long as no search root holds its own `std/prelude.aelys`.

## The 0.1 promise

This is what the 0.1 milestone promises, and what it does not. It covers programs the compiler accepts, on Linux x86-64.

**What is guaranteed**

- **The same answer at every level.** A program accepted at one `-O` level is accepted at every level, prints the same output and exits with the same status. A program refused at `-O0` is refused with the same error at `-O3`. Every example in this file is run at `-O0` and `-O2` and must agree.
- **No use-after-free and no double free, outside `unsafe`.** In a program that has no `unsafe { }` block and calls no `extern` function itself (the `unsafe` calls inside `std` are covered), nothing is read after it is freed and nothing is freed twice. Under the default runtime `rc` and under `rc+cycles`, such a program's memory stays bounded by what it keeps alive, with one exception: a cycle of `Rc` values under `rc` (below).
- **Cycles are collected during execution under `--runtime rc+cycles`.** The collector runs at points the compiler inserts at function entry and on loop back edges, never inside `nogc` code.
- **Run-time checks stop the program.** An index outside a `Vec`, an array or a slice, a slice range past the end, a division or remainder by zero, the minimum of a signed type divided by `-1` or taken modulo `-1` in every signed type, `Vec::pop` on an empty `Vec` and `char::from_i64` of a value that is not a Unicode scalar value print a message on standard error and stop the program with `SIGABRT`, at every level. A shell shows the status as 134, and the system may write a core dump.
- **Every refusal is named.** A form the compiler does not support is refused with an error code and a message that says what the form is. `E0901` and `E0432` mean the compiler itself is wrong; an ordinary program that produces either one is a compiler bug. `E0903` means a tool outside the compiler, the linker or the file system, refused after the compiler did its part.

The level point is checked by `aelys/tests/opt_invariance_tests.rs` and `aelys/tests/group_release01_s2b_tests.rs`, the memory points by the `group_rc*` and `group_release01*` suites in `aelys/tests` and by `scripts/memory_growth_sweep.sh`, which measures peak memory at 10^5 and 10^6 iterations.

Each run below allocates 200000 `Rc` nodes in 100000 two-node cycles. `[rc]` counts the managed allocations and frees, `[cycles]` counts the collections.

```rust run rc rc+cycles stats
struct Node { val: i64, next: Rc<Node> }

fn churn(i: i64) -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: i, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: a })
    a.next = b
    return Rc::get(Rc::get(a).next).val
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 100000 {
        n = n + churn(i)
        i = i + 1
    }
    println(n)
    return 0
}
```

```output
200000
```

```stderr rc
[rc] allocs=200000 frees=0
[raw] allocs=0 frees=0
```

```stderr rc+cycles
[rc] allocs=200000 frees=200000
[raw] allocs=0 frees=0
[cycles] collections=24 clamped=0
```

**What is not guaranteed**

- Under the default runtime `rc`, a cycle of `Rc` values is never freed, as the run above shows. Use `--runtime rc+cycles` for a program that builds cycles.
- Inside an `unsafe { }` block and through an `extern` function, memory is the programmer's responsibility: a pointer handed to C can be freed twice, or read after C freed it, and nothing checks it. Outside `unsafe`, calling an `extern` function is refused (`E0617`, see [External functions](#external-functions)).
- `+`, `-`, `*` and negation wrap on overflow, silently, in every integer type. A shift amount is taken modulo the width of the left operand, and the result has the type of the left operand.
- `as` between integer types truncates; `as` from a float to an integer saturates, and a NaN converts to 0. An integer literal above `i64::MAX` is refused (`E0003`), even for a `u64`.
- A global holding a `string`, directly or in a struct field, that the program assigns at run time is not released at exit. The memory goes back with the process, and LeakSanitizer reports nothing; `AELYS_RC_STATS` counts one allocation more than frees per such string.
- A `u64` above `i64::MAX` prints as a negative number, through `println` and through interpolation. The stored value is right modulo 2^64.
- A slice must start at 0: `v[1..3]` is refused with `E0425` (see [Lengths and indexing](#lengths-and-indexing)).
- The object file is left beside the source, as shown above.

```rust run
fn main() -> i64 {
    let mut big: i64 = 9223372036854775807
    big = big + 1
    println(big)
    let mut b: u8 = 250
    b = b + 10
    println(b)
    let one: u8 = 1
    let k: u8 = 9
    println(one << k)
    let mut u: u64 = 9223372036854775807
    u = u + 1
    println(u)
    println("{u}")
    let min: i64 = -9223372036854775807 - 1
    println(-min)
    return 0
}
```

```output
-9223372036854775808
4
2
-9223372036854775808
-9223372036854775808
-9223372036854775808
```

```rust run
fn main() -> i64 {
    let x: i64 = 300
    println(x as u8)
    let big: f64 = 1.0e30
    println(big as i64)
    let z: f64 = 0.0
    println((z / z) as i64)
    println(-2.7 as i64)
    return 0
}
```

```output
44
9223372036854775807
0
-2
```

```rust refused E0003
fn main() -> i64 {
    let x: u64 = 18446744073709551615
    return 0
}
```

```rust run stats
let mut last: string = ""

fn main() -> i64 {
    let i: i64 = 3
    last = "run {i}"
    println(last)
    return 0
}
```

```output
run 3
```

```stderr
[rc] allocs=2 frees=1
[raw] allocs=0 frees=0
```

Floating point follows IEEE 754: `NaN != NaN` is `true`, a NaN prints as `nan` whatever its sign bit, and `%` works on floats.

```rust run
fn main() -> i64 {
    let z: f64 = 0.0
    let n: f64 = z / z
    println(n)
    println(n != n)
    println(n == n)
    println(1.0 / z)
    let x: f64 = 7.5
    let y: f64 = 2.0
    println(x % y)
    return 0
}
```

```output
nan
true
false
inf
1.5
```

## Lengths and indexing

`.len` is a field, not a call. On a `string` (also spelled `str`) it counts **bytes**. On an array, a `Vec` and a slice it counts elements. `s.bytes` is the byte view of a string, a `&[u8]`. On a `&string` it is refused: pass the `string` itself.

```rust run
fn main() -> i64 {
    let s: string = "héllo"
    let t: str = "héllo"
    let a: [i64; 4] = [1, 2, 3, 4]
    let mut v: Vec<i64> = vec[1, 2, 3]
    Vec::push(v, 4)
    let view = v[0..2]
    println(s.len)
    println(t.len)
    println(s.bytes.len)
    println(a.len)
    println(view.len)
    println(v.len)
    return 0
}
```

```output
6
6
6
4
2
4
```

```rust refused E0304
fn size(s: &string) -> i64 {
    return s.len
}

fn main() -> i64 {
    let t: string = "abc"
    println(size(&t))
    return 0
}
```

`v[i]` on a `Vec`, an array or a slice is the element at position `i`, counted from 0. Every index is checked. An index outside the elements, negative included, stops the program with `index out of bounds` and status 134:

```rust run exit=134
fn main() -> i64 {
    let v: Vec<i64> = vec[10, 20, 30]
    let mut i: i64 = 0
    while i <= v.len {
        println(v[i])
        i = i + 1
    }
    return 0
}
```

```output
10
20
30
```

```stderr
index out of bounds
```

A constant index into a fixed array is checked when the program is compiled:

```rust refused E0433
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    println(a[3])
    return 0
}
```

```diagnostic
error[E0433]: [const-index] index 3 is outside `[_; 3]`, whose elements are 0 to 2
 --> main.aelys:3:15
  |
3 |     println(a[3])
  |               ^ this index is outside the array

For more information about this error, try `aelys --explain E0433`
error: aborting due to 1 previous error
```

`v[0..n]` is a slice of the first `n` elements. A range past the end stops the program:

```rust run exit=134
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let n: i64 = 5
    let s = v[0..n]
    println(s.len)
    return 0
}
```

```output
```

```stderr
slice range out of bounds
```

A range that does not start at 0 is refused:

```rust refused E0425
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let tail = v[1..3]
    return 0
}
```

`Vec::pop` on an empty `Vec` stops the program:

```rust run exit=134
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1]
    println(Vec::pop(v))
    println(Vec::pop(v))
    return 0
}
```

```output
1
```

```stderr
pop from an empty Vec
```

`i64::MIN / -1` has no `i64` answer and stops the program:

```rust run exit=134
fn main() -> i64 {
    let a: i64 = -9223372036854775807 - 1
    let b: i64 = -1
    println(a / b)
    return 0
}
```

```output
```

```stderr
division overflow
```

The minimum of a signed type modulo `-1` stops the program in every signed type, here an `i32`:

```rust run exit=134
fn main() -> i64 {
    let a: i32 = -2147483647 - 1
    let b: i32 = -1
    println(a % b)
    return 0
}
```

```output
```

```stderr
division overflow
```

`char::from_i64` of a value that is not a Unicode scalar value stops the program:

```rust run exit=134
fn main() -> i64 {
    println(char::from_i64(233) as i64)
    let c = char::from_i64(55296)
    println(c as i64)
    return 0
}
```

```output
233
```

```stderr
char::from_i64: 55296 is not a unicode scalar value, the valid ranges are 0..=1114111 excluding 55296..=57343
```

Division and remainder by zero stop the program the same way:

```rust run exit=134
fn main() -> i64 {
    let d: i64 = 0
    println(1)
    println(7 / d)
    println(2)
    return 0
}
```

```output
1
```

```stderr
division by zero
```

## Text

`string` is UTF-8. `.len` counts bytes. `s[i]` does not exist: it was removed rather than fixed, because `.len` counted bytes while `s[i]` counted characters, and the two could not both be right.

```rust refused E0304
fn main() -> i64 {
    let s: string = "abc"
    println(s[0])
    return 0
}
```

```diagnostic
error[E0304]: a `string` cannot be indexed by integer; `.len` counts bytes while `s[i]` counted characters, and the two could not both be right
 --> main.aelys:3:13
  |
3 |     println(s[0])
  |             ^ invalid member access
   = help: read the characters with `for c in s`, which yields a `char`, or ask for one with `str.char_at(s, i)`

For more information about this error, try `aelys --explain E0304`
error: aborting due to 1 previous error
```

Character access is `for c in s`, which yields a `char`, or `str.char_at(s, i)`, which yields an `Option<char>` and walks the string to reach character `i`.

A `char` is a Unicode scalar value. Literals are written `'x'`. `c as i64` converts out, and `char::from_i64` converts in.

```rust run
needs std.str

fn main() -> i64 {
    let s: string = "héllo"
    println(s.len)
    let mut chars: i64 = 0
    for c in s {
        chars = chars + 1
        if c == 'é' { println(c as i64) }
    }
    println(chars)
    match str.char_at(s, 1) {
        Some(c) => println(c as i64),
        None => println(-1)
    }
    match str.char_at(s, 9) {
        Some(c) => println(c as i64),
        None => println(-1)
    }
    return 0
}
```

```output
6
233
5
233
-1
```

Six bytes, five characters.

A scalar value is not a user-perceived character. One thing a reader sees as a single letter can be several scalar values, and `for c in s` hands back every one of them separately.

```rust run
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
    println(composed == decomposed)
    return 0
}
```

```output
2
3
1
2
false
```

The two strings render identically, are not equal, and one of them counts two characters where a reader counts one. There is no normalization and no grapheme segmentation, in the language or in the library. Counting what a user would call characters is not something Aelys does for you today, and a `char` loop is not that count.

## Memory model

The language provides two levels of control, each narrowing what the runtime provides.

**Default mode.** Reference counting manages the heap, with an optional cycle collector (`--runtime rc+cycles`). Types are inferred and standard collections are managed. `?` propagates errors and `catch` handles them, over the `Result` the prelude puts in scope. This is the intended level for most code.

**`nogc` functions.** A function-level opt-out from managed allocation. Inside a `nogc` function, managed allocation is rejected at compile time, managed containers cannot be created, and calls that reach managed allocation are rejected, naming the path that reaches it. The compiler proves that the function satisfies these constraints or rejects it. No warnings.

The boundary is a `nogc` function taking `&[T]` and `&mut [T]` cut from managed containers and writing through the mutable one. This compiles and runs:

```rust run
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

```output
6
```

**References.** In every function, references and slices are checked when the program is compiled, with no user-written lifetime annotations. A `Vec` cannot be written while a slice of it is still used:

```rust refused E0711
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let view = v[0..2]
    Vec::push(v, 4)
    println(view[0])
    return 0
}
```

```diagnostic
error[E0711]: [borrow] cannot write to `v` while it is borrowed
 --> main.aelys:4:5
  |
3 |     let view = v[0..2]
  |                ------- borrow created here
4 |     Vec::push(v, 4)
  |     ^^^^^^^^^^^^^^^ borrow occurs here
5 |     println(view[0])
  |     ---------------- borrow last used here

For more information about this error, try `aelys --explain E0711`
error: aborting due to 1 previous error
```

### How a refusal reads

`report` is `nogc` and calls `total`, which builds a `Vec`:

```rust refused E0727
fn total(n: i64) -> i64 {
    let v: Vec<i64> = vec[n, n]
    return v.len
}

nogc fn report(n: i64) -> i64 {
    return total(n)
}

fn main() -> i64 {
    println(report(3))
    return 0
}
```

```diagnostic
error[E0727]: [nogc] function `report` is declared nogc but its inferred effects reach managed memory via `report -> total -> a vec literal`
 --> main.aelys:2:23
  |
2 |     let v: Vec<i64> = vec[n, n]
  |                       ^^^^^^^^^ managed memory reached here
...
6 | nogc fn report(n: i64) -> i64 {
  |      -------------------------- `report` is declared nogc here
7 |     return total(n)
  |            -------- calls `total` here
   = help: keep the value on the stack (an array or a `&[T]` slice), make every function on this path nogc, or drop `nogc` from `report`

For more information about this error, try `aelys --explain E0727`
error: aborting due to 1 previous error
```

The first line is the error code, the area of the compiler in brackets, and what is wrong: here the whole path from the `nogc` function to the allocation. The `-->` line is the file, line and column. The labelled source lines show each step of the path, `help` says what would make the program acceptable, and `aelys-cli --explain E0727` prints a longer explanation of the code. The last line counts the errors.

### What 0.1 refuses

A form outside the 0.1 surface is refused where it is written, with a code, never compiled into something else. A `Vec` inside a struct:

```rust refused E0902
struct Bag { items: Vec<i64> }

fn main() -> i64 {
    let b = Bag { items: vec[1, 2] }
    println(b.items.len)
    return 0
}
```

```diagnostic
error[E0902]: field `items` of struct `Bag` has type `Vec<i64>`: a Vec inside a struct is not supported in 0.1; keep the Vec in its own binding and pass it with `&mut`, or use a fixed-size array field
 --> main.aelys:1:14
  |
1 | struct Bag { items: Vec<i64> }
  |              ^^^^^^^^^^^^^^^ not supported yet

For more information about this error, try `aelys --explain E0902`
error: aborting due to 1 previous error
```

A generic struct:

```rust refused E0902
struct Pair<T> { a: T, b: T }

fn main() -> i64 {
    let p: Pair<i64> = Pair { a: 1, b: 2 }
    println(p.a)
    return 0
}
```

A nested pattern:

```rust refused E0101
fn main() -> i64 {
    let o: Option<Option<i64>> = Some(Some(3))
    match o {
        Some(Some(v)) => println(v),
        _ => println(0)
    }
    return 0
}
```

A `Vec` function other than the six builtins. The whole builtin `Vec` surface is `new`, `push`, `pop`, `len`, `as_slice` and `try_as_unique_mut_slice`, and the compiler says so:

```rust refused E0301
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    Vec::remove(v, 0)
    return 0
}
```

```diagnostic
error[E0301]: unknown method 'remove' on Vec; supported: 'new', 'push', 'pop', 'len', 'as_slice', 'try_as_unique_mut_slice'
 --> main.aelys:3:5
  |
3 |     Vec::remove(v, 0)
  |     ^^^^^^^^^^^^^^^^^ unknown method 'remove' on Vec; supported: 'new', 'push', 'pop', 'len', 'as_slice', 'try_as_unique_mut_slice'

For more information about this error, try `aelys --explain E0301`
error: aborting due to 1 previous error
```

## Modules and the prelude

A `needs` names a module path. `needs std.io` binds the name `io`, and the module's public functions are reached through it as `io.print_out(...)`.

`a.b.c` is the file `a/b/c.aelys` under a search root. The roots are the directory of the file named on the command line, first, then every `-I <dir>` given, in the order they were written. `std.` needs no root: the standard library is built into the compiler.

```rust module file=geom/area.aelys
pub fn area(w: i64, h: i64) -> i64 {
    return w * h
}
```

```rust run
needs geom.area

fn main() -> i64 {
    println(area.area(6, 7))
    return 0
}
```

```output
42
```

A module beside the root file wins over one under an `-I` root, silently:

```rust module file=util.aelys
pub fn twice(n: i64) -> i64 {
    return n * 3
}
```

```rust module file=a/util.aelys
pub fn twice(n: i64) -> i64 {
    return n * 2
}
```

```rust run include=a
needs util

fn main() -> i64 {
    println(util.twice(21))
    return 0
}
```

```output
63
```

If one module path resolves under two `-I` roots and not beside the root file, the compiler stops with `E0622` instead of picking one:

```rust module file=a/util.aelys
pub fn twice(n: i64) -> i64 {
    return n * 2
}
```

```rust module file=b/util.aelys
pub fn twice(n: i64) -> i64 {
    return n + n
}
```

```rust refused E0622 include=a,b
needs util

fn main() -> i64 {
    println(util.twice(21))
    return 0
}
```

```diagnostic
error[E0622]: module 'util' resolves in 2 search roots
 --> main.aelys:1:1
  |
1 | needs util
  | ^^^^^^^^^^^ found in more than one search root
   = note: resolved in: a, b
   = help: drop one of the `-I` roots, or delete the copy that duplicates it

For more information about this error, try `aelys --explain E0622`
error: aborting due to 1 previous error
```

The prelude is the module `std.prelude`. It brings `Option` and `Result` into scope, which is why `?` and `catch` work in a program that imports nothing, and why `Some`, `None`, `Ok` and `Err` name their variants without the enum in front.

```rust run
needs std.io
needs std.str

fn main() -> i64 {
    let parts = str.split("a,b,c", ",")
    for p in Vec::as_slice(parts) {
        io.print_out(p)
        io.print_out("\n")
    }
    match str.parse_int("41") {
        Ok(v) => println(v + 1),
        Err(_) => println(-1)
    }
    return 0
}
```

```output
a
b
c
42
```

A prelude type loses to a type the file declares or imports itself, **silently and with no diagnostic**: define your own `Option` and it is yours, everywhere in that file, and `Some` names its variant. Only the four prelude names are written bare; any other variant keeps its enum in front.

```rust run
enum Option<T> { Some(T), None, Unknown }

fn main() -> i64 {
    let o: Option<i64> = Option::Unknown
    match o {
        Some(v) => println(v),
        None => println(0),
        Option::Unknown => println(-1)
    }
    return 0
}
```

```output
-1
```

The four variant names are reserved: no function, global, variable, parameter or import may take one.

```rust refused E0205
fn main() -> i64 {
    let None: i64 = 3
    return 0
}
```

`--no-prelude` turns the prelude off, and then `Option` is unknown and `Some` names nothing, two errors:

```rust refused E0301 E0301 no-prelude
fn main() -> i64 {
    let o: Option<i64> = Some(4)
    return 0
}
```

`io.print_out(s)` writes a `string` to standard output and answers the number of bytes written. Every function of `std.io` is `nogc`, so a `nogc` function can print. The `println` builtin is the shortest way to print a number.

```rust run
needs std.io

nogc fn greet() -> i64 {
    return io.print_out("from nogc\n")
}

fn main() -> i64 {
    println(greet())
    return 0
}
```

```output
from nogc
10
```

## Generics

A function takes a type parameter and unifies it against the element of a slice at the call site.

Bounds are built-in and structural. There are exactly three, `nogc`, `eq` and `ord`, combined with `+`. There is no user-written bound and no user-written impl: **this is not a trait system**, and a name that is not one of the three is refused where it is written (`E0106`). `nogc` says the type is a value a `nogc` function may hold: a primitive, a fixed array, a reference, a `nogc fn`, or a non-generic struct or enum built only from those. `eq` is the numeric types, `char`, `bool` and `string`. `ord` is the numeric types and `char`.

```rust run
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

```output
9
1.5
```

A `nogc` function's type parameters are `nogc` too, so `largest` does not take a `&[string]`, and the compiler says what a `nogc` value is:

```rust refused E0730
nogc fn largest<T: ord>(xs: &[T]) -> T {
    let mut best: T = xs[0]
    for x in xs {
        if x > best { best = x }
    }
    return best
}

fn main() -> i64 {
    let mut ss: Vec<string> = vec["pear", "apple"]
    println(largest(ss[0..2]))
    return 0
}
```

```diagnostic
error[E0730]: [nogc] type parameter `T` of `largest` is bound `nogc`, but this call instantiates it with `string`, which is not a nogc value
 --> main.aelys:11:21
   |
11 |     println(largest(ss[0..2]))
   |             ------- `T` of `largest` carries a `nogc` bound
   |                     ^^^^^^^^ `nogc` bound not satisfied here
   = help: a nogc value is a primitive, a fixed array, a reference, a `nogc fn`, or a non-generic struct/enum built only from those

For more information about this error, try `aelys --explain E0730`
error: aborting due to 1 previous error
```

`eq` takes `string` and `bool`, and bounds combine with `+`:

```rust run
fn count<T: eq>(xs: &[T], x: T) -> i64 {
    let mut n: i64 = 0
    for y in xs {
        if y == x { n = n + 1 }
    }
    return n
}

fn top<T: ord + nogc>(a: T, b: T) -> T {
    if a > b { return a }
    return b
}

fn main() -> i64 {
    let mut ws: Vec<string> = vec["a", "b", "a"]
    println(count(ws[0..3], "a"))
    let mut bs: Vec<bool> = vec[true, false, false]
    println(count(bs[0..3], false))
    println(top(3, 8))
    println(top('a', 'z') as i64)
    return 0
}
```

```output
2
2
8
122
```

A name that is not a bound is refused. The second error, `E0102` on the closing brace, is a known defect of the parser's recovery after the first one:

```rust refused E0106 E0102
fn show<T: Display>(x: T) -> T {
    return x
}

fn main() -> i64 {
    return 0
}
```

```diagnostic
error[E0106]: `Display` is not a bound; the bounds are `nogc`, `eq` and `ord`, combined with `+`
 --> main.aelys:1:12
  |
1 | fn show<T: Display>(x: T) -> T {
  |            ^^^^^^^ not a bound

For more information about this error, try `aelys --explain E0106`
error[E0102]: expected expression
 --> main.aelys:3:1
  |
3 | }
  | ^

For more information about this error, try `aelys --explain E0102`
error: aborting due to 2 previous errors
```

`ord` deliberately excludes `string`:

```rust refused E0732
fn first<T: ord>(xs: &[T]) -> T {
    return xs[0]
}

fn main() -> i64 {
    let mut ss: Vec<string> = vec["pear", "apple"]
    println(first(ss[0..2]))
    return 0
}
```

```diagnostic
error[E0732]: [bound] type parameter `T` of `first` is bound `ord`, but this call instantiates it with `string`, which is not `ord`
 --> main.aelys:7:19
  |
7 |     println(first(ss[0..2]))
  |             ----- `T` of `first` carries an `ord` bound
  |                   ^^^^^^^^ bound not satisfied here
   = help: the integers, the floats and `char` are `ord`; `string` is not, use `str.compare`

For more information about this error, try `aelys --explain E0732`
error: aborting due to 1 previous error
```

Strings order through `str.compare(a, b)`, which answers `-1`, `0` or `1` over the bytes, and `sort.strings` is built on it.

```rust run
needs std.sort
needs std.str
needs std.io

fn main() -> i64 {
    println(str.compare("apple", "pear"))
    let mut ss: Vec<string> = vec["pear", "apple", "fig"]
    sort.strings(ss[0..3])
    for s in Vec::as_slice(ss) {
        io.print_out(s)
        io.print_out("\n")
    }
    return 0
}
```

```output
-1
apple
fig
pear
```

## External functions

An `unsafe extern fn` declares a C function, and the program links against it. Every call of an external function sits inside an `unsafe { }` block; `unsafe` does not re-enable managed allocation in a `nogc` function.

```rust run
unsafe extern nogc fn strlen(s: &u8) -> u64

fn main() -> i64 {
    let s: string = "héllo\0"
    let b = s.bytes
    let n = unsafe { strlen(&b[0]) }
    println(n)
    return 0
}
```

```output
6
```

```rust refused E0617
unsafe extern nogc fn strlen(s: &u8) -> u64

fn main() -> i64 {
    let s: string = "abc\0"
    let b = s.bytes
    println(strlen(&b[0]))
    return 0
}
```

```rust refused E0727
nogc fn make() -> i64 {
    let n = unsafe { vec[1, 2] }
    return 0
}

fn main() -> i64 {
    return make()
}
```

## Status

Aelys is an experimental language and compiler project under active rewrite. It is not ready for use.

```text
source → parser → semantic analysis → AIR (Aelys IR) → LLVM IR → native code
```

The standard library under `std/` is written in Aelys, not in the compiler, and is built into the compiler binary. Language semantics and the IR are not stable, and neither is the standard library. Open design questions include the collection strategy, iterator design, whether normalization or grapheme segmentation belong in the library at all, and whether a freestanding mode is in scope.

## Contributing

Bug reports, design feedback, and discussion around language semantics and compiler behavior are the most useful contributions right now. The codebase changes quickly, so if you want to contribute code, open an issue first so the work can be aligned with the current direction.
