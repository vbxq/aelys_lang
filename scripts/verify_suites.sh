#!/usr/bin/env bash
set -u

DEFAULT_BASE="${TMPDIR:-/tmp}/aelys-verify"

UPDATE_NAME_PIN=0
ALLOW_DROP=0
VERIFY_REQUESTED=0
ARG_RESULTS_DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --update-name-pin) UPDATE_NAME_PIN=1 ;;
        --allow-drop)      ALLOW_DROP=1 ;;
        --verify)          VERIFY_REQUESTED=1 ;;
        -*) echo "verify_suites: unknown flag $1" >&2; exit 2 ;;
        *)  ARG_RESULTS_DIR="$1" ;;
    esac
    shift
done

if [ "$UPDATE_NAME_PIN" -eq 1 ] && [ "$VERIFY_REQUESTED" -eq 1 ]; then
    echo "verify_suites: --update-name-pin and --verify cannot share an invocation, a pin rewritten by the same command that checks it gates nothing" >&2
    exit 2
fi

if [ "$ALLOW_DROP" -eq 1 ] && [ "$UPDATE_NAME_PIN" -eq 0 ]; then
    echo "verify_suites: --allow-drop is only meaningful with --update-name-pin, on its own it permits nothing" >&2
    exit 2
fi

RESULTS_DIR="${ARG_RESULTS_DIR:-$DEFAULT_BASE/verify_suites_$(date +%Y%m%d-%H%M%S)}"

if ! command -v jq >/dev/null 2>&1; then
    echo "verify_suites: jq is required" >&2
    exit 2
fi

ROOT_MANIFEST=$(cargo locate-project --workspace --message-format plain)
if [ -z "$ROOT_MANIFEST" ]; then
    echo "verify_suites: not inside a cargo workspace" >&2
    exit 2
fi
ROOT=$(dirname "$ROOT_MANIFEST")
cd "$ROOT" || exit 2

mkdir -p "$RESULTS_DIR/logs" || exit 2
META="$RESULTS_DIR/metadata.json"
TARGETS="$RESULTS_DIR/targets.tsv"
MEMBERS="$RESULTS_DIR/members.txt"
UNCOVERED="$RESULTS_DIR/uncovered.txt"
UNCOMPILED="$RESULTS_DIR/uncompiled.txt"
RESULTS="$RESULTS_DIR/results.tsv"
EMPTY_PIN="${EMPTY_PIN:-$ROOT/scripts/empty_targets.tsv}"
TARGET_PIN="${TARGET_PIN:-$ROOT/scripts/target_pin.tsv}"
FEATURES_PIN="${FEATURES_PIN:-$ROOT/scripts/features_pin.tsv}"
CENSUS="${CENSUS:-$ROOT/aelys/tests/ignored_census.tsv}"
NAME_PIN="${NAME_PIN:-$ROOT/scripts/test_name_pin.tsv}"
SWEEPS_PIN="${SWEEPS_PIN:-$ROOT/scripts/sweeps_pin.tsv}"

echo "verify_suites: root $ROOT"
echo "verify_suites: results $RESULTS_DIR"

if ! cargo metadata --no-deps --format-version 1 >"$META" 2>"$RESULTS_DIR/metadata.err"; then
    echo "verify_suites: cargo metadata failed" >&2
    cat "$RESULTS_DIR/metadata.err" >&2
    exit 2
fi

jq -r '
  (.workspace_members | map({(.): true}) | add) as $wm
  | .packages[]
  | select($wm[.id])
  | .name as $pkg
  | .targets[]
  | select(
      (.kind | index("test"))
      or ((.test == true) and ((.kind | index("lib")) or (.kind | index("bin"))))
    )
  | [ $pkg,
      .name,
      (if (.kind | index("test")) then "test"
       elif (.kind | index("lib")) then "lib"
       else "bin" end),
      .src_path ]
  | @tsv
' "$META" >"$TARGETS" || exit 2

# the lib pass skips doctests, without this one a documented example is a check that exists and is never run
jq -r '
  (.workspace_members | map({(.): true}) | add) as $wm
  | .packages[]
  | select($wm[.id])
  | .name as $pkg
  | .targets[]
  | select((.kind | index("lib")) and (.doctest == true))
  | [ $pkg, .name, "doc", .src_path ]
  | @tsv
' "$META" >>"$TARGETS" || exit 2

jq -r '
  (.workspace_members | map({(.): true}) | add) as $wm
  | .packages[] | select($wm[.id]) | .manifest_path
' "$META" >"$MEMBERS" || exit 2

N_TARGETS=$(wc -l <"$TARGETS")
N_TEST=$(awk -F'\t' '$3=="test"' "$TARGETS" | wc -l)
N_DOC=$(awk -F'\t' '$3=="doc"' "$TARGETS" | wc -l)
N_UNIT=$((N_TARGETS - N_TEST - N_DOC))
echo "verify_suites: enumerated $N_TARGETS targets ($N_TEST integration, $N_UNIT unit, $N_DOC doc)"

# an empty target list would exit zero having run nothing, so it dies here instead of reporting green
if [ "$N_TARGETS" -eq 0 ]; then
    echo "verify_suites: the enumeration produced 0 targets, nothing would run and nothing would fail" >&2
    exit 2
fi

pin_problems=""

# a pin that holds no row gates nothing, so present-but-empty is treated exactly like absent
load_pin() {
    label="$1"; path="$2"; out="$3"; want="$4"
    if [ ! -f "$path" ]; then
        pin_problems="${pin_problems}PIN MISSING        $label $path"$'\n'
        : >"$out"
        return 1
    fi
    awk -F'\t' -v want="$want" 'NF >= want && $1 !~ /^#/' "$path" >"$out"
    if [ ! -s "$out" ]; then
        pin_problems="${pin_problems}PIN EMPTY          $label $path (present but holds no row, an empty pin gates nothing)"$'\n'
        return 1
    fi
    return 0
}

TARGET_MEASURED="$RESULTS_DIR/target_measured.tsv"
TARGET_PINNED="$RESULTS_DIR/target_pinned.tsv"
TARGET_PIN_ROWS="$RESULTS_DIR/target_pin_rows.tsv"
awk -F'\t' '{ printf "%s\t%s\t%s\n", $1, $2, $3 }' "$TARGETS" | sort -u >"$TARGET_MEASURED"
if load_pin "target pin" "$TARGET_PIN" "$TARGET_PIN_ROWS" 3; then
    awk -F'\t' '{ printf "%s\t%s\t%s\n", $1, $2, $3 }' "$TARGET_PIN_ROWS" | sort -u >"$TARGET_PINNED"
else
    : >"$TARGET_PINNED"
fi
target_new=$(comm -23 "$TARGET_MEASURED" "$TARGET_PINNED")
target_gone=$(comm -13 "$TARGET_MEASURED" "$TARGET_PINNED")

FEATURES_MEASURED="$RESULTS_DIR/features_measured.tsv"
FEATURES_PINNED="$RESULTS_DIR/features_pinned.tsv"
FEATURES_PIN_ROWS="$RESULTS_DIR/features_pin_rows.tsv"
FEATURE_RUNS="$RESULTS_DIR/feature_runs.tsv"
jq -r '
  (.workspace_members | map({(.): true}) | add) as $wm
  | .packages[]
  | select($wm[.id])
  | .name as $pkg
  | (.features | keys[])
  | [ $pkg, . ]
  | @tsv
' "$META" | sort -u >"$FEATURES_MEASURED" || exit 2

feature_bad=""
: >"$FEATURE_RUNS"
if load_pin "features pin" "$FEATURES_PIN" "$FEATURES_PIN_ROWS" 4; then
    awk -F'\t' '{ printf "%s\t%s\n", $1, $2 }' "$FEATURES_PIN_ROWS" | sort -u >"$FEATURES_PINNED"
    while IFS=$'\t' read -r fpkg feat decision reason; do
        case "$decision" in
            run|excluded) ;;
            *) feature_bad="${feature_bad}BAD FEATURE DECISION $fpkg $feat: \`$decision\` is neither run nor excluded"$'\n' ;;
        esac
        if [ -z "${reason// /}" ]; then
            feature_bad="${feature_bad}FEATURE WITHOUT REASON $fpkg $feat: a decision with no measured reason is not a decision"$'\n'
        fi
        [ "$decision" = run ] && printf '%s\t%s\n' "$fpkg" "$feat" >>"$FEATURE_RUNS"
    done <"$FEATURES_PIN_ROWS"
else
    : >"$FEATURES_PINNED"
fi
feature_new=$(comm -23 "$FEATURES_MEASURED" "$FEATURES_PINNED")
feature_gone=$(comm -13 "$FEATURES_MEASURED" "$FEATURES_PINNED")

# a suite file with no enumerated target is a suite that exists in the tree and is never run
awk -F'\t' '$3=="test" {print $4}' "$TARGETS" | sort -u >"$RESULTS_DIR/test_src_paths.txt"
: >"$UNCOVERED"
: >"$UNCOMPILED"
while IFS= read -r manifest; do
    tdir="$(dirname "$manifest")/tests"
    [ -d "$tdir" ] || continue
    # cargo makes a target of tests/*.rs and of tests/<dir>/main.rs, and of nothing else under tests/
    while IFS= read -r f; do
        if ! command grep -Fxq "$f" "$RESULTS_DIR/test_src_paths.txt"; then
            echo "$f" >>"$UNCOVERED"
        fi
    done < <( { find "$tdir" -maxdepth 1 -type f -name '*.rs'
                find "$tdir" -mindepth 2 -maxdepth 2 -type f -name 'main.rs'; } | sort )
    while IFS= read -r d; do
        [ -f "$d/main.rs" ] && continue
        if [ -n "$(find "$d" -type f -name '*.rs' -print -quit)" ]; then
            echo "$d" >>"$UNCOMPILED"
        fi
    done < <(find "$tdir" -mindepth 1 -maxdepth 1 -type d | sort)
done <"$MEMBERS"
N_UNCOVERED=$(wc -l <"$UNCOVERED")
N_UNCOMPILED=$(wc -l <"$UNCOMPILED")
echo "verify_suites: uncovered test sources: $N_UNCOVERED"
if [ "$N_UNCOVERED" -gt 0 ]; then
    while IFS= read -r f; do echo "UNCOVERED	$f"; done <"$UNCOVERED"
fi
echo "verify_suites: test dirs cargo never compiles: $N_UNCOMPILED"
if [ "$N_UNCOMPILED" -gt 0 ]; then
    while IFS= read -r d; do echo "UNCOMPILED	$d (holds *.rs but no main.rs)"; done <"$UNCOMPILED"
fi

# two cargo jobs and two test threads are a hard cap on this machine, above that it runs out of ram
echo "verify_suites: building all targets"
if ! cargo build --workspace --all-targets -j2 >"$RESULTS_DIR/build.log" 2>&1; then
    echo "verify_suites: workspace build failed, aborting before any suite runs" >&2
    tail -n 40 "$RESULTS_DIR/build.log" >&2
    exit 3
fi

# the default build leaves every cfg(feature) target uncompiled, so each run feature is built before any suite runs
while IFS=$'\t' read -r fpkg feat; do
    echo "verify_suites: building $fpkg with --features $feat"
    if ! cargo build -p "$fpkg" --all-targets --features "$feat" -j2 \
        >"$RESULTS_DIR/build_${fpkg}_${feat}.log" 2>&1; then
        echo "verify_suites: build of $fpkg --features $feat failed, aborting before any suite runs" >&2
        tail -n 40 "$RESULTS_DIR/build_${fpkg}_${feat}.log" >&2
        exit 3
    fi
done <"$FEATURE_RUNS"

NAME_MEASURED="$RESULTS_DIR/name_measured.tsv"
NAME_PINNED="$RESULTS_DIR/name_pinned.tsv"
NAME_PIN_ROWS="$RESULTS_DIR/name_pin_rows.tsv"
NAME_UNMEASURED="$RESULTS_DIR/name_unmeasured.tsv"
IGN_MEASURED="$RESULTS_DIR/ignored_measured.tsv"

list_target() {
    pkg="$1"; tgt="$2"; kind="$3"; feat="$4"
    featargs=()
    kindlabel="$kind"
    if [ -n "$feat" ]; then
        kindlabel="$kind+$feat"
        featargs=(--features "$feat")
    fi
    log="$RESULTS_DIR/logs/${pkg}__${tgt}__${kindlabel}.list"
    case "$kind" in
        test) cargo test -p "$pkg" --test "$tgt" "${featargs[@]}" -j2 -- --list >"$log" 2>&1 </dev/null ;;
        lib)  cargo test -p "$pkg" --lib          "${featargs[@]}" -j2 -- --list >"$log" 2>&1 </dev/null ;;
        bin)  cargo test -p "$pkg" --bin "$tgt"   "${featargs[@]}" -j2 -- --list >"$log" 2>&1 </dev/null ;;
        doc)  cargo test -p "$pkg" --doc          "${featargs[@]}" -j2 -- --list >"$log" 2>&1 </dev/null ;;
        *)    echo "verify_suites: unknown kind $kind" >&2; exit 2 ;;
    esac
    awk -v p="$pkg" -v t="$tgt" -v k="$kindlabel" '
        /: test$/ || /: benchmark$/ {
            n = $0
            sub(/: (test|benchmark)$/, "", n)
            printf "%s\t%s\t%s\t%s\n", p, t, k, n
        }
    ' "$log" >>"$NAME_MEASURED"
}

# the refresh is the only pass that still pays for a listing run, a pin taken from run logs would inherit whatever the runs happened to register
if [ "$UPDATE_NAME_PIN" -eq 1 ]; then
    echo "verify_suites: listing registered test names for the pin refresh"
    name_t0=$(date +%s)
    : >"$NAME_MEASURED"
    while IFS=$'\t' read -r pkg tgt kind src <&3; do
        list_target "$pkg" "$tgt" "$kind" ""
    done 3<"$TARGETS"
    while IFS=$'\t' read -r fpkg feat <&4; do
        while IFS=$'\t' read -r pkg tgt kind src <&3; do
            [ "$pkg" = "$fpkg" ] || continue
            list_target "$pkg" "$tgt" "$kind" "$feat"
        done 3<"$TARGETS"
    done 4<"$FEATURE_RUNS"
    sort -u -o "$NAME_MEASURED" "$NAME_MEASURED"
    N_NAMES=$(wc -l <"$NAME_MEASURED")
    echo "verify_suites: listed $N_NAMES registered test names in $(( $(date +%s) - name_t0 ))s"

    if [ -f "$NAME_PIN" ]; then
        awk -F'\t' 'NF >= 4 && $1 !~ /^#/ { printf "%s\t%s\t%s\t%s\n", $1, $2, $3, $4 }' "$NAME_PIN" \
            | sort -u >"$NAME_PINNED"
    else
        : >"$NAME_PINNED"
    fi
    pin_adds=$(comm -23 "$NAME_MEASURED" "$NAME_PINNED")
    pin_drops=$(comm -13 "$NAME_MEASURED" "$NAME_PINNED")
    n_pin_adds=$(printf '%s' "$pin_adds" | command grep -c .)
    n_pin_drops=$(printf '%s' "$pin_drops" | command grep -c .)
    echo "verify_suites: refresh would add $n_pin_adds name(s) and drop $n_pin_drops"
    if [ "$n_pin_drops" -gt 0 ]; then
        printf '%s\n' "$pin_drops" | while IFS= read -r r; do
            echo "WOULD DROP           $r (pinned and no longer registered)"
        done
    fi
    if [ "$n_pin_drops" -gt 0 ] && [ "$ALLOW_DROP" -ne 1 ]; then
        echo "verify_suites: refusing to drop $n_pin_drops pinned name(s), $NAME_PIN was not written" >&2
        echo "verify_suites: removing a check must cost strictly more than adding one, an additive refresh needs no flag and a refresh that forgets a check needs --allow-drop" >&2
        exit 2
    fi
    if [ "$n_pin_drops" -gt 0 ]; then
        echo "verify_suites: --allow-drop given, the $n_pin_drops name(s) above stop being gated"
    fi
    cp "$NAME_MEASURED" "$NAME_PIN" || exit 2
    echo "verify_suites: rewrote $NAME_PIN with $N_NAMES rows"
    echo "verify_suites: no suite ran and nothing was gated, this invocation only refreshed the name pin"
    exit 0
fi

: >"$NAME_MEASURED"
: >"$IGN_MEASURED"
: >"$NAME_UNMEASURED"

printf 'pkg\ttarget\tkind\tstatus\tpassed\tfailed\tignored\n' >"$RESULTS"

n_pass=0; n_fail=0; n_empty=0; n_buildfail=0
tot_p=0; tot_f=0; tot_i=0
ftot_p=0; ftot_f=0; ftot_i=0
nonpass=""
idx=0
n_runs=$N_TARGETS
while IFS=$'\t' read -r fpkg feat; do
    n_runs=$((n_runs + $(awk -F'\t' -v p="$fpkg" '$1==p' "$TARGETS" | wc -l)))
done <"$FEATURE_RUNS"

run_target() {
    pkg="$1"; tgt="$2"; kind="$3"; feat="$4"
    featargs=()
    kindlabel="$kind"
    if [ -n "$feat" ]; then
        kindlabel="$kind+$feat"
        featargs=(--features "$feat")
    fi
    idx=$((idx + 1))
    log="$RESULTS_DIR/logs/${pkg}__${tgt}__${kindlabel}.log"
    printf '[%3d/%3d] %s %s (%s)\n' "$idx" "$n_runs" "$pkg" "$tgt" "$kindlabel"
    case "$kind" in
        test) cargo test -p "$pkg" --test "$tgt" "${featargs[@]}" -j2 -- --test-threads=2 >"$log" 2>&1 </dev/null ;;
        lib)  cargo test -p "$pkg" --lib          "${featargs[@]}" -j2 -- --test-threads=2 >"$log" 2>&1 </dev/null ;;
        bin)  cargo test -p "$pkg" --bin "$tgt"   "${featargs[@]}" -j2 -- --test-threads=2 >"$log" 2>&1 </dev/null ;;
        doc)  cargo test -p "$pkg" --doc          "${featargs[@]}" -j2 -- --test-threads=2 >"$log" 2>&1 </dev/null ;;
        *)    echo "verify_suites: unknown kind $kind" >&2; exit 2 ;;
    esac
    rc=$?

    nres=$(command grep -c '^test result:' "$log")
    counts=$(awk '/^test result:/ {
        for (k = 1; k <= NF; k++) {
            if ($(k+1) ~ /^passed/)  p += $k;
            else if ($(k+1) ~ /^failed/)  f += $k;
            else if ($(k+1) ~ /^ignored/) g += $k;
        }
    } END { printf "%d %d %d", p+0, f+0, g+0 }' "$log")
    p=$(echo "$counts" | cut -d' ' -f1)
    f=$(echo "$counts" | cut -d' ' -f2)
    g=$(echo "$counts" | cut -d' ' -f3)

    # exit zero with nothing executed is a suite that quietly stopped running, never a pass
    if [ "$rc" -ne 0 ]; then
        if [ "$nres" -eq 0 ]; then status=BUILDFAIL; else status=FAIL; fi
    elif [ "$nres" -eq 0 ] || { [ "$p" -eq 0 ] && [ "$f" -eq 0 ]; }; then
        status=EMPTY
    else
        status=PASS
    fi

    case "$status" in
        PASS)      n_pass=$((n_pass + 1)) ;;
        FAIL)      n_fail=$((n_fail + 1)) ;;
        EMPTY)     n_empty=$((n_empty + 1)) ;;
        BUILDFAIL) n_buildfail=$((n_buildfail + 1)) ;;
    esac
    [ "$status" = PASS ] || nonpass="$nonpass$status	$pkg	$tgt	$kindlabel	$log"$'\n'

    # a binary that never printed a result line witnessed none of its names, which is not the same as registering none
    if [ "$nres" -eq 0 ]; then
        printf '%s\t%s\t%s\t%s\n' "$pkg" "$tgt" "$kindlabel" "$status" >>"$NAME_UNMEASURED"
    else
        # a listing spells a should_panic test with its bare name while the run log appends ` - should panic`
        awk -v p="$pkg" -v t="$tgt" -v k="$kindlabel" -v ign="$IGN_MEASURED" '
            /^test / {
                rest = substr($0, 6)
                i = index(rest, " ... ")
                if (i == 0) next
                n = substr(rest, 1, i - 1)
                sub(/ - should panic$/, "", n)
                printf "%s\t%s\t%s\t%s\n", p, t, k, n
                if (substr(rest, i + 5) ~ /^ignored/) printf "%s\t%s\t%s\t%s\n", p, t, k, n >> ign
            }
        ' "$log" >>"$NAME_MEASURED"
    fi

    if [ -n "$feat" ]; then
        ftot_p=$((ftot_p + p)); ftot_f=$((ftot_f + f)); ftot_i=$((ftot_i + g))
    else
        tot_p=$((tot_p + p)); tot_f=$((tot_f + f)); tot_i=$((tot_i + g))
    fi
    printf '%s\t%s\t%s\t%s\t%d\t%d\t%d\n' "$pkg" "$tgt" "$kindlabel" "$status" "$p" "$f" "$g" >>"$RESULTS"
}

while IFS=$'\t' read -r pkg tgt kind src <&3; do
    run_target "$pkg" "$tgt" "$kind" ""
done 3<"$TARGETS"

while IFS=$'\t' read -r fpkg feat <&4; do
    while IFS=$'\t' read -r pkg tgt kind src <&3; do
        [ "$pkg" = "$fpkg" ] || continue
        run_target "$pkg" "$tgt" "$kind" "$feat"
    done 3<"$TARGETS"
done 4<"$FEATURE_RUNS"

sort -u -o "$NAME_MEASURED" "$NAME_MEASURED"
sort -u -o "$IGN_MEASURED" "$IGN_MEASURED"
N_NAMES=$(wc -l <"$NAME_MEASURED")

SWEEPS_MEASURED="$RESULTS_DIR/sweeps_measured.tsv"
SWEEPS_PINNED="$RESULTS_DIR/sweeps_pinned.tsv"
SWEEPS_PIN_ROWS="$RESULTS_DIR/sweeps_pin_rows.tsv"
SWEEP_RUNS="$RESULTS_DIR/sweep_runs.tsv"

# a standing check that lives in scripts/ and that no list invokes is exactly the class this harness exists to close
find "$ROOT/scripts" -maxdepth 1 -type f -printf '%f\n' | sort -u >"$SWEEPS_MEASURED"

sweep_bad=""
: >"$SWEEP_RUNS"
if load_pin "sweeps pin" "$SWEEPS_PIN" "$SWEEPS_PIN_ROWS" 3; then
    awk -F'\t' '{ print $1 }' "$SWEEPS_PIN_ROWS" | sort -u >"$SWEEPS_PINNED"
    while IFS=$'\t' read -r sname sdecision sreason; do
        case "$sdecision" in
            run|excluded) ;;
            *) sweep_bad="${sweep_bad}BAD SWEEP DECISION   $sname: \`$sdecision\` is neither run nor excluded"$'\n' ;;
        esac
        if [ -z "${sreason// /}" ]; then
            sweep_bad="${sweep_bad}SWEEP WITHOUT REASON $sname: a decision with no measured reason is not a decision"$'\n'
        fi
        [ "$sdecision" = run ] && printf '%s\n' "$sname" >>"$SWEEP_RUNS"
    done <"$SWEEPS_PIN_ROWS"
else
    : >"$SWEEPS_PINNED"
fi
sweep_new=$(comm -23 "$SWEEPS_MEASURED" "$SWEEPS_PINNED")
sweep_gone=$(comm -13 "$SWEEPS_MEASURED" "$SWEEPS_PINNED")

RELEASE_CLI="$ROOT/target/release/aelys-cli"
SWEEP_ARGS=()
sweep_argv() {
    case "$1" in
        reserved_symbols.sh)       SWEEP_ARGS=() ;;
        place_addressing_sweep.sh) SWEEP_ARGS=() ;;
        corpus_sweep.sh)           SWEEP_ARGS=(xo "$RELEASE_CLI") ;;
        *) return 1 ;;
    esac
    return 0
}

sweep_fail=""
n_sweep_pass=0
n_sweep_fail=0
sweep_seconds=0
N_SWEEP_RUNS=$(wc -l <"$SWEEP_RUNS")
if [ "$N_SWEEP_RUNS" -gt 0 ]; then
    # the sweeps drive the optimised compiler, which the all-targets debug build above never produces
    echo "verify_suites: building the release cli the sweeps drive"
    if ! cargo build --release -p aelys-cli -j2 >"$RESULTS_DIR/build_release_cli.log" 2>&1; then
        echo "verify_suites: release cli build failed, the sweeps cannot run" >&2
        tail -n 40 "$RESULTS_DIR/build_release_cli.log" >&2
        exit 3
    fi
    sidx=0
    while IFS= read -r sname <&5; do
        sidx=$((sidx + 1))
        if ! sweep_argv "$sname"; then
            sweep_bad="${sweep_bad}SWEEP WITHOUT INVOCATION $sname (pinned run and verify_suites holds no argv for it, it would never be executed)"$'\n'
            continue
        fi
        slog="$RESULTS_DIR/logs/sweep__${sname}.log"
        printf '[sweep %d/%d] %s %s\n' "$sidx" "$N_SWEEP_RUNS" "$sname" "${SWEEP_ARGS[*]}"
        st0=$(date +%s)
        bash "$ROOT/scripts/$sname" "${SWEEP_ARGS[@]}" >"$slog" 2>&1 </dev/null
        src=$?
        sdt=$(( $(date +%s) - st0 ))
        sweep_seconds=$((sweep_seconds + sdt))
        if [ "$src" -eq 0 ]; then
            n_sweep_pass=$((n_sweep_pass + 1))
            echo "           $sname exit 0 in ${sdt}s"
        else
            n_sweep_fail=$((n_sweep_fail + 1))
            sweep_fail="${sweep_fail}SWEEP FAILED         $sname exit $src after ${sdt}s, $slog"$'\n'
            echo "           $sname exit $src in ${sdt}s"
        fi
    done 5<"$SWEEP_RUNS"
fi

UNMEASURED_KEYS="$RESULTS_DIR/name_unmeasured_keys.tsv"
UNMEASURED_BASEKEYS="$RESULTS_DIR/name_unmeasured_basekeys.tsv"
awk -F'\t' '{ printf "%s\t%s\t%s\n", $1, $2, $3 }' "$NAME_UNMEASURED" | sort -u >"$UNMEASURED_KEYS"
awk -F'\t' '{ k = $3; sub(/\+.*$/, "", k); printf "%s\t%s\t%s\n", $1, $2, k }' "$NAME_UNMEASURED" \
    | sort -u >"$UNMEASURED_BASEKEYS"

if load_pin "name pin" "$NAME_PIN" "$NAME_PIN_ROWS" 4; then
    awk -F'\t' '{ printf "%s\t%s\t%s\t%s\n", $1, $2, $3, $4 }' "$NAME_PIN_ROWS" | sort -u >"$NAME_PINNED"
else
    : >"$NAME_PINNED"
fi
N_NAME_PINNED=$(wc -l <"$NAME_PINNED")

NAME_GONE_ALL="$RESULTS_DIR/name_gone_all.tsv"
NAME_GONE="$RESULTS_DIR/name_gone.tsv"
NAME_UNCHECKED="$RESULTS_DIR/name_unchecked.tsv"
comm -13 "$NAME_MEASURED" "$NAME_PINNED" >"$NAME_GONE_ALL"
: >"$NAME_UNCHECKED"
awk -F'\t' -v keys="$UNMEASURED_KEYS" -v held="$NAME_UNCHECKED" '
    BEGIN { while ((getline l < keys) > 0) k[l] = 1 }
    { key = $1 "\t" $2 "\t" $3; if (key in k) print >> held; else print }
' "$NAME_GONE_ALL" >"$NAME_GONE"
name_new=$(comm -23 "$NAME_MEASURED" "$NAME_PINNED")
name_gone=$(cat "$NAME_GONE")
N_NAME_UNCHECKED=$(wc -l <"$NAME_UNCHECKED")

name_unchecked_report=""
while IFS=$'\t' read -r up ut uk ust; do
    held=$(awk -F'\t' -v p="$up" -v t="$ut" -v k="$uk" '$1==p && $2==t && $3==k' "$NAME_PINNED" | wc -l)
    name_unchecked_report="${name_unchecked_report}NAMES UNCHECKED      $up $ut $uk ($ust, the binary printed no result line, $held pinned name(s) held back rather than reported as vanished)"$'\n'
done <"$NAME_UNMEASURED"

# some targets are structurally empty forever, so the gate compares the EMPTY set against a pin instead of counting
MEASURED_EMPTY="$RESULTS_DIR/empty_measured.tsv"
PINNED_EMPTY="$RESULTS_DIR/empty_pinned.tsv"
EMPTY_PIN_ROWS="$RESULTS_DIR/empty_pin_rows.tsv"
awk -F'\t' 'NR > 1 && $4 == "EMPTY" { printf "%s\t%s\t%s\n", $1, $2, $3 }' "$RESULTS" \
    | sort -u >"$MEASURED_EMPTY"
if load_pin "empty pin" "$EMPTY_PIN" "$EMPTY_PIN_ROWS" 3; then
    awk -F'\t' '{ printf "%s\t%s\t%s\n", $1, $2, $3 }' "$EMPTY_PIN_ROWS" | sort -u >"$PINNED_EMPTY"
else
    : >"$PINNED_EMPTY"
fi
empty_new=$(comm -23 "$MEASURED_EMPTY" "$PINNED_EMPTY")
empty_gone=$(comm -13 "$MEASURED_EMPTY" "$PINNED_EMPTY")

# the census is textual and the harness is numeric, so an ignore that dodges one of them still trips the other
census_always=-1
census_conditional=0
if [ -f "$CENSUS" ]; then
    census_always=$(awk -F'\t' 'NF >= 4 && $1 !~ /^#/ && $3 == "always"' "$CENSUS" | wc -l)
    census_conditional=$(awk -F'\t' 'NF >= 4 && $1 !~ /^#/ && $3 != "always"' "$CENSUS" | wc -l)
fi

# an unconditional census row is ignored in every pass its target runs in, so a feature run must ignore the base pass set of its package and nothing else
feature_ignore_bad=""
if [ "$census_conditional" -gt 0 ]; then
    awk -F'\t' 'NF >= 4 && $1 !~ /^#/ && $3 != "always" { printf "%s %s [%s]\n", $1, $2, $3 }' "$CENSUS" \
        >"$RESULTS_DIR/census_conditional.txt"
    while IFS= read -r r; do
        feature_ignore_bad="${feature_ignore_bad}CENSUS CONDITIONAL ROW $r (the feature pass expectation is the base pass ignored set and models no conditional row, so this row is asserted by nothing)"$'\n'
    done <"$RESULTS_DIR/census_conditional.txt"
fi
while IFS=$'\t' read -r fpkg feat; do
    exp="$RESULTS_DIR/ignored_base_${fpkg}_${feat}.tsv"
    got="$RESULTS_DIR/ignored_feature_${fpkg}_${feat}.tsv"
    awk -F'\t' -v p="$fpkg" -v keys="$UNMEASURED_BASEKEYS" '
        BEGIN { while ((getline l < keys) > 0) k[l] = 1 }
        $1 == p && $3 !~ /\+/ {
            if (($1 "\t" $2 "\t" $3) in k) next
            printf "%s\t%s\t%s\t%s\n", $1, $2, $3, $4
        }
    ' "$IGN_MEASURED" | sort -u >"$exp"
    awk -F'\t' -v p="$fpkg" -v sfx="+$feat" -v keys="$UNMEASURED_BASEKEYS" '
        BEGIN { while ((getline l < keys) > 0) k[l] = 1 }
        $1 == p && length($3) > length(sfx) && substr($3, length($3) - length(sfx) + 1) == sfx {
            kind = substr($3, 1, length($3) - length(sfx))
            if (($1 "\t" $2 "\t" kind) in k) next
            printf "%s\t%s\t%s\t%s\n", $1, $2, kind, $4
        }
    ' "$IGN_MEASURED" | sort -u >"$got"
    only_feat=$(comm -13 "$exp" "$got")
    only_base=$(comm -23 "$exp" "$got")
    if [ -n "$only_feat" ]; then
        while IFS= read -r r; do
            feature_ignore_bad="${feature_ignore_bad}FEATURE ONLY IGNORE  --features $feat: $r (ignored under the feature and not ignored in the base pass, enabling a feature took a check away)"$'\n'
        done < <(printf '%s\n' "$only_feat")
    fi
    if [ -n "$only_base" ]; then
        while IFS= read -r r; do
            feature_ignore_bad="${feature_ignore_bad}BASE ONLY IGNORE     --features $feat: $r (ignored in the base pass and not ignored under the feature, its census row calls the ignore unconditional)"$'\n'
        done < <(printf '%s\n' "$only_base")
    fi
done <"$FEATURE_RUNS"

echo
echo "==== verify_suites summary ===="
echo "targets enumerated : $N_TARGETS ($N_TEST integration, $N_UNIT unit, $N_DOC doc)"
echo "runs performed     : $idx"
echo "PASS               : $n_pass"
echo "FAIL               : $n_fail"
echo "EMPTY              : $n_empty"
echo "BUILDFAIL          : $n_buildfail"
echo "UNCOVERED          : $N_UNCOVERED"
echo "UNCOMPILED         : $N_UNCOMPILED"
echo "tests passed       : $tot_p"
echo "tests failed       : $tot_f"
echo "tests ignored      : $tot_i"
echo "feature passed     : $ftot_p"
echo "feature failed     : $ftot_f"
echo "feature ignored    : $ftot_i"
echo "registered names   : $N_NAMES"
echo "name pin rows      : $N_NAME_PINNED"
echo "names unchecked    : $N_NAME_UNCHECKED"
echo "name source        : parsed from the run logs, no --list pass"
echo "sweeps enumerated  : $(wc -l <"$SWEEPS_MEASURED")"
echo "sweeps run         : $n_sweep_pass"
echo "sweeps failed      : $n_sweep_fail"
echo "sweep seconds      : $sweep_seconds"
echo "census always rows : $census_always"
echo "census cfg rows    : $census_conditional"
echo "results tsv        : $RESULTS"
echo "empty pin          : $EMPTY_PIN"
echo "target pin         : $TARGET_PIN"
echo "features pin       : $FEATURES_PIN"
echo "name pin           : $NAME_PIN"
echo "sweeps pin         : $SWEEPS_PIN"
echo "ignore census      : $CENSUS"
echo
echo "---- non PASS targets ----"
if [ -n "$nonpass" ]; then printf '%s' "$nonpass"; else echo "(none)"; fi
echo "---- uncovered test sources ----"
if [ "$N_UNCOVERED" -gt 0 ]; then cat "$UNCOVERED"; else echo "(none)"; fi
echo "---- test dirs cargo never compiles ----"
if [ "$N_UNCOMPILED" -gt 0 ]; then cat "$UNCOMPILED"; else echo "(none)"; fi
echo "---- pin availability ----"
if [ -n "$pin_problems" ]; then printf '%s' "$pin_problems"; else echo "(none, every pin is present and holds rows)"; fi
echo "---- target set drift ----"
if [ -n "$target_new" ]; then
    printf '%s\n' "$target_new" | while IFS= read -r r; do
        echo "UNPINNED TARGET      $r (enumerated and absent from the pin)"
    done
fi
if [ -n "$target_gone" ]; then
    printf '%s\n' "$target_gone" | while IFS= read -r r; do
        echo "PINNED TARGET MISSING $r (pinned and no longer enumerated, it was de-registered or renamed)"
    done
fi
if [ -z "$target_new" ] && [ -z "$target_gone" ]; then
    echo "(none, the enumerated target set equals the pin)"
fi
echo "---- feature drift ----"
if [ -n "$feature_new" ]; then
    printf '%s\n' "$feature_new" | while IFS= read -r r; do
        echo "UNPINNED FEATURE     $r (declared in a manifest and absent from the pin, so nothing decided whether its cfg gated checks run)"
    done
fi
if [ -n "$feature_gone" ]; then
    printf '%s\n' "$feature_gone" | while IFS= read -r r; do
        echo "PINNED FEATURE MISSING $r (pinned and no longer declared, drop its row)"
    done
fi
if [ -n "$feature_bad" ]; then printf '%s' "$feature_bad"; fi
if [ -z "$feature_new" ] && [ -z "$feature_gone" ] && [ -z "$feature_bad" ]; then
    echo "(none, the declared feature set equals the pin)"
fi
echo "---- empty target pin drift ----"
if [ -n "$empty_new" ]; then
    printf '%s\n' "$empty_new" | while IFS= read -r r; do
        echo "UNPINNED EMPTY     $r (empty and absent from the pin, it went hollow or the pin is stale)"
    done
fi
if [ -n "$empty_gone" ]; then
    printf '%s\n' "$empty_gone" | while IFS= read -r r; do
        echo "PINNED BUT NOT EMPTY $r (it runs tests now, drop its row from the pin)"
    done
fi
if [ -z "$empty_new" ] && [ -z "$empty_gone" ]; then
    echo "(none, the measured EMPTY set equals the pin)"
fi
echo "---- test name drift ----"
if [ -n "$name_gone" ]; then
    printf '%s\n' "$name_gone" | while IFS= read -r r; do
        echo "PINNED TEST MISSING  $r (pinned and no longer registered, the check is still in the tree and never runs)"
    done
fi
if [ -n "$name_new" ]; then
    printf '%s\n' "$name_new" | while IFS= read -r r; do
        echo "NEW TESTS            $r (registered and absent from the pin, a check appearing is not the defect, refresh with --update-name-pin)"
    done
fi
if [ -n "$name_unchecked_report" ]; then printf '%s' "$name_unchecked_report"; fi
if [ -z "$name_gone" ] && [ -z "$name_new" ] && [ -z "$name_unchecked_report" ]; then
    echo "(none, the registered name set equals the pin)"
fi
echo "---- feature pass ignored ----"
if [ -n "$feature_ignore_bad" ]; then
    printf '%s' "$feature_ignore_bad"
else
    echo "(none, every feature run ignores exactly the base pass ignored set of its package)"
fi
echo "---- sweeps pin drift ----"
if [ -n "$sweep_new" ]; then
    printf '%s\n' "$sweep_new" | while IFS= read -r r; do
        echo "UNPINNED SWEEP       $r (present in scripts/ and absent from the pin, so nothing decided whether it runs)"
    done
fi
if [ -n "$sweep_gone" ]; then
    printf '%s\n' "$sweep_gone" | while IFS= read -r r; do
        echo "PINNED SWEEP MISSING $r (pinned and no longer present in scripts/, drop its row)"
    done
fi
if [ -n "$sweep_bad" ]; then printf '%s' "$sweep_bad"; fi
if [ -z "$sweep_new" ] && [ -z "$sweep_gone" ] && [ -z "$sweep_bad" ]; then
    echo "(none, the scripts/ file set equals the pin)"
fi
echo "---- standing sweeps ----"
if [ -n "$sweep_fail" ]; then
    printf '%s' "$sweep_fail"
else
    echo "(none, $n_sweep_pass pinned run script(s) exited 0 in ${sweep_seconds}s)"
fi
echo "---- ignored total ----"
if [ "$census_always" -lt 0 ]; then
    echo "CENSUS MISSING     $CENSUS"
elif [ "$tot_i" -ne "$census_always" ]; then
    echo "IGNORED TOTAL MISMATCH measured $tot_i over the base pass, census holds $census_always unconditional rows"
else
    echo "(none, $tot_i ignored equals the $census_always unconditional census rows)"
fi

rc=0
[ "$n_fail" -eq 0 ] || rc=1
[ "$n_buildfail" -eq 0 ] || rc=1
[ "$N_UNCOVERED" -eq 0 ] || rc=1
[ "$N_UNCOMPILED" -eq 0 ] || rc=1
[ -z "$pin_problems" ] || rc=1
[ -z "$target_new" ] || rc=1
[ -z "$target_gone" ] || rc=1
[ -z "$feature_new" ] || rc=1
[ -z "$feature_gone" ] || rc=1
[ -z "$feature_bad" ] || rc=1
[ -z "$empty_new" ] || rc=1
[ -z "$empty_gone" ] || rc=1
# a name appearing is not a defect, only a name disappearing is, so the gate reads one way
[ -z "$name_gone" ] || rc=1
[ "$N_NAME_UNCHECKED" -eq 0 ] || rc=1
[ "$census_always" -ge 0 ] || rc=1
[ "$tot_i" -eq "$census_always" ] || rc=1
[ -z "$feature_ignore_bad" ] || rc=1
[ -z "$sweep_new" ] || rc=1
[ -z "$sweep_gone" ] || rc=1
[ -z "$sweep_bad" ] || rc=1
[ "$n_sweep_fail" -eq 0 ] || rc=1
echo
echo "verify_suites: exit $rc"
exit "$rc"
