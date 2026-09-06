use super::Severity;

pub struct DiagnosticInfo {
    pub code: &'static str,
    pub title: &'static str,
    pub explanation: &'static str,
    pub severity: Severity,
}

// e04xx mutability

static REGISTRY: &[DiagnosticInfo] = &[
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
    // mutability (e04xx)
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
`Vec<T>` gives value semantics only where every copied buffer share is
accounted for. Forms outside that proven surface are rejected rather than
silently miscompiled.

Form: in a Vec-producing position (a `let` initializer, an assignment
right-hand side, a `*p = e` value, a `return` operand) the expression must
be `Vec::new()`, a `vec[...]` literal, a bare identifier, or a call:

    let w = (v)                 // E0412
    let w = if c { v } else { v }   // E0412
    let w = v                   // ok

Shape: a COW byte copy that carries a `Vec` inside another value has no
transitive retain/release for the inner buffer. With the guard disabled,
the nested reallocation witness returned 7 under immix but 32 under malloc.
The same surface also conservatively refuses a `Vec` held by an aggregate or
generic value:

    Vec::push(vv, inner)        // E0412, `inner` is a Vec
    let a = [inner, inner]      // E0412
    keep(v)                     // E0412 for `fn keep<T>(x: T)`

An indirect producer such as parentheses, a conditional, or a block is
also rejected when ownership transfer cannot be proven from its shape; the
syntax itself is not the memory defect. Keep the `Vec` in a direct local or
pass it through a direct call whose ownership is known.",
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

There is no Vec foreach lowering arm. Disabling this check reaches the
unnamed `E0901 [air-lowering]` failure at all three optimization levels and
both allocators, so use an array or string, or index the Vec in a counting
loop.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0415",
        title: "a mutable reference through an element or field projection is not supported yet",
        explanation: "\
`&mut v[i]`, `&mut a[i]`, or `&mut p.f` forms a mutable reference through a
projection. The refusal matches the shape of the operand and reads no type
at all, so it fires on every base alike -- on a plain array element and on a
plain struct field exactly as it fires on a managed container:

    let mut a: [i64; 3] = [1, 2, 3]
    let r = &mut a[0]      // E0415, and no managed storage is involved

    struct D { n: i64 }
    let mut d: D = D{n: 1}
    let r = &mut d.n       // E0415, likewise

What the fence stands in for is therefore not one obligation. Lifting it for
a projection into a managed container owes a uniqueness proof for that
container's buffer; lifting it for a disjoint field or an array element owes
nothing, and that half is a missing feature rather than a soundness fence.

An immutable `&v[i]` stays valid (a read through it is sound), and so does
`&mut` of a whole binding. Write the place directly, or reference the
binding:

    let x = v[0]           // ok, read the element
    let r = &v[0]          // ok, immutable element reference
    v[0] = 9               // ok, write the element directly
    let r = &mut v         // ok, reference the whole binding

The same fence also covers an indexed global projection, which the
whole-global check cannot name.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0416",
        title: "a shared borrow where a mutable borrow is required",
        explanation: "\
`&T` and `&mut T` are different types. A shared `&` grants read access
and may be copied; a `&mut` grants exclusive write access and may not.
Binding a shared borrow to a `&mut` position would hand out write
access nobody checked, breaking `mut XOR shared`.

    let mut x: i64 = 7919
    let p: &mut i64 = &x       // E0416
    let q: &mut i64 = &mut x   // ok

The same refusal applies wherever a value flows into a required type:
a call argument, a return, an assignment, a field, an array element.

    fn poke(r: &mut i64) { *r = 101 }
    poke(&x)                   // E0416
    poke(&mut x)               // ok

The other direction is a safe weakening and stays accepted, because
dropping write access can never create aliasing:

    let r: &i64 = &mut x       // ok",
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
A nested `fn` declared with the same bare name as an outer function enters
the shared function namespace with an ambiguous dispatch binding. The
generic collision witness returned 10 while its unique-name twin returned 9
when this fence was disabled, so the collision is rejected.

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
`Vec` and `Rc` are builtin type names. Their intrinsic paths share the
compiler's type namespace with user declarations, so a user `struct` or
`enum` with either name would make name resolution depend on the builtin
interception order. The declaration is rejected to keep that namespace
unambiguous; rename the type.

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
(`handle.field = rhs`, where `field` is itself an `Rc`) needs a retain of
the new value and a release of the old one. The provenance check only knows
the direct producer forms. With this fence disabled, the indirect witness
returned the right value but reported `allocs=2 frees=1`, so the ownership
accounting is unbalanced.

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
    DiagnosticInfo {
        code: "E0425",
        title: "this slice form is not supported yet",
        explanation: "\
A slice is built from the address of element zero of its base, plus a
length. Two forms have neither yet.

A NON-ZERO START BOUND would have to move the base pointer as well as
the length, and the slice lowering addresses element zero:

    let a = [1, 2, 3, 4]
    let s = a[1..3]        // E0425
    let s = a[0..3]        // ok

A BASE THAT CARRIES NO LENGTH -- a string, or a type the checker left
open -- has no header to read the length from:

    let t = \"abcdef\"
    let s = t[0..2]        // E0425

Arrays, `Vec<T>` and slices all carry a length and can be sliced from
zero.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0427",
        title: "two functions compile to the same symbol",
        explanation: "\
An ordinary function's symbol is its bare name, whatever scope it was
declared in. Two declarations that land on one symbol share a single
emitted body, and every call to either reaches whichever one was emitted.

    fn outer() -> i64 {
        fn dup() -> i64 { return 1 }
        return dup()
    }
    fn other() -> i64 {
        fn dup() -> i64 { return 2 }   // E0427, same symbol as the one above
        return dup()
    }

A generic instance is named from the function name and the type arguments
it was instantiated with, joined by `_`, so two functions that share no
name at all can still land on one symbol:

    struct B { v: i64 }
    struct A_B { v: i64 }
    fn f<T>(x: T) -> i64 { return 1 }     // at T = A_B
    fn f_A<T>(x: T) -> i64 { return 2 }   // at T = B
// , both are `__mono_f_a_b`

`main` counts here too, because it is emitted as `__aelys_main`.

Nested functions do not get scope-qualified symbols yet, and generic
instances are not disambiguated, so the fix is to rename one of them.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0428",
        title: "function name starts with the reserved `__` prefix",
        explanation: "\
Names beginning with `__` belong to the compiler and the runtime. The
compiler emits `__aelys_main` for your `main`, `__mono_*` for each
instance of a generic, `__lambda_*` for each lambda and `__closure_env_*`
for a closure's captured state, and the C runtime exports a further set of
`__aelys_*` symbols that are linked into every binary.

    fn __aelys_main() -> i64 { return 1 }   // E0428
    fn __helper(x: i64) -> i64 { return x } // E0428

A user function landing on one of those names produces two definitions of
the same linker symbol, and a call to one reaches the other. The runtime's
half of the set lives in another language's object files, so the compiler
cannot check the names one by one; the whole prefix is reserved instead.

Rename the function. Locals, parameters and struct fields are unaffected:

    let __x = 5                             // ok",
        severity: Severity::Error,
    },
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
    DiagnosticInfo {
        code: "E0601",
        title: "module not found",
        explanation: "\
A `needs` names a module whose file does not exist. A module path
`a.b.c` resolves to exactly one file, `<root>/a/b/c.aelys`, where
`<root>` is the directory of the file named on the command line.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0602",
        title: "circular module dependency",
        explanation: "\
Two or more modules import each other, directly or through a chain.
Each module is type-checked on its own, in dependency order, so a cycle
has no order to check it in. Break the cycle by moving the shared items
into a third module that both can import.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0603",
        title: "conflicting import name",
        explanation: "\
Two `needs` declarations introduce the same name into one module, or an
import introduces a name the module already defines. Use `as` to bind
one of them to another name.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0604",
        title: "reserved module path segment",
        explanation: "\
A segment of a module path starts with `__`, which is reserved for
compiler- and runtime-generated symbols. Module paths become part of
symbol names, so a reserved segment could collide with them.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0605",
        title: "item is not public",
        explanation: "\
A module exports only the items it declares `pub`. Everything else is
private to the file that defines it.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0606",
        title: "no such item in module",
        explanation: "\
The named module exists and compiles, but exports nothing under that
name. Note that a private item is reported as E0605, not as this code.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0607",
        title: "C header import is not implemented",
        explanation: "\
`needs \"some/header.h\"` is the foreign form of the import keyword: the
target is a string literal rather than a module path. The form is
reserved and parsed, but nothing reads C headers yet.

The route that does work is to declare by hand what you need:
`unsafe extern fn NAME(...) -> T`. It carries no header, so the types are
yours to get right, and the surface it accepts is narrow: integers,
floats, `bool` and references, and nothing else, as E0615 spells out.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0608",
        title: "`needs` outside the module prologue",
        explanation: "\
Every `needs` belongs at the top of the file, before any other
top-level declaration, and never inside a function or a block. Imports
bind for the whole module, so a position further down would suggest a
scope the language does not have.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0609",
        title: "wildcard import is not implemented",
        explanation: "\
`needs a.b.*` would bind every exported name of `a.b` at once. Name
what you need instead: `needs alpha, beta from a.b`.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0610",
        title: "field is not public",
        explanation: "\
A struct exports only the fields it declares `pub`. Everything else is
private to the module that defines the struct, whether it is read,
written, or supplied in a struct literal.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0611",
        title: "private type in a public signature",
        explanation: "\
A `pub` item names a type its own module keeps private, so an importer
would receive a value of a type it can never name. Make the type `pub`,
or stop exporting the item that mentions it.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0612",
        title: "conflicting declarations of the same external symbol",
        explanation: "\
One symbol has one signature, and this code fires when the program says
two things about it.

Either an external declaration names a symbol that this program also
defines, and only one of the two survives the link, so a call written
against the foreign function can silently reach the Aelys body; or two
external declarations name the same symbol and disagree about its types,
its calling convention or its `nogc` claim, which the lowered ir cannot
tell apart because it carries neither the claim nor the difference
between a borrow and a reference counted pointer.

Make the declarations agree, or rename the Aelys function.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0613",
        title: "a definition claims a symbol the runtime links",
        explanation: "\
The Aelys runtime is a C archive. It defines some symbols of its own and
imports others from libc, and the linker matches both against whatever
else the program defines under the same name.

A body named `malloc`, `free` or `main` therefore captures the runtime's
own calls, which is a crash or a wrong answer rather than a link error.
Rename the function.

An `unsafe extern` declaration is held to the narrower half: it claims no
symbol, so it may name `malloc` or any other name the runtime merely
imports, and only the five the runtime defines itself stay closed to it,
because calling one of those would reach the runtime's own code.

Within one compilation unit the duplicate check runs first, so a
declaration that names a symbol the program also defines under that name
is reported as E0301 and never reaches this check. `unsafe extern fn
main` in a program that also writes `fn main` is the reachable case: both
lower to `__aelys_main`, and the verdict is E0301.

A library named by `-l` is not source, so E0613 never sees it. The link
line puts the user libraries after `-laelys-core-*`, which keeps an
archive from capturing the five the runtime defines, and by the same
mechanism exposes the sixteen it imports: `malloc` is still undefined
when the user archive is searched, so its member is pulled in. E0618 is
the check that answers there, after the link, and it sees only what
entered the executable statically.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0614",
        title: "malformed external declaration",
        explanation: "\
An external declaration names a function that lives outside this program,
so it has a signature and no body, and calling it is unchecked.

The only accepted form is `unsafe extern [nogc] fn NAME(PARAMS) [-> T]`,
written at the top level, without `pub`, without a decorator and without
a type parameter.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0615",
        title: "type outside the external type surface",
        explanation: "\
An external declaration is an abi promise, so every parameter type and
the return type must have a meaning in C that the compiler can hold to.

The surface is the eight integer widths, `f32`, `f64`, `bool` and `&T`;
`void` is accepted as a return type and nowhere else. Everything else,
a string, an `Rc<T>`, a struct, an enum, an array, a `vec`, a slice or a
function type, is an Aelys value whose layout is not promised to C.

The verdict is on the type itself, not on how it is spelled: `sTRING`
names the same type as `string`.

`&T` is admitted as a parameter and refused as a return type. A returned
`&T` would be a reference the compiler did not prove, safe to dereference
outside any `unsafe` block, and its value has been measured to change with
the optimization level. An opaque foreign handle is a `u64`: it carries a
pointer on x86-64 and aarch64, it cannot be dereferenced from Aelys, and
it costs no grammar. It is not exact on an ILP32 target.

A C `char *` argument is `&buf[0]` on a `[u8; N]` whose last byte is zero,
and the pointer+length pair of a C prototype is two parameters, `&T` and
an integer, never a slice. That form is only sound while the callee does
not keep the pointer: a `[u8; N]` local is stack memory, and a C library
that stores the address and reads it after the call reads a dead frame,
which has been measured to answer differently at `-O0` and at `-O1`.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0616",
        title: "an external declaration is not a value",
        explanation: "\
An external declaration names a symbol that lives outside this program.
Naming it anywhere but in the callee position of a call would build an
Aelys closure over it, which passes an environment pointer the foreign
function never expects and calls it under the wrong convention.

Call it directly. Aelys has no spelling for a C function pointer, so
there is no type an external declaration could be held under.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0617",
        title: "an external call requires an `unsafe` block",
        explanation: "\
An external declaration is a promise about code the compiler cannot see.
Its `unsafe` is the binding author's claim that the signature matches the
symbol; it says nothing about any particular call site.

Every call of an external function must therefore be written inside an
`unsafe { }` block, which is the caller's own claim that the arguments,
the lifetimes and the aliasing the foreign code expects are honoured here.

The block is a gate on the call site, not a proof about its contents, and
it removes no effect: once the block is written, a `nogc` body calling a
managed external is still rejected by E0727. That second rejection is
decided after this one is cleared, so the two are never reported together.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0618",
        title: "a linked library claims a symbol the runtime links",
        explanation: "\
The Aelys runtime is a C archive. It defines five symbols of its own and
imports sixteen from libc, and the linker matches both against whatever
else lands on the link line.

The user libraries are placed after `-laelys-core-*`, so an archive
cannot capture the five the runtime defines. That same order exposes the
sixteen it imports: when the user archive is searched, `malloc` is still
undefined, so a member defining it is pulled in and the runtime's own
allocation calls go there.

After a successful link the compiler reads the defined symbols of the
executable and of the aelys-core archive. A reserved symbol defined in
the executable that the core does not define came from a requested
library, and the program is rejected here rather than left to crash.

The check covers what entered the executable statically, and nothing
else. `-lfoo` takes `libfoo.so` over `libfoo.a` when both sit in the
same directory, which is what a distribution package ships: the symbol
is then bound by the dynamic linker at load time, it is not defined in
the executable, and this check says nothing. The same library refused as
an archive is accepted as a shared object. Interposition by `LD_PRELOAD`
is outside the check for the same reason.

The check runs only when `-l` is given, and it needs `nm`. Without `nm`
it cannot conclude and it does not reject.",
        severity: Severity::Error,
    },
    DiagnosticInfo {
        code: "E0901",
        title: "backend error",
        explanation: "\
Something went wrong during LLVM code generation. This is a compiler
bug. Please open an issue with the source file that triggered it.",
        severity: Severity::Error,
    },
];

pub fn lookup(code: &str) -> Option<&'static DiagnosticInfo> {
    REGISTRY.iter().find(|info| info.code == code)
}

pub fn all_codes() -> &'static [DiagnosticInfo] {
    REGISTRY
}
