#!/usr/bin/env bash
set -uo pipefail

GREP=grep
[ -x /usr/bin/grep ] && GREP=/usr/bin/grep
export LC_ALL=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SELF="$ROOT/scripts/$(basename "${BASH_SOURCE[0]}")"
WORK="${TMPDIR:-/tmp}/m_neutral.$$"
CHECKOUT_CLI="$ROOT/target/release/aelys-cli"
ALLOWLIST="$ROOT/scripts/ir_neutrality_allowlist.tsv"
EXPECT_ALLOWLIST_ROWS=0
PLANT_LEG="o5_a_rc_arg.O0"
BASE=""
HEAD_BIN=""
SELFTEST=0
SELF_MODE=0

usage() {
    echo "usage: m_neutral.sh --base BIN --head BIN | --self | --selftest [--head BIN]" >&2
    echo "       [--allowlist PATH --expect-allowlist-rows N]" >&2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --base) BASE="$2"; shift 2 ;;
        --head) HEAD_BIN="$2"; shift 2 ;;
        --self) SELF_MODE=1; shift ;;
        --selftest) SELFTEST=1; shift ;;
        --allowlist) ALLOWLIST="$2"; shift 2 ;;
        --expect-allowlist-rows) EXPECT_ALLOWLIST_ROWS="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; usage; exit 2 ;;
    esac
done

# self mode is a regression detector for emission nondeterminism, it cannot see a runtime one since no leg here runs a program
if [ "$SELF_MODE" -eq 1 ]; then
    if [ "$SELFTEST" -eq 1 ] || [ -n "$BASE" ] || [ -n "$HEAD_BIN" ]; then
        echo "--self takes no binary, it is the binary of this checkout on both sides" >&2
        usage
        exit 2
    fi
    BASE="$CHECKOUT_CLI"
    HEAD_BIN="$CHECKOUT_CLI"
fi

mkdir -p "$WORK/corpus" "$WORK/base" "$WORK/head"
trap 'rm -rf "$WORK"' EXIT


emit() { cat > "$WORK/corpus/$1.aelys"; }

emit o1_a_vec_slice_write <<'EOF'
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    s[0] = 9
    return s[0]
}
EOF
emit o1_b_vec_slice_read <<'EOF'
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    println(s[0])
    return 0
}
EOF
emit o1_c_vec_reslice_write <<'EOF'
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    let t = s[..]
    t[0] = 9
    return t[0]
}
EOF
emit o1_d_vec_slice_copy_write <<'EOF'
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    let t = s
    t[0] = 9
    return t[0]
}
EOF

emit o2_a_vec_elem_ref <<'EOF'
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let r = &v[0]
    return *r
}
EOF
emit o2_b_vec_elem_ref_read <<'EOF'
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let r = &v[1]
    println(*r)
    return 0
}
EOF
emit o2_c_vec_elem_ref_conflict <<'EOF'
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1,2,3]
    let r = &v[0]
    v = vec[4,5,6]
    return *r
}
EOF
emit o2_d_whole_vec_ref <<'EOF'
fn rd(r: &Vec<i64>) -> i64 { return (*r)[0] }
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    println(rd(&v))
    return 0
}
EOF

emit o3_a_reborrow_write <<'EOF'
fn get(r: &Vec<i64>) -> i64 {
    let s = (*r)[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return 0 }
EOF
emit o3_b_reborrow_read <<'EOF'
fn get(r: &Vec<i64>) -> i64 {
    let s = (*r)[..]
    return s[0]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    println(get(&v))
    return 0
}
EOF
emit o3_c_reborrow_forwarded <<'EOF'
fn inner(r: &Vec<i64>) -> i64 { return (*r)[0] }
fn outer(r: &Vec<i64>) -> i64 { return inner(r) }
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    println(outer(&v))
    return 0
}
EOF
emit o3_d_reborrow_of_a_ref_local <<'EOF'
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let r = &v
    let q = &*r
    println((*q)[0])
    return 0
}
EOF

emit o4_a_slice_param_write <<'EOF'
fn wr(s: &[i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    println(wr(s))
    return 0
}
EOF
emit o4_b_vec_slice_to_writer <<'EOF'
fn wr(s: &[i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    return wr(s)
}
EOF
emit o4_c_vec_slice_to_reader <<'EOF'
fn rd(s: &[i64]) -> i64 { return s[0] }
fn main() -> i64 {
    let v: Vec<i64> = vec[1,2,3]
    let s = v[..]
    println(rd(s))
    return 0
}
EOF
emit o4_d_slice_param_recursion <<'EOF'
fn rec(s: &[i64], n: i64) -> i64 {
    if n == 0 {
        s[0] = 9
        return s[0]
    }
    return rec(s, n - 1)
}
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    println(rec(s, 2))
    return 0
}
EOF

emit o5_a_rc_arg <<'EOF'
fn take(p: &Rc<i64>) -> i64 { return 0 }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let q = take(&r)
    println(q)
    return 0
}
EOF
emit o5_b_rc_two_borrows <<'EOF'
fn take(p: &Rc<i64>) -> i64 { return 0 }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let a = take(&r)
    let b = take(&r)
    println(a + b)
    return 0
}
EOF
emit o5_c_rc_get <<'EOF'
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(7)
    let p = &r
    return Rc::get(r)
}
EOF
emit o5_d_rc_through_a_chain <<'EOF'
fn inner(p: &Rc<i64>) -> i64 { return 0 }
fn outer(p: &Rc<i64>) -> i64 { return inner(p) }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(5)
    println(outer(&r))
    return 0
}
EOF

emit o6_a_array_slice_write <<'EOF'
fn main() -> i64 {
    let a: [i64;3] = [1,2,3]
    let s = a[..]
    s[0] = 9
    return s[0]
}
EOF
emit o6_b_scalar_ref <<'EOF'
fn main() -> i64 {
    let x: i64 = 1
    let r = &x
    return *r
}
EOF
emit o6_c_array_conflict <<'EOF'
fn main() -> i64 {
    let mut a: [i64;3] = [1,2,3]
    let s = a[..]
    a = [4,5,6]
    return s[0]
}
EOF
emit o6_d_scalar_conflict <<'EOF'
fn main() -> i64 {
    let mut x: i64 = 1
    let r = &x
    x = 5
    return *r
}
EOF

emit o7_a_struct_rc_field_slice <<'EOF'
struct S { r: Rc<i64>, a: [i64;3] }
fn main() -> i64 {
    let x: S = S { r: Rc::new(5), a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
EOF
emit o7_b_struct_slice_to_writer <<'EOF'
struct S { r: Rc<i64>, a: [i64;3] }
fn w(s: &[i64]) -> i64 {
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let x: S = S { r: Rc::new(5), a: [4,5,6] }
    let s = x.a[..]
    println(w(s))
    return 0
}
EOF
emit o7_c_nested_struct <<'EOF'
struct Inner { r: Rc<i64> }
struct Outer { i: Inner, a: [i64;3] }
fn main() -> i64 {
    let x: Outer = Outer { i: Inner { r: Rc::new(5) }, a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
EOF
emit o7_d_struct_by_value_param <<'EOF'
struct S { r: Rc<i64>, a: [i64;3] }
fn f(x: S) -> i64 {
    let s = x.a[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 {
    let x: S = S { r: Rc::new(5), a: [4,5,6] }
    println(f(x))
    return 0
}
EOF
emit o7_e_struct_field_reslice <<'EOF'
struct S { r: Rc<i64>, a: [i64;3] }
fn main() -> i64 {
    let x: S = S { r: Rc::new(5), a: [4,5,6] }
    let s = x.a[..]
    let t = s[..]
    t[0] = 9
    println(t[0])
    return 0
}
EOF

emit o8_a_indirect_rc <<'EOF'
struct S { a: [i64;3] }
fn pick(p: &Rc<i64>, x: &S) -> &S { return x }
fn go(g: fn(&Rc<i64>, &S) -> &S) -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let x: S = S { a: [4,5,6] }
    let w = g(&r, &x)
    let s = (*w).a[..]
    s[0] = 9
    println(s[0])
    return 0
}
fn main() -> i64 { return go(pick) }
EOF
emit o8_b_indirect_vec <<'EOF'
struct S { a: [i64;3] }
fn pick(p: &Vec<i64>, x: &S) -> &S { return x }
fn go(g: fn(&Vec<i64>, &S) -> &S) -> i64 {
    let r: Vec<i64> = vec[5,6,7]
    let x: S = S { a: [4,5,6] }
    let w = g(&r, &x)
    let s = (*w).a[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return go(pick) }
EOF

emit o9_a_field_slice_direct <<'EOF'
struct S { r: Rc<i64>, a: [i64;3] }
fn main() -> i64 {
    let x: S = S { r: Rc::new(5), a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
EOF
emit o9_b_field_slice_through_a_ref <<'EOF'
struct S { r: Rc<i64>, a: [i64;3] }
fn f(w: &S) -> i64 {
    let s = (*w).a[..]
    s[0] = 9
    println(s[0])
    return 0
}
fn main() -> i64 {
    let x: S = S { r: Rc::new(5), a: [4,5,6] }
    return f(&x)
}
EOF

emit o10_a_known_over_rejection <<'EOF'
struct S { a: [i64;3] }
fn pick(p: &Vec<i64>, x: &S) -> &S { return x }
fn go(g: fn(&Vec<i64>, &S) -> &S) -> i64 {
    let r: Vec<i64> = vec[5,6,7]
    let x: S = S { a: [4,5,6] }
    let w = g(&r, &x)
    let s = (*w).a[..]
    s[0] = 9
    return s[0]
}
fn main() -> i64 { return go(pick) }
EOF
emit o10_b_plain_struct_control <<'EOF'
struct T { b: i64, a: [i64;3] }
fn main() -> i64 {
    let x: T = T { b: 1, a: [4,5,6] }
    let s = x.a[..]
    s[0] = 9
    println(s[0])
    return 0
}
EOF

emit o11_a_enum_rc <<'EOF'
enum E { A(Rc<i64>) }
fn f(e: &E) -> i64 { return 0 }
fn main() -> i64 {
    let e: E = E::A(Rc::new(7))
    let q = f(&e)
    println(q)
    return 0
}
EOF
emit o11_b_enum_plain <<'EOF'
enum E { A(i64), B(i64) }
fn f(e: &E) -> i64 { return 0 }
fn main() -> i64 {
    let e: E = E::A(1)
    let q = f(&e)
    println(q)
    return 0
}
EOF
emit o11_c_array_of_managed_struct <<'EOF'
struct S { r: Rc<i64> }
fn f(a: [S;2]) -> i64 {
    let p = &a
    return 0
}
fn main() -> i64 { return 0 }
EOF
emit o11_d_peeled_ref_no_field <<'EOF'
struct S { r: Rc<i64>, a: [i64;3] }
fn f(w: &S) -> i64 {
    let q = &*w
    return 0
}
fn main() -> i64 {
    let x: S = S { r: Rc::new(5), a: [4,5,6] }
    return f(&x)
}
EOF


declare -A EXPECT_COUNT=( [o1]=4 [o2]=4 [o3]=4 [o4]=4 [o5]=4 [o6]=4 [o7]=5 [o8]=2 [o9]=2 [o10]=2 [o11]=4 )
EXPECT_PROGRAMS=39
EXPECT_LEGS=156
# 16 of the 39 programs compile, the other 23 are rejected at every level
EXPECT_ACCEPT_LEGS=64
EXPECT_REJECT_LEGS=92

origin_fail=0
for o in "${!EXPECT_COUNT[@]}"; do
    got=$(ls "$WORK/corpus" | $GREP -c "^${o}_")
    if [ "$got" -ne "${EXPECT_COUNT[$o]}" ]; then
        echo "ORIGIN $o: expected ${EXPECT_COUNT[$o]} programs, found $got"
        origin_fail=1
    fi
done
programs=$(ls "$WORK/corpus"/*.aelys | wc -l)
if [ "$programs" -ne "$EXPECT_PROGRAMS" ]; then
    echo "PROGRAMS: expected $EXPECT_PROGRAMS, found $programs"
    origin_fail=1
fi


declare -A ALLOW_ROW=()
declare -A ALLOW_HIT=()
ALLOW_ROWS=0
ABSORBED=0
ALLOW_UNOBSERVED=0
UNOBSERVED_LINES=""

allow_key() { printf '%s\t%s\t%s' "$1" "$2" "$3"; }

load_allowlist() {
    local f l c r n=0 key
    if [ ! -f "$ALLOWLIST" ]; then
        echo "ALLOWLIST MISSING   $ALLOWLIST"
        return 1
    fi
    if [ ! -r "$ALLOWLIST" ]; then
        echo "ALLOWLIST UNREADABLE $ALLOWLIST"
        return 1
    fi
    while IFS=$'\t' read -r f l c r || [ -n "$f" ]; do
        case "$f" in ''|'#'*) continue ;; esac
        n=$((n + 1))
        case "$l" in
            O0|O1|O2|O3) ;;
            *) echo "ALLOWLIST PARSE     row $n: level \`$l\` is not spelled O0 O1 O2 or O3"; return 1 ;;
        esac
        case "$c" in
            ir|text) ;;
            *) echo "ALLOWLIST PARSE     row $n: class \`$c\` is neither ir nor text"; return 1 ;;
        esac
        if [ -z "${r//[[:space:]]/}" ]; then
            echo "ALLOWLIST PARSE     row $n: $f $l $c carries no reason"
            return 1
        fi
        key="$(allow_key "$f" "$l" "$c")"
        if [ -n "${ALLOW_ROW[$key]:-}" ]; then
            echo "ALLOWLIST PARSE     row $n: $f $l $c is listed twice"
            return 1
        fi
        ALLOW_ROW[$key]=1
    done < "$ALLOWLIST"
    ALLOW_ROWS=$n
    if [ "$n" -ne "$EXPECT_ALLOWLIST_ROWS" ]; then
        echo "ALLOWLIST ROWS      $ALLOWLIST holds $n rows, expected $EXPECT_ALLOWLIST_ROWS"
        return 1
    fi
    return 0
}

sweep_unobserved() {
    [ "${#ALLOW_ROW[@]}" -gt 0 ] || return 0
    local key
    for key in "${!ALLOW_ROW[@]}"; do
        [ -z "${ALLOW_HIT[$key]:-}" ] || continue
        ALLOW_UNOBSERVED=$((ALLOW_UNOBSERVED + 1))
        UNOBSERVED_LINES="${UNOBSERVED_LINES}ALLOWLIST UNOBSERVED $(printf '%s' "$key" | tr '\t' ' ')"$'\n'
    done
}

compile_leg() {
    local bin="$1" out="$2" src="$3" level="$4" name="$5"
    local work="$out/$name.O$level"
    mkdir -p "$work"
    cp "$src" "$work/prog.aelys"
    "$bin" compile "-O$level" --emit-llvm-ir "$work/prog.aelys" > "$work/text" 2>&1
    echo "$?" > "$work/exit"
}

IDENTICAL=0; IR_DIFF=0; VERDICT_FLIP=0; BOTH_REJECT=0; TEXT_DIFF=0
IR_DIFF_NEW=0; TEXT_DIFF_NEW=0
UNCLASSIFIED=0; LEGS=0
ACCEPT_LEGS=0; REJECT_LEGS=0
FAILED_LEGS=""

run_matrix() {
    for src in "$WORK/corpus"/*.aelys; do
        name="$(basename "$src" .aelys)"
        for level in 0 1 2 3; do
            LEGS=$((LEGS + 1))
            compile_leg "$BASE" "$WORK/base" "$src" "$level" "$name"
            compile_leg "$HEAD_BIN" "$WORK/head" "$src" "$level" "$name"
            local b="$WORK/base/$name.O$level" h="$WORK/head/$name.O$level"
            local be he key
            be=$(cat "$b/exit"); he=$(cat "$h/exit")

            if [ "$be" != "0" ] && [ "$be" != "1" ]; then
                UNCLASSIFIED=$((UNCLASSIFIED + 1))
                FAILED_LEGS="$FAILED_LEGS base-exit:$name.O$level"; continue
            fi
            if [ "$he" != "0" ] && [ "$he" != "1" ]; then
                UNCLASSIFIED=$((UNCLASSIFIED + 1))
                FAILED_LEGS="$FAILED_LEGS head-exit:$name.O$level"; continue
            fi
            if [ "$be" != "$he" ]; then
                VERDICT_FLIP=$((VERDICT_FLIP + 1))
                FAILED_LEGS="$FAILED_LEGS flip:$name.O$level"; continue
            fi

            if [ "$he" = "0" ]; then
                ACCEPT_LEGS=$((ACCEPT_LEGS + 1))
                if [ ! -f "$b/prog.ll" ] || [ ! -f "$h/prog.ll" ]; then
                    UNCLASSIFIED=$((UNCLASSIFIED + 1))
                    FAILED_LEGS="$FAILED_LEGS no-ir:$name.O$level"; continue
                fi
                if ! diff -q "$b/prog.ll" "$h/prog.ll" > /dev/null; then
                    IR_DIFF=$((IR_DIFF + 1))
                    key="$(allow_key "$name" "O$level" ir)"
                    if [ -n "${ALLOW_ROW[$key]:-}" ]; then
                        ALLOW_HIT[$key]=1
                        ABSORBED=$((ABSORBED + 1))
                    else
                        IR_DIFF_NEW=$((IR_DIFF_NEW + 1))
                        FAILED_LEGS="$FAILED_LEGS ir:$name.O$level"
                    fi
                    continue
                fi
                IDENTICAL=$((IDENTICAL + 1))
            else
                REJECT_LEGS=$((REJECT_LEGS + 1))
                BOTH_REJECT=$((BOTH_REJECT + 1))
                sed "s#$WORK/base/##" < "$b/text" > "$b/text.norm"
                sed "s#$WORK/head/##" < "$h/text" > "$h/text.norm"
                if ! diff -q "$b/text.norm" "$h/text.norm" > /dev/null; then
                    TEXT_DIFF=$((TEXT_DIFF + 1))
                    key="$(allow_key "$name" "O$level" text)"
                    if [ -n "${ALLOW_ROW[$key]:-}" ]; then
                        ALLOW_HIT[$key]=1
                        ABSORBED=$((ABSORBED + 1))
                    else
                        TEXT_DIFF_NEW=$((TEXT_DIFF_NEW + 1))
                        FAILED_LEGS="$FAILED_LEGS text:$name.O$level"
                    fi
                fi
            fi

        done
    done
}


assert_accounting() {
    if [ "$2" -eq "$3" ]; then return 0; fi
    echo "$1 accounting $2 != $3"
    return 1
}

ST_LEGS=0
ST_ASSERTS=0
ST_MET=0

st_assert() {
    local label="$1" want="$2" body="$3"
    ST_ASSERTS=$((ST_ASSERTS + 1))
    if $GREP -qxF -- "$want" <<< "$body"; then
        ST_MET=$((ST_MET + 1))
        echo "SELFTEST $label: reads \`$want\`"
        return 0
    fi
    echo "SELFTEST $label: expected \`$want\`, absent from the run"
    return 1
}

selftest() {
    local fails=0
    local cli="$HEAD_BIN"
    [ -n "$cli" ] || cli="$CHECKOUT_CLI"
    if [ ! -x "$cli" ]; then
        echo "SELFTEST: no compiler to drive at $cli"
        return 1
    fi

    local d="$WORK/st"
    mkdir -p "$d"
    local wrap="$d/planting-cli"
    # the wrapper keys on the leg directory because compile_leg always names the source prog.aelys
    cat > "$wrap" <<EOF
#!/usr/bin/env bash
"$cli" "\$@"
rc=\$?
src="\${@: -1}"
d="\$(dirname "\$src")"
if [ "\$(basename "\$d")" = "$PLANT_LEG" ] && [ -f "\$d/prog.ll" ]; then
    printf '; planted divergence\n' >> "\$d/prog.ll"
fi
exit \$rc
EOF
    chmod 755 "$wrap"

    local listed="$d/allowlist_listed.tsv" empty="$d/allowlist_empty.tsv"
    {
        echo "# the planted divergence, listed"
        printf '%s\tO0\tir\tplanted by the selftest wrapper\n' "${PLANT_LEG%.O0}"
    } > "$listed"
    echo "# no rows" > "$empty"

    local out

    ST_LEGS=$((ST_LEGS + 1))
    out="$("$SELF" --base "$cli" --head "$wrap" --allowlist "$listed" --expect-allowlist-rows 1 2>&1)"
    st_assert "a listed-divergence IR_DIFF" "IR_DIFF         1" "$out" || fails=1
    st_assert "a listed-divergence IR_DIFF_NEW" "IR_DIFF_NEW     0" "$out" || fails=1
    st_assert "a listed-divergence absorption" "allowlist       rows 1, absorbed 1, unobserved 0" "$out" || fails=1
    st_assert "a listed-divergence verdict" "M-NEUTRAL: OK" "$out" || fails=1

    ST_LEGS=$((ST_LEGS + 1))
    out="$("$SELF" --base "$cli" --head "$wrap" --allowlist "$empty" --expect-allowlist-rows 0 2>&1)"
    st_assert "an unlisted-divergence IR_DIFF_NEW" "IR_DIFF_NEW     1" "$out" || fails=1
    st_assert "an unlisted-divergence failed leg" "FAILED LEGS: ir:$PLANT_LEG" "$out" || fails=1
    st_assert "an unlisted-divergence verdict" "M-NEUTRAL: FAILED" "$out" || fails=1

    ST_LEGS=$((ST_LEGS + 1))
    out="$("$SELF" --base "$cli" --head "$cli" --allowlist "$listed" --expect-allowlist-rows 1 2>&1)"
    st_assert "an unobserved-row report" "ALLOWLIST UNOBSERVED ${PLANT_LEG%.O0} O0 ir" "$out" || fails=1
    st_assert "an unobserved-row count" "allowlist       rows 1, absorbed 0, unobserved 1" "$out" || fails=1
    st_assert "an unobserved-row verdict" "M-NEUTRAL: FAILED" "$out" || fails=1

    return $fails
}

if [ "$SELFTEST" -eq 1 ]; then
    selftest
    st=$?
    echo "SELFTEST: $ST_LEGS legs, $ST_ASSERTS assertions, $ST_MET met"
    if [ "$st" -ne 0 ]; then
        echo "SELFTEST: FAILED"
        exit 1
    fi
    echo "SELFTEST: OK"
    exit 0
fi

if [ -z "$BASE" ] || [ -z "$HEAD_BIN" ]; then
    echo "both --base and --head are required (or --self, or --selftest)" >&2
    usage
    exit 2
fi
if [ ! -x "$BASE" ] || [ ! -x "$HEAD_BIN" ]; then
    echo "base or head binary is not executable" >&2
    exit 2
fi

if ! load_allowlist; then
    echo "M-NEUTRAL: FAILED"
    exit 1
fi

run_matrix
sweep_unobserved

PRIMARY_SUM=$((IDENTICAL + IR_DIFF + VERDICT_FLIP + BOTH_REJECT + UNCLASSIFIED))

echo "root            $ROOT"
echo "base            $BASE"
echo "head            $HEAD_BIN"
echo "programs        $programs (expected $EXPECT_PROGRAMS)"
echo "legs            $LEGS (expected $EXPECT_LEGS)"
echo "accept legs     $ACCEPT_LEGS (expected $EXPECT_ACCEPT_LEGS)"
echo "reject legs     $REJECT_LEGS (expected $EXPECT_REJECT_LEGS)"
echo "IDENTICAL       $IDENTICAL"
echo "IR_DIFF         $IR_DIFF"
echo "IR_DIFF_NEW     $IR_DIFF_NEW"
echo "VERDICT_FLIP    $VERDICT_FLIP"
echo "BOTH_REJECT     $BOTH_REJECT"
echo "TEXT_DIFF       $TEXT_DIFF"
echo "TEXT_DIFF_NEW   $TEXT_DIFF_NEW"
echo "UNCLASSIFIED    $UNCLASSIFIED"
echo "allowlist       rows $ALLOW_ROWS, absorbed $ABSORBED, unobserved $ALLOW_UNOBSERVED"
echo "accounting      primary $PRIMARY_SUM/$LEGS"
[ -z "$UNOBSERVED_LINES" ] || printf '%s' "$UNOBSERVED_LINES"

status=0
[ "$origin_fail" -eq 0 ] || status=1
[ "$LEGS" -eq "$EXPECT_LEGS" ] || { echo "LEGS magnitude control failed"; status=1; }
[ "$ACCEPT_LEGS" -eq "$EXPECT_ACCEPT_LEGS" ] || { echo "accept legs $ACCEPT_LEGS, expected exactly $EXPECT_ACCEPT_LEGS"; status=1; }
[ "$REJECT_LEGS" -eq "$EXPECT_REJECT_LEGS" ] || { echo "reject legs $REJECT_LEGS, expected exactly $EXPECT_REJECT_LEGS"; status=1; }
[ "$((ACCEPT_LEGS + REJECT_LEGS))" -eq "$EXPECT_LEGS" ] || { echo "accept plus reject $((ACCEPT_LEGS + REJECT_LEGS)), expected exactly $EXPECT_LEGS"; status=1; }
[ "$IR_DIFF_NEW" -eq 0 ] || status=1
[ "$VERDICT_FLIP" -eq 0 ] || status=1
[ "$TEXT_DIFF_NEW" -eq 0 ] || status=1
[ "$UNCLASSIFIED" -eq 0 ] || status=1
[ "$ALLOW_UNOBSERVED" -eq 0 ] || status=1
assert_accounting primary "$PRIMARY_SUM" "$LEGS" || status=1

if [ "$status" -ne 0 ]; then
    echo "FAILED LEGS:$FAILED_LEGS"
    echo "M-NEUTRAL: FAILED"
else
    echo "M-NEUTRAL: OK"
fi
exit $status
