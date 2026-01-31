# Performance Benchmarks

These benchmarks were run on Linux x86_64. Your results will vary based on hardware

## Recursive Fibonacci

Computing `fib(35)` with naive recursive implementation (no memoization)  
I think it's a good benchmark for interpreters since it stresses function call overhead and recursion depth.

```rust
fn fib(n: int) -> int {
    if n <= 1 { return n }
    return fib(n - 1) + fib(n - 2)
}
```

### Results

| Implementation | Time  | Notes |
|----------------|-------|-------|
| **Aelys (typed)** | 873ms | With type annotations |
| **Aelys (untyped)** | 995ms | Without type annotations |
| Python 3.11 | 982ms | Similar performance |
| Lua 5.4 | 616ms | Faster |
| Node.js | 86ms  | V8's JIT is hard to beat |

Aelys is roughly on par with Python for this workload, not great, not terrible. Node.js with V8's JIT compiler is in a different league entierely.

Type annotations give a small speedup because the VM can skip some runtime type checks.

## Startup Time

Time from launching the binary to executing the first instruction:

```bash
time aelys-cli examples/hello.aelys
```

**Result: <1ms**

Startup is essentially instantaneous. The binary loads fast, and parsing + compilation of a small file takes microseconds.

For larger programs, a few thousand lines still compiles in under 50ms.

## Memory Usage

The VM starts with a small heap and grows as needed. Typical memory usage:

| Program | RSS |
|---------|-----|
| Hello world | ~2 MB |
| HTTP server (idle) | ~4 MB |
| Mandelbrot demo | ~3 MB |

Memory usage is dominated by the Rust runtime and stdlib. Aelys's own data structures are pretty compact.

But for `@no_gc` code, obviously emory usage depends entirely on what you allocate.  
The manual heap is separate from the GC heap.

## What's Slow

- **No JIT**: Everything is interpreted bytecode. Compared to V8, LuaJIT, or even PyPy, we're at a disadvantage for compute-heavy code.
- **GC pauses**: The GC is stop-the-world (mark and sweep), for most code this is « fine » but if you're allocating heavily in a loop, you'll feel it.

I plan on adding a JIT compiler in the future but that's a big project  
Either using LLVM or Cranelift as a backend

Regarding the GC, I'll probably use this in the future : https://github.com/kyren/gc-arena

## What's Fast

- **Startup**: Near-instant
- **Compilation**: Very fast
- **Simple loops**: Superinstructions help
- **Native stdlib calls**: Optimized with inline caching

## Array and Vec Performance

Arrays and vectors got some work to make them competitive with Python. Here's the results:

### Benchmarks vs Python

| Operation | Aelys | Python | Winner |
|-----------|-------|--------|--------|
| Vec Push (1M ops) | 40ms | 64ms | **Aelys (1.6x)** |
| Vec Pop (1M ops) | 32ms | 77ms | **Aelys (2.4x)** |
| Matrix Multiply 100x100 | 66ms | 84ms | **Aelys (1.3x)** |
| Bubble Sort (5K elements) | 1673ms | 2260ms | **Aelys (1.35x)** |
| Array Sum (1M elements) | 37ms | 26ms | Python (1.4x) |

Aelys wins on mutations (push/pop) and compute-heavy loops. Python wins on simple iteration because CPython has decades of micro-optimizations for patterns like `for x in list`.

### Why it's fast

The VM uses specialized storage for each type:
- `Array<Int>` and `Vec<Int>` → `Box<[i64]>` (8 bytes/element, no boxing overhead)
- `Array<Float>` → `Box<[f64]>` (8 bytes/element)
- `Array<Bool>` → `Box<[u8]>` (1 byte/element, 8x more compact than boxing each bool)

The compiler emits type-specific opcodes too. `arr[i]` compiles to `ArrayLoadI` for Int, `ArrayLoadF` for Float, etc. No runtime type checking.

### Vec growth

Vecs double in capacity when they run out of space:
- Start at 4
- Growth: 4 → 8 → 16 → 32 → ...
- Use `reserve(n)` to pre-allocate if you know the final size

```rust
let v = Vec[]
v.reserve(1000)  // one allocation instead of multiple
for i in 0..1000 {
    v.push(i)
}
```

### Tips

1. **Pre-allocate when you know the size**
   ```rust
   let v = Vec[]
   v.reserve(10000)
   ```

2. **Use Array for fixed-size data**
   ```rust
   let coords = Array[x, y, z]  // simpler than Vec
   ```

3. **Batch operations**
   ```rust
   // One loop
   for i in 0..arr.len() {
       arr[i] = arr[i] * 2 + 1
   }

   // Instead of two
   for i in 0..arr.len() { arr[i] = arr[i] * 2 }
   for i in 0..arr.len() { arr[i] = arr[i] + 1 }
   ```

Run the benchmarks:
```bash
./examples/benchmark/run_array_benchmarks.sh
```

## Improving Performance

If your Aelys code is too slow:

1. **Add type annotations** - small speedup from fewer runtime checks
2. **Use `@no_gc` for hot paths** - eliminates GC pauses
3. **Pre-allocate arrays/vecs** - avoid reallocation overhead
4. **Precompile to bytecode** - skip parse/compile on each run
5. **Profile first** - make sure you're optimizing the right thing

For truly performance-critical code, consider writing a native module in Rust and calling it from Aelys. That's what the native FFI is for.