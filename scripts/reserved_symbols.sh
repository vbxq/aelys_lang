#!/bin/bash
# re-runnable evidence for E0428: which names the compiler and the runtime already own, and how
# many tracked declarations the `__` rule refuses.
#
# usage: scripts/reserved_symbols.sh            # enumerate + check the controls
#        scripts/reserved_symbols.sh sites      # enumeration only
set -u
# locale-pinned, and every multi-path list is written out rather than held in a variable: inline
# shells here do not word-split unquoted variables and a silently empty scope reads as a clean run
export LC_ALL=C

ROOT="${AELYS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$ROOT" || exit 2

RS_DIRS="air/src codegen/src driver/src core/src sema/src common/src frontend/src opt/src"

sites() {
    # S-A literals, S-B synthesised names, S-C the C runtime's own exports
    { grep -rhoE '"__[A-Za-z0-9_]+"' --include='*.rs' $RS_DIRS | tr -d '"'
      grep -rhoE 'format!\("__[A-Za-z0-9_]*' --include='*.rs' $RS_DIRS | sed 's/format!("//'
      grep -rhoE '__aelys_[a-z0-9_]+' --include='*.c' --include='*.h' core
    } | sort -u
}

if [ "${1:-all}" = sites ]; then
    sites
    exit 0
fi

SITES=$(sites)
TOTAL=$(printf '%s\n' "$SITES" | grep -c .)
UNPREFIXED=$(printf '%s\n' "$SITES" | grep -cv '^__')

# an enumeration that lost the two entry symbols enumerated nothing
for want in __aelys_main __aelys_user_main; do
    printf '%s\n' "$SITES" | grep -qx "$want" || {
        echo "reserved_symbols.sh: enumeration is void, $want missing" >&2
        exit 3
    }
done

# gnu grep, not the ugrep wrapper: a recursive search from this root cannot see the gitignored
# corpora otherwise, and the control below would read 0 for the wrong reason
GREP=/usr/bin/grep
[ -x "$GREP" ] || GREP=grep

DECLS=$(find . -path ./target -prune -o -name '*.aelys' -print0 \
        | xargs -0 "$GREP" -lE '^[[:space:]]*(nogc[[:space:]]+)?fn[[:space:]]+__' 2>/dev/null | wc -l)
# scoped to the suites, because the registry's --explain text quotes `fn __*` on purpose
RUST_DECLS=$(find aelys/tests driver/tests cli/tests -name '*.rs' -print0 \
        | xargs -0 "$GREP" -lE '(nogc[[:space:]]+)?fn[[:space:]]+__[A-Za-z0-9_]*\(' 2>/dev/null | wc -l)
RUST_FILES=$(find aelys/tests driver/tests cli/tests -name '*.rs' -print | wc -l)
CORPUS=$(find . -path ./target -prune -o -name '*.aelys' -print | wc -l)
MAIN=$(find . -path ./target -prune -o -name '*.aelys' -print0 \
        | xargs -0 "$GREP" -lE '^[[:space:]]*fn[[:space:]]+main' 2>/dev/null | wc -l)

echo "reserved names and prefixes enumerated : $TOTAL"
echo "of those NOT starting with __          : $UNPREFIXED   (must be 0)"
echo ".aelys files in the tracked tree       : $CORPUS"
echo ".aelys declaring fn main               : $MAIN         (magnitude control, must be >400)"
echo ".aelys declaring fn __*                : $DECLS        (over-rejection, must be 0)"
echo "suite .rs files scanned                : $RUST_FILES   (magnitude control, must be >70)"
echo "suite .rs declaring an aelys fn __*    : $RUST_DECLS    (over-rejection, must be 0)"

rc=0
[ "$UNPREFIXED" -eq 0 ] || { echo "FAIL: a reserved name lies outside the __ namespace" >&2; rc=1; }
[ "$MAIN" -gt 400 ] || { echo "FAIL: control fired below magnitude, the scan saw almost nothing" >&2; rc=1; }
[ "$DECLS" -eq 0 ] || { echo "FAIL: a tracked .aelys declares fn __*" >&2; rc=1; }
[ "$RUST_FILES" -gt 70 ] || { echo "FAIL: suite scan fired below magnitude" >&2; rc=1; }
[ "$RUST_DECLS" -eq 0 ] || { echo "FAIL: a suite .rs embeds an aelys fn __*" >&2; rc=1; }
exit $rc
