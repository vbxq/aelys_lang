# Examples

Working examples to learn from. Run any of them with `aelys-cli <path>`.

## Basics

### [hello.aelys](hello.aelys)

The classic hello world, plus some system info:

```bash
aelys-cli examples/hello.aelys
```

## Language Features

### [lang/factorial.aelys](lang/factorial.aelys)

Recursive factorial with type annotations. Shows basic function definitions and loops.

### [lang/fibonacci.aelys](lang/fibonacci.aelys)

Several fibonacci implementations comparing different approaches.

### [lang/type_annotations.aelys](lang/type_annotations.aelys)

Demonstrates the type system: annotations, inference, closures, and higher-order functions.

### [lang/simple_no_gc_demo.aelys](lang/simple_no_gc_demo.aelys)

Shows `@no_gc` usage with manual memory:
- `alloc` / `free` for buffer management
- `store` / `load` for data access

## Arrays and Vectors

The `arrays/` directory has examples showing how to work with collections:

### Quick Start

- **[arrays/intro.aelys](arrays/intro.aelys)** - Basic intro to arrays and vectors
- **[arrays/statistics.aelys](arrays/statistics.aelys)** - Calculate average, min/max, median
- **[arrays/histogram.aelys](arrays/histogram.aelys)** - Draw ASCII histogram from data
- **[arrays/grade_calculator.aelys](arrays/grade_calculator.aelys)** - Weighted grade calculation

### More Examples

- **[arrays/basics.aelys](arrays/basics.aelys)** - Array/Vec creation and modification
- **[arrays/typed_arrays.aelys](arrays/typed_arrays.aelys)** - Array<Int>, Array<Float>, Array<Bool>
- **[arrays/typed_vecs.aelys](arrays/typed_vecs.aelys)** - Vec<Int>, Vec<Float>, Vec<Bool>
- **[arrays/vec_operations.aelys](arrays/vec_operations.aelys)** - push, pop, capacity, reserve
- **[arrays/vec_stack.aelys](arrays/vec_stack.aelys)** - Using Vec as a stack
- **[arrays/sum_product.aelys](arrays/sum_product.aelys)** - Sum, product, max, min
- **[arrays/bubble_sort.aelys](arrays/bubble_sort.aelys)** - Sorting algorithm
- **[arrays/typed_algorithms.aelys](arrays/typed_algorithms.aelys)** - Generic algorithms

Performance comparisons with Python are in [docs/performance-benchmarks.md](../docs/performance-benchmarks.md#array-and-vec-performance).

## Demos

Visual demos that run in the terminal.

### [graphical_demo/mandelbrot.aelys](graphical_demo/mandelbrot.aelys)

Animated ASCII Mandelbrot set with zoom. Uses `@no_gc` for the framebuffer.

```bash
aelys-cli examples/graphical_demo/mandelbrot.aelys
```

### [graphical_demo/game_of_life.aelys](graphical_demo/game_of_life.aelys)

Conway's Game of Life. Runs in a terminal.

### [graphical_demo/doom_fire.aelys](graphical_demo/doom_fire.aelys)

Recreation of the PSX Doom fire effect.

### [graphical_demo/donut.aelys](graphical_demo/donut.aelys)

Spinning 3D donut rendered in ASCII. The classic demo.

## Applications

### [aelys-http-server/](aelys-http-server/)

A working HTTP server implementation. Shows:
- Module organization
- TCP networking with `std.net`
- File serving with `std.fs`
- Request parsing and routing

Run it:

```bash
cd examples/aelys-http-server
aelys-cli --allow-caps=fs,net server.aelys
```

Then visit http://localhost:8080 in your browser.

## Benchmarks

### [benchmark/](benchmark/)

Performance test files:

- `fib_typed.aelys` - Fibonacci with type annotations
- `fib_untyped.aelys` - Fibonacci without types
- `mandelbrot_gc.aelys` - Mandelbrot with normal GC
- `mandelbrot_nogc.aelys` - Mandelbrot with `@no_gc`

## Native Extensions

### [native/opengl/](native/opengl/)

OpenGL bindings via native Rust library. Demonstrates how to write native modules and call them from Aelys.

Requires building the native library first - see the README in that directory.

## Optimizer Tests

### [test_opt/](test_opt/)

Test cases for the optimizer. Not really examples, but useful if you want to understand what the optimizer does or if you're debugging optimization passes.

---

## Running Examples

Most examples just need:

```bash
aelys-cli examples/<path>
```

Some need capabilities:

```bash
# File system access
aelys-cli --allow-caps=fs examples/file_example.aelys

# Network access
aelys-cli --allow-caps=net examples/net_example.aelys

# Both
aelys-cli --allow-caps=fs,net examples/aelys-http-server/server.aelys
```

For demos that use terminal control, make sure your terminal supports ANSI escape sequences (most do).
