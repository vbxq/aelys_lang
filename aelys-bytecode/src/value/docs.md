# NaN-Boxed Value Representation

This module implements Aelys values using NaN-boxing to pack primitives and object
pointers into a single 64-bit word.

- Integers are limited to 48 bits (±2^47).
- Floats are stored as raw IEEE-754 bits.
- Tagged values encode int/bool/null/ptr in the NaN payload.

Use `Value::int_checked` for user-provided integers to avoid silent wraparound.
