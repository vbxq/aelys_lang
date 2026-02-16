<p align="center">
  <img src="docs/aelys_banner.png?v=2" alt="aelys virtual machine" width="1000">
</p>

<a href="https://github.com/vbxq/aelys_lang/actions/workflows/ci.yml"><img src="https://github.com/vbxq/aelys_lang/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/github/license/vbxq/aelys_lang?color=8A2BE2" alt="License"></a>
<a href="https://github.com/vbxq/aelys_lang/releases/latest"><img src="https://img.shields.io/github/v/release/vbxq/aelys_lang?color=8A2BE2" alt="Release"></a>

# aelys 0.19.14-a

Register-based VM with dual memory management: GC by default, `@no_gc` for performance-critical code.

You choose between comfort and performance on a per-function basis.

## Quick Example

```rust
// No imports needed for basic functions
println("Hello from Aelys!")

// String methods
let name = "  Aelys  ".trim().to_upper()
println("Name: {name}")

// Arrays and iteration
let numbers = [1, 2, 3, 4, 5]
let mut sum = 0
for n in numbers {
    sum += n
}
println("Sum: {sum}")

// Vectors (dynamic)
let items = Vec[10, 20, 30]
items.push(40)
println("Length: {items.len()}")

// Functions
fn factorial(n: int) -> int {
    if n <= 1 { return 1 }
    return n * factorial(n - 1)
}
println("5! = {factorial(5)}")
```

## Documentation

- [Build Instructions](docs/installation.md)
- [Getting Started Guide](docs/getting-started.md)
- [Language Specification](docs/language-spec.md)
- [Standard Library Documentation](docs/standard-library.md)

## Additional Information

- [Performance Benchmarks](docs/performance-benchmarks.md)
- [Acknowledgements](ACKNOWLEDGEMENTS.md)
- [Changelog](CHANGELOG.md)
- [Examples](examples/README.md)
- [License](LICENSE)
- [FAQ](docs/faq.md)
