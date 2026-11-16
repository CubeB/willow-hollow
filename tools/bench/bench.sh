#!/bin/bash
# Willow Hollow frame-time benchmark: the official before/after suite.
#
#   tools/bench/bench.sh [scene ...]   run the named scenes (default: all in scenes.txt), REPS times each
#
# Environment:
#   GAME     game binary (default: target/release/game.exe of this checkout)
#   OUT      directory for logs, per-frame CSVs and results (default: ./bench-<timestamp>)
#   REPS     repetitions per scene (default 2; use 3 for a sign-off)
#   FRAMES   frames per run for static scenes (default 6000, so 30 s of warm frames remain);
#            drift scenes use DFRAMES (default 3000)
#   SIZE     window size (default 1920x1080; 960x540 with LAPTOP=1)
#   EXTRA    extra game arguments for every run (e.g. "--radius 16")
#   LAPTOP   1 = the laptop proxy: the game held to 8 logical CPUs (AFFINITY=0xFF) at
#            960x540, so that the run is bound by the CPU rather than the GPU. The game
#            still sizes its chunk pool from all the machine's CPUs (Rust's
#            available_parallelism ignores the mask on Windows), so it runs more workers
#            on those 8 than an 8-CPU laptop would.
#   AFFINITY a hex mask of the logical CPUs the game may run on (default: all)
#   WAIT     how many seconds to wait for other load to go before giving up (default 600)
#   MAXCPU   the machine's CPU use, in percent, that counts as other load (default 15)
#   FORCE    1 = run even with other load, and say so in meta.txt (numbers not comparable)
#
# Every run is under `timeout 300` and writes its full log to a file. The per-frame CSV
# (WH_FRAME_CSV) gives <scene>_r<k>.csv, from which per-pass and per-system averages,
# 1% lows, hitch counts and worst frames are computed.
#
# Other load makes the numbers meaningless, so before and after each run the machine is
# checked for cargo, rustc, the linker, another game.exe or mapgen, and for CPU use above
# MAXCPU. If any is there before a run and does not go within WAIT seconds, or turned up
# during a run, the bench stops with an error rather than carry on.
#
# Results: $OUT/results.csv (one row per run) and a table on stdout (median of the reps).
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
GAME="${GAME:-$root/target/release/game.exe}"
OUT="${OUT:-$PWD/bench-$(date +%Y%m%d-%H%M%S)}"
REPS="${REPS:-2}"
FRAMES="${FRAMES:-6000}"
DFRAMES="${DFRAMES:-3000}"
LAPTOP="${LAPTOP:-0}"
if [ "$LAPTOP" = 1 ]; then
  SIZE="${SIZE:-960x540}"
  AFFINITY="${AFFINITY:-0xFF}"
fi
SIZE="${SIZE:-1920x1080}"
AFFINITY="${AFFINITY:-}"
WAIT="${WAIT:-600}"
MAXCPU="${MAXCPU:-15}"
FORCE="${FORCE:-0}"
PY="${PY:-python}"
# The Windows Store alias answers to "python" without being one, so try it.
"$PY" -c "" >/dev/null 2>&1 < /dev/null || PY=/d/tools/python312/python
mkdir -p "$OUT"

busy() {
  # Anything else that would compete for the CPU or GPU: by name, then by the CPU use of
  # the whole machine over the last second.
  local names cpu
  names="$(tasklist 2>/dev/null | grep -iE '^(cargo|rustc|link|game|mapgen|bench)\.exe' | awk '{print $1}' | sort | uniq -c | awk '{print $2 "*" $1}' | tr '\n' ' ')"
  cpu="$(powershell -NoProfile -Command '[int](Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average' </dev/null 2>/dev/null | tr -d '\r')"
  if [ -n "$cpu" ] && [ "$cpu" -gt "$MAXCPU" ] 2>/dev/null; then
    names="${names}cpu=${cpu}%"
  fi
  echo "$names" | sed 's/ *$//'
}

# Waits up to WAIT seconds for the machine to be quiet. Prints what is still there, if anything.
quiet() {
  local load waited=0
  load="$(busy)"
  while [ -n "$load" ] && [ "$waited" -lt "$WAIT" ]; do
    sleep 10
    waited=$((waited + 10))
    load="$(busy)"
  done
  echo "$load"
}

fail() {
  echo "bench: $*" | tee -a "$OUT/meta.txt" >&2
  exit 3
}

# Runs the game with the given log file and arguments, held to AFFINITY if that is set.
# The mask is set straight after the process starts: it applies to every thread the game
# has then and every one it starts later.
run_game() {
  local log=$1; shift
  if [ -z "$AFFINITY" ]; then
    timeout 300 "$GAME" "$@" < /dev/null > "$log" 2>&1
    return $?
  fi
  timeout 300 "$GAME" "$@" < /dev/null > "$log" 2>&1 &
  local tpid=$! winpid="" rc
  for _ in $(seq 1 100); do
    winpid="$(ps | awk -v p="$tpid" '$2 == p { print $4 }' | head -1)"
    [ -n "$winpid" ] && break
    sleep 0.05
  done
  if [ -z "$winpid" ] || ! powershell -NoProfile -Command "(Get-Process -Id $winpid).ProcessorAffinity = $AFFINITY" </dev/null >/dev/null 2>&1; then
    echo "could not set the affinity of game.exe (pid ${winpid:-?})" >> "$log.affinity"
  fi
  wait "$tpid"
  rc=$?
  return $rc
}

want=" $* "
{
  echo "binary: $GAME"
  echo "commit: $(cd "$(dirname "$GAME")" && git rev-parse --short HEAD 2>/dev/null)"
  echo "reps $REPS, frames $FRAMES/$DFRAMES, size $SIZE, affinity ${AFFINITY:-all}, laptop $LAPTOP, extra '${EXTRA:-}', started $(date)"
  [ "$FORCE" = 1 ] && echo "FORCE=1: run whatever the load, so these numbers are not for comparison"
} | tee "$OUT/meta.txt"
: > "$OUT/load.txt"
while IFS='|' read -r name kind args; do
  args="${args%$'\r'}"  # a checkout may have given the list CRLF line endings
  case "$name" in ''|'#'*) continue ;; esac
  if [ "$#" -gt 0 ] && [[ "$want" != *" $name "* ]]; then continue; fi
  n=$FRAMES; [ "$kind" = D ] && n=$DFRAMES
  for k in $(seq 1 "$REPS"); do
    load="$(quiet)"
    echo "$name r$k before: ${load:-none}" >> "$OUT/load.txt"
    if [ -n "$load" ] && [ "$FORCE" != 1 ]; then
      fail "other load before $name r$k ($load), still there after ${WAIT} s; stopping (FORCE=1 to run anyway)"
    fi
    printf '%-14s r%s ... ' "$name" "$k"
    # shellcheck disable=SC2086
    WH_FRAME_CSV="$OUT/${name}_r$k.csv" run_game "$OUT/${name}_r$k.log" --seed 1 --no-audio --no-vsync --size "$SIZE" \
      --day-minutes 0 --frames "$n" $args ${EXTRA:-}
    rc=$?
    # What started while the game ran spoiled it too. The CPU can stay busy for a few
    # seconds after the game quits (its threads winding down, a virus scan of the files it
    # wrote), so only load that is still there after three looks 5 s apart counts.
    load="$(busy)"
    for _ in 1 2; do
      [ -z "$load" ] && break
      sleep 5
      load="$(busy)"
    done
    echo "$name r$k after: ${load:-none}" >> "$OUT/load.txt"
    echo "rc=$rc"
    if [ -n "$load" ] && [ "$FORCE" != 1 ]; then
      mv "$OUT/${name}_r$k.log" "$OUT/${name}_r$k.log.spoilt"
      mv "$OUT/${name}_r$k.csv" "$OUT/${name}_r$k.csv.spoilt" 2>/dev/null
      fail "other load turned up during $name r$k ($load); its log is kept as .spoilt, stopping"
    fi
    sleep 3
  done
done < "$here/scenes.txt"

"$PY" "$here/summarize.py" "$OUT"
