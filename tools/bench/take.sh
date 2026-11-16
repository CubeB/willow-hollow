#!/bin/bash
# Screenshots of the benchmark scenes, for checking that a change leaves the picture alone.
#
#   tools/bench/take.sh OUTDIR [scene ...]   one PNG and one log per scene (default: all in scenes.txt)
#   python tools/bench/imgdiff.py BEFORE AFTER   then compares two such directories
#
# Environment:
#   GAME     game binary (default: target/release/game.exe of this checkout)
#   SIZE     image size (default 1920x1080, as the bench runs)
#   AMBIENT  seconds of ambient life before each shot (default 0: see below)
#   EXTRA    extra game arguments for every shot
#
# The same build shot twice gives the same picture only with no ambient life simulated
# first: the leaves, flowers and birds spawned in those seconds are placed from the
# frame times, which differ from run to run. So AMBIENT defaults to 0. With AMBIENT set
# above 0, take the "before" twice to see how much the shots differ on their own.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
GAME="${GAME:-$root/target/release/game.exe}"
SIZE="${SIZE:-1920x1080}"
AMBIENT="${AMBIENT:-0}"
out="${1:?usage: take.sh OUTDIR [scene ...]}"
shift
mkdir -p "$out"
want=" $* "
while IFS='|' read -r name kind args; do
  case "$name" in ''|'#'*) continue ;; esac
  if [ "$#" -gt 0 ] && [[ "$want" != *" $name "* ]]; then continue; fi
  # shellcheck disable=SC2086
  timeout 300 "$GAME" --seed 1 --no-audio --size "$SIZE" --day-minutes 0 --ambient "$AMBIENT" \
    --screenshot "$out/$name.png" $args ${EXTRA:-} < /dev/null > "$out/$name.log" 2>&1
  echo "$name rc=$?"
done < "$here/scenes.txt"
