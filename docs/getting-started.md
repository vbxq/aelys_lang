# Getting Started

This guide walks you through writing your first Aelys programs. I'll assume you've already [installed](installation.md) the compiler.

## Hello World

Create a file called `hello.aelys`:

```rust
needs std.io

io.print("Hello, World!")
```

Run it:

```bash
aelys hello.aelys
```

That's your first program ! The `needs` statement imports the `std.io` module, and `io.print` writes to stdout.

You don't need a `main` function - top-level code just runs.
## Variables

```rust
let name = "Alice"
let age = 30
let pi = 3.14159
```

Variables are immutable by default. If you need to change a value, use `mut`:

```rust
let mut counter = 0
counter = counter + 1
counter = counter + 1
io.print(counter)  // 2
```

## Type Annotations

Types are optional. You can add them when you want clarity or when the compiler can't figure it out:

```rust
let x: int = 42
let pi: float = 3.14159
let name: string = "Bob"
let active: bool = true
```

Available types: `int`, `float`, `string`, `bool`, and `null`.

The type system uses Hindley-Milner inference with gradual typing. In practice, this means you get type checking where you add annotations, and flexibility where you don't. If inference fails somewhere, you'll get a warning (not an error) and the value becomes dynamic.

## Functions

```rust
fn greet(name: string) {
    io.print("Hello, " + name + "!")
}

greet("World")
```

Return values:

```rust
fn add(a: int, b: int) -> int {
    return a + b
}

// or implicitly (last expression is returned)
fn multiply(a: int, b: int) -> int {
    a * b
}
```

You can omit types entirely if you prefer:

```rust
fn add(a, b) {
    return a + b
}
```

This works, but you lose some type safety. I recommend adding types at the function signature !

## Control Flow

### if/else

```rust
needs std.io

let x = 10

if (x > 5) {                           // parentheses optional
    io.print("big")
} else if x > 0 {
    io.print("small")
} else {
    io.print("zero or negative")
}
```

No need to put parentheses around the condition, braces are mandatory though

### while

```rust
needs std.io

let mut i = 0
while i < 5 {
    io.print(i)
    i = i + 1
}
```

### for

For loops use ranges:

```rust
needs std.io

// 0 to 9 (exclusive end)
for i in 0..10 {
    io.print(i)
}

// 1 to 10 (inclusive end)
for i in 1..=10 {
    io.print(i)
}

// with step
for i in 0..100 step 10 {
    io.print(i)  // 0, 10, 20, ...
}
```

`break` and `continue` work as expected:

```rust
needs std.io

for i in 0..100 {
    if i == 50 { break }
    if i % 2 == 0 { continue }
    io.print(i)  // odd numbers only, up to 49
}
```

## Logical Operators

It's `and`, `or`, and `not`, you can also use `&&`, `||`, and `!` if you prefer :

```rust
needs std.io

if x > 0 and y > 0 {
    io.print("both positive")
}

if not valid {
    io.print("invalid")
}
```

## Strings

Strings support the usual escape sequences (`\n`, `\t`, `\\`, `\"`):

```rust
io.print("Line 1\nLine 2")
io.print("Tab\there")
```

Concatenation uses `+`:

```rust
let greeting = "Hello, " + name + "!"
```

No string interpolation yet, it's on my list but not a priority

## A Complete Example

Here's a program that calculates factorials:

```rust
needs std.io
needs std.convert

fn factorial(n: int) -> int {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}

io.print("Factorials:")

for i in 1..=10 {
    let result = factorial(i)
    io.print(convert.to_string(i) + "! = " + convert.to_string(result))
}
```

(I will change the convert module to make conversions easier and the code less uglier, I promise)

## What's Next

- [Language Specification](language-spec.md) - complete syntax reference
- [Standard Library](standard-library.md) - available modules and functions
- [Examples](../examples/examples.md) - more code to learn from

If you want to understand what makes Aelys different, check out the section on [@no_gc](language-spec.md#no_gc-manual-memory) in the language spec.
