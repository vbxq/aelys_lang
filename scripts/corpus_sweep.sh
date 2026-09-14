#!/usr/bin/env bash
set -u
ROOT="${AELYS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
MODE="$1"; shift

COW=$(mktemp -d)
FIX="$COW" python3 "$ROOT/scripts/cow_probes.py" || { echo "corpus_sweep.sh: cow probes failed" >&2; exit 5; }
COW_N=$(ls "$COW"/*.aelys 2>/dev/null | wc -l)
[ "$COW_N" -ge 18 ] || { echo "corpus_sweep.sh: only $COW_N cow probes generated" >&2; exit 5; }
cleanup() { python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)' "$COW"; }
trap 'cleanup' EXIT INT TERM

is_cow() { case "$1" in "$COW"/*) return 0 ;; *) return 1 ;; esac; }

# # a sweep that swept nothing, or whose every leg failed to compile, must not read as clean
corpus_guard() { # <files> <compile_fail_legs> <total_legs>
  if [ "$1" -lt 300 ]; then
    echo "corpus_sweep.sh: swept only $1 files, expected >300 -- check ROOT" >&2
    exit 3
  fi
  if [ "$2" -ge "$3" ]; then
    echo "corpus_sweep.sh: every leg failed to compile ($2/$3) -- the CLI path is wrong" >&2
    exit 4
  fi
}

abspath() {
  local p="$1"
  [ -x "$p" ] || { echo "corpus_sweep: not executable: $p" >&2; exit 2; }
  ( cd "$(dirname "$p")" && printf '%s/%s\n' "$(pwd)" "$(basename "$p")" )
}

run_one() { # <cli> <src> <level> <workdir> -> "exit|stdout" or "COMPILEFAIL|<code>"
  local cli="$1" src="$2" lvl="$3" wd="$4"
  local b; b=$(basename "$src" .aelys)
  cp "$src" "$wd/$b.aelys"
  # the previous level's binary would be run as this level's answer
  command rm -f -- "$wd/$b"
  local err code
  err=$(cd "$wd" && "$cli" compile "$b.aelys" "$lvl" --no-color 2>&1 >/dev/null) || {
    code=$(printf '%s' "$err" | command grep -oE 'error\[E[0-9]{4}\]' | head -1)
    echo "COMPILEFAIL|${code:-nocode}"; return
  }
  local out rc
  out=$(cd "$wd" && timeout 25 "./$b" 2>/dev/null); rc=$?
  echo "$rc|$(echo "$out" | head -c 200000 | tr '\n' '~')"
}

if [ "$MODE" = diff ]; then
  OLD=$(abspath "$1"); NEW=$(abspath "$2")
  WA=$(mktemp -d); WB=$(mktemp -d); n=0; d=0; nd=0; cf=0; cd_=0
  for f in $ROOT/tests_e2e/*.aelys $ROOT/torture/*.aelys "$COW"/*.aelys; do
    n=$((n+1))
    a=$(run_one "$OLD" "$f" -O0 "$WA")
    b=$(run_one "$NEW" "$f" -O0 "$WB")
    case "$a" in COMPILEFAIL*) cf=$((cf+1)) ;; esac
    case "$b" in COMPILEFAIL*) cf=$((cf+1)) ;; esac
    if [ "$a" != "$b" ]; then
      b2=$(cd "$WB" && timeout 25 "./$(basename $f .aelys)" 2>/dev/null; echo "rc=$?")
      b3=$(cd "$WB" && timeout 25 "./$(basename $f .aelys)" 2>/dev/null; echo "rc=$?")
      if [ "$b2" != "$b3" ]; then nd=$((nd+1)); echo "NONDETERMINISTIC $(basename $f .aelys)"; continue; fi
      d=$((d+1)); is_cow "$f" && cd_=$((cd_+1))
      echo "DIVERGENT $(basename $f .aelys)"; echo "   old: $a"; echo "   new: $b"
    fi
  done
  cleanup_wa() { python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)' "$1"; }
  cleanup_wa "$WA"; cleanup_wa "$WB"
  echo "files=$n (local $((n - COW_N)) + cow $COW_N) divergent=$d nondeterministic=$nd compile-fail-legs=$cf"
  echo "cow_divergent=$cd_   <- the half that exists on any checkout and can express the CoW class"
  corpus_guard "$n" "$cf" $((n * 2))
elif [ "$MODE" = xo ]; then
  CLI=$(abspath "$1"); W=$(mktemp -d); n=0; d=0; nd=0; cf=0; cd_=0
  for f in $ROOT/tests_e2e/*.aelys $ROOT/torture/*.aelys "$COW"/*.aelys; do
    n=$((n+1)); sigs=(); levels=()
    for O in -O0 -O1 -O2 -O3; do
      r=$(run_one "$CLI" "$f" "$O" "$W")
      # a refusal is a verdict, so it is compared and not dropped: dropping it is what hid the class
      sigs+=("$r"); levels+=("$O")
      case "$r" in COMPILEFAIL*) cf=$((cf+1)) ;; esac
    done
    if [ ${#sigs[@]} -gt 1 ]; then
      first="${sigs[0]}"
      for i in "${!sigs[@]}"; do
        if [ "${sigs[$i]}" != "$first" ]; then
          b=$(basename $f .aelys)
          r2=$(cd "$W" && timeout 25 "./$b" 2>/dev/null; echo "rc=$?")
          r3=$(cd "$W" && timeout 25 "./$b" 2>/dev/null; echo "rc=$?")
          if [ "$r2" != "$r3" ]; then nd=$((nd+1)); echo "NONDETERMINISTIC $b"; break; fi
          d=$((d+1)); is_cow "$f" && cd_=$((cd_+1))
          echo "DIVERGENT $b"
          for j in "${!sigs[@]}"; do echo "   ${levels[$j]}: ${sigs[$j]}"; done
          break
        fi
      done
    fi
  done
  python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)' "$W"
  echo "files=$n (local $((n - COW_N)) + cow $COW_N) divergent=$d nondeterministic=$nd compile-fail-legs=$cf"
  echo "cow_divergent=$cd_   <- the half that exists on any checkout and can express the CoW class"
  corpus_guard "$n" "$cf" $((n * 4))
else
  echo "corpus_sweep.sh: unknown mode '$MODE' (expected 'diff' or 'xo')" >&2
  exit 2
fi
