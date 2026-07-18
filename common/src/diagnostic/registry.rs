// TODO: international language support
// TODO: better messages with code example + a link to the aelys documentation website

use super::Severity;

pub struct DiagnosticInfo {
    pub code: &'static str,
    pub title: &'static str,
    pub explanation: &'static str,
    pub severity: Severity,
}

// Error code scheme:
//   E00xx  lexer
//   E01xx  parser
//   E02xx  name resolution
//   E03xx  types
//   E04xx  mutability
//   E05xx  control flow
//   E09xx  backend / internal
//   W01xx+ warnings

static REGISTRY: &[DiagnosticInfo] = &[
    // Lexer (E00xx)
    DiagnosticInfo {
        code: "E0001",
        title: "unterminated string literal",
        explanation: "\
A `\"` opens a string but it's never closed.

    let s = \"hello\"     // ok
    let s = \"hello       // E0001, no closing quote",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0002",
        title: "invalid character",
        explanation: "\
The lexer hit a character it doesn't recognize. This is usually a stray
unicode symbol or a copy-pasted curly quote that isn't valid syntax.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0003",
        title: "invalid number literal",
        explanation: "\
A numeric literal couldn't be parsed. Either the digits are invalid for
the base, or the value overflows during lexing.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0004",
        title: "block comment nesting too deep",
        explanation: "\
Aelys supports nested /* */ comments, but there's a depth limit. If you
hit this, you almost certainly have an unclosed `/*` somewhere above.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0005",
        title: "invalid escape sequence",
        explanation: "\
Unknown escape in a string literal. The recognized sequences are:

    \\n  \\t  \\r  \\\\  \\\"  \\0",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0006",
        title: "unterminated format expression",
        explanation: "\
A `{` inside a format string starts an interpolation, but the matching
`}` is missing.

    let msg = f\"hello {name\"   // E0006, missing `}`
    let msg = f\"hello {name}\"  // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0007",
        title: "unmatched close brace in string",
        explanation: "\
A lone `}` appeared in a string without an opening `{`.
To write a literal `}`, double it: `}}`.",
        severity: Severity::Error,
    },
    // Parser (E01xx)
    DiagnosticInfo {
        code: "E0101",
        title: "unexpected token",
        explanation: "\
The parser found a token that doesn't belong here. Common causes:
a missing comma, an extra parenthesis, or a forgotten newline between
two statements.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0102",
        title: "expected expression",
        explanation: "\
The parser expected an expression but got something else. For instance
an operator with nothing on its right side, or an empty argument slot.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0103",
        title: "expected identifier",
        explanation: "\
A name was expected (variable, function, struct, field) but a different
token appeared instead.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0104",
        title: "invalid assignment target",
        explanation: "\
The left-hand side of `=` must be something assignable: a variable, a
struct field (`p.x`), or an indexed element (`arr[i]`).

    1 + 2 = 3          // E0104
    x = 3              // ok
    point.x = 3        // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0105",
        title: "expression nesting too deep",
        explanation: "\
Expressions are nested beyond the compiler's recursion limit. This is a
stack-overflow guard. The expression needs to be simplified or broken
into intermediate variables.",
        severity: Severity::Error,
    },
    // Name resolution (E02xx)
    DiagnosticInfo {
        code: "E0201",
        title: "undefined variable",
        explanation: "\
This name hasn't been declared with `let` in any reachable scope.

    println(x)    // E0201 if `x` was never defined
    let x = 42
    println(x)    // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0202",
        title: "variable already defined",
        explanation: "\
A `let` binding with this name already exists in the same scope. Aelys
does not allow shadowing within the same scope. Pick a different name
or assign to the existing variable if it's `mut`.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0203",
        title: "undefined function",
        explanation: "\
No function with this name exists. Functions must be defined before the
call site (there is no hoisting).",
        severity: Severity::Error,
    },
    // Types (E03xx)
    DiagnosticInfo {
        code: "E0301",
        title: "type mismatch",
        explanation: "\
The compiler expected one type but got another. This shows up in many
situations: return values, function arguments, variable assignments,
binary operators, cast targets, etc.

The error message tells you the two types that conflict. Read it
carefully, the fix is almost always in the surrounding expression.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0302",
        title: "wrong number of arguments",
        explanation: "\
The function was called with more or fewer arguments than its signature
declares.

    fn add(a: i64, b: i64) -> i64 { return a + b }
    add(1)          // E0302, expected 2 args, got 1
    add(1, 2, 3)    // E0302, expected 2 args, got 3",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0303",
        title: "type is not callable",
        explanation: "\
The `()` call syntax was used on something that isn't a function.

    let x = 42
    x()    // E0303, `i64` is not callable",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0304",
        title: "invalid member access",
        explanation: "\
Field access (`.field`) was used on a type that either doesn't support
it or doesn't have a field with that name. Struct fields, string `.len`,
and vec `.len` are the valid member accesses.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0305",
        title: "infinite type detected",
        explanation: "\
A type ended up containing itself, which would make it infinitely large.
This happens when the inference engine tries to unify a type variable
with a type that already references it (e.g. `T = [T]`).",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0306",
        title: "unknown type",
        explanation: "\
A type annotation uses a name that doesn't correspond to any built-in
type or user-defined struct.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0307",
        title: "invalid cast",
        explanation: "\
`as` casts are only allowed between numeric types and bool:

    42 as f64       // ok, i64 to f64
    3.14 as i64     // ok, f64 to i64 (truncates)
    1 as bool       // ok
    \"hi\" as i64     // E0307, string to number not allowed",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0309",
        title: "type inference recursion limit",
        explanation: "\
The type solver hit its recursion ceiling. This points to a circular
or extremely deep type dependency that the inference engine can't
untangle. Simplify the involved types or add explicit annotations.",
        severity: Severity::Error,
    },
    // Mutability (E04xx)
    DiagnosticInfo {
        code: "E0401",
        title: "cannot assign to immutable variable",
        explanation: "\
Variables are immutable by default. Writing to one without `mut`
is an error:

    let x = 5
    x = 10        // E0401

Add `mut` to the binding to allow reassignment:

    let mut x = 5
    x = 10        // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0402",
        title: "cannot assign to loop variable",
        explanation: "\
The iteration variable of a `for` loop is controlled by the loop itself
and can't be overwritten inside the body.

    for i in 0..10 {
        i = 99    // E0402
    }",
        severity: Severity::Error,
    },
    // Control flow (E05xx)
    DiagnosticInfo {
        code: "E0501",
        title: "`break` outside of loop",
        explanation: "\
`break` only makes sense inside `for` or `while`. To leave a function
early, use `return`.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0502",
        title: "`continue` outside of loop",
        explanation: "\
`continue` skips to the next iteration of a loop. It has no meaning
outside `for` or `while`.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0503",
        title: "`return` outside of function",
        explanation: "\
`return` can only appear inside a function body. At the top level there's
nowhere to return to.",
        severity: Severity::Error,
    },
    // Backend / internal (E09xx)
    DiagnosticInfo {
        code: "E0901",
        title: "backend error",
        explanation: "\
Something went wrong during LLVM code generation. This is a compiler
bug. Please open an issue with the source file that triggered it.",
        severity: Severity::Error,
    },
];

/// Look up a diagnostic code in the registry.
pub fn lookup(code: &str) -> Option<&'static DiagnosticInfo> {
    REGISTRY.iter().find(|info| info.code == code)
}

/// Get all registered diagnostic codes.
pub fn all_codes() -> &'static [DiagnosticInfo] {
    REGISTRY
}
