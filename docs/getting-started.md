# Getting Started

This guide walks you through writing your first Aelys programs. I'll assume you've already [installed](installation.md) the compiler.

## Hello World

Create a file called `hello.aelys`:

```rust
println("Hello, World!")
```

Run it:

```bash
aelys hello.aelys
```

That's your first program ! The safe standard modules (`std.io`, `std.math`, `std.string`, `std.convert`, `std.time`) are auto-registered at startup, so you don't need to import them. The `needs` statement is still required for privileged modules like `std.fs`, `std.net`, `std.sys`, and `std.bytes`.

You don't need a `main` function, top-level code just runs.

## Variables

```rust
let name = "Reimu"
let age = 30
let pi = 3.14159
```

Variables are immutable by default. If you need to change a value, use `mut`:

```rust
let mut counter = 0
counter++
counter++
println(counter)  // 2
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
    println("Hello, {name}!")
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
let x = 10

if (x > 5) {                           // parentheses optional
    println("big")
} else if x > 0 {
    println("small")
} else {
    println("zero or negative")
}
```

No need to put parentheses around the condition, braces are mandatory though

### while

```rust
let mut i = 0
while i < 5 {
    println(i)
    i++
}
```

### for

For loops use ranges:

```rust
// 0 to 9 (exclusive end)
for i in 0..10 {
    println(i)
}

// 1 to 10 (inclusive end)
for i in 1..=10 {
    println(i)
}

// with step
for i in 0..100 step 10 {
    println(i)  // 0, 10, 20, ...
}
```

`break` and `continue` work as expected:

```rust
for i in 0..100 {
    if i == 50 { break }
    if i % 2 == 0 { continue }
    println(i)  // odd numbers only, up to 49
}
```

You can iterate directly over arrays and vectors:

```rust
let numbers = Array[10, 20, 30]
for item in numbers {
    println(item)
}

let names = Vec["Alice", "Bob", "Charlie"]
for name in names {
    println(name)
}
```

You can also iterate over a string's characters:

```rust
for letter in "hello" {
    println(letter)
}
```

This works with variables too:

```rust
let name = "Marisa"
for c in name {
    println(c)
}
```

## Logical Operators

It's `and`, `or`, and `not`, you can also use `&&`, `||`, and `!` if you prefer :

```rust
if x > 0 and y > 0 {
    println("both positive")
}

if not valid {
    println("invalid")
}
```

## Strings

Strings support the usual escape sequences (`\n`, `\t`, `\\`, `\"`):

```rust
println("Line 1\nLine 2")
println("Tab\there")
```

You can access individual characters with `[]`:

```rust
let s = "hello"
println(s[0])  // h
println(s[4])  // o
```

Each index returns a single-character string. Indexing is Unicode-aware (it counts characters, not bytes).

Concatenation uses `+`:

```rust
let greeting = "Hello, " + name + "!"
```

Or use interpolation with `{expression}`:

```rust
let name = "Quar"
println("Hello, {name}!")        // Hello, Quar!
println("2500 + 2500 = {2500 + 2500}")       // 2500 + 2500 = 5000
```

Double braces for literal braces: `"{{key}}"` gives `{key}`

## Arrays and Vectors

### Arrays

Arrays hold multiple values of the same type:

```rust
let numbers = Array[10, 20, 30, 40, 50]
println(numbers[0])  // 10
println(numbers[2])  // 30
println("{numbers.len()}")  // 5
```

Modify elements like this:

```rust
let scores = Array[95, 87, 92]
scores[1] = 90
println(scores[1])  // 90
```

You can add type annotations if you want to be explicit:

```rust
let ints = Array<Int>[1, 2, 3]
let floats = Array<Float>[1.5, 2.7, 3.9]
```

Empty arrays need a type:

```rust
let empty = Array<Int>[]
```

If you need an array of a specific size without listing each element:

```rust
let zeros = Array<Int>(10)    // 10 zeros
let buffer = Array(100)       // 100 nulls
let short = [; 5]             // same as Array(5)
```

### Vectors

Vectors can grow after you create them:

```rust
let v = Vec[1, 2, 3]
v.push(4)
v.push(5)
println("{v.len()}")  // 5

let last = v.pop()
println("{last}")  // 5
```

Good for building lists on the fly:

```rust
let scores = Vec[]
scores.push(95)
scores.push(87)
scores.push(92)

let mut sum = 0
for i in 0..scores.len() {
    sum += scores[i]
}
println("{sum / scores.len()}")  // average
```

### When to use which

- **Array**: Size is fixed (coordinates, color values)
- **Vec**: Size changes (user input, dynamic lists)

## A Complete Example

Here's a program that calculates factorials:

```rust
fn factorial(n: int) -> int {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}

println("Factorials:")

for i in 1..=10 {
    let result = factorial(i)
    println("{i}! = {result}")
}
```

## Editor Support

Writing Aelys code is way more pleasant with syntax highlighting.  
Many other editor coming, don't worry !

### VSCode Extension

Someone made a VSCode Extension for Aelys, see [Acknowledgements](../ACKNOWLEDGEMENTS.md)  
Download [Aelys Language Support](https://marketplace.visualstudio.com/items?itemName=SpaceGame.aelys-lang) from the VSCode marketplace, or directly to the [repository link](https://github.com/SpaceGame-wq/aelys-vscode)

## What's Next

- [Language Specification](language-spec.md) - complete syntax reference
- [Standard Library](standard-library.md) - available modules and functions
- [Examples](../examples/README.md) - array examples, demos, and more
- [Performance](performance-benchmarks.md) - benchmarks and optimization tips

If you want to understand what makes Aelys different, check out the [@no_gc](language-spec.md#attributes) section in the language spec.
