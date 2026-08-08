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
// e07xx borrow / ownership
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
    DiagnosticInfo {
        code: "E0412",
        title: "`Vec<T>` used outside the guaranteed value-semantics surface",
        explanation: "\
`Vec<T>` gives value semantics only inside a closed surface, and anything
outside it is rejected rather than silently miscompiled.

Form: in a Vec-producing position (a `let` initializer, an assignment
right-hand side, a `*p = e` value, a `return` operand) the expression must
be `Vec::new()`, a `vec[...]` literal, a bare identifier, or a call:

    let w = (v)                 // E0412
    let w = if c { v } else { v }   // E0412
    let w = v                   // ok

Shape: no `Vec` may be held by value inside another `Vec`, an array, a
struct, an enum payload or an `Rc` payload, and no generic function may be
instantiated with a type that holds a `Vec` by value:

    Vec::push(vv, inner)        // E0412, `inner` is a Vec
    let a = [inner, inner]      // E0412
    keep(v)                     // E0412 for `fn keep<T>(x: T)`

Bind the value to a name first, or restructure so the `Vec` is held
directly by a local.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0413",
        title: "slicing a `Vec<T>` is not supported yet",
        explanation: "\
A `Vec<T>` is a `{ptr, len, cap}` header whose elements live in a separate
heap buffer. Slicing it would take the address of that header rather than
the buffer, so it is rejected instead of silently miscompiled.

    let v = vec[1, 2, 3]
    let s = v[0..2]        // E0413

Slice an array, whose elements are stored inline, or read the `Vec`
element by element:

    let a = [1, 2, 3]
    let s = a[0..2]        // ok, arrays are sliceable
    let x = v[0]           // ok, index a Vec element directly

A sound slice-of-Vec needs the heap-buffer-view path (a place address into
the buffer, with copy-on-write for a mutable view), which is deferred.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0414",
        title: "iterating a `Vec<T>` with `for` is not supported yet",
        explanation: "\
`for x in <collection>` has dedicated lowering for arrays and strings, but
not for a `Vec<T>`, so iterating a `Vec` is rejected instead of failing
later with a link error.

    let v = vec[10, 20, 30]
    for x in v { }         // E0414

Iterate an array or a string, or index the `Vec` by hand with a counting
loop over its length:

    let a = [10, 20, 30]
    for x in a { }         // ok, arrays are iterable
    for i in 0..3 {        // ok, index the Vec element by element
        let x = v[i]
    }

By-value iteration over a `Vec` shares its buffer, which needs the same
heap-buffer-view path as slicing (deferred).",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0415",
        title: "a mutable reference into an element is not supported yet",
        explanation: "\
`&mut v[i]` (or `&mut a[i]`) forms a mutable reference into a Vec or array
element. Today the reference is taken of a loaded stack copy of the element,
so writing through it never reaches the real element, silently miscompiling.
It is rejected rather than accepted.

    let mut v = vec[1, 2, 3]
    let r = &mut v[0]      // E0415
    *r = 9                 // would write a stack copy, not v[0]

An immutable `&v[i]` stays valid (a read through it is sound), and so does
`&mut` of a whole binding. Write the element directly, or reference the
binding:

    let x = v[0]           // ok, read the element
    let r = &v[0]          // ok, immutable element reference
    v[0] = 9               // ok, write the element directly
    let r = &mut v         // ok, reference the whole binding

Sound `&mut` into an element needs a place-address path with copy-on-write
for a shared buffer, which is deferred.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0416",
        title: "a reference into a call-result field is not supported yet",
        explanation: "\
`&Rc::get(r).x` (and `&<call>().field` in general) takes a reference into a
field of a temporary produced by a call. No loan is formed and the address
points into a value that does not outlive the expression, so it is rejected
rather than accepted unsoundly.

    let r = Rc::new(Cell{x: 1})
    let p = &Rc::get(r).x  // E0416

Bind the value to a local first, then reference the local:

    let c = Rc::get(r)     // bind the payload
    let p = &c.x           // ok

A reference into a plain binding's field (`&p.x` where `p` is a local) stays
valid.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0417",
        title: "mutable reference to an immutable binding",
        explanation: "\
`&mut x` where `x` was declared without `mut` would hand out a mutable
reference to an immutable place. Writing through it mutates a binding the
program declared it would not change, which is unsound, so it is rejected.

    let x = 1
    let a = &mut x         // E0417
    *a = 2                 // would mutate an immutable binding

Declare the binding mutable, or take an immutable reference:

    let mut x = 1          // ok, then `&mut x` is valid
    let a = &x             // ok, immutable reference of an immutable binding",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0418",
        title: "nested function shadows an outer function",
        explanation: "\
A nested `fn` declared with the same name as an outer function reuses that
function's dispatch slot. A call to the outer function can then be silently
lowered to the nested body, a miscompile, so the collision is rejected.

    fn pick(a: i64, b: i64) -> i64 { return a }
    fn other() -> i64 {
        fn pick(a: i64, b: i64) -> i64 { return b }   // E0418
        return pick(1, 2)
    }

Rename the nested function so its name is unique:

    fn other() -> i64 {
        fn choose(a: i64, b: i64) -> i64 { return b } // ok
        return choose(1, 2)
    }",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0419",
        title: "reserved builtin type name",
        explanation: "\
`Vec` and `Rc` are builtin type names. A `Vec::` or `Rc::` path is
intercepted by the compiler before any user type of that name is looked
up, so a user `struct`/`enum` named `Vec` or `Rc` would be silently
rerouted to the builtin lowering. The declaration is rejected instead.

    enum Vec { Empty, One(i64) }   // E0419
    struct Rc { count: i64 }       // E0419

Rename the type to anything else:

    enum MyList { Empty, One(i64) } // ok
    struct RefCount { count: i64 }  // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0420",
        title: "indirect right-hand side of an `Rc` field assignment",
        explanation: "\
Reassigning an `Rc` field through an `Rc` handle
(`handle.field = rhs`, where `field` is itself an `Rc`) balances the
store with a retain of the new value and a release of the old one. The
retain is only emitted for a direct right-hand side; an indirect form
slips past the provenance check and undercounts the refcount by one,
which can free a value that is still reachable.

    node.next = if c { a } else { b }   // E0420
    node.next = (a)                     // E0420

Use a direct producer: `Rc::new(...)`, `Rc::null()`, a bare identifier,
an `Rc`-field read, or a call. Bind an indirect value to a name first:

    node.next = a                       // ok, bare identifier
    node.next = Rc::new(x)              // ok, fresh producer
    let picked = if c { a } else { b }  // bind first
    node.next = picked                  // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0421",
        title: "this expression denotes no storage",
        explanation: "\
`&`, `&mut`, `Vec::push`'s target and the left-hand side of an
assignment all need an ADDRESS. A call result, an aggregate literal or
an arithmetic expression is a value, not a place: it lives in a
temporary the compiler is free to discard, so a write through it would
land nowhere.

    let r = &make_cell()        // E0421
    make_cell().f = 101         // E0421
    Vec::push(make_vec(), 1)    // E0421

Bind the value to a name first, then use the binding:

    let mut c = make_cell()
    c.f = 101                   // ok
    let r = &mut c              // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0422",
        title: "mutation through a shared reference",
        explanation: "\
A shared `&` grants read access only. Writing through one -- directly,
or through any projection of it -- would break `mut XOR shared`, the
rule the whole reference model rests on.

    fn poke(r: &Cell)  { (*r).f = 101 }     // E0422
    fn poke(r: &i64)   { *r = 101 }         // E0422
    fn grow(r: &Vec<i64>) { Vec::push((*r), 1) }  // E0422

Take the reference mutably instead:

    fn poke(r: &mut Cell) { (*r).f = 101 }  // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0423",
        title: "unchecked reference inside a closure body",
        explanation: "\
A lambda body is lowered without its own borrow-check pass, so a
reference FORMED or RETURNED inside one is never checked: it can outlive
what it points at with no diagnostic. Until a closure body carries its
own checked region, forming a reference in one is refused.

    let f = fn () -> i64 { let r = &v[0]  return *r }   // E0423

Form the reference outside the lambda and pass it in; a lambda that
merely TAKES a reference parameter, or dereferences one formed outside,
is unaffected:

    let r = &n
    let f = fn () -> i64 { return *r }          // ok
    apply(fn (p: &i32) -> i64 { return *p }, &n)  // ok",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0424",
        title: "reference to module-level storage",
        explanation: "\
The borrow checker works on function locals. A module-level `let` has no
local to attach a loan to, so two live `&mut` of the same global would
alias with nothing to notice. Rather than accept a silent aliasing hole,
a reference to a global is refused.

    let mut g: i64 = 1
    fn main() -> i64 { let r = &mut g  ... }    // E0424

Copy the global into a local, work on that, and store it back:

    let mut local = g
    let r = &mut local
    *r = 101
    g = local",
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
// borrow / ownership (e07xx)
    DiagnosticInfo {
        code: "E0701",
        title: "use of a moved value",
        explanation: "\
An affine value was used after it had been moved out. A move transfers
ownership, leaving the source unusable.

    let a = Resource{id: 1}
    let b = a          // `a` is moved into `b`
    use(a)             // E0701, `a` was moved

Copy the value instead, or restructure so the value is used before it is
moved.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0702",
        title: "double move",
        explanation: "\
An affine value was moved a second time after it had already been moved.
Ownership can only be transferred once.

    let a = Resource{id: 1}
    let b = a          // first move
    let c = a          // E0702, `a` was already moved",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0703",
        title: "use of a possibly-moved value",
        explanation: "\
A value may have been moved on one branch but not another, so at this
point it cannot be proven to still be owned. Run 1 does not track
conditional moves, so the use is rejected.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0704",
        title: "affine value not deterministically destructible",
        explanation: "\
An affine value may have been moved on one branch and not another, so
the compiler cannot decide at a scope exit or a reassignment whether it
still needs to be destroyed. Conditional moves are not supported in
Run 1; make the move unconditional or avoid it.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0711",
        title: "mutation of a borrowed value",
        explanation: "\
A value was written to while a borrow of it was still live. Aelys
enforces mutable-xor-shared: no writes may occur through the owner while
any borrow is outstanding.

    let mut x = 3
    let r = &x
    x = 5              // E0711, `x` is borrowed by `r`
    return *r          // the borrow is still used here",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0712",
        title: "move of a borrowed value",
        explanation: "\
An affine value was moved while a borrow of it was still live. Moving
would invalidate the outstanding reference.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0713",
        title: "conflicting borrow",
        explanation: "\
A borrow conflicts with another live borrow of the same place, violating
mutable-xor-shared: two `&mut` overlapping, a `&` while a `&mut` is live,
or a use of the owner while a `&mut` is live.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0714",
        title: "reference stored into a container",
        explanation: "\
A reference was placed into an array, vec, or enum payload. Run 1 does
not track references that flow through aggregate containers, so this is
rejected at the container boundary.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0721",
        title: "function returns a reference to a local",
        explanation: "\
The function returns a reference borrowing one of its own locals. That
local is destroyed when the function returns, so the reference would
dangle.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0722",
        title: "borrow outlives its referent",
        explanation: "\
A value does not live long enough: it is borrowed, and the borrow is
still used after the value's scope has ended. The referent must outlive
every borrow of it.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0723",
        title: "cannot infer the origin of a returned reference",
        explanation: "\
The compiler could not determine what a returned reference borrows, so
it cannot prove the reference is safe to return. This is the
conservative floor of the Run 1 escape check.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0724",
        title: "reference stored into a container",
        explanation: "\
A reference reached an aggregate container through a projected store
(a field, index, or deref assignment). Run 1 does not track references
inside containers, so this is rejected.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0725",
        title: "closure captures a reference",
        explanation: "\
A closure captured a reference in its environment. Run 1 cannot track a
borrow that escapes into an opaque closure environment, so this is
rejected.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0726",
        title: "reference to a reference",
        explanation: "\
A reference to a reference was formed. Run 1's whole-local provenance
cannot follow a loan through the extra layer of indirection, so nested
references are not supported.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0727",
        title: "nogc function reaches managed memory",
        explanation: "\
A function declared `nogc` was inferred to reach managed (reference-counted
or heap-allocating) memory, directly or through a call it makes. A `nogc`
function must stay free of managed effects.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0728",
        title: "`nogc fn` type or parameter used out of position",
        explanation: "\
A `nogc fn(...)` type may only appear as an immutable function parameter
type, and such a parameter must keep the function the caller passed.
Three shapes are rejected under this code. The type itself out of
position: in a let binding, a return type, an aggregate element or a
nested function type. A `nogc fn` parameter declared `mut`, which could
be reassigned to a general function. And a `let` binding that shadows a
`nogc fn` parameter, which would rebind the name to something the callers
never checked. Each of them could let a general function stand in for a
nogc one.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0729",
        title: "argument is not a `nogc fn`",
        explanation: "\
A parameter typed `nogc fn(...)` was given an argument that is not a
direct reference to a `nogc`-declared function or a `nogc fn` parameter.
Lambdas, general functions, and let-bound variables are rejected so the
callee is provably free of managed effects.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0730",
        title: "`nogc` bound not satisfied",
        explanation: "\
A generic function whose type parameter is bound `nogc` (written
`<T: nogc>`, or implied for every type parameter of a `nogc`-declared
function) was instantiated with a type that is not a nogc value, or with
a type the call site cannot pin down. Managed types such as `vec`, `Rc`
and `string` are rejected, and so is an abstract type parameter, because
the bound could not be proven there.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0731",
        title: "`nogc` generic used as a value",
        explanation: "\
A generic function with a `nogc` bound was referenced outside a direct
call. Taking it as a value would let it be instantiated somewhere the
bound is never checked, so only direct calls are allowed.",
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

