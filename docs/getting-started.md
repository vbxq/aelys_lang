# Getting Started

This guide walks you through writing your first Aelys programs. I'll assume you've already [installed](installation.md) the compiler.

## Hello World

Create a file called `hello.aelys`:

```rust
needs std.io

print("Hello, World!")
```

Run it:

```bash
aelys hello.aelys
```

That's your first program ! The `needs` statement imports the `std.io` module. You can also use `io.print` if you prefer the qualified form.

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
counter = counter + 1
counter = counter + 1
print(counter)  // 2
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
    print("Hello, {name}!")
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
    print("big")
} else if x > 0 {
    print("small")
} else {
    print("zero or negative")
}
```

No need to put parentheses around the condition, braces are mandatory though

### while

```rust
needs std.io

let mut i = 0
while i < 5 {
    print(i)
    i = i + 1
}
```

### for

For loops use ranges:

```rust
needs std.io

// 0 to 9 (exclusive end)
for i in 0..10 {
    print(i)
}

// 1 to 10 (inclusive end)
for i in 1..=10 {
    print(i)
}

// with step
for i in 0..100 step 10 {
    print(i)  // 0, 10, 20, ...
}
```

`break` and `continue` work as expected:

```rust
needs std.io

for i in 0..100 {
    if i == 50 { break }
    if i % 2 == 0 { continue }
    print(i)  // odd numbers only, up to 49
}
```

## Logical Operators

It's `and`, `or`, and `not`, you can also use `&&`, `||`, and `!` if you prefer :

```rust
needs std.io

if x > 0 and y > 0 {
    print("both positive")
}

if not valid {
    print("invalid")
}
```

## Strings

Strings support the usual escape sequences (`\n`, `\t`, `\\`, `\"`):

```rust
print("Line 1\nLine 2")
print("Tab\there")
```

Concatenation uses `+`:

```rust
let greeting = "Hello, " + name + "!"
```

Or use interpolation with `{expression}`:

```rust
let name = "Quar"
print("Hello, {name}!")        // Hello, Quar!
print("2 + 2 = {2 + 2}")       // 2 + 2 = 4
```

Double braces for literal braces: `"{{key}}"` gives `{key}`

## Arrays and Vectors

### Arrays

Arrays hold multiple values of the same type:

```rust
needs std.io

let numbers = Array[10, 20, 30, 40, 50]
print(numbers[0])  // 10
print(numbers[2])  // 30
print("{numbers.len()}")  // 5
```

Modify elements like this:

```rust
let scores = Array[95, 87, 92]
scores[1] = 90
print(scores[1])  // 90
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
needs std.io

let v = Vec[1, 2, 3]
v.push(4)
v.push(5)
print("{v.len()}")  // 5

let last = v.pop()
print("{last}")  // 5
```

Good for building lists on the fly:

```rust
let scores = Vec[]
scores.push(95)
scores.push(87)
scores.push(92)

let mut sum = 0
for i in 0..scores.len() {
    sum = sum + scores[i]
}
print("{sum / scores.len()}")  // average
```

### When to use which

- **Array**: Size is fixed (coordinates, color values)
- **Vec**: Size changes (user input, dynamic lists)

## A Complete Example

Here's a program that calculates factorials:

```rust
needs std.io

fn factorial(n: int) -> int {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}

print("Factorials:")

for i in 1..=10 {
    let result = factorial(i)
    print("{i}! = {result}")
}
```

## What's Next

- [Language Specification](language-spec.md) - complete syntax reference
- [Standard Library](standard-library.md) - available modules and functions
- [Examples](../examples/README.md) - array examples, demos, and more
- [Performance](performance-benchmarks.md) - benchmarks and optimization tips

If you want to understand what makes Aelys different, check out the [@no_gc](language-spec.md#attributes) section in the language spec.
