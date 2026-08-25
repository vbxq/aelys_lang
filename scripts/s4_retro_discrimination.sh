# # by construction and a fixture cannot drift from the row it stands for.
set -u
ROOT="${AELYS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
EFFECTS="$ROOT/air/src/bir/effects.rs"
CORPUS="$ROOT/aelys/tests/run3_stage2_positions_tests.rs"
WORK="$(mktemp -d)"
BACKUP="$WORK/effects.rs.orig"
FIX="$WORK/fixtures"
BASE_CLI="$WORK/aelys-cli-base"

restore() {
  if [ -f "$BACKUP" ]; then
    cp "$BACKUP" "$EFFECTS"
    ( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1
  fi
}
trap 'restore' EXIT INT TERM

fail() { echo "s4_retro_discrimination.sh: $*" >&2; exit 1; }


APPLICABLE_POST="KP K9 K1 K2 K3 K4 K5 K6 K7 KI K9K5"
CONTROL_POST="A01:ACCEPT B01:E0727"

EXPECT_KP="\
A01:E0727 A02:E0727 A03:E0727 A04:E0727 A05:E0727 A06:E0727 A07:E0727 A08:E0727 A09:E0727 \
A10:E0727 A11:E0727 A12:E0727 A13:E0727 A14:E0727 A15:E0727 A16:E0727 A17:E0727 A18:E0727 \
A19:E0727 A20:E0727 A21:E0727 A22:E0727 A23:E0727 A24:E0727 A25:E0727 A26:E0727 A27:E0727 \
A28:E0727 A29:E0727 A30:E0727 A31:E0727 A32:E0727 A33:E0727 A34:E0727 A35:E0727 B28:E0727 \
C21:E0714,E0727 C22:E0714,E0727"

EXPECT_K9="B03:E0901 B04:ACCEPT B05:E0901 B06:E0901 B17:ACCEPT B18:ACCEPT B22:E0412 B24:E0412"

EXPECT_K1="A02:E0727 A03:E0727 A17:E0727 A32:E0727 A33:E0727 A34:E0727 B28:E0727"

EXPECT_K2="\
A01:E0727 A05:E0727 A06:E0727 A07:E0727 A08:E0727 A09:E0727 A10:E0727 A11:E0727 A12:E0727 \
A13:E0727 A14:E0727 A15:E0727 A16:E0727 A18:E0727 A19:E0727 A20:E0727 A21:E0727 A22:E0727 \
A23:E0727 A24:E0727 A25:E0727 A26:E0727 A27:E0727 A28:E0727 A29:E0727 A30:E0727 A31:E0727 \
A33:E0727 A34:E0727 A35:E0727 C21:E0714,E0727 C22:E0714,E0727"

EXPECT_K3="A04:E0727 A17:E0727"

EXPECT_K4="A22:E0727"

# # of what refuses b20 and b25 and no fence renders behind it
EXPECT_K5="B01:ACCEPT B02:ACCEPT B16:ACCEPT B20:ACCEPT B21:ACCEPT B25:ACCEPT"

EXPECT_K6="\
A01:E0727 A02:E0727 A03:E0727 A04:E0727 A05:E0727 A06:E0727 A07:E0727 A08:E0727 A09:E0727 \
A10:E0727 A11:E0727 A12:E0727 A13:E0727 A14:E0727 A15:E0727 A16:E0727 A17:E0727 A18:E0727 \
A19:E0727 A20:E0727 A21:E0727 A22:E0727 A23:E0727 A24:E0727 A25:E0727 A26:E0727 A27:E0727 \
A28:E0727 A29:E0727 A30:E0727 A31:E0727 A33:E0727 A34:E0727 A35:E0727 B28:E0727 \
C21:E0714,E0727 C22:E0714,E0727"

# # a fence swap and no acceptance: the condition is hygiene, not soundness
EXPECT_K7="B24:E0412"

EXPECT_KI=""

EXPECT_K9K5="\
B01:ACCEPT B02:ACCEPT B03:E0901 B04:ACCEPT B05:E0901 B06:E0901 B16:ACCEPT B17:ACCEPT \
B18:ACCEPT B20:ACCEPT B21:ACCEPT B22:E0412 B24:E0412 B25:ACCEPT"

APPLICABLE_PRE="K9PRE"
EXPECT_K9PRE="\
A01:ACCEPT A02:ACCEPT A03:ACCEPT A04:ACCEPT A05:ACCEPT A06:ACCEPT A07:ACCEPT A08:ACCEPT \
A09:ACCEPT A10:ACCEPT A11:ACCEPT A12:ACCEPT A13:ACCEPT A14:ACCEPT A15:ACCEPT A16:ACCEPT \
A17:ACCEPT A18:ACCEPT A19:ACCEPT A20:ACCEPT A21:ACCEPT A22:ACCEPT A23:ACCEPT A24:ACCEPT \
A25:ACCEPT A26:ACCEPT A27:ACCEPT A28:ACCEPT A29:ACCEPT A30:ACCEPT A31:ACCEPT A32:ACCEPT \
B01:ACCEPT B02:ACCEPT B03:E0901 B04:ACCEPT B05:E0901 B06:E0901 B16:ACCEPT B17:ACCEPT \
B18:ACCEPT B20:E0429 B21:ACCEPT B22:E0412 B24:E0412 B25:E0429 C21:E0714 C22:E0714"

# # a stale or already-patched target/release is the failure this run has hit most often
CONTROL_PRE="A01:E0727 B01:E0727"

HELD_AT_BASE="\
B26:E0727 B27:E0727 B29:E0416 B30:E0416 B31:E0416 B32:E0416 B33:E0416 B34:E0416 B35:E0416 \
B36:E0416"


if [ -n "$(cd "$ROOT" && git status --porcelain -- air/src/bir/effects.rs)" ]; then
  fail "air/src/bir/effects.rs is already modified; refusing to patch over uncommitted work"
fi

mkdir -p "$FIX"

echo "== generating fixtures from the corpus =="
CORPUS="$CORPUS" FIX="$FIX" python3 "$ROOT/scripts/extract_corpus.py" || exit 1
IDS=$(cd "$FIX" && ls ./*.aelys 2>/dev/null | sed 's#\./##; s/\.aelys$//' | LC_ALL=C sort | tr '\n' ' ')
ROWS=$(echo "$IDS" | wc -w)
[ "$ROWS" -ge 108 ] || fail "only $ROWS fixtures; the corpus has more rows than that"

# # a fixture that is not there must never look like a compiler verdict: a missing file otherwise
verdict() { # <cli> <id> -> ACCEPT | the sorted set of error codes | UNCLASSIFIED
  local cli="$1" id="$2" out rc codes
  if [ ! -s "$FIX/$id.aelys" ]; then echo UNCLASSIFIED; return; fi
  out=$(cd "$FIX" && LC_ALL=C "$cli" compile --no-color --emit-air -O0 "$id.aelys" 2>&1)
  rc=$?
  if [ "$rc" -eq 0 ]; then echo ACCEPT; return; fi
  codes=$(printf '%s' "$out" | LC_ALL=C grep -o 'error\[E[0-9][0-9][0-9][0-9]\]' \
    | LC_ALL=C sed 's/error\[\(E[0-9]*\)\]/\1/' | LC_ALL=C sort -u | tr '\n' ',' | sed 's/,$//')
  if [ -n "$codes" ]; then echo "$codes"; else echo UNCLASSIFIED; fi
}

sweep() { # <cli> <outfile>
  local id
  : > "$2"
  for id in $IDS; do echo "$id $(verdict "$1" "$id")" >> "$2"; done
  if LC_ALL=C grep -q ' UNCLASSIFIED$' "$2"; then
    LC_ALL=C grep ' UNCLASSIFIED$' "$2" >&2
    fail "a fixture produced no diagnostic code and no acceptance; that is not a verdict"
  fi
}

echo "== building the base compiler from the current source =="
( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1 || fail "the base build failed"
cp "$ROOT/target/release/aelys-cli" "$BASE_CLI"

echo "== which clauses the current source admits =="
APPLICABLE=$(AELYS_EFFECTS="$EFFECTS" python3 "$ROOT/scripts/over_narrow_s4.py" --applicable \
  | LC_ALL=C sort | tr '\n' ' ')
APPLICABLE="${APPLICABLE% }"
sorted() { echo "$1" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' '; }
POST=$(sorted "$APPLICABLE_POST"); POST="${POST% }"
PRE=$(sorted "$APPLICABLE_PRE"); PRE="${PRE% }"
if [ "$APPLICABLE" = "$POST" ]; then
  PHASE=post; CONTROL="$CONTROL_POST"
elif [ "$APPLICABLE" = "$PRE" ]; then
  PHASE=pre; CONTROL="$CONTROL_PRE"
else
  fail "the clauses this source admits match neither declared phase
  admitted: $APPLICABLE
  post:     $POST
  pre:      $PRE
  an ablation that silently stops applying reads as a clean matrix"
fi
echo "   $PHASE: $APPLICABLE"

echo "== control: the base build's own verdicts, before any ablation is built =="
for c in $CONTROL; do
  id="${c%%:*}"; want="${c##*:}"
  got=$(verdict "$BASE_CLI" "$id")
  [ "$got" = "$want" ] || fail "$id is $got under the base build, expected $want; \
target/release/aelys-cli is stale or already patched"
  echo "   $id = $got"
done

sweep "$BASE_CLI" "$WORK/base.txt"
echo "   $ROWS rows swept, unclassified=0"

echo "== the rows this matrix no longer discriminates, pinned against the base build =="
for c in $HELD_AT_BASE; do
  id="${c%%:*}"; want="${c##*:}"
  got=$(verdict "$BASE_CLI" "$id")
  [ "$got" = "$want" ] || fail "$id is $got at the base build, expected $want; this row left the \
ablation matrix because a repair closed it, so a change here is a REGRESSION and not a re-measure"
done
echo "   $(echo "$HELD_AT_BASE" | wc -w) row(s) held, by E0727 and E0416 rather than by an edge"

echo "== measuring, one build per clause =="
cp "$EFFECTS" "$BACKUP"
failures=0
for name in $APPLICABLE; do
  cp "$BACKUP" "$EFFECTS"
  AELYS_EFFECTS="$EFFECTS" AELYS_ABLATION="$name" python3 "$ROOT/scripts/over_narrow_s4.py" \
    || fail "$name did not apply"
  ( cd "$ROOT" && cargo build --release --bin aelys-cli ) >/dev/null 2>&1 \
    || fail "the $name build failed"
  cp "$ROOT/target/release/aelys-cli" "$WORK/aelys-cli-$name"

  if [ "$name" = K9 ]; then
    PRE=$(cd "$ROOT" && cargo test -p aelys --test nogc_run2_effects_tests 2>&1 \
      | LC_ALL=C grep -m1 '^test result:')
    echo "   nogc_run2_effects_tests under $name: $PRE"
    case "$PRE" in
      *"11 passed; 2 failed"*) ;;
      *) fail "expected 11 passed / 2 failed from nogc_run2_effects_tests under $name, got: $PRE" ;;
    esac
  fi
  cp "$BACKUP" "$EFFECTS"

  sweep "$WORK/aelys-cli-$name" "$WORK/$name.txt"
  eval "want=\${EXPECT_$name}"
  got=$(LC_ALL=C join "$WORK/base.txt" "$WORK/$name.txt" \
    | awk '$2 != $3 { printf "%s:%s ", $1, $3 }')
  got="${got% }"
  wantn=$(echo "$want" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' ')
  gotn=$(echo "$got" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' ')
  n=$(echo "$got" | wc -w)
  echo "$got" | tr ' ' '\n' | LC_ALL=C sed 's/:.*//' | LC_ALL=C grep -v '^$' \
    | LC_ALL=C sort > "$WORK/moved.$name"
  if [ "$wantn" = "$gotn" ]; then
    echo "   $name moved $n row(s), exactly as declared"
  else
    echo "   $name MISMATCH"
    echo "     declared: $wantn"
    echo "     measured: $gotn"
    failures=$((failures + 1))
  fi
done

if [ "$PHASE" = post ] && [ "$failures" -eq 0 ]; then
  echo "== the two set relations the matrix has to satisfy =="
  LC_ALL=C sort -u "$WORK/moved.K1" "$WORK/moved.K2" "$WORK/moved.K6" \
    | LC_ALL=C grep '^A' > "$WORK/edge_union"
  LC_ALL=C grep '^A' "$WORK/base.txt" | awk '{print $1}' | LC_ALL=C sort > "$WORK/group_a"
  if LC_ALL=C diff -q "$WORK/edge_union" "$WORK/group_a" >/dev/null; then
    echo "   K1 u K2 u K6 covers all $(wc -l < "$WORK/group_a") group a rows"
  else
    echo "   group a rows carried by no measured edge:"
    LC_ALL=C comm -13 "$WORK/edge_union" "$WORK/group_a" | sed 's/^/     /'
    failures=$((failures + 1))
  fi
  LC_ALL=C sort -u "$WORK/moved.K9" "$WORK/moved.K5" > "$WORK/single_union"
  only_double=$(LC_ALL=C comm -13 "$WORK/single_union" "$WORK/moved.K9K5" | wc -l)
  if [ "$only_double" -eq 0 ]; then
    echo "   rows moving only under K9 and K5 together: 0"
  else
    echo "   $only_double row(s) move only under the double ablation, so no per-clause \
measurement can show them defended"
    failures=$((failures + 1))
  fi
fi

restore
rm -f "$BACKUP"

for c in $CONTROL; do
  id="${c%%:*}"; want="${c##*:}"
  got=$(verdict "$ROOT/target/release/aelys-cli" "$id")
  [ "$got" = "$want" ] || fail "target/release/aelys-cli was not restored to the base build \
($id is $got, expected $want)"
done

[ "$failures" -eq 0 ] || fail "$failures ablation(s) did not move what this guard declares"
echo "OK: $(echo "$APPLICABLE" | wc -w) ablation(s), every moved set exact, unclassified=0, \
target/release restored and discriminating"
