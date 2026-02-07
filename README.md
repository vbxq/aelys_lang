<p align="center">
  <img src="docs/aelys_banner.png" alt="aelys virtual machine" width="1000">
</p>

# Developer note :

Aelys isn't going in the direction I initially wanted it to go. My internal layout choices are fundamentally flawed.

I'm thinking of potentially rewriting parts of it to make it a real system programming language with interesting features.  
I don't want it to be a video game scripting language.

Not just another VM, but something interesting.

# aelys 0.19.6-a

Register-based VM with dual memory management: GC by default, `@no_gc` for performance-critical code.

You choose between comfort and performance on a per-function basis.

## Documentation

- [Getting Started Guide](docs/getting-started.md)
- [Build Instructions](docs/installation.md)
- [Language Specification](docs/language-spec.md)
- [Standard Library Documentation](docs/standard-library.md)

## Additional Information

- [Performance Benchmarks](docs/performance-benchmarks.md)
- [Acknowledgements](docs/acknowledgements.md)
- [Changelog](CHANGELOG.md)
- [Examples](examples/README.md)
- [License](LICENSE)
- [FAQ](docs/faq.md)
