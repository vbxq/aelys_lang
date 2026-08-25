# # actually emits lands inside the reserved namespace.
set -u
export LC_ALL=C

ROOT="${AELYS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$ROOT" || exit 2

RS_DIRS="air/src codegen/src driver/src core/src sema/src common/src frontend/src opt/src"

# # gnu grep, not the ugrep wrapper: a recursive search from this root cannot see the gitignored
GREP=/usr/bin/grep
[ -x "$GREP" ] || GREP=grep

WITNESS_SUITE=aelys/tests/semantic_invariants_tests.rs

sites() {
    { grep -rhoE '"__[A-Za-z0-9_]+"' --include='*.rs' $RS_DIRS | tr -d '"'
      grep -rhoE 'format!\("__[A-Za-z0-9_]*' --include='*.rs' $RS_DIRS | sed 's/format!("//'
      grep -rhoE '__aelys_[a-z0-9_]+' --include='*.c' --include='*.h' core
    } | sort -u
}

# # the capturing `add` below never reaches
PROBE_SRC='struct Pair { a: i64, b: i64 }
let twice = fn(x: i64) -> i64 { return x * 2 }
fn ident<T>(x: T) -> T { return x }
fn helper(n: i64) -> i64 { return n + 1 }
fn main() -> i64 {
    let base = 10
    let add = fn(x: i64) -> i64 { return x + base }
    let p = Pair { a: 1, b: 2 }
    println(helper(ident(p.a)))
    println(add(ident(3)))
    println(twice(21))
    return 0
}'

EMIT_DIR=
cleanup() { [ -n "$EMIT_DIR" ] && rm -rf "$EMIT_DIR"; }
trap cleanup EXIT

emit_probe() {
    CLI="${AELYS_CLI:-$ROOT/target/release/aelys-cli}"
    if [ ! -x "$CLI" ]; then
        echo "reserved_symbols.sh: $CLI is missing, run cargo build --release first" >&2
        exit 2
    fi
    EMIT_DIR=$(mktemp -d) || exit 2
    printf '%s\n' "$PROBE_SRC" > "$EMIT_DIR/probe.aelys"
    if ! (cd "$EMIT_DIR" && "$CLI" compile probe.aelys --emit-llvm-ir > compile.log 2>&1); then
        echo "reserved_symbols.sh: the probe did not compile" >&2
        cat "$EMIT_DIR/compile.log" >&2
        exit 3
    fi
    "$GREP" -oE '^define[^@]*@[A-Za-z0-9_.$]+' "$EMIT_DIR/probe.ll" | sed 's/.*@//' | sort -u
}

case "${1:-all}" in
    sites)
        sites
        exit 0
        ;;
    emitted)
        emit_probe
        exit 0
        ;;
esac

SITES=$(sites)
TOTAL=$(printf '%s\n' "$SITES" | grep -c .)

for want in __aelys_main __aelys_user_main; do
    printf '%s\n' "$SITES" | grep -qx "$want" || {
        echo "reserved_symbols.sh: enumeration is void, $want missing" >&2
        exit 3
    }
done

EMITTED=$(emit_probe)
DECLARED=$(printf '%s\n' "$PROBE_SRC" | "$GREP" -oE '^fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' \
        | sed 's/^fn[[:space:]]*//' | sort -u)

USER_SYMS=0
RESERVED_SYMS=0
UNCLASSIFIED_SYMS=
LAMBDA_SYMS=0
HAVE_MONO=0
for sym in $EMITTED; do
    if printf '%s\n' "$DECLARED" | grep -qx "$sym"; then
        USER_SYMS=$((USER_SYMS + 1))
        continue
    fi
    case "$sym" in
        __*)
            RESERVED_SYMS=$((RESERVED_SYMS + 1))
            case "$sym" in
                __lambda_*) LAMBDA_SYMS=$((LAMBDA_SYMS + 1)) ;;
                __mono_*) HAVE_MONO=1 ;;
            esac
            ;;
        *) UNCLASSIFIED_SYMS="$UNCLASSIFIED_SYMS $sym" ;;
    esac
done
UNCLASSIFIED=$(printf '%s\n' $UNCLASSIFIED_SYMS | grep -c . )

TRACKED=$(git ls-files '*.aelys' | wc -l)
DECLS=$(find . -path ./target -prune -o -name '*.aelys' -print0 \
        | xargs -0 "$GREP" -lE '^[[:space:]]*(nogc[[:space:]]+)?fn[[:space:]]+__' 2>/dev/null | wc -l)
RUST_DECLS=$(find aelys/tests driver/tests cli/tests -name '*.rs' ! -path "$WITNESS_SUITE" -print0 \
        | xargs -0 "$GREP" -lE '(nogc[[:space:]]+)?fn[[:space:]]+__[A-Za-z0-9_]*\(' 2>/dev/null | wc -l)
WITNESS_DECLS=$("$GREP" -cE '(nogc[[:space:]]+)?fn[[:space:]]+__[A-Za-z0-9_]*\(' "$WITNESS_SUITE" 2>/dev/null)
RUST_FILES=$(git ls-files 'aelys/tests/*.rs' 'driver/tests/*.rs' 'cli/tests/*.rs' | wc -l)
CORPUS=$(find . -path ./target -prune -o -name '*.aelys' -print | wc -l)
MAIN=$(find . -path ./target -prune -o -name '*.aelys' -print0 \
        | xargs -0 "$GREP" -lE '^[[:space:]]*fn[[:space:]]+main' 2>/dev/null | wc -l)

echo "reserved names and prefixes enumerated       : $TOTAL"
echo "function symbols the probe module emits      : $((USER_SYMS + RESERVED_SYMS + UNCLASSIFIED))"
echo "  of those declared by the probe source      : $USER_SYMS   (magnitude control, must be >0)"
echo "  of those inside the reserved namespace     : $RESERVED_SYMS   (magnitude control, must be >=5, incl. one __mono_*)"
echo "  of those a __lambda_* symbol               : $LAMBDA_SYMS   (magnitude control, must be >=2, one per lambda naming site)"
echo "  unclassified                               : $UNCLASSIFIED   (must be 0)"
echo ".aelys files tracked by git                  : $TRACKED   (magnitude control, must be >0)"
echo ".aelys files present in this working tree    : $CORPUS   (the corpora are gitignored, so this is 1 in a fresh clone)"
echo ".aelys declaring fn main                     : $MAIN   (magnitude control, must be >0 and >400 once the corpora are present)"
echo ".aelys declaring fn __*                      : $DECLS   (over-rejection, must be 0)"
echo "suite .rs files tracked                      : $RUST_FILES   (magnitude control, must be >70)"
echo "suite .rs declaring an aelys fn __*          : $RUST_DECLS   (over-rejection, must be 0, witness suite excluded)"
echo "fn __* fixtures inside the witness suite     : $WITNESS_DECLS   (the exclusion is stale below 1)"

rc=0
[ "$UNCLASSIFIED" -eq 0 ] || {
    echo "FAIL: the compiler emits a function symbol outside the reserved namespace:$UNCLASSIFIED_SYMS" >&2
    rc=1
}
[ "$USER_SYMS" -gt 0 ] || { echo "FAIL: the probe module emitted no user symbol" >&2; rc=1; }
[ "$RESERVED_SYMS" -ge 5 ] || { echo "FAIL: the probe module fired below magnitude" >&2; rc=1; }
[ "$LAMBDA_SYMS" -ge 2 ] || {
    echo "FAIL: the probe reached fewer than both lambda naming sites, got $LAMBDA_SYMS" >&2
    rc=1
}
[ "$HAVE_MONO" -eq 1 ] || { echo "FAIL: the probe emitted no __mono_* symbol" >&2; rc=1; }
[ "$TRACKED" -gt 0 ] || { echo "FAIL: no tracked .aelys, the scan saw nothing" >&2; rc=1; }
[ "$MAIN" -gt 0 ] || { echo "FAIL: control fired below magnitude, the scan saw almost nothing" >&2; rc=1; }
[ "$CORPUS" -le 100 ] || [ "$MAIN" -gt 400 ] || {
    echo "FAIL: the corpora are present but the fn main control fired below magnitude" >&2
    rc=1
}
[ "$DECLS" -eq 0 ] || { echo "FAIL: a .aelys in the working tree declares fn __*" >&2; rc=1; }
[ "$RUST_FILES" -gt 70 ] || { echo "FAIL: suite scan fired below magnitude" >&2; rc=1; }
[ "$RUST_DECLS" -eq 0 ] || { echo "FAIL: a suite .rs outside the witness suite embeds an aelys fn __*" >&2; rc=1; }
[ "$WITNESS_DECLS" -ge 1 ] || { echo "FAIL: the witness suite exclusion is stale" >&2; rc=1; }
exit $rc
