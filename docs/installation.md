# Installation

## Requirements

- **Rust 1.70 or later** - I use some newer language features
- **Cargo** - comes with Rust

## Pre-built Binaries

The easiest way to get started is to download a pre-built binary from the [Releases page](https://github.com/vbxq/aelys_lang/releases).

Pick the binary for your platform and add it to your PATH.

## Building from Source

```bash
git clone https://github.com/vbxq/aelys_lang.git
cd aelys_lang
cargo build --release
```

This takes around 20 seconds on a modern machine. The binary lands in `target/release/aelys-cli`.

Add it to your PATH for convenience:

```bash
# In ~/.bashrc or ~/.zshrc
export PATH="$PATH:/path/to/aelys_lang/target/release"

# Or create a symlink
sudo ln -s /path/to/aelys_lang/target/release/aelys-cli /usr/local/bin/aelys
```

For Windows, you can add it manually to your PATH via System Properties, or copy the aelys-cli.exe file to your directory of choice.

## Verifying Installation

```bash
aelys-cli version
# Aelys v0.17.6-a
```

Run the hello world example:

```bash
aelys-cli examples/hello.aelys
# Hello, World!
# Welcome to Aelys v0.17.6-a!
# Platform: linux (x86_64)
```

## CLI Reference

### Basic Usage

```bash
aelys-cli <file.aelys>              # run a source file
aelys-cli run <file.aelys>          # same thing, explicit
aelys-cli run <file.avbc>           # run compiled bytecode
aelys-cli compile <file.aelys>      # compile to bytecode
aelys-cli asm <file.aelys>          # show disassembly
aelys-cli repl                      # interactive mode
aelys-cli version                   # show version
```

### Flags

**Optimization**

| Flag | Description |
|------|-------------|
| `-O0` | No optimization |
| `-O1` | Basic optimization |
| `-O2` | Standard optimization (default) |
| `-O3` | Aggressive optimization |

In practice, optimization levels don't make a huge difference for most code right now. The optimizer handles constant folding, dead code elimination, and some loop optimizations

**Output**

| Flag | Description |
|------|-------------|
| `-o <path>` | Output file (for compile/asm) |
| `--stdout` | Print asm to stdout instead of file |

**VM Options**

You can tune VM behavior with `-ae.<option>=<value>`:

```bash
aelys-cli -ae.max-heap=128M program.aelys
aelys-cli -ae.trusted=true program.aelys
```

`max-heap` sets the heap size limit. `trusted` disables some security checks

**Capabilities**

Some modules require explicit permission:

```bash
# Allow file system access
aelys-cli --allow-caps=fs program.aelys

# Allow networking
aelys-cli --allow-caps=net server.aelys

# Allow both
aelys-cli --allow-caps=fs,net program.aelys

# Deny specific capability
aelys-cli --deny-caps=net program.aelys
```

Without these flags, `std.fs` and `std.net` will error when you try to use them.  
This is a security feature - untrusted scripts can't access the filesystem or network unless you explicitly allow it.

**Development**

| Flag | Description                                                   |
|------|---------------------------------------------------------------|
| `--dev` | Enable development mode (for example experimental hot reload) |

### Examples

```bash
# Run with arguments passed to the program
aelys-cli program.aelys arg1 arg2

# Compile and save bytecode
aelys-cli compile main.aelys -o main.avbc

# Run compiled bytecode
aelys-cli run main.avbc

# View disassembly for debugging
aelys-cli asm main.aelys --stdout

# REPL with increased heap
aelys-cli repl -ae.max-heap=1G
```

## File Types

| Extension | Description |
|-----------|-------------|
| `.aelys` | Source code |
| `.avbc` | Compiled bytecode |
| `.aasm` | Assembly (human-readable bytecode) |

Bytecode files are portable across machines with the same Aelys version. They skip parsing and compilation.

## Project Structure

Here's a sample project layout to explain module organization:

```
my_project/
  main.aelys           # entry point
  config.aelys         # configuration module
  lib/
    utils.aelys        # utility module
    http.aelys         # http helpers
  tests/
    test_utils.aelys   # tests
```

Modules are imported relative to the file that imports them. From `main.aelys`:

```rust
needs config
needs lib.utils
needs lib.http
```

There's no package manager or dependency system for now, just files and directories.

## Editor Support

Soon :tm:

## Troubleshooting

### "cannot find module 'foo'"

Module resolution is relative to the importing file. Check:
- Is the file path correct?
- Did you spell the module name right?
- For `std.*` modules, make sure you're using the exact name (`std.io`, not `std.IO`)

### "capability denied: std.fs"

You're trying to use file system operations without permission. Add `--allow-caps=fs` to your command.

### "error: expected X, found Y"

Parse error. Check:
- Missing braces?
- Typo in keyword?
- String not closed?

Error messages include line numbers. The compiler tries to be helpful but some messages could be clearer, soon (tm)

### "integer overflow"

You've exceeded the 48-bit integer range (±140 trillion). Use floats for larger values.. or refactor your code

### "invalid bytecode"

The bytecode file is corrupted or was compiled with a different Aelys version, try recompiling from source.

### The REPL is behaving weirdly

The REPL has some edge cases with multi

## Running Tests

If you want to verify the build:

```bash
cargo test
```

This runs the full test suite, there's 596 of them at the time of writing this (0.17.6-a).