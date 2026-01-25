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

## Improving Performance

If your Aelys code is too slow:

1. **Add type annotations** - small speedup from fewer runtime checks
2. **Use `@no_gc` for hot paths** - eliminates GC pauses
3. **Precompile to bytecode** - skip parse/compile on each run
4. **Profile first** - make sure you're optimizing the right thing

For truly performance-critical code, consider writing a native module in Rust and calling it from Aelys. That's what the native FFI is for.