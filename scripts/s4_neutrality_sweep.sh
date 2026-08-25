# # swept and reported but never asserted on. `git ls-files '*.aelys'` returns 1, which is why a
set -u
ROOT="${AELYS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
EFFECTS="$ROOT/air/src/bir/effects.rs"
BASE_REV="${AELYS_BASE_REV:-HEAD~1}"
EXPECT_FLIPPED="${AELYS_EXPECT_FLIPPED:-0}"
WORK="$(mktemp -d)"
BACKUP="$WORK/effects.rs.orig"
BASE_CLI="$WORK/aelys-cli-base"
CAND_CLI="$WORK/aelys-cli-cand"
CTL="$WORK/controls"

restore() {
  if [ -f "$BACKUP" ]; then
    cp "$BACKUP" "$EFFECTS"
    ( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1
  fi
}
trap 'restore' EXIT INT TERM

fail() { echo "s4_neutrality_sweep.sh: $*" >&2; exit 1; }

classify() { # <base verdict> <candidate verdict> -> same | flipped | unclassified
  case "$1$2" in
    *UNCLASSIFIED*) echo unclassified; return ;;
  esac
  if [ "$1" = "$2" ]; then echo same; else echo flipped; fi
}

[ "$(classify E0727 E0727)" = same ] || fail "the comparator cannot see agreement"
[ "$(classify E0727 ACCEPT)" = flipped ] || fail "the comparator cannot see a flip"
[ "$(classify UNCLASSIFIED ACCEPT)" = unclassified ] || fail "the comparator swallows a non-verdict"
echo "== comparator self-test: same, flipped and unclassified all reachable =="

if [ -n "$(cd "$ROOT" && git status --porcelain -- air/src/bir/effects.rs)" ]; then
  fail "air/src/bir/effects.rs is already modified; refusing to patch over uncommitted work"
fi

mkdir -p "$CTL"
# # the a01 shape, which the narrowing accepts, and the b01 shape, which it must not
cat > "$CTL/__CTL_FLIP.aelys" <<'EOF'
nogc fn peek(r: &Vec<i64>) -> i64 { return (*r)[0] }
fn main() -> i64 { return 0 }
EOF
cat > "$CTL/__CTL_HOLD.aelys" <<'EOF'
nogc fn poke(r: &mut Vec<i64>) -> i64 {
    (*r)[0] = 101
    return 0
}
fn main() -> i64 { return 0 }
EOF

CORPUS_FIX="$WORK/corpus"
mkdir -p "$CORPUS_FIX"
CORPUS="$ROOT/aelys/tests/run3_stage2_positions_tests.rs" FIX="$CORPUS_FIX" \
  python3 "$ROOT/scripts/extract_corpus.py" || fail "the corpus fixtures did not generate"

CORPUS_FILES="$WORK/corpus_files.txt"
LOCAL_FILES="$WORK/local_files.txt"
FILES="$WORK/files.txt"
( cd "$CORPUS_FIX" && find . -name '*.aelys' -print | sed "s#^\.#$CORPUS_FIX#" ) \
  | LC_ALL=C sort > "$CORPUS_FILES"
echo "$CTL/__CTL_FLIP.aelys" >> "$CORPUS_FILES"
echo "$CTL/__CTL_HOLD.aelys" >> "$CORPUS_FILES"
( cd "$ROOT" && find . -path ./target -prune -o -name '*.aelys' -print ) | LC_ALL=C sort > "$LOCAL_FILES"
cat "$CORPUS_FILES" "$LOCAL_FILES" > "$FILES"
NC=$(wc -l < "$CORPUS_FILES")
NL=$(wc -l < "$LOCAL_FILES")
N=$(wc -l < "$FILES")
[ "$NC" -ge 110 ] || fail "only $NC reproducible fixtures; the corpus generates more than that"
echo "== population: $NC from the tracked corpus (reproducible), $NL local .aelys files =="

echo "== building the candidate from the current source =="
( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1 || fail "candidate build failed"
cp "$ROOT/target/release/aelys-cli" "$CAND_CLI"

if [ -n "${AELYS_BASE_CLI:-}" ]; then
  echo "== baseline supplied: $AELYS_BASE_CLI =="
  [ -x "$AELYS_BASE_CLI" ] || fail "no executable at $AELYS_BASE_CLI"
  cp "$AELYS_BASE_CLI" "$BASE_CLI"
else
  echo "== building the baseline from $BASE_REV =="
  cp "$EFFECTS" "$BACKUP"
  ( cd "$ROOT" && git show "$BASE_REV:air/src/bir/effects.rs" ) > "$EFFECTS" \
    || fail "no air/src/bir/effects.rs at $BASE_REV"
  ( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1 \
    || fail "baseline build failed"
  cp "$ROOT/target/release/aelys-cli" "$BASE_CLI"
  restore
  rm -f "$BACKUP"
fi

verdict() { # <cli> <file> -> ACCEPT | the sorted set of error codes | UNCLASSIFIED
  local out rc codes
  out=$(cd "$ROOT" && LC_ALL=C "$1" compile --no-color --emit-air -O0 "$2" 2>&1)
  rc=$?
  if [ "$rc" -eq 0 ]; then echo ACCEPT; return; fi
  codes=$(printf '%s' "$out" | LC_ALL=C grep -o 'error\[E[0-9][0-9][0-9][0-9]\]' \
    | LC_ALL=C sed 's/error\[\(E[0-9]*\)\]/\1/' | LC_ALL=C sort -u | tr '\n' ',' | sed 's/,$//')
  if [ -n "$codes" ]; then echo "$codes"; else echo UNCLASSIFIED; fi
}

echo "== sweeping $N files =="
same=0; flipped=0; unclassified=0
corpus_flipped=0
ctl_flip=none; ctl_hold=none
while IFS= read -r f; do
  b=$(verdict "$BASE_CLI" "$f")
  c=$(verdict "$CAND_CLI" "$f")
  k=$(classify "$b" "$c")
  case "$k" in
    same) same=$((same + 1)) ;;
    flipped)
      flipped=$((flipped + 1)); echo "   FLIP $f: $b -> $c"
      case "$f" in "$CORPUS_FIX"/*|"$CTL"/*) corpus_flipped=$((corpus_flipped + 1)) ;; esac
      ;;
    unclassified) unclassified=$((unclassified + 1)); echo "   UNCLASSIFIED $f: $b -> $c" ;;
  esac
  case "$f" in
    *__CTL_FLIP.aelys) ctl_flip="$k" ;;
    *__CTL_HOLD.aelys) ctl_hold="$k" ;;
  esac
done < "$FILES"

echo "files=$N (corpus $NC + local $NL) same=$same flipped=$flipped unclassified=$unclassified"
echo "corpus_flipped=$corpus_flipped   <- the half that reproduces on any checkout"
[ $((same + flipped + unclassified)) -eq "$N" ] || fail "the buckets do not partition $N files"
[ "$unclassified" -eq 0 ] || fail "$unclassified file(s) produced no verdict at all"
[ "$ctl_hold" = same ] || fail "the planted hold control moved ($ctl_hold); the sweep is widening"
# # the magnitude is asserted on the reproducible half; the local half is reported, never asserted,
[ "$corpus_flipped" -eq "$EXPECT_FLIPPED" ] \
  || fail "corpus_flipped=$corpus_flipped, expected exactly $EXPECT_FLIPPED"
if [ "$EXPECT_FLIPPED" -gt 0 ]; then
  [ "$ctl_flip" = flipped ] \
    || fail "the planted flip control did not move ($ctl_flip); the sweep proves nothing"
fi
echo "OK: $NC reproducible fixtures and $NL local files swept, $corpus_flipped corpus flip(s), \
$flipped in total, the hold control held"
