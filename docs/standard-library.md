# Standard Library Reference

All standard library modules live under `std.*`. Import them with `needs`:

```rust
needs std.io
needs std.math as m
needs print from std.io
```

---

## std.io

Console input/output.

```rust
needs std.io
```

### Output Functions

| Function | Description |
|----------|-------------|
| `print(value)` | Print value with newline |
| `println(value)` | Same as print |
| `print_inline(value)` | Print without newline |
| `eprint(value)` | Print to stderr, no newline |
| `eprintln(value)` | Print to stderr with newline |
| `flush()` | Flush stdout buffer |
| `eflush()` | Flush stderr buffer |

```rust
io.print("Hello")           // Hello\n
io.print_inline("Loading")  // Loading (no newline)
io.print_inline(".")        // .
io.print("")                // newline
```

### Input Functions

| Function | Description |
|----------|-------------|
| `readline()` | Read line from stdin (null on EOF) |
| `read_char()` | Read single character |
| `input(prompt)` | Print prompt, read line |

```rust
let name = io.input("What's your name? ")
io.print("Hello, " + name)
```

### Terminal Control

ANSI escape sequence wrappers. Work on most modern terminals.

| Function | Description |
|----------|-------------|
| `clear_screen()` | Clear the terminal |
| `cursor_home()` | Move cursor to top-left |
| `hide_cursor()` | Hide the cursor |
| `show_cursor()` | Show the cursor |
| `move_cursor(x, y)` | Move cursor to position (1-indexed) |

```rust
io.clear_screen()
io.move_cursor(10, 5)
io.print("Here!")
```

---

## std.math

Math functions and constants.

```rust
needs std.math
```

### Constants

| Name | Value |
|------|-------|
| `PI` | 3.141592653589793 |
| `E` | 2.718281828459045 |
| `TAU` | 6.283185307179586 (2π) |
| `INF` | Positive infinity |
| `NEG_INF` | Negative infinity |

### Basic Math

| Function | Description |
|----------|-------------|
| `abs(x)` | Absolute value |
| `sign(x)` | Sign: -1, 0, or 1 |
| `sqrt(x)` | Square root |
| `cbrt(x)` | Cube root |
| `pow(base, exp)` | Exponentiation |
| `min(a, b)` | Minimum of two values |
| `max(a, b)` | Maximum of two values |
| `clamp(x, min, max)` | Clamp value to range |

```rust
math.abs(-5)        // 5
math.sqrt(16.0)     // 4.0
math.pow(2, 10)     // 1024
math.clamp(15, 0, 10)  // 10
```

### Trigonometry

All functions work in radians.

| Function | Description |
|----------|-------------|
| `sin(x)` | Sine |
| `cos(x)` | Cosine |
| `tan(x)` | Tangent |
| `asin(x)` | Arc sine |
| `acos(x)` | Arc cosine |
| `atan(x)` | Arc tangent |
| `atan2(y, x)` | Two-argument arc tangent |
| `sinh(x)` | Hyperbolic sine |
| `cosh(x)` | Hyperbolic cosine |
| `tanh(x)` | Hyperbolic tangent |

### Angle Conversion

| Function | Description |
|----------|-------------|
| `deg_to_rad(deg)` | Degrees to radians |
| `rad_to_deg(rad)` | Radians to degrees |

### Exponential & Logarithmic

| Function | Description |
|----------|-------------|
| `exp(x)` | e^x |
| `log(x)` | Natural logarithm |
| `log10(x)` | Base-10 logarithm |
| `log2(x)` | Base-2 logarithm |

### Rounding

| Function | Description |
|----------|-------------|
| `floor(x)` | Round down |
| `ceil(x)` | Round up |
| `round(x)` | Round to nearest |
| `trunc(x)` | Truncate toward zero |

### Special

| Function | Description |
|----------|-------------|
| `hypot(x, y)` | sqrt(x² + y²) |
| `fmod(x, y)` | Floating-point modulo |
| `is_nan(x)` | Check if NaN |
| `is_inf(x)` | Check if infinite |
| `is_finite(x)` | Check if finite |

---

## std.string

String manipulation. Strings are UTF-8.

```rust
needs std.string as str
```

### Length

| Function | Description |
|----------|-------------|
| `len(s)` | Length in bytes |
| `char_len(s)` | Length in Unicode characters |

```rust
str.len("café")       // 5 (bytes)
str.char_len("café")  // 4 (characters)
```

This distinction matters for non-ASCII text.

### Character Access

| Function | Description |
|----------|-------------|
| `char_at(s, i)` | Character at index (empty if out of bounds) |
| `byte_at(s, i)` | Byte at index (-1 if out of bounds) |
| `substr(s, start, len)` | Extract substring by character position |

```rust
str.char_at("hello", 0)    // "h"
str.char_at("hello", 10)   // "" (out of bounds)
str.substr("hello", 1, 3)  // "ell"
```

### Case Conversion

| Function | Description |
|----------|-------------|
| `to_upper(s)` | Convert to uppercase |
| `to_lower(s)` | Convert to lowercase |
| `capitalize(s)` | Capitalize first character |

### Search

| Function | Description |
|----------|-------------|
| `contains(s, needle)` | Check if string contains substring |
| `starts_with(s, prefix)` | Check prefix |
| `ends_with(s, suffix)` | Check suffix |
| `find(s, needle)` | First occurrence index (-1 if not found) |
| `rfind(s, needle)` | Last occurrence index |
| `count(s, needle)` | Count occurrences |

Note: `find` and `rfind` return byte positions, not character positions. This can be surprising with Unicode - I might change this in a future version.

### Transformation

| Function | Description |
|----------|-------------|
| `replace(s, old, new)` | Replace all occurrences |
| `replace_first(s, old, new)` | Replace first occurrence |
| `reverse(s)` | Reverse string |
| `repeat(s, n)` | Repeat n times |
| `concat(a, b)` | Concatenate (same as `+`) |

### Whitespace

| Function | Description |
|----------|-------------|
| `trim(s)` | Remove whitespace from both ends |
| `trim_start(s)` | Remove from start |
| `trim_end(s)` | Remove from end |
| `pad_left(s, width, char)` | Pad start to width |
| `pad_right(s, width, char)` | Pad end to width |

```rust
str.trim("  hello  ")        // "hello"
str.pad_left("42", 5, "0")   // "00042"
```

### Splitting

| Function | Description |
|----------|-------------|
| `split(s, sep)` | Split by separator |
| `join(parts, sep)` | Join parts with separator |
| `lines(s)` | Split into lines |
| `line_count(s)` | Count lines |

`split` and `join` use newline-separated strings as the "list" representation. This is awkward (I know), but arrays aren't first-class yet.

```rust
let parts = str.split("a,b,c", ",")  // "a\nb\nc"
str.join(parts, "-")                  // "a-b-c"
```

### Predicates

| Function | Description |
|----------|-------------|
| `is_empty(s)` | Check if empty |
| `is_whitespace(s)` | Only whitespace? |
| `is_numeric(s)` | Only digits? |
| `is_alphabetic(s)` | Only letters? |
| `is_alphanumeric(s)` | Letters and digits only? |

---

## std.convert

Type conversions and introspection.

```rust
needs std.convert
```

### Parsing Strings

| Function | Description |
|----------|-------------|
| `parse_int(s)` | Parse string to int (null on failure) |
| `parse_int_radix(s, radix)` | Parse with base 2-36 |
| `parse_float(s)` | Parse string to float |
| `parse_bool(s)` | Parse boolean ("true"/"false"/"1"/"0"/"yes"/"no") |

`parse_int` automatically handles `0x`, `0o`, `0b` prefixes:

```rust
convert.parse_int("42")      // 42
convert.parse_int("0xFF")    // 255
convert.parse_int("0b1010")  // 10
convert.parse_int("nope")    // null
```

### Type Conversion

| Function | Description |
|----------|-------------|
| `to_string(x)` | Convert anything to string |
| `to_int(x)` | Convert to int |
| `to_float(x)` | Convert to float |
| `to_bool(x)` | Convert to bool (truthiness) |

### Numeric Formatting

| Function | Description |
|----------|-------------|
| `to_hex(n)` | Int to hexadecimal string |
| `to_binary(n)` | Int to binary string |
| `to_octal(n)` | Int to octal string |
| `to_radix(n, radix)` | Int to string in given radix |

```rust
convert.to_hex(255)      // "ff"
convert.to_binary(10)    // "1010"
convert.to_radix(100, 7) // "202"
```

### Character Conversion

| Function | Description |
|----------|-------------|
| `ord(char)` | Character to Unicode code point |
| `chr(code)` | Code point to character |

```rust
convert.ord("A")    // 65
convert.chr(65)     // "A"
convert.chr(0x1F600)  // "😀"
```

### Type Checking

| Function | Description |
|----------|-------------|
| `type_of(x)` | Get type name as string |
| `is_int(x)` | Check if int |
| `is_float(x)` | Check if float |
| `is_string(x)` | Check if string |
| `is_bool(x)` | Check if bool |
| `is_null(x)` | Check if null |
| `is_function(x)` | Check if function |

```rust
convert.type_of(42)        // "int"
convert.type_of("hello")   // "string"
convert.is_int(42)         // true
```

---

## std.time

Time operations and measurement.

```rust
needs std.time
```

### Current Time

| Function | Description |
|----------|-------------|
| `now()` | Unix timestamp in seconds (float) |
| `now_ms()` | Timestamp in milliseconds |
| `now_us()` | Timestamp in microseconds |
| `now_ns()` | Timestamp in nanoseconds |

### High-Precision Timers

For measuring elapsed time accurately:

| Function | Description |
|----------|-------------|
| `timer()` | Create timer, returns handle |
| `elapsed(h)` | Seconds since timer created |
| `elapsed_ms(h)` | Milliseconds elapsed |
| `elapsed_us(h)` | Microseconds elapsed |
| `reset(h)` | Reset timer to now |

```rust
let t = time.timer()
// ... do work ...
let ms = time.elapsed_ms(t)
io.print("Took " + convert.to_string(ms) + "ms")
```

### Sleep

| Function | Description |
|----------|-------------|
| `sleep(ms)` | Sleep for milliseconds |
| `sleep_us(us)` | Sleep for microseconds |

### Date/Time Components

These return UTC time. Local timezone support isn't implemented yet.

| Function | Description |
|----------|-------------|
| `year()` | Current year |
| `month()` | Month (1-12) |
| `day()` | Day of month (1-31) |
| `hour()` | Hour (0-23) |
| `minute()` | Minute (0-59) |
| `second()` | Second (0-59) |
| `weekday()` | Day of week (0=Sunday, 6=Saturday) |
| `yearday()` | Day of year (1-366) |

### Formatting

| Function | Description |
|----------|-------------|
| `format(fmt)` | Format current time with pattern |
| `iso()` | ISO 8601 format (YYYY-MM-DDTHH:MM:SSZ) |
| `date()` | Date only (YYYY-MM-DD) |
| `time_str()` | Time only (HH:MM:SS) |

Format specifiers:
- `%Y` - 4-digit year
- `%y` - 2-digit year
- `%m` - month (01-12)
- `%d` - day (01-31)
- `%H` - hour (00-23)
- `%M` - minute (00-59)
- `%S` - second (00-59)
- `%a` - weekday name (Sun, Mon, ...)
- `%b` - month name (Jan, Feb, ...)
- `%%` - literal %

```rust
time.format("%Y-%m-%d %H:%M:%S")  // "2024-01-15 14:30:45"
time.iso()                        // "2024-01-15T14:30:45Z"
```

---

## std.fs

File system operations. **Requires `--allow-caps=fs`**.

```rust
needs std.fs
```

### Reading Files

| Function | Description |
|----------|-------------|
| `read_text(path)` | Read entire file as string |
| `open(path, mode)` | Open file, returns handle |
| `read(handle)` | Read entire file from handle |
| `read_line(handle)` | Read one line (null on EOF) |
| `read_bytes(handle, n)` | Read up to n bytes |
| `close(handle)` | Close file handle |

Modes: `"r"` (read), `"w"` (write), `"a"` (append), `"rw"` (read+write).

```rust
// Simple way
let content = fs.read_text("config.txt")

// Handle-based way
let f = fs.open("data.txt", "r")
while true {
    let line = fs.read_line(f)
    if line == null { break }
    io.print(line)
}
fs.close(f)
```

### Writing Files

| Function | Description |
|----------|-------------|
| `write_text(path, content)` | Write string to file (overwrite) |
| `append_text(path, content)` | Append string to file |
| `write(handle, data)` | Write to handle |
| `write_line(handle, line)` | Write with newline |

```rust
fs.write_text("output.txt", "Hello, file!")
fs.append_text("log.txt", "New entry\n")
```

### File Information

| Function | Description |
|----------|-------------|
| `exists(path)` | Check if path exists |
| `is_file(path)` | Check if regular file |
| `is_dir(path)` | Check if directory |
| `size(path)` | File size in bytes |

### Directory Operations

| Function | Description |
|----------|-------------|
| `mkdir(path)` | Create directory |
| `mkdir_all(path)` | Create directory and parents |
| `rmdir(path)` | Remove empty directory |
| `readdir(path)` | List directory (newline-separated) |

### File Management

| Function | Description |
|----------|-------------|
| `delete(path)` | Delete file |
| `rename(from, to)` | Rename or move |
| `copy(from, to)` | Copy file |

### Path Utilities

| Function | Description |
|----------|-------------|
| `basename(path)` | Get file name |
| `dirname(path)` | Get directory part |
| `extension(path)` | Get file extension |
| `join(base, path)` | Join paths safely |
| `absolute(path)` | Convert to absolute path |

`join` is secure against path traversal - it won't let `..` escape the base directory.

```rust
fs.join("/home/user", "../../etc/passwd")  // stays within /home/user
```

---

## std.net

Network operations. **Requires `--allow-caps=net`**.

```rust
needs std.net
```

### TCP Client

| Function | Description |
|----------|-------------|
| `connect(host, port)` | Connect to server, returns handle |
| `send(handle, data)` | Send data |
| `recv(handle)` | Receive available data |
| `recv_bytes(handle, max)` | Receive up to max bytes |
| `recv_line(handle)` | Receive one line |
| `close(handle)` | Close connection |

```rust
let sock = net.connect("example.com", 80)
net.send(sock, "GET / HTTP/1.0\r\nHost: example.com\r\n\r\n")
let response = net.recv(sock)
io.print(response)
net.close(sock)
```

### TCP Server

| Function | Description |
|----------|-------------|
| `listen(host, port)` | Start listening, returns handle |
| `accept(handle)` | Accept connection, returns new handle |

```rust
let server = net.listen("0.0.0.0", 8080)
io.print("Listening on port 8080")

while true {
    let client = net.accept(server)
    let request = net.recv_line(client)
    net.send(client, "HTTP/1.0 200 OK\r\n\r\nHello!")
    net.close(client)
}
```

### Socket Options

| Function | Description |
|----------|-------------|
| `set_timeout(handle, ms)` | Set read/write timeout (0 to disable) |
| `set_nodelay(handle, bool)` | Enable/disable Nagle's algorithm |
| `shutdown(handle, how)` | Partial shutdown ("read", "write", "both") |
| `local_addr(handle)` | Get local address |
| `peer_addr(handle)` | Get peer address |

---

## std.sys

System information.

```rust
needs std.sys
```

| Function | Description |
|----------|-------------|
| `platform()` | OS name ("linux", "macos", "windows") |
| `arch()` | CPU architecture ("x86_64", "aarch64", etc.) |

```rust
io.print("Running on " + sys.platform() + " " + sys.arch())
// "Running on linux x86_64"
```

---

## std.bytes

Byte-level memory operations for binary data manipulation. Use with `@no_gc` functions for best performance.

```rust
needs std.bytes
```

### Allocation

| Function | Description |
|----------|-------------|
| `alloc(size)` | Allocate `size` bytes, initialized to zero. Returns handle. |
| `free(handle)` | Free buffer. `free(null)` is a no-op. |
| `size(handle)` | Return buffer size in bytes. |

### Integer Operations

All multi-byte operations use little-endian byte order.

| Function | Description |
|----------|-------------|
| `read_u8(buf, offset)` | Read unsigned byte (0-255) |
| `write_u8(buf, offset, value)` | Write byte (value 0-255) |
| `read_u16(buf, offset)` | Read 16-bit unsigned int |
| `write_u16(buf, offset, value)` | Write 16-bit unsigned int |
| `read_u32(buf, offset)` | Read 32-bit unsigned int |
| `write_u32(buf, offset, value)` | Write 32-bit unsigned int |
| `read_u64(buf, offset)` | Read 64-bit int |
| `write_u64(buf, offset, value)` | Write 64-bit int |

### Float Operations

| Function | Description |
|----------|-------------|
| `read_f32(buf, offset)` | Read 32-bit float |
| `write_f32(buf, offset, value)` | Write 32-bit float |
| `read_f64(buf, offset)` | Read 64-bit float |
| `write_f64(buf, offset, value)` | Write 64-bit float |

### Bulk Operations

| Function | Description |
|----------|-------------|
| `copy(src, src_offset, dst, dst_offset, len)` | Copy `len` bytes. Handles overlapping regions. |
| `fill(buf, offset, len, value)` | Fill `len` bytes with `value` (0-255). |

### Example

```rust
needs std.bytes

@no_gc
fn parse_header() {
    let buf = bytes.alloc(8)
    bytes.write_u32(buf, 0, 0x12345678)
    bytes.write_u32(buf, 4, 256)

    let magic = bytes.read_u32(buf, 0)
    let size = bytes.read_u32(buf, 4)

    bytes.free(buf)
    return magic == 0x12345678 and size == 256
}
```

### Notes

- All operations are bounds-checked
- Invalid handles produce runtime errors (not panics)
- Maximum allocation size: 256MB
- Buffers are NOT garbage collected—always call `free()`

---

That's all for now. More modules might be added in future versions!
