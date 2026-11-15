# Setting up to work on Willow Hollow

This is what a new machine needs before it can build, run and change the game. The
[README](../README.md) covers playing it and how the code is laid out.

## What to install

| What | Why | Notes |
|---|---|---|
| Git | To get the code | |
| Rust, stable, through [rustup](https://rustup.rs) | To build everything | The code uses edition 2024, so Rust 1.85 or newer. It is developed on 1.99. There is no pinned toolchain file: `rustup update stable` is enough. |
| MSVC build tools (Windows) | Rust's linker on Windows | Install "Desktop development with C++" from the Visual Studio Build Tools installer. rustup offers to do this for you. |
| A graphics card with Vulkan or DirectX 12 | The renderer uses wgpu | Up-to-date drivers matter more than the card. The target is a laptop GTX 1650 Ti at 60 fps, 1080p. |
| GitHub CLI (`gh`), optional | Opening issues from the terminal | Run `gh auth login` once. |
| Python 3 with Pillow, optional | Only for redrawing the logo (`docs/logo.py`) | `pip install pillow` |

The game has no asset files to download. The only data it reads is the structures drawn
in the structure editor, kept as text in `assets/structures` and compiled in. It has only been run
on Windows so far. Nothing in the code is Windows-only on purpose, but Linux and macOS
are untested.

## Getting and building it

```sh
git clone https://github.com/CubeB/willow-hollow.git
cd willow-hollow
cargo build --release -p game
```

The first build downloads and compiles every dependency and takes several minutes.
After that, a change to the game crate rebuilds in about three minutes with two jobs.

If `cargo` is not found in Git Bash on Windows, add it to the path for that shell:

```sh
export PATH="$USERPROFILE/.cargo/bin:$PATH"
```

### On a machine short of memory or cores

The release build uses thin link-time optimisation and can take a lot of memory. To
keep the machine usable while it builds, limit the number of jobs:

```sh
cargo build --release -p game -j 2
cargo test --release -p game -j 2 -- --test-threads=2
```

### Debug builds

World generation and meshing are far too slow unoptimised, so the workspace optimises
even `dev` builds (see the profiles in `Cargo.toml`). Day to day, use `--release`.

## Running it

```sh
cargo run --release -p game               # a new random world, on foot
cargo run --release -p game -- --seed 1   # world 1, from where you left off
cargo run --release -p game -- --fly      # free camera
cargo run --release -p game -- --help     # every option
```

Settings and your place in each world are kept outside the repository:

- Windows: `%APPDATA%\Willow Hollow`
- Elsewhere: `$XDG_CONFIG_HOME/willow-hollow`, or `~/.config/willow-hollow`

Each world has a few plain text files there: `world-N.txt` (where you are), `pack-N.txt`
(the pack, the hotbar and what has been picked) and `placed-N.txt` (what has been set
down: flowers, sticks, fires), a line to a thing.

Delete `settings.txt` there to go back to the defaults. A hand-edited or damaged file
never stops the game from starting: anything unreadable falls back to its default.

Each world N keeps its own files beside it, all plain text but the map: `world-N.txt`
(where you stand), `pack-N.txt` (what you carry), `journal-N.txt` (the journal: a line
to each find, place, river and note, and the distances gone) and `map-N/` (what the map
has seen). Any of them can be deleted on its own; a line the game cannot read is left
out and the rest still loads.

## Tests

```sh
cargo test --release --workspace
```

The tests check the world as well as the code: drainage reaches the sea, rivers only
grow on the way down, lakes are flat, chunk edges line up. Run them before pushing. The
worldgen tests take about a minute.

One test is known to fail at the time of writing:
`world_tests::overhangs_hang_from_faces_and_agree_across_chunks` in `worldgen` finds a
mismatch across one chunk border. Any other failure is new.

## Checking a change

Most changes are to how the world looks or how fast it runs, and a passing test says
little about either. These are the tools for that.

**One frame, without a window.** Renders a fixed view to a PNG. Take one before the
change and one after, at the same place and hour, and compare them.

```sh
./target/release/game.exe --seed 1 --screenshot out.png --over 42186,30679,60 --look 180,-10 --time 18 --no-audio
```

`--over X,Z,H` puts the camera H above the ground at X,Z. `--look` is yaw and pitch in
degrees: yaw 0 looks toward +x (where the sun rises), 180 toward -x (where it sets).
`--weather` and `--season` hold those for the shot, and `--date` takes a month
(`--date may`) or a fraction of the year from the spring equinox. Each flower is out in
its own months (`Flower::months` in `worldgen/src/features/plants.rs`), so a shot of a
bluebell wood needs `--date may` and one of heather `--date august`; the default date is
the middle of July. PNG files are ignored by git everywhere except under `docs/`, so
shots can be written anywhere.

The windows can be put on a shot too: `--pack` for the pack, and `--journal [PAGE]` for
the journal open at `finds`, `places`, `walk` or `days`. A screenshot reads no save, so
the journal shown is one written up as if after a walk to the sixteen named places
nearest the camera; `--journal-fill N` changes how many, and `--journal-fill 0` shows a
journal with nothing in it yet. In a live run, `RUST_LOG=info,game::journal=debug` logs
each line as the journal writes it.

**Frame timing.** Flies a fixed path and logs where each frame's time went, on the CPU
and on the GPU pass by pass. Compare the numbers before and after a change.

```sh
./target/release/game.exe --seed 1 --frames 2500 --over 42186,30679,60 --look 0,-15 --drift 25 --no-vsync --no-audio --size 1920x1080
```

**The in-game map.** A screenshot can have the corner map or the large map drawn on it,
as the player would have it after walking to where the camera is. `--minimap` draws the
corner map; `--bigmap KM` opens the large map, KM kilometres across (0.16 to 26), with
`--bigmap-turn YAW,PITCH` to turn and tilt it in degrees and `--bigmap-pan X,Z` to move
it X metres east and Z south. Only land that has been seen is on the map, so
`--walked X,Z;X,Z;..` lays a walk through those points (in voxels, as `mapgen --poi`
prints them) to the camera first: what was seen along it is on the map, and the places
passed are visited, which is what puts a name on the things people built. `--time` and
`--season` change the map as they change the world.

```sh
./target/release/game.exe --seed 1 --screenshot map.png --over 36830,35610,2 --time 10 --weather clear --no-audio --bigmap 6 --bigmap-turn 200,40 --walked "21846,27358;26424,28346;30866,30264;33928,37020"
```

The map's models are built on a worker thread, and how long each took is logged at
debug level (`RUST_LOG=warn,game=debug`, "map model at step ..."). The columns of a
model are `step` voxels wide, a power of two from 4 to 512 chosen from the zoom; up to
32 the model is read from the voxel world and has buildings, bridges and tors standing
on it, and up to 16 every tree. Its light is worked out once, as it is built
(`minimap/light.rs`), so drawing it costs no more than drawing its triangles: check
that with a timed run with the corner map on (the default), and with `--bigmap KM`
added, which keeps the large map open throughout.

**Maps.** Draws relief, biome and rainfall maps of a world cell and prints the
coordinates of landmarks, which is how to find a place worth a screenshot.

```sh
cargo run --release -p mapgen -- --seed 1 --poi
```

Each `poi` line names a kind of place and its world coordinates. `poi site` lines are
the landmarks; `poi marker` the milestones, benches, waymarks and cairns; `poi delight`
the rare small things left beside a trail (wind chimes, a wayside shelf, a seat at a
view, prayer flags, balanced stones, a carved face), each with why it stands there.
Marker and delight lines end with a place on the trail to look from and the yaw to
face, ready for `--over` and `--look`.

**What things are built of.** Stonework is never a colour picked by biome: a place is
built of the rock in the ground where it stands. `surface::local_rock` gives the bed a
cliff there would show, `materials::masonry` the stone that goes with it, and
`Palette::at` (or `trails::build::palette_at` from continent-local coordinates) the
palette to build from. Use those for anything new that people have built, and
`Palette::of(biome)` only in tests.

**Structures: seeing and drawing the landmarks.** The structure editor shows one
landmark at a time on a patch of plain ground, and lets you draw your own, which the
world then builds in place of the generator's.

```sh
cargo run --release -p game -- --structures
cargo run --release -p game -- --structures --structure shrine --variant 7
```

The list on the left has every kind of landmark. Under each is *the generator's own*,
which steps through the variants it makes (the arrows, or Left and Right), and any you
have saved. On the right, *Seen in* builds it of the stone and timber of another kind of
country. *Copy this variant to edit* starts a structure of your own from the one shown;
*Start from nothing* starts an empty one. The controls are listed in the editor too:

| | |
|---|---|
| Drag | turn the view |
| Shift+drag, or middle drag | move the view |
| Wheel | zoom |
| F | frame the structure again |
| Left click | use the tool: Build (B), Paint (P), Erase (E), Pick (I) or Box (X) |
| Right click | erase; with Box, empty a box |
| Middle click | take the material clicked to build with |
| 1 to 8, 0 | stone, dark stone, weathered stone, mossy stone, timber, planks, roof, accent; air |
| L, Page Up, Page Down | cut away everything above a layer, and move the cut |
| Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z) | undo, redo |
| Ctrl+S | save |

Build adds a voxel against the face under the cursor, or on the ground; the white
outline is the voxel the cursor is on and the coloured one where a voxel would go. Box
takes two clicks, one at each corner. *Mirror* repeats every change on the other side of
the middle. With a layer cut, you build on the grid of that layer: layer 0 is the first
above the ground, and the layers below it are footings, which keep a structure from
floating where the ground falls away. The yellow arrow shows the side the trail arrives
from: the world turns a site so its door faces the way most people come. The dashed ring
is the edge of the ground levelled for it.

Most of what you build with are roles rather than materials: *stone* is the rock in the
ground where the structure stands, *timber* the trees round about, *roof* whatever the
place roofs with, so one drawn shrine is granite on the moor and sandstone in the
desert, as the generator's are. *Other materials* are the same everywhere (a lantern,
iron, charcoal). *Air* clears the ground or plants where it is. The game still lays its
props (barrels, woodpiles, offerings) round a saved structure, decayed or not as the
site's age has it, but does not weather or ruin it: it stands as drawn.

Saving writes `assets/structures/<kind>/<name>.txt`. The world uses it from the next
build of the game. Each structure has a *weight*, how often it is chosen against the
others of its kind, and so does the generator, in `<kind>/generator.txt` (the editor
writes it from *How often in the world*). The generator's weight is 0 once a kind has
a structure saved, so every site of that kind is one of yours until you give the
generator a weight again; a kind with nothing saved is the generator's alone, and the
world is as it was. Which one a site gets comes from its seed, so a world stays the same
from run to run. Landings, trail lanterns and signposts can be looked at but not saved,
since they are fitted to the shore and the trail.

A structure file is plain text and fine to edit by hand. Each `layer Y:` is that level
seen from above, a character to a voxel, x along a row and z down the rows, starting at
`corner`; `.` is nothing and the `key` says what the other characters are:

```text
kind: cairn
name: Little cairn
weight: 1
corner: -1 -1
key:
  S stone
  a LANTERN
layer 0:
  .S.
  SaS
  .S.
fine:
  0 4 0  4 8 4  timber
keep clear:
  -3 0 -6  3 6 -2
```

`fine` lists parts finer than a voxel (the frame of a lamp), in quarter voxels, and
`keep clear` boxes props are kept out of (doors, stairs); both come from the copied
variant and can be left as they are. A file the game cannot read fails
`designs::tests::every_saved_structure_reads`, which says which line is wrong.
`--screenshot FILE` with `--structures` draws the editor once without a window
(`--help` lists `--layer`, `--copy`, `--orbit` and `--hover` for it).

**Small delights** live in `crates/worldgen/src/trails/delights.rs`. Each kind has a
reason to stand where it does and a chance of being taken up when the reason holds;
`APART` and `APART_SAME` keep any two from standing near each other. They are meant to
be a surprise, so add to the chances with care. Nothing makes a sound yet:
`GameWorld::delights_near(x, z, reach)` lists the ones about a place, which is where
the wind chimes' sound should start from when there are recordings.

**Generation speed.** Times chunk generation and prints a checksum of everything
generated. A change that is only meant to make generation faster must leave the
checksum as it was.

```sh
cargo run --release -p bench -- --seed 1
```

**The pack and camping.** `--pack` shows the pack's window on a screenshot and
`--hotbar` the hotbar. `--camp lit` (or `laid`, `out`, `burnt`) makes camp a few steps
ahead of the camera: a fire in that state (`out` is put out a moment ago, and still
smoulders) and a few things set down. `--camp-ahead M` makes it M metres ahead instead,
to see its smoke from far off; give it `--ambient 40` or so for the column to rise. Add `--rest` for the
view of someone lying by the fire, facing it, as high as `--look` says. (There were
tents once; a save from then still loads, without them.)

```sh
./target/release/game.exe --seed 1 --screenshot camp.png --over 42186,30679,5 --look 180,-14 --time 22 --camp lit --hotbar
./target/release/game.exe --seed 1 --screenshot stars.png --over 42186,30679,5 --look 180,25 --time 23 --camp lit --rest
```

Sticks lie fallen under the trees (`crates/game/src/fallen.rs`). They are not part of
the world's generation, so they leave the `bench` checksum alone: where they lie comes
from the trees the world grows and a hash of each tree's place. They are found again
every couple of seconds round the player, on the same thread that looks for falling
leaves; `RUST_LOG=warn,game::fallen=trace` logs where each lies, which is how to aim a
screenshot at some. One picked up is a gathered spot in the pack, as a flower is.

In a window, `--camp lit --rest` starts the run resting by the fire, and logs the hour
the rest ended at: `--seed 1 --time 21 --camp lit --rest --frames 1500` watches a night
go by.

**The lantern.** Every pack is given a lantern, at the far end of the hotbar; it is lit
while it is held, and a right click shutters it. `--lantern` puts a lit one on a
screenshot: in the hand of the figure with `--third`, or else in the hand of whoever's
eyes the camera is, which shows only by its light.

```sh
./target/release/game.exe --seed 1 --screenshot lantern.png --over 42186,30679,5 --look 180,-14 --time 23 --lantern --third --figure 60 --figure-speed 0 --no-audio
```

**Clouds forming and clearing.** The clouds' field is two patterns of heaps, and each
stretch of sky goes slowly from the one to the other and back (`cloud_mix` in
`common.wgsl` and `render.rs`), faster while the weather is on the change. On a
screenshot, `VOXEL_CLOUD_AGE=M` shows the sky M minutes on in steady weather; a few
shots a few minutes apart show clouds forming and breaking up.

```sh
VOXEL_CLOUD_AGE=4 ./target/release/game.exe --seed 1 --screenshot clouds.png --over 42186,30679,60 --look 180,10 --time 14 --weather broken --no-audio
```

Stills cannot show flicker, shimmer or popping. Anything that moves (shadows, wind,
light shafts, streaming) also needs a look in a live window.

## House rules

- **Do not run `cargo fmt`.** The code is formatted by hand in a style rustfmt's
  defaults do not match, and there is no `rustfmt.toml`. Running it would rewrite most
  of the codebase. Match the style of the file you are in.
- **Shaders are compiled in.** The `.wgsl` files under `crates/game/src/shaders` are
  included into the binary at build time, so a shader change needs a rebuild.
  `common.wgsl` is put in front of every scene shader.
- **Line endings.** The repository stores Unix line endings. On Windows, git's usual
  `core.autocrlf=true` converts them on checkout and back on commit, so files on disk
  have Windows line endings. Scripts that edit source files need to allow for that.
- **The world comes from the seed.** A change to generation changes every existing
  world. If that is not the intent, the `bench` checksum will say so.
- **Comments say why.** Plain sentences about what the code is for, not what the next
  line does.
- **The year is drawn, not generated.** What grows where never depends on the date, so
  the season costs no regeneration. A part of a plant that comes and goes carries the
  number of its season (`PlantBox::season`, a row of `plants::seasons()`: when it starts,
  how long it lasts, how long it is in bud before and in seed after). The number rides
  to the shader in the low bits of the vertex colour, where a leaf carries its tree's
  tag, and `season_phase` in `season.wgsl` decides from the date and the plant's column
  whether it is there and what colour it is. `Season::phase` in Rust is the same sum,
  which is how picking and the butterflies know which flowers are out. Change one and
  change the other.

## How work gets in

- Work happens on a branch (currently `roadmap-68`) and is merged into `master` with a
  merge commit once a piece is finished.
- Each piece of work has a GitHub issue saying what was wrong or wanted, and its commit
  message ends with the issue number, for example `(#107)`.
- One commit per piece where possible, with a message that says what changed for
  someone playing the game.
