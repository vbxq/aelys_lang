# Language Specification

This is the complete reference for Aelys syntax and semantics. If you're new to the language, start with [Getting Started](getting-started.md) instead.

## Lexical Structure

### Comments

```rust
// single line comment
```

Block comments aren't implemented for now, it's planned.

### Identifiers

Must start with a letter or underscore, then letters, digits, or underscores:

```
foo
_private
camelCase
snake_case
MAX_VALUE
Thing2
```

### Reserved Words

```
let mut fn if else while for in step return break continue
and or not pub needs as from true false null
```

### Literals

**Integers**
```rust
42
-17
0
1_000_000      // underscores for readability
0xFF           // hexadecimal
0b1010         // binary
0o755          // octal
```

Integers are 48-bit signed (roughly +-140 trillion). This is due to NaN-boxing, the VM packs type information into the unused bits of IEEE 754 NaN values. You'll probably never hit this limit in practice.

Underscores are ignored, so `1_000_000` is just `1000000`. Handy for big numbers or binary patterns like `0b1111_0000`.

**Floats**
```rust
3.14
-0.5
1.0
2.5e10
```

Standard IEEE 754 double precision (64-bit).

**Strings**
```rust
"hello"
"line1\nline2"
"tab\there"
"quote: \""
"backslash: \\"
```

UTF-8 encoded. Escape sequences: `\n` (newline), `\t` (tab), `\r` (carriage return), `\\` (backslash), `\"` (quote).

**String Interpolation**

Embed expressions directly in strings using `{expression}`:

```rust
let name = "Kaito"
let age = 67
"Hello, {name}! You are {age} years old."  // "Hello, Kaito! You are 67 years old."
```

Any expression works inside the braces:

```rust
let x = 10
"x + 5 = {x + 5}"           // "x + 5 = 15"
"doubled: {x * 2}"          // "doubled: 20"
```

Values are converted to strings automatically. To include a literal brace, double it:

```rust
"JSON: {{key}}"             // "JSON: {key}"
```

**Placeholder Syntax**

You can also use `{}` as placeholders filled by function call arguments:

```rust
print("Hello, {}!", "world")     // "Hello, world!"
print("x={}, y={}", 10, 20)      // "x=10, y=20"
```

Placeholders are filled left-to-right. You can mix inline expressions and placeholders:

```rust
let name = "Reimu"
print("Hi {name}, your number is {}", 42)
```

**Booleans**
```rust
true
false
```

**Null**
```rust
null
```

Represents absence of value. Functions without explicit return give `null`.

## Types

| Type | Description | Size |
|------|-------------|------|
| `int` | Signed integer | 48-bit |
| `float` | Floating point | 64-bit |
| `string` | UTF-8 text | heap allocated |
| `bool` | Boolean | 1 bit (packed) |
| `null` | Null value | - |
| `function` | Function/closure | heap allocated |
| `Array<T>` | Fixed-size array | heap allocated |
| `Vec<T>` | Growable vector | heap allocated |

### Type Annotations

Always optional. The inference engine handles most cases.

Variables:
```rust
let x: int = 42
let mut name: string = "Reimu"
```

Function parameters and return:
```rust
fn process(input: string, count: int) -> bool {
    // ...
}
```

Lambdas:
```rust
let f = fn(x: int) -> int { x * 2 }
```

### Type Inference

The type system uses Hindley-Milner inference. When you write:

```rust
let x = 42
let y = x + 10
```

The compiler knows `x` is `int` (from the literal) and `y` is `int` (from the `+` operation).

Gradual typing means missing type information doesn't cause errors. If the compiler can't determine a type, it treats the value as dynamic and inserts runtime checks. This gives you flexibility but less safety.

## Variables

### Declaration

```rust
let x = 10          // immutable
let mut y = 20      // mutable
```

### Shadowing

You can redeclare variables in the same scope:

```rust
let x = 10
let x = "now a string"  // shadows previous x
```

Inner scopes can also shadow:

```rust
needs std.io 

let x = 1
if true {
    let x = 2    // different x
    print(x)  // 2
}
print(x)      // 1
```

### Scope

Block-scoped. Variables live until their enclosing `}`.

## Functions

### Declaration

```rust
fn name(param1, param2) {
    // body
}

fn typed(a: int, b: int) -> int {
    return a + b
}
```

### Return

Explicit:
```rust
fn foo() -> int {
    return 42
}
```

Implicit (last expression):
```rust
fn foo() -> int {
    42
}
```

Functions without a return type annotation return `null` if no value is returned.

### Lambdas (Anonymous Functions)

```rust
let add = fn(a, b) { a + b }
let square = fn(x: int) -> int { x * x }
```

### Closures

Functions capture variables from their enclosing scope:

```rust
fn make_counter() {
    let mut count = 0
    return fn() -> int {
        count = count + 1
        return count
    }
}

let counter = make_counter()
counter()  // 1
counter()  // 2
counter()  // 3
```

The inner function holds a reference to `count`, which persists across calls.

### Higher-Order Functions

Functions are first-class values:

```rust
fn apply_twice(f, x) {
    return f(f(x))
}

fn double(n) { n * 2 }

apply_twice(double, 5)  // 20
```

## Operators

### Arithmetic

| Operator | Description |
|----------|-------------|
| `+` | Addition (and string concatenation) |
| `-` | Subtraction |
| `*` | Multiplication |
| `/` | Division |
| `%` | Modulo |

Integer division truncates: `7 / 2` gives `3`. Use floats if you need decimal results.

### Comparison

| Operator | Description |
|----------|-------------|
| `==` | Equal |
| `!=` | Not equal |
| `<` | Less than |
| `<=` | Less than or equal |
| `>` | Greater than |
| `>=` | Greater than or equal |

### Logical

| Operator | Description |
|----------|-------------|
| `and` | Logical AND (short-circuit) |
| `or` | Logical OR (short-circuit) |
| `not` | Logical NOT |

Short-circuit evaluation: `a and b` doesn't evaluate `b` if `a` is false. Same for `or` with true.

### Bitwise

| Operator | Description |
|----------|-------------|
| `&` | Bitwise AND |
| `\|` | Bitwise OR |
| `^` | Bitwise XOR |
| `~` | Bitwise NOT |
| `<<` | Left shift |
| `>>` | Right shift (arithmetic) |

### Precedence (lowest to highest)

1. `or`
2. `and`
3. `not`
4. `==`, `!=`, `<`, `<=`, `>`, `>=`
5. `|`
6. `^`
7. `&`
8. `<<`, `>>`
9. `+`, `-`
10. `*`, `/`, `%`
11. Unary `-`, `~`, `not`
12. Call `()`, member access `.`

When in doubt, use parentheses

## Control Flow

### if/else

```rust
if condition {
    // ...
} else if other_condition {
    // ...
} else {
    // ...
}
```

Braces are required. Parentheses around conditions are not.

### while

```rust
while condition {
    // body
}
```

### for

Iterates over integer ranges:

```rust
for i in start..end {       // exclusive: start to end-1
    // ...
}

for i in start..=end {      // inclusive: start to end
    // ...
}

for i in start..end step n {  // with step
    // ...
}
```

The loop variable is immutable within the body.

### break and continue

```rust
for i in 0..100 {
    if i == 50 { break }      // exit loop
    if i % 2 == 0 { continue }  // skip to next iteration
    // ...
}
```

Work in `while` loops too.

## Modules

### Imports

The `needs` keyword imports modules:

```rust
needs std.io                     // whole module
needs std.math as m              // aliased
needs print from std.io          // single function
needs sqrt, pow from std.math    // multiple functions
```

After `needs std.io`, you can use both `print()` directly and `io.print()`.

With an alias (`needs std.io as x`), only the aliased form works: `x.print()`.

After `needs print from std.io`, you call `print()` directly.

### Standard Library Modules

- `std.io` - console I/O
- `std.math` - math functions and constants
- `std.string` - string manipulation
- `std.convert` - type conversions
- `std.time` - time and timers
- `std.fs` - file system (requires capability)
- `std.net` - networking (requires capability)
- `std.sys` - system info

See [Standard Library](standard-library.md) for full documentation.

### Custom Modules

Any `.aelys` file is a module. If you have:

```
project/
  main.aelys
  utils.aelys
  lib/
    helper.aelys
```

From `main.aelys`:
```rust
needs utils              // imports utils.aelys
needs lib.helper         // imports lib/helper.aelys
```

Top-level definitions in a file become the module's exports.

### Visibility

The `pub` keyword marks something as explicitly public:

```rust
pub fn api_function() {
    // ...
}

fn internal_function() {
    // ...
}
```

## Attributes

### @no_gc (Manual Memory)

This is Aelys's distinguishing feature! By default, everything is garbage collected. But for performance-critical code, you can disable GC on a per-function basis:

```rust
@no_gc
fn hot_path(data: int, size: int) {
    let buffer = alloc(size)

    for i in 0..size {
        store(buffer, i, compute(data, i))
    }

    // use buffer...

    free(buffer)
}
```

#### Value-Based Allocation (Original)

Inside `@no_gc` functions, you get four primitives for working with Value slots:

| Function | Description |
|----------|-------------|
| `alloc(size)` | Allocate buffer of `size` slots, returns handle |
| `store(buf, index, value)` | Store value at index |
| `load(buf, index)` | Load value from index |
| `free(buf)` | Deallocate buffer |

Each slot holds one Value (int, float, bool, or pointer).

#### Byte-Level Allocation (std.bytes)

For true byte-level access, use the `std.bytes` module:

```rust
needs std.bytes

@no_gc
fn parse_binary() {
    let buf = bytes.alloc(1024)        // Allocate 1024 bytes
    bytes.write_u32(buf, 0, 0xDEADBEEF) // Write 32-bit int
    let magic = bytes.read_u32(buf, 0)  // Read it back
    bytes.free(buf)
}
```

| Function | Description |
|----------|-------------|
| `bytes.alloc(size)` | Allocate `size` raw bytes |
| `bytes.free(buf)` | Free byte buffer |
| `bytes.size(buf)` | Get buffer size |
| `bytes.read_u8(buf, off)` | Read byte (0-255) |
| `bytes.write_u8(buf, off, val)` | Write byte |
| `bytes.read_u16(buf, off)` | Read 16-bit int (little-endian) |
| `bytes.write_u16(buf, off, val)` | Write 16-bit int |
| `bytes.read_u32(buf, off)` | Read 32-bit int (little-endian) |
| `bytes.write_u32(buf, off, val)` | Write 32-bit int |
| `bytes.read_u64(buf, off)` | Read 64-bit int (little-endian) |
| `bytes.write_u64(buf, off, val)` | Write 64-bit int |
| `bytes.read_f32(buf, off)` | Read 32-bit float |
| `bytes.write_f32(buf, off, val)` | Write 32-bit float |
| `bytes.read_f64(buf, off)` | Read 64-bit float |
| `bytes.write_f64(buf, off, val)` | Write 64-bit float |
| `bytes.copy(src, soff, dst, doff, len)` | Copy bytes between buffers |
| `bytes.fill(buf, off, len, val)` | Fill bytes with value |

**When to use which:**

- **Value-based (`alloc/store/load`)**: Simple arrays of mixed types (ints, floats, bools)
- **Byte-level (`std.bytes`)**: Binary protocols, image buffers, audio data, FFI, zero-copy I/O

**Important details:**

1. The GC won't run while you're inside a `@no_gc` function. This prevents GC pauses in your critical path.

2. `@no_gc` functions can call other `@no_gc` functions. The runtime tracks nesting depth.

3. You can call `@no_gc` functions from normal code, and vice versa.

4. Memory allocated with `alloc` or `bytes.alloc` is not garbage collected! If you don't free it, it leaks.

5. All byte operations are bounds-checked for safety.

**When to use `@no_gc`:**

- Game loops
- Audio processing
- Real-time graphics
- Binary file parsing
- Network protocol handling
- Any code where GC pauses are unacceptable

**When not to use it:**

- Honestly, most code, stick to the GC unless you have a specific need

### @inline and @inline_always (Function Inlining)

These attributes tell the compiler to substitute a function's body directly at the call site, avoiding function call overhead

```rust
@inline
fn add(a: int, b: int) -> int {
    a + b
}

@inline_always
fn clamp(x: int, min: int, max: int) -> int {
    if x < min { return min }
    if x > max { return max }
    x
}
```

The difference:

| Attribute | Behavior |
|-----------|----------|
| `@inline` | Hint to the compiler. Respects code size thresholds. |
| `@inline_always` | Forces inlining. Ignores size limits. |

`@inline` can be ignored in some cases (recursive functions, mutual recursion). `@inline_always` forces substitution no matter what (except truly impossible cases like recursion)

**When to use `@inline`:**

- Small utility functions called frequently
- Thin wrappers around simple operations
- Performance-critical functions in tight loops

```rust
@inline
fn square(x: int) -> int { x * x }

@inline
fn is_even(n: int) -> bool { n % 2 == 0 }
```

**When to use `@inline_always`:**

- When you know inlining helps despite the size
- Functions with captures you still want inlined
- Micro-benchmarks where every cycle counts

**Limitations:**

Some functions simply can't be inlined:

| Case | Why |
|------|-----|
| Recursive function | Inlining would expand forever |
| Mutual recursion | Cycle like A → B → A detected |
| Native function | No Aelys body to substitute |

```rust
// ✗ Won't work
@inline
fn factorial(n: int) -> int {
    if n <= 1 { return 1 }
    n * factorial(n - 1)  // recursive!
}
```

The compiler will warn you if you try

**Functions with captures:**

Functions that capture variables from their enclosing scope are handled differently:

- `@inline`: won't be inlined (you'll get a warning)
- `@inline_always`: forces it anyway

```rust
fn make_adder(x: int) {
    @inline_always
    fn add(y: int) -> int {
        x + y  // captures x
    }
    return add
}
```

Use `@inline_always` here only if you know what you're doing

**Public functions:**

Functions marked `pub` with `@inline` get inlined locally, but the original code is kept for callers from other modules:

```rust
@inline
pub fn helper() {
    // inlined within this module
    // still available for importers
}
```

**Automatic inlining:**

Even without any attribute, the optimizer already inlines:

- Trivial functions (3 statements or less)
- Functions called only once

So you don't need to annotate everything. `@inline` and `@inline_always` are for when you want explicit control !

## Semicolons

Optional. The parser automatically inserts them after certain tokens (like Go does):

```rust
let x = 1
let y = 2

// same as:
let x = 1;
let y = 2;
```

Explicit semicolons let you put multiple statements on one line:

```rust
let x = 1; let y = 2; let z = 3
```

## Collections

### Arrays

Arrays hold a fixed number of elements. Once created, their size doesn't change.

```rust
let numbers = Array<Int>[1, 2, 3, 4, 5]
let floats = Array<Float>[1.0, 2.5, 3.7]
let flags = Array<Bool>[true, false, true]

// Type inference works
let auto = Array[10, 20, 30]  // infers Array<Int>
let short = [1, 2, 3]          // shorthand syntax
```

Empty arrays need a type:

```rust
let empty = Array<Int>[]
```

**Creating arrays with a specific size:**

Instead of writing `Array[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]`, you can just specify the size:

```rust
let zeros = Array<Int>(10)      // 10 zeros
let floats = Array<Float>(5)    // 5 zeros (0.0)
let nulls = Array(8)            // 8 nulls (untyped)
let also = [; 10]               // shorthand for Array(10)
```

Typed arrays are initialized with the type's default value (0 for int, 0.0 for float, false for bool). Untyped arrays are filled with `null`.

**Basic operations:**

```rust
let arr = Array[1, 2, 3, 4, 5]

arr[0]         // 1
arr[4]         // 5
arr[2] = 99    // modify element
arr.len()      // 5

arr[100]       // panic: index out of bounds
```

Under the hood, arrays use compact storage: 8 bytes per Int/Float, 1 byte per Bool.

### Vectors

Vectors are like arrays but they can grow. Need to add more elements? No problem.

```rust
let v = Vec<Int>[1, 2, 3]
v.push(4)
v.push(5)
v.len()  // 5

let last = v.pop()  // removes and returns 5
v.len()  // back to 4
```

Empty vectors:

```rust
let v = Vec<Int>[]  // or just Vec[]
```

**Operations:**

```rust
let v = Vec[10, 20, 30]

// Array stuff works
v[0]                // 10
v.len()             // 3
v[1] = 25           // modify

// Vec-specific
v.push(40)          // add element
let x = v.pop()     // remove last and return it
v.capacity()        // how much space is allocated
v.reserve(100)      // pre-allocate for 100 elements
```

Vecs grow automatically when they run out of space. The capacity is usually bigger than the length to avoid reallocating every time you push.

**When to use which:**

- **Array**: Size is fixed, mostly reading (coordinates, RGB colors)
- **Vec**: Size changes, lots of push/pop (lists, stacks, dynamic buffers)

### Type Safety

All elements must be the same type:

```rust
let good = Array[1, 2, 3]           // ✓ all int
let bad = Array[1, "two", 3.0]      // ✗ won't compile
```

`Array<Int>` and `Array<Float>` are different types. The compiler keeps track.

### Passing to Functions

Arrays and Vecs are passed by reference, so changes inside a function affect the original:

```rust
fn sum(arr) -> int {
    let mut total = 0
    for i in 0..arr.len() {
        total = total + arr[i]
    }
    return total
}

let numbers = Array[1, 2, 3, 4, 5]
sum(numbers)  // 15
```

Mutations persist:

```rust
fn double_all(v) {
    for i in 0..v.len() {
        v[i] = v[i] * 2
    }
}

let v = Vec[1, 2, 3]
double_all(v)
v[0]  // 2 (modified by the function)
```

### Iteration

For now, use index-based loops:

```rust
let arr = Array[10, 20, 30, 40]

for i in 0..arr.len() {
    print(arr[i])
}
```

Iterator methods like `arr.iter()`, `arr.map()`, `arr.filter()` are coming later.

## Compiler Warnings

The compiler can emit warnings for various situations. Warnings don't stop compilation but indicate potential issues :

### Warning Categories

| Code | Category | Description |
|------|----------|-------------|
| W01xx | inline | Issues with `@inline` or `@inline_always` |
| W02xx | unused | Unused variables, functions, imports |
| W03xx | deprecated | Deprecated features or functions |
| W04xx | shadow | Variable shadowing |

### Inline Warnings

```rust
// W0101: can't inline recursive functions
@inline
fn factorial(n: int) -> int {
    if n <= 1 { return 1 }
    n * factorial(n - 1)
}
```

The compiler warns because inlining a recursive function would cause infinite expansion. Remove `@inline` or break the recursion.

Here's some other inline warnings:

- **W0102**: Mutual recursion (A calls B, B calls A)
- **W0103**: Function captures variables from outer scope
- **W0104**: Public function is being inlined (original kept for external callers)
- **W0105**: Native function can't be inlined

### Warning Flags

Control warnings from the command line:

```bash
# enable all warnings
aelys compile main.aelys -Wall

# treat warnings as errors
aelys compile main.aelys -Werror

# enable a specific category
aelys compile main.aelys -Winline

# disable a category
aelys compile main.aelys -Wno-inline

# combine: all warnings, but not unused, treat as errors
aelys compile main.aelys -Wall -Wno-unused -Werror
```

The `-Werror` flag is useful in CI to catch issues early

## Error Handling

Currently there's no try/catch mechanism. Functions that can fail return `null` on failure:

```rust
needs std.io
needs std.convert

let result = convert.parse_int("not a number")
if result == null {
    print("parsing failed")
}
```

Standard library functions follow this pattern. A proper error handling system is planned, don't worry !

## Future Plans

Things I'm considering but haven't implemented yet:

- `match` expressions
- `struct` types
- `enum` types
- Async/await
- Iterator methods on arrays