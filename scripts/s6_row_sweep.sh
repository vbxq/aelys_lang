set -u
CLI="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
FIX="$2"; OUT="$3"
wipe() { python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)' "$1"; }
: > "$OUT"
for f in "$FIX"/*.aelys; do
  id="$(basename "$f" .aelys)"
  work="$FIX/.run"; wipe "$work"; mkdir -p "$work"
  cp "$f" "$work/$id.aelys"
  out=$(cd "$work" && timeout 120 "$CLI" compile --no-color -O0 "$id.aelys" 2>&1); rc=$?
  if [ "$rc" -ne 0 ]; then
    codes=$(printf '%s' "$out" | LC_ALL=C grep -o 'error\[E[0-9][0-9][0-9][0-9]\]' \
      | LC_ALL=C sed 's/error\[\(E[0-9]*\)\]/\1/' | LC_ALL=C sort -u | tr '\n' ',' | sed 's/,$//')
    echo "$id ${codes:-UNCLASSIFIED}" >> "$OUT"
    continue
  fi
  if [ ! -x "$work/$id" ]; then echo "$id NOBINARY" >> "$OUT"; continue; fi
  r=$(cd "$work" && AELYS_RC_STATS=1 timeout 60 "./$id" 2>/dev/null); prc=$?
  s=$(cd "$work" && AELYS_RC_STATS=1 timeout 60 "./$id" 2>&1 >/dev/null \
    | LC_ALL=C grep -m1 -o 'allocs=[0-9]* frees=[0-9]*' | tr ' ' '/')
  echo "$id RUN:$(printf '%s' "$r" | tr '\n' '/')|exit=$prc|${s:-nostats}" >> "$OUT"
done
wipe "$FIX/.run"
LC_ALL=C sort -o "$OUT" "$OUT"
if LC_ALL=C grep -q ' UNCLASSIFIED$' "$OUT"; then
  echo "s6_row_sweep.sh: a fixture produced no diagnostic code and no acceptance" >&2
  LC_ALL=C grep ' UNCLASSIFIED$' "$OUT" >&2
  exit 1
fi
