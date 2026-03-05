<p align="center">
  <img src="docs/aelys_banner.png?v=2" alt="aelys" width="1000">
</p>

<a href="LICENSE"><img src="https://img.shields.io/github/license/vbxq/aelys_lang?color=8A2BE2" alt="License"></a>
<a href="https://github.com/vbxq/aelys_lang/releases/latest"><img src="https://img.shields.io/github/v/release/vbxq/aelys_lang?color=8A2BE2" alt="Release"></a>

# aelys

**A programming language that treats you like an adult**
```rust
fn add(a, b) {
    a + b  // dynamic, GC'd
}

@no_gc
fn add(a: i8, b: i8) -> i8 {
    return a + b  // zero-cost when you need it
}
```

**Write like Python, control like Rust. You choose your level.**

Simple syntax, GC by default, easy to learn. When you need C-level performance, add `@no_gc`, explicit types, manual memory.

You decide how close to the metal you want to be, function by function.

From beginner-friendly scripts to zero-cost systems code, *you* control the abstraction level.

---


- Arena GC by default, swap allocators with `std.mem`
- FFI that just works: `needs "gtk.h"`
- LLVM backend
- Metaprogramming planned for stable releases

> [!WARNING]
> **Version 0.21.8-a**: LLVM backend rewrite in progress, expect breaking changes. Docs may be outdated.

You can see the LLVM implementation roadmap via this pull request: https://github.com/vbxq/aelys_lang/pull/13

## Documentation

- [Installation Instructions](docs/installation.md)
- [Getting Started](docs/getting-started.md)
- [Language Spec](docs/language-spec.md)
- [Standard Library](docs/standard-library.md)
- [Examples](examples/README.md)

## Additional Resources

- [Benchmarks](docs/performance-benchmarks.md)
- [FAQ](docs/faq.md)
- [Changelog](CHANGELOG.md)
- [License](LICENSE)

---

## Contributing

Aelys is currently mid-rewrite, so the codebase is moving fast and PRs may conflict or become obsolete quickly. It's not the best time for large contributions.

That said, bug reports and feedback are always useful. If you want to contribute code, open an issue first, we'll let you know if it's a good fit given where things are.