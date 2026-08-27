set -u
ROOT="${AELYS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
TABLE="$ROOT/scripts/run3_s6_census.tsv"
EFFECTS="$ROOT/air/src/bir/effects.rs"
PLACE="$ROOT/air/src/lower/place.rs"
EXPR="$ROOT/air/src/lower/expr.rs"
WORK="$(mktemp -d)"
FIX="$WORK/fix"
FULL=0
[ "${1:-}" = "--full" ] && FULL=1

BRANCH="$(cd "$ROOT" && git rev-parse --abbrev-ref HEAD)"
restore() {
  ( cd "$ROOT" && git checkout -q -- "$EFFECTS" "$PLACE" "$EXPR" 2>/dev/null )
  if [ "$FULL" = 1 ]; then
    ( cd "$ROOT" && git checkout -q "$BRANCH" 2>/dev/null )
  fi
  ( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1
}
trap 'restore' EXIT INT TERM

fail() { echo "s6_census.sh: $*" >&2; exit 1; }

ROW_FILES="\
$ROOT/aelys/tests/run3_stage1_loan_roots_tests.rs \
$ROOT/aelys/tests/run3_stage2_positions_tests.rs \
$ROOT/aelys/tests/run3_stage3_cow_tests.rs \
$ROOT/aelys/tests/run3_stage4_ref_mutability_tests.rs \
$ROOT/aelys/tests/run3_stage4_slice_views_tests.rs \
$ROOT/aelys/tests/run3_stage6_boundary_tests.rs"

ANCHORS="run25_sentinel:run25/sentinel run25_stage1tip:run25/stage1-tip \
run25_stage2base:run25/stage2-base run3_base:run3/base"
PREFIXES="p1_stage1tip:34eb78a7 p2_stage2tip:2ea0a5f4 p3_stage3tip:98a70c1b \
p4_prerefmodel:77b47612^ p5_stage4pre:3dbb3513 p6_stage4tip:2f293795"

mkdir -p "$FIX"
echo "== generating the row set from the run's test sources =="
FIX="$FIX" python3 "$ROOT/scripts/extract_rows.py" $ROW_FILES || fail "extraction failed"
( cd "$FIX" && ls ./*.aelys | sed 's#\./##; s/\.aelys$//' ) | LC_ALL=C sort > "$WORK/ids"
LC_ALL=C grep -v '^#' "$TABLE" | cut -f1 | LC_ALL=C sort > "$WORK/declared"
if ! LC_ALL=C diff -q "$WORK/ids" "$WORK/declared" >/dev/null; then
  echo "rows in the tree but not in the table:" >&2
  LC_ALL=C comm -23 "$WORK/ids" "$WORK/declared" | sed 's/^/  /' >&2
  echo "rows in the table but not in the tree:" >&2
  LC_ALL=C comm -13 "$WORK/ids" "$WORK/declared" | sed 's/^/  /' >&2
  fail "the census does not partition the tree's row set"
fi
echo "   $(wc -l < "$WORK/ids") rows, and the table names exactly those"

build() { ( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1; }
sweep() { bash "$ROOT/scripts/s6_row_sweep.sh" "$ROOT/target/release/aelys-cli" "$FIX" "$1"; }

echo "== HEAD =="
build || fail "the HEAD build failed"
sweep "$WORK/HEAD.txt" || fail "the HEAD sweep produced an unclassified row"

bucket() { LC_ALL=C grep -v '^#' "$TABLE" | LC_ALL=C awk -F'\t' -v b="$1" '$2 == b {print $1}' \
  | LC_ALL=C sort; }
for b in ANCHOR PREFIX CTRL-ABLATION CTRL-NOSTORE CTRL-INERT CTRL-FENCE; do
  bucket "$b" > "$WORK/b.$b"
  echo "   $b: $(wc -l < "$WORK/b.$b")"
done
LC_ALL=C sort -u "$WORK"/b.* > "$WORK/all_buckets"
LC_ALL=C diff -q "$WORK/all_buckets" "$WORK/declared" >/dev/null \
  || fail "a row is in two buckets or in none"

echo "== the shape of the two verdict-only buckets, at HEAD =="
n=0
while read -r id; do
  v=$(LC_ALL=C awk -v i="$id" '$1 == i {print $2}' "$WORK/HEAD.txt")
  case "$v" in
    RUN:*allocs=0*) n=$((n + 1)) ;;
    *) fail "$id is booked CTRL-INERT but reads '$v'; an inert row must be accepted at allocs=0" ;;
  esac
done < "$WORK/b.CTRL-INERT"
echo "   CTRL-INERT: $n rows accepted at allocs=0"
n=0
while read -r id; do
  v=$(LC_ALL=C awk -v i="$id" '$1 == i {print $2}' "$WORK/HEAD.txt")
  case "$v" in
    RUN:*) fail "$id is booked CTRL-FENCE but it compiles and runs" ;;
    *) n=$((n + 1)) ;;
  esac
done < "$WORK/b.CTRL-FENCE"
echo "   CTRL-FENCE: $n rows refused"

echo "== the ablation controls, one build per rule =="
moved() { # <sweep file> -> the ids whose reading differs from HEAD
  LC_ALL=C join "$WORK/HEAD.txt" "$1" | LC_ALL=C awk '$2 != $3 {print $1}' | LC_ALL=C sort
}
: > "$WORK/moved_any"
for rule in detach k9 E_PARAM; do
  ( cd "$ROOT" && git checkout -q -- "$EFFECTS" "$PLACE" "$EXPR" )
  case "$rule" in
    detach) AELYS_ROOT="$ROOT" python3 "$ROOT/scripts/s6_ablate.py" D1 D2 D3 >/dev/null ;;
    k9) AELYS_EFFECTS="$EFFECTS" AELYS_ABLATION=K9 python3 "$ROOT/scripts/over_narrow_s4.py" \
      >/dev/null || fail "K9 did not apply" ;;
    E_PARAM) AELYS_ROOT="$ROOT" python3 "$ROOT/scripts/s6_ablate.py" E_PARAM >/dev/null ;;
  esac
  build || fail "the $rule build failed"
  sweep "$WORK/$rule.txt" || fail "the $rule sweep produced an unclassified row"
  moved "$WORK/$rule.txt" > "$WORK/moved.$rule"
  echo "   $rule moved $(wc -l < "$WORK/moved.$rule") row(s)"
  cat "$WORK/moved.$rule" >> "$WORK/moved_any"
done
( cd "$ROOT" && git checkout -q -- "$EFFECTS" "$PLACE" "$EXPR" )
LC_ALL=C sort -u -o "$WORK/moved_any" "$WORK/moved_any"

LC_ALL=C comm -12 "$WORK/moved_any" "$WORK/b.CTRL-ABLATION" > "$WORK/hit"
if ! LC_ALL=C diff -q "$WORK/hit" "$WORK/b.CTRL-ABLATION" >/dev/null; then
  echo "booked CTRL-ABLATION but unmoved by every named rule:" >&2
  LC_ALL=C comm -13 "$WORK/hit" "$WORK/b.CTRL-ABLATION" | sed 's/^/  /' >&2
  fail "a control that reads clean over nothing is the failure this census exists to catch"
fi
echo "   every CTRL-ABLATION row moves under the rule it names"
for b in CTRL-NOSTORE CTRL-INERT CTRL-FENCE; do
  if LC_ALL=C comm -12 "$WORK/moved_any" "$WORK/b.$b" | LC_ALL=C grep -q .; then
    LC_ALL=C comm -12 "$WORK/moved_any" "$WORK/b.$b" | sed 's/^/  /' >&2
    fail "a $b row moved under an ablation, so it is not the control it is booked as"
  fi
done
echo "   no CTRL-NOSTORE, CTRL-INERT or CTRL-FENCE row moves under any of them"

if [ "$FULL" = 1 ]; then
  echo "== --full: rebuilding the anchors and the prefixes =="
  : > "$WORK/hist_moved"
  for pt in $ANCHORS $PREFIXES; do
    name="${pt%%:*}"; ref="${pt#*:}"
    ( cd "$ROOT" && git checkout -q "$ref" ) || fail "$name: checkout failed"
    build || fail "$name: build failed"
    sweep "$WORK/h.$name.txt" || fail "$name: sweep produced an unclassified row"
    moved "$WORK/h.$name.txt" >> "$WORK/hist_moved"
    echo "   $name: $(moved "$WORK/h.$name.txt" | wc -l) row(s) differ from HEAD"
  done
  ( cd "$ROOT" && git checkout -q "$BRANCH" ) || fail "could not return to $BRANCH"
  LC_ALL=C sort -u -o "$WORK/hist_moved" "$WORK/hist_moved"
  LC_ALL=C sort -u "$WORK/b.ANCHOR" "$WORK/b.PREFIX" > "$WORK/b.hist"
  if ! LC_ALL=C diff -q "$WORK/hist_moved" "$WORK/b.hist" >/dev/null; then
    echo "discriminating in the tree but not booked so:" >&2
    LC_ALL=C comm -23 "$WORK/hist_moved" "$WORK/b.hist" | sed 's/^/  /' >&2
    echo "booked as discriminating but reading identically everywhere:" >&2
    LC_ALL=C comm -13 "$WORK/hist_moved" "$WORK/b.hist" | sed 's/^/  /' >&2
    fail "the ANCHOR and PREFIX columns do not match the tree"
  fi
  echo "   the historical columns re-derive exactly"
else
  echo "== the ANCHOR and PREFIX columns were not re-derived; run with --full =="
fi

restore
trap - EXIT INT TERM
echo "OK: $(wc -l < "$WORK/ids") rows, partitioned, every control checked against a deleted rule"
