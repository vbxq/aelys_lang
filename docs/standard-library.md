# Standard Library Reference

All standard library modules live under `std.*`.

### Auto-registered modules

The safe modules : `std.io`, `std.math`, `std.string`, `std.convert`, and `std.time`, are auto-registered at VM startup. You can use their functions immediately without `needs`. For example, `println("hello")` and `math.sqrt(16.0)` work out of the box.

You can still use `needs` with an alias if you want a shorter name:

```rust
needs std.math as m
```

### Capability-gated modules

The following modules require an explicit `needs` import and the corresponding `--allow-caps` flag:

- `std.fs` -- file system access (`--allow-caps=fs`)
- `std.net` -- network access (`--allow-caps=net`)
- `std.sys` -- system information
- `std.bytes` -- raw byte buffers

**Important** : you can also use `--ae-trusted=true` to enable all caps.

```rust
needs std.fs
needs std.net
```

---

## std.io

Console input/output. Auto-registered -- no `needs` required.

### Output Functions

| Function | Description |
|----------|-------------|
| `print(value)` | Print value without newline |
| `println(value)` | Print value with newline |
| `eprint(value)` | Print to stderr, no newline |
| `eprintln(value)` | Print to stderr with newline |
| `flush()` | Flush stdout buffer |
| `eflush()` | Flush stderr buffer |

```rust
println("Hello")         // Hello\n
print("Loading")         // Loading (no newline)
print(".")               // .
println("")              // newline
```

### Input Functions

| Function | Description |
|----------|-------------|
| `readline()` | Read line from stdin (null on EOF) |
| `read_char()` | Read single character |
| `input(prompt)` | Print prompt, read line |

```rust
let name = input("What's your name? ")
println("Hello, {name}")
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
clear_screen()
move_cursor(10, 5)
println("Here!")
```

---

## std.math

Math functions and constants. Auto-registered -- no `needs` required.

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
| `randint(debut, fin)` | Random integer in range [debut, fin] (inclusive) |

```rust
math.abs(-5)        // 5
math.sqrt(16.0)     // 4.0
math.pow(2, 10)     // 1024
math.clamp(15, 0, 10)  // 10
math.randint(1, 6)  // random int from 1 to 6 (dice roll)
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

String manipulation. Strings are UTF-8. Auto-registered -- no `needs` required.

String functions can be called as methods on string values. The qualified `string.func(s, ...)` syntax also still works.

```rust
// Method syntax (preferred)
let n = "hello".len()
let up = "hello".to_upper()

// Qualified syntax (also valid)
let n = string.len("hello")
let up = string.to_upper("hello")
```

### Length

| Method | Description |
|--------|-------------|
| `s.len()` | Length in bytes |
| `s.char_len()` | Length in Unicode characters |

```rust
"café".len()       // 5 (bytes)
"café".char_len()  // 4 (characters)
```

This distinction matters for non-ASCII text.

### Character Access

| Method | Description |
|--------|-------------|
| `s.char_at(i)` | Character at index (empty if out of bounds) |
| `s.byte_at(i)` | Byte at index (-1 if out of bounds) |
| `s.substr(start, len)` | Extract substring by character position |

```rust
"hello".char_at(0)    // "h"
"hello".char_at(10)   // "" (out of bounds)
"hello".substr(1, 3)  // "ell"
```

### Case Conversion

| Method | Description |
|--------|-------------|
| `s.to_upper()` | Convert to uppercase |
| `s.to_lower()` | Convert to lowercase |
| `s.capitalize()` | Capitalize first character |

### Search

| Method | Description |
|--------|-------------|
| `s.contains(needle)` | Check if string contains substring |
| `s.starts_with(prefix)` | Check prefix |
| `s.ends_with(suffix)` | Check suffix |
| `s.find(needle)` | First occurrence index (-1 if not found) |
| `s.rfind(needle)` | Last occurrence index |
| `s.count(needle)` | Count occurrences |

Note: `find` and `rfind` return byte positions, not character positions. This can be surprising with Unicode - I might change this in a future version.

### Transformation

| Method | Description |
|--------|-------------|
| `s.replace(old, new)` | Replace all occurrences |
| `s.replace_first(old, new)` | Replace first occurrence |
| `s.reverse()` | Reverse string |
| `s.repeat(n)` | Repeat n times |
| `s.concat(other)` | Concatenate (same as `+`) |

### Whitespace

| Method | Description |
|--------|-------------|
| `s.trim()` | Remove whitespace from both ends |
| `s.trim_start()` | Remove from start |
| `s.trim_end()` | Remove from end |
| `s.pad_left(width, char)` | Pad start to width |
| `s.pad_right(width, char)` | Pad end to width |

```rust
"  hello  ".trim()        // "hello"
"42".pad_left(5, "0")     // "00042"
```

### Splitting

| Method | Description |
|--------|-------------|
| `s.split(sep)` | Split by separator |
| `string.join(parts, sep)` | Join parts with separator |
| `s.lines()` | Split into lines |
| `s.line_count()` | Count lines |

`split` and `join` use newline-separated strings as the "list" representation. This is awkward (I know), but arrays aren't first-class yet.

```rust
let parts = "a,b,c".split(",")       // "a\nb\nc"
string.join(parts, "-")              // "a-b-c"
```

### Predicates

| Method | Description |
|--------|-------------|
| `s.is_empty()` | Check if empty |
| `s.is_whitespace()` | Only whitespace? |
| `s.is_numeric()` | Only digits? |
| `s.is_alphabetic()` | Only letters? |
| `s.is_alphanumeric()` | Letters and digits only? |

---

## std.convert

Type conversions and introspection. Auto-registered -- no `needs` required.

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

`.to_string()` can be called as a method on any value:

```rust
(42).to_string()      // "42"
true.to_string()      // "true"
(3.14).to_string()    // "3.14"
```

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

Time operations and measurement. Auto-registered -- no `needs` required.

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
println("Took {ms}ms")
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
    println(line)
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
| `connect(host, port)` | Connect to server (30s timeout), returns handle |
| `connect_timeout(host, port, ms)` | Connect with custom timeout in milliseconds, returns handle |
| `send(handle, data)` | Send data |
| `recv(handle)` | Receive available data |
| `recv_bytes(handle, max)` | Receive up to max bytes |
| `recv_line(handle)` | Receive one line |
| `close(handle)` | Close connection |

```rust
let sock = net.connect("example.com", 80)
net.send(sock, "GET / HTTP/1.0\r\nHost: example.com\r\n\r\n")
let response = net.recv(sock)
println(response)
net.close(sock)
```

### TCP Server

| Function | Description |
|----------|-------------|
| `listen(host, port)` | Start listening, returns handle |
| `accept(handle)` | Accept connection, returns new handle |

```rust
let server = net.listen("0.0.0.0", 8080)
println("Listening on port 8080")

while true {
    let client = net.accept(server)
    let request = net.recv_line(client)
    net.send(client, "HTTP/1.0 200 OK\r\n\r\nHello!")
    net.close(client)
}
```

### UDP

| Function | Description |
|----------|-------------|
| `udp_bind(host, port)` | Bind a UDP socket, returns handle |
| `udp_send_to(handle, data, addr)` | Send datagram to `"host:port"` |
| `udp_recv_from(handle, max)` | Receive datagram (up to max bytes) |
| `udp_connect(handle, host, port)` | Connect UDP socket to fixed remote |
| `udp_send(handle, data)` | Send on connected UDP socket |
| `udp_recv(handle, max)` | Receive on connected UDP socket |
| `udp_set_broadcast(handle, enabled)` | Enable/disable broadcast |

```rust
// Connectionless (send_to / recv_from)
let sock = net.udp_bind("0.0.0.0", 0)
net.udp_send_to(sock, "hello", "127.0.0.1:9000")
let data = net.udp_recv_from(sock, 1024)
net.close(sock)

// Connected mode
let sock = net.udp_bind("0.0.0.0", 0)
net.udp_connect(sock, "127.0.0.1", 9000)
net.udp_send(sock, "hello")
let data = net.udp_recv(sock, 1024)
net.close(sock)
```

`close`, `set_timeout`, and `local_addr` also work with UDP handles.

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
println("Running on {sys.platform()} {sys.arch()}")
// "Running on linux x86_64"
```

---

## std.bytes

Byte-level memory operations for binary data manipulation. Use with `@no_gc` functions for best performance.

```rust
needs std.bytes
```

### Allocation & Buffer Management

| Function | Description |
|----------|-------------|
| `alloc(size)` | Allocate `size` bytes, initialized to zero. Returns handle. |
| `free(handle)` | Free buffer. `free(null)` is a no-op. |
| `size(handle)` | Return buffer size in bytes. |
| `resize(handle, new_size)` | Resize buffer (preserves existing data). |
| `clone(handle)` | Create a copy of the buffer. |
| `equals(a, b)` | Compare two buffers for equality. |

### Unsigned Integer Operations (Little-Endian)

| Function | Description |
|----------|-------------|
| `read_u8(buf, offset)` | Read unsigned byte (0-255) |
| `write_u8(buf, offset, value)` | Write unsigned byte |
| `read_u16(buf, offset)` | Read 16-bit unsigned int |
| `write_u16(buf, offset, value)` | Write 16-bit unsigned int |
| `read_u32(buf, offset)` | Read 32-bit unsigned int |
| `write_u32(buf, offset, value)` | Write 32-bit unsigned int |
| `read_u64(buf, offset)` | Read 64-bit unsigned int |
| `write_u64(buf, offset, value)` | Write 64-bit unsigned int |

### Signed Integer Operations (Little-Endian)

| Function | Description |
|----------|-------------|
| `read_i8(buf, offset)` | Read signed byte (-128 to 127) |
| `write_i8(buf, offset, value)` | Write signed byte |
| `read_i16(buf, offset)` | Read 16-bit signed int |
| `write_i16(buf, offset, value)` | Write 16-bit signed int |
| `read_i32(buf, offset)` | Read 32-bit signed int |
| `write_i32(buf, offset, value)` | Write 32-bit signed int |
| `read_i64(buf, offset)` | Read 64-bit signed int |
| `write_i64(buf, offset, value)` | Write 64-bit signed int |

### Big-Endian Operations

All `_be` variants for network byte order:

| Function | Description |
|----------|-------------|
| `read_u16_be`, `write_u16_be` | 16-bit unsigned big-endian |
| `read_i16_be`, `write_i16_be` | 16-bit signed big-endian |
| `read_u32_be`, `write_u32_be` | 32-bit unsigned big-endian |
| `read_i32_be`, `write_i32_be` | 32-bit signed big-endian |
| `read_u64_be`, `write_u64_be` | 64-bit unsigned big-endian |
| `read_i64_be`, `write_i64_be` | 64-bit signed big-endian |
| `read_f32_be`, `write_f32_be` | 32-bit float big-endian |
| `read_f64_be`, `write_f64_be` | 64-bit float big-endian |

### Float Operations (Little-Endian)

| Function | Description |
|----------|-------------|
| `read_f32(buf, offset)` | Read 32-bit float |
| `write_f32(buf, offset, value)` | Write 32-bit float |
| `read_f64(buf, offset)` | Read 64-bit float |
| `write_f64(buf, offset, value)` | Write 64-bit float |

### Bulk Operations

| Function | Description |
|----------|-------------|
| `copy(src, src_off, dst, dst_off, len)` | Copy `len` bytes. Handles overlapping regions. |
| `fill(buf, offset, len, value)` | Fill `len` bytes with `value` (0-255). |
| `reverse(buf, offset, len)` | Reverse bytes in range. |
| `swap(buf, i, j)` | Swap bytes at indices i and j. |
| `find(buf, start, end, byte)` | Find first occurrence of byte. Returns -1 if not found. Use -1 for end to search to buffer end. |

### String Operations

| Function | Description |
|----------|-------------|
| `from_string(str)` | Create buffer from UTF-8 string. |
| `decode(buf, offset, len)` | Decode UTF-8 bytes to string. |
| `write_string(buf, offset, str)` | Write string to buffer. Returns bytes written. |

### Example

```rust
needs std.bytes

@no_gc
fn parse_network_packet() {
    let buf = bytes.alloc(16)

    // Write big-endian header (network byte order)
    bytes.write_u32_be(buf, 0, 0xDEADBEEF)  // magic
    bytes.write_u16_be(buf, 4, 1024)         // length
    bytes.write_i16_be(buf, 6, -100)         // signed field

    // String data
    bytes.write_string(buf, 8, "Meow")

    // Read back
    let magic = bytes.read_u32_be(buf, 0)
    let msg = bytes.decode(buf, 8, 4)

    bytes.free(buf)
}
```

### Notes

- All operations are bounds-checked
- Invalid handles produce runtime errors
- Maximum allocation size: 256MB
- Buffers are NOT garbage collected, always call `free()`

---

That's all for now. More modules might be added in future versions!
