#!/usr/bin/env bash
set -u

ROOT="${AELYS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
CLI="${AELYS_SWEEP_CLI:-$ROOT/target/release/aelys-cli}"
CC="${CC:-clang}"
AR="${AR:-ar}"
FLOOR=300
TIMEOUT_SECONDS=4
ASAN_TIMEOUT_SECONDS=8
SWEEP_TMP=

cleanup_sweep() {
    if [ -n "${SWEEP_TMP:-}" ]; then
        rm -rf "$SWEEP_TMP"
        SWEEP_TMP=
    fi
}

abspath() {
    local p="$1"
    [ -x "$p" ] || { printf 'place_addressing_sweep: not executable: %s\n' "$p" >&2; return 2; }
    (cd "$(dirname "$p")" && printf '%s/%s\n' "$(pwd)" "$(basename "$p")")
}

encode() {
    tr '\n' '~' < "$1"
}

write_source() {
    local kind="$1" seed="$2" delta="$3" alias="$4" mutation="$5" path="$6"
    local src op
    case "$mutation" in
        assign) op="%s = %s + %s" ;;
        compound) op="%s += %s" ;;
        postfix) op="%s++"; delta=1 ;;
    esac
    case "$kind" in
        scalar)
            if [ "$alias" -eq 0 ]; then
                printf -v src 'fn main() -> i64 {\n    let mut x: i64 = %s\n    %s\n    return x - %s\n}\n' "$seed" "$(printf "$op" x x "$delta")" "$((seed + delta))"
            else
                printf -v src 'fn main() -> i64 {\n    let mut x: i64 = %s\n    let keep = x\n    %s\n    return x - %s + keep - %s\n}\n' "$seed" "$(printf "$op" x x "$delta")" "$((seed + delta))" "$seed"
            fi
            ;;
        scalar_ptr|scalar_reborrow)
            local target='*r'
            local setup=''
            if [ "$kind" = scalar_reborrow ]; then
                target='*q'
                setup='    let q = &mut *r\n'
            fi
            printf -v src 'fn bump(r: &mut i64) -> i64 {\n%s    %s\n    return 0\n}\nfn main() -> i64 {\n    let mut x: i64 = %s\n    let q = bump(&mut x)\n    return x - %s\n}\n' "$setup" "$(printf "$op" "$target" "$target" "$delta")" "$seed" "$((seed + delta))"
            ;;
        struct)
            local target='c.f'
            if [ "$alias" -eq 1 ]; then
                printf -v src 'struct Cell { f: i64, g: i64 }\nfn main() -> i64 {\n    let mut c: Cell = Cell { f: %s, g: 0 }\n    let keep = c.g\n    %s\n    return c.f - %s + keep\n}\n' "$seed" "$(printf "$op" "$target" "$target" "$delta")" "$((seed + delta))"
            else
                printf -v src 'struct Cell { f: i64, g: i64 }\nfn bump(r: &mut Cell) -> i64 {\n    %s\n    return 0\n}\nfn main() -> i64 {\n    let mut c: Cell = Cell { f: %s, g: 0 }\n    let q = bump(&mut c)\n    return c.f - %s\n}\n' "$(printf "$op" '(*r).f' '(*r).f' "$delta")" "$seed" "$((seed + delta))"
            fi
            ;;
        array_slice)
            printf -v src 'fn main() -> i64 {\n    let mut a: [i64; 3] = [%s, 0, 0]\n    let s = a[..]\n    %s\n    return a[0] - %s\n}\n' "$seed" "$(printf "$op" 's[0]' 's[0]' "$delta")" "$((seed + delta))"
            ;;
        vec_elem)
            local target='(*r)[0]'
            local setup=''
            if [ "$alias" -eq 1 ]; then
                target='(*q)[0]'
                setup='    let q = &mut *r\n'
            fi
            printf -v src 'fn bump(r: &mut Vec<i64>) -> i64 {\n%s    %s\n    return 0\n}\nfn main() -> i64 {\n    let mut v: Vec<i64> = vec[%s, 0, 0]\n    let q = bump(&mut v)\n    return v[0] - %s\n}\n' "$setup" "$(printf "$op" "$target" "$target" "$delta")" "$seed" "$((seed + delta))"
            ;;
        vec_push)
            local target='(*r)'
            local setup=''
            if [ "$alias" -eq 1 ]; then
                target='(*q)'
                setup='    let q = &mut *r\n'
            fi
            printf -v src 'fn grow(r: &mut Vec<i64>) -> i64 {\n%s    Vec::push(%s, %s)\n    return 0\n}\nfn main() -> i64 {\n    let mut v: Vec<i64> = vec[%s]\n    let q = grow(&mut v)\n    return v[1] - %s\n}\n' "$setup" "$target" "$delta" "$seed" "$delta"
            ;;
        cow)
            printf -v src 'fn main() -> i64 {\n    let mut v: Vec<i64> = vec[%s, 0, 0]\n    let w = v\n    %s\n    return w[0] - %s\n}\n' "$seed" "$(printf "$op" 'v[0]' 'v[0]' "$delta")" "$seed"
            ;;
        rc_payload)
            printf -v src 'struct Cell { f: i64 }\nfn main() -> i64 {\n    let r = Rc::new(Cell { f: %s })\n    %s\n    return Rc::get(r).f - %s\n}\n' "$seed" "$(printf "$op" 'Rc::get(r).f' 'Rc::get(r).f' "$delta")" "$((seed + delta))"
            ;;
        global)
            if [ "$alias" -eq 0 ]; then
                printf -v src 'let mut g: i64 = %s\nfn main() -> i64 {\n    %s\n    return g - %s\n}\n' "$seed" "$(printf "$op" g g "$delta")" "$((seed + delta))"
            else
                printf -v src 'let mut g: i64 = %s\nfn main() -> i64 {\n    let r = &mut g\n    %s\n    return g - %s\n}\n' "$seed" "$(printf "$op" '*r' '*r' "$delta")" "$((seed + delta))"
            fi
            ;;
        shared_read)
            printf -v src 'fn peek(r: &i64) -> i64 {\n    return *r\n}\nfn main() -> i64 {\n    let x: i64 = %s\n    return peek(&x) - %s\n}\n' "$seed" "$seed"
            ;;
        slice_read)
            printf -v src 'fn peek(s: &[i64]) -> i64 {\n    let t = s[0..2]\n    return t[1]\n}\nfn main() -> i64 {\n    let a: [i64; 4] = [0, %s, 0, 0]\n    return peek(a[..]) - %s\n}\n' "$seed" "$seed"
            ;;
        closure)
            printf -v src 'fn main() -> i64 {\n    let x: i64 = %s\n    let f = fn () -> i64 { return x + %s }\n    return f() - %s\n}\n' "$seed" "$delta" "$((seed + delta))"
            ;;
        *)
            return 1
            ;;
    esac
    printf '%s' "$src" > "$path"
}

write_question_source() {
    local kind="$1" seed="$2" delta="$3" alias="$4" path="$5"
    local src
    if [ "$kind" = q_vec ]; then
        if [ "$alias" -eq 0 ]; then
            printf -v src 'enum Result<T, E> { Ok(T), Err(E) }\nfn step(r: &mut Vec<i64>) -> Result<i64, i64> {\n    Vec::push((*r), %s)\n    return Result::Ok(0)\n}\nfn chain(r: &mut Vec<i64>) -> Result<i64, i64> {\n    let q = step(r)?\n    return Result::Ok((*r)[1])\n}\nfn main() -> i64 {\n    let mut v: Vec<i64> = vec[%s]\n    return match chain(&mut v) { Result::Ok(n) => n - %s, Result::Err(e) => e }\n}\n' "$delta" "$seed" "$delta"
        else
            printf -v src 'enum Result<T, E> { Ok(T), Err(E) }\nfn step(r: &mut Vec<i64>) -> Result<i64, i64> {\n    let q = &mut *r\n    Vec::push((*q), %s)\n    return Result::Ok(0)\n}\nfn chain(r: &mut Vec<i64>) -> Result<i64, i64> {\n    let q = step(r)?\n    return Result::Ok((*r)[1])\n}\nfn main() -> i64 {\n    let mut v: Vec<i64> = vec[%s]\n    return match chain(&mut v) { Result::Ok(n) => n - %s, Result::Err(e) => e }\n}\n' "$delta" "$seed" "$delta"
        fi
    else
        if [ "$alias" -eq 0 ]; then
            printf -v src 'enum Result<T, E> { Ok(T), Err(E) }\nfn step(r: &mut i64) -> Result<i64, i64> {\n    *r = *r + %s\n    return Result::Ok(0)\n}\nfn chain(r: &mut i64) -> Result<i64, i64> {\n    let q = step(r)?\n    return Result::Ok(*r)\n}\nfn main() -> i64 {\n    let mut x: i64 = %s\n    return match chain(&mut x) { Result::Ok(n) => n - %s, Result::Err(e) => e }\n}\n' "$delta" "$seed" "$((seed + delta))"
        else
            printf -v src 'enum Result<T, E> { Ok(T), Err(E) }\nfn step(r: &mut i64) -> Result<i64, i64> {\n    let q = &mut *r\n    (*q)++\n    return Result::Ok(0)\n}\nfn chain(r: &mut i64) -> Result<i64, i64> {\n    let q = step(r)?\n    return Result::Ok(*r)\n}\nfn main() -> i64 {\n    let mut x: i64 = %s\n    return match chain(&mut x) { Result::Ok(n) => n - %s, Result::Err(e) => e }\n}\n' "$seed" "$((seed + 1))"
        fi
    fi
    printf '%s' "$src" > "$path"
}

build_asan_archive() {
    local out="$1" core="$ROOT/core/src"
    mkdir -p "$out"
    for unit in aelys_core_common.c aelys_alloc_immix.c aelys_rc_real.c; do
        "$CC" -fsanitize=address -g -c "$core/$unit" -I"$core" -o "$out/${unit%.c}.o" || return 1
    done
    "$AR" rcs "$out/libaelys-core-rc-asan.a" "$out"/*.o
}

run_binary() {
    local exe="$1" alloc="$2" out="$3" err="$4" rc
    timeout "$TIMEOUT_SECONDS" env AELYS_ALLOC="$alloc" AELYS_RC_STATS=1 "$exe" >"$out" 2>"$err"
    rc=$?
    printf '%s|%s\n' "$rc" "$(encode "$out")"
}

run_asan_binary() {
    local exe="$1" alloc="$2" out="$3" err="$4" rc
    timeout "$ASAN_TIMEOUT_SECONDS" env AELYS_ALLOC="$alloc" ASAN_OPTIONS=detect_leaks=1 "$exe" >"$out" 2>"$err"
    rc=$?
    if [ "$rc" -ne 0 ] || rg -q 'AddressSanitizer|LeakSanitizer' "$err"; then
        return 1
    fi
    return 0
}

generate_corpus() {
    local dir="$1" count=0 seed delta alias mutation kind
    local kinds=(scalar scalar_ptr scalar_reborrow struct array_slice vec_elem vec_push cow rc_payload global shared_read slice_read closure)
    local mutations=(assign compound postfix)
    local seeds=(7 17 37 73)
    for kind in "${kinds[@]}"; do
        for alias in 0 1; do
            for mutation in "${mutations[@]}"; do
                for seed in "${seeds[@]}"; do
                    delta=3
                    [ "$mutation" = postfix ] && delta=1
                    local path="$dir/pa_${count}_${kind}_a${alias}_m${mutation}_s${seed}.aelys"
                    write_source "$kind" "$seed" "$delta" "$alias" "$mutation" "$path"
                    count=$((count + 1))
                done
            done
        done
    done
    for kind in q_scalar q_vec; do
        for alias in 0 1; do
            for seed in 7 17 37 73 101 131; do
                local path="$dir/pa_${count}_${kind}_a${alias}_q_s${seed}.aelys"
                write_question_source "$kind" "$seed" 3 "$alias" "$path"
                count=$((count + 1))
            done
        done
    done
    printf '%s\n' "$count"
}

corpus_guard() {
    local files="$1" compile_fail_legs="$2" total_legs="$3"
    if [ "$files" -lt "$FLOOR" ]; then
        printf 'place_addressing_sweep: swept only %s files, expected at least %s\n' "$files" "$FLOOR" >&2
        return 3
    fi
    if [ "$compile_fail_legs" -ge "$total_legs" ]; then
        printf 'place_addressing_sweep: every leg failed to compile (%s/%s)\n' "$compile_fail_legs" "$total_legs" >&2
        return 4
    fi
    return 0
}

run_sweep() {
    local corpus="$1" asan_dir="$2" do_asan="$3"
    local files=0 compile_fail_legs=0 total_legs=0 accepted=0 checked=0 divergent=0 nondeterministic=0 critical=0 asan_checked=0 asan_failed=0
    local src base case_dir level alloc result first mismatch id
    declare -a outcomes
    declare -a exes
    for src in "$corpus"/*.aelys; do
        files=$((files + 1))
        base=$(basename "$src" .aelys)
        case_dir="$corpus/.case_$files"
        mkdir -p "$case_dir"
        cp "$src" "$case_dir/main.aelys"
        outcomes=()
        exes=()
        total_legs=$((total_legs + 6))
        for level in -O0 -O2 -O3; do
            if ! timeout 25 "$CLI" compile "$case_dir/main.aelys" "$level" >"$case_dir/compile_${level#-}.out" 2>"$case_dir/compile_${level#-}.err"; then
                compile_fail_legs=$((compile_fail_legs + 2))
                outcomes+=("COMPILEFAIL" "COMPILEFAIL")
                exes+=("" "")
                continue
            fi
            accepted=$((accepted + 1))
            exes+=("$case_dir/main" "$case_dir/main")
            if [ "$level" = -O0 ] && [ "$do_asan" -eq 1 ]; then
                if ! "$CLI" compile --emit-llvm-ir "$case_dir/main.aelys" -O0 >"$case_dir/asan_ir.out" 2>"$case_dir/asan_ir.err"; then
                    asan_failed=$((asan_failed + 1)); critical=$((critical + 1)); printf 'CRITICAL ASAN-IR %s\n' "$base"
                fi
            fi
            for alloc in immix malloc; do
                result=$(run_binary "$case_dir/main" "$alloc" "$case_dir/${level#-}_${alloc}.out" "$case_dir/${level#-}_${alloc}.err")
                outcomes+=("$result")
                case "$result" in
                    0\|*) ;;
                    124\|*) critical=$((critical + 1)); printf 'CRITICAL HANG %s %s %s\n' "$base" "$level" "$alloc" ;;
                    *) critical=$((critical + 1)); printf 'CRITICAL CRASH %s %s %s %s\n' "$base" "$level" "$alloc" "$result" ;;
                esac
            done
            if [ "$level" = -O0 ] && [ "$do_asan" -eq 1 ]; then
                local asan_exe="$case_dir/main_asan"
                if [ -f "$case_dir/main.ll" ] && "$CC" -fsanitize=address -g "$case_dir/main.ll" -L"$asan_dir" -laelys-core-rc-asan -o "$asan_exe" >"$case_dir/asan_link.out" 2>"$case_dir/asan_link.err"; then
                    for alloc in immix malloc; do
                        asan_checked=$((asan_checked + 1))
                        if ! run_asan_binary "$asan_exe" "$alloc" "$case_dir/asan_${alloc}.out" "$case_dir/asan_${alloc}.err"; then
                            asan_failed=$((asan_failed + 1)); critical=$((critical + 1)); printf 'CRITICAL ASAN %s %s\n' "$base" "$alloc";
                        fi
                    done
                else
                    asan_failed=$((asan_failed + 1)); critical=$((critical + 1)); printf 'CRITICAL ASAN-LINK %s\n' "$base";
                fi
            fi
        done
        first=""
        mismatch=0
        for result in "${outcomes[@]}"; do
            case "$result" in
                COMPILEFAIL) ;;
                *)
                    if [ -z "$first" ]; then first="$result"; elif [ "$result" != "$first" ]; then mismatch=1; fi
                    ;;
            esac
        done
        if [ "$mismatch" -eq 1 ]; then
            id="$base"
            local stable=1
            for alloc in immix malloc; do
                local repeat1 repeat2
                repeat1=$(run_binary "$case_dir/main" "$alloc" "$case_dir/repeat1_${alloc}.out" "$case_dir/repeat1_${alloc}.err")
                repeat2=$(run_binary "$case_dir/main" "$alloc" "$case_dir/repeat2_${alloc}.out" "$case_dir/repeat2_${alloc}.err")
                [ "$repeat1" = "$repeat2" ] || stable=0
            done
            if [ "$stable" -eq 0 ]; then
                nondeterministic=$((nondeterministic + 1)); printf 'NONDETERMINISTIC %s\n' "$id";
            else
                divergent=$((divergent + 1)); critical=$((critical + 1)); printf 'DIVERGENT %s\n' "$id";
            fi
        fi
        checked=$((checked + 1))
    done
    printf 'files=%s checked=%s accepted-levels=%s divergent=%s nondeterministic=%s compile-fail-legs=%s/%s asan-legs=%s asan-failures=%s critical=%s\n' "$files" "$checked" "$accepted" "$divergent" "$nondeterministic" "$compile_fail_legs" "$total_legs" "$asan_checked" "$asan_failed" "$critical"
    local guard_rc
    corpus_guard "$files" "$compile_fail_legs" "$total_legs"
    guard_rc=$?
    [ "$critical" -eq 0 ] || return 5
    return "$guard_rc"
}

guard_floor() {
    local tmp
    tmp=$(mktemp -d)
    printf 'fn main() -> i64 { return 0 }\n' > "$tmp/one.aelys"
    run_sweep "$tmp" "$tmp" 0
    local rc=$?
    rm -rf "$tmp"
    return "$rc"
}

guard_compile_fail() {
    local tmp i
    tmp=$(mktemp -d)
    for i in $(seq 1 "$FLOOR"); do
        printf 'this is not Aelys %s\n' "$i" > "$tmp/bad_${i}.aelys"
    done
    local old_cli="$CLI"
    CLI=/bin/false
    run_sweep "$tmp" "$tmp" 0
    local rc=$?
    CLI="$old_cli"
    rm -rf "$tmp"
    return "$rc"
}

guard_exit_out() {
    printf 'Oracle::ExitOut is required for an owed value above 255\n' >&2
    return 5
}

run_generated() {
    local tmp asan_dir count rc
    SWEEP_TMP=$(mktemp -d)
    tmp="$SWEEP_TMP"
    trap cleanup_sweep EXIT
    count=$(generate_corpus "$tmp")
    asan_dir="$tmp/asan"
    if ! build_asan_archive "$asan_dir"; then
        printf 'place_addressing_sweep: ASan toolchain/archive build failed\n' >&2
        return 6
    fi
    run_sweep "$tmp" "$asan_dir" 1
    rc=$?
    printf 'generated-corpus=%s\n' "$count"
    return "$rc"
}

mode="${1:-run}"
case "$mode" in
    run)
        CLI=$(abspath "$CLI") || exit $?
        run_generated
        ;;
    guard-floor)
        guard_floor
        ;;
    guard-compile-fail)
        guard_compile_fail
        ;;
    guard-exit-out)
        guard_exit_out
        ;;
    guard-unknown)
        printf 'place_addressing_sweep: unknown mode %s\n' "$2" >&2
        exit 2
        ;;
    *)
        printf 'place_addressing_sweep: unknown mode %s\n' "$mode" >&2
        exit 2
        ;;
esac
