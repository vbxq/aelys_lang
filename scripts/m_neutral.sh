
set -uo pipefail

GREP=grep
[ -x /usr/bin/grep ] && GREP=/usr/bin/grep
export LC_ALL=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${TMPDIR:-/tmp}/m_neutral.$$"
BASE=""
HEAD_BIN=""
SELFTEST=0

while [ $# -gt 0 ]; do
    case "$1" in
        --base) BASE="$2"; shift 2 ;;
        --head) HEAD_BIN="$2"; shift 2 ;;
        --selftest) SELFTEST=1; shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

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


compile_leg() {
    local bin="$1" out="$2" src="$3" level="$4" name="$5"
    local work="$out/$name.O$level"
    mkdir -p "$work"
    cp "$src" "$work/prog.aelys"
    "$bin" compile "-O$level" --emit-llvm-ir "$work/prog.aelys" > "$work/text" 2>&1
    echo "$?" > "$work/exit"
}

IDENTICAL=0; IR_DIFF=0; VERDICT_FLIP=0; BOTH_REJECT=0; TEXT_DIFF=0
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
            local be he
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
                    FAILED_LEGS="$FAILED_LEGS ir:$name.O$level"; continue
                fi
                IDENTICAL=$((IDENTICAL + 1))
            else
                REJECT_LEGS=$((REJECT_LEGS + 1))
                BOTH_REJECT=$((BOTH_REJECT + 1))
                sed "s#$WORK/base/##" < "$b/text" > "$b/text.norm"
                sed "s#$WORK/head/##" < "$h/text" > "$h/text.norm"
                if ! diff -q "$b/text.norm" "$h/text.norm" > /dev/null; then
                    TEXT_DIFF=$((TEXT_DIFF + 1)); FAILED_LEGS="$FAILED_LEGS text:$name.O$level"
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

selftest() {
    local fails=0
    local d="$WORK/st"
    mkdir -p "$d/base/p.O0" "$d/head/p.O0"
    plant_leg() {
        printf '0' > "$d/base/p.O0/exit"; printf '0' > "$d/head/p.O0/exit"
        printf 'ir\n' > "$d/base/p.O0/prog.ll"; printf 'ir\n' > "$d/head/p.O0/prog.ll"
        : > "$d/base/p.O0/text"; : > "$d/head/p.O0/text"
    }
    plant_leg; printf 'ir changed\n' > "$d/head/p.O0/prog.ll"
    if diff -q "$d/base/p.O0/prog.ll" "$d/head/p.O0/prog.ll" > /dev/null; then
        echo "SELFTEST leg IR read clean"; fails=1
    else echo "SELFTEST leg IR detected"; fi
    plant_leg; printf '1' > "$d/head/p.O0/exit"
    if [ "$(cat "$d/base/p.O0/exit")" = "$(cat "$d/head/p.O0/exit")" ]; then
        echo "SELFTEST leg VERDICT read clean"; fails=1
    else echo "SELFTEST leg VERDICT detected"; fi
    plant_leg; rm -f "$d/head/p.O0/prog.ll"
    if [ -f "$d/head/p.O0/prog.ll" ]; then
        echo "SELFTEST leg UNCLASSIFIED read clean"; fails=1
    else echo "SELFTEST leg UNCLASSIFIED detected"; fi
    if assert_accounting selftest "$((EXPECT_LEGS - 1))" "$EXPECT_LEGS" >/dev/null; then
        echo "SELFTEST leg ACCOUNTING read clean"; fails=1
    else echo "SELFTEST leg ACCOUNTING detected"; fi
    assert_accounting selftest "$EXPECT_LEGS" "$EXPECT_LEGS" >/dev/null || {
        echo "SELFTEST leg ACCOUNTING rejects a correct sum"; fails=1; }
    return $fails
}

if [ "$SELFTEST" -eq 1 ]; then
    selftest
    st=$?
    if [ "$st" -ne 0 ]; then
        echo "SELFTEST: FAILED"
        exit 1
    fi
    echo "SELFTEST: 4 legs, 4 detected"
    exit 0
fi

if [ -z "$BASE" ] || [ -z "$HEAD_BIN" ]; then
    echo "both --base and --head are required (or --selftest)" >&2
    exit 2
fi
if [ ! -x "$BASE" ] || [ ! -x "$HEAD_BIN" ]; then
    echo "base or head binary is not executable" >&2
    exit 2
fi

run_matrix

PRIMARY_SUM=$((IDENTICAL + IR_DIFF + VERDICT_FLIP + BOTH_REJECT + UNCLASSIFIED))

echo "root            $ROOT"
echo "programs        $programs (expected $EXPECT_PROGRAMS)"
echo "legs            $LEGS (expected $EXPECT_LEGS)"
echo "accept legs     $ACCEPT_LEGS"
echo "reject legs     $REJECT_LEGS"
echo "IDENTICAL       $IDENTICAL"
echo "IR_DIFF         $IR_DIFF"
echo "VERDICT_FLIP    $VERDICT_FLIP"
echo "BOTH_REJECT     $BOTH_REJECT"
echo "TEXT_DIFF       $TEXT_DIFF"
echo "UNCLASSIFIED    $UNCLASSIFIED"
echo "accounting      primary $PRIMARY_SUM/$LEGS"

status=0
[ "$origin_fail" -eq 0 ] || status=1
[ "$LEGS" -eq "$EXPECT_LEGS" ] || { echo "LEGS magnitude control failed"; status=1; }
[ "$ACCEPT_LEGS" -ge 96 ] || { echo "accept legs below the expected magnitude"; status=1; }
[ "$REJECT_LEGS" -ge 32 ] || { echo "reject legs below the expected magnitude"; status=1; }
[ "$IR_DIFF" -eq 0 ] || status=1
[ "$VERDICT_FLIP" -eq 0 ] || status=1
[ "$TEXT_DIFF" -eq 0 ] || status=1
[ "$UNCLASSIFIED" -eq 0 ] || status=1
assert_accounting primary "$PRIMARY_SUM" "$LEGS" || status=1

if [ "$status" -ne 0 ]; then
    echo "FAILED LEGS:$FAILED_LEGS"
    echo "M-NEUTRAL: FAILED"
else
    echo "M-NEUTRAL: OK"
fi
exit $status
