#!/bin/bash
# Screenshots of the benchmark scenes, for checking that a change leaves the picture alone.
#
#   tools/bench/take.sh OUTDIR [scene ...]   one PNG and one log per scene (default: all in scenes.txt)
#   python tools/bench/imgdiff.py BEFORE AFTER   then compares two such directories
#
# Environment:
#   GAME     game binary (default: target/release/game.exe of this checkout)
#   SIZE     image size (default 1920x1080, as the bench runs)
#   AMBIENT  seconds of ambient life before each shot (default 8, as a plain screenshot)
#   CLOCK    the renderer's clock for the shot, in seconds (default 10)
#   EXTRA    extra game arguments for every shot
#
# A plain screenshot is drawn at whatever time its loading took, so the leaves sway and
# the water moves by a different amount each run, and the same build shot twice differs
# by up to 200 in a channel. CLOCK holds that time still (VOXEL_CLOCK), and the shots
# repeat exactly; compare only shots taken with the same CLOCK.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
GAME="${GAME:-$root/target/release/game.exe}"
SIZE="${SIZE:-1920x1080}"
AMBIENT="${AMBIENT:-8}"
CLOCK="${CLOCK:-10}"
out="${1:?usage: take.sh OUTDIR [scene ...]}"
shift
mkdir -p "$out"
want=" $* "
while IFS='|' read -r name kind args; do
  args="${args%$'\r'}"  # a checkout may have given the list CRLF line endings
  case "$name" in ''|'#'*) continue ;; esac
  if [ "$#" -gt 0 ] && [[ "$want" != *" $name "* ]]; then continue; fi
  # shellcheck disable=SC2086
  VOXEL_CLOCK="$CLOCK" timeout 300 "$GAME" --seed 1 --no-audio --size "$SIZE" --day-minutes 0 --ambient "$AMBIENT" \
    --screenshot "$out/$name.png" $args ${EXTRA:-} < /dev/null > "$out/$name.log" 2>&1
  echo "$name rc=$?"
done < "$here/scenes.txt"
