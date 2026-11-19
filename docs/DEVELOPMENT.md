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
After that, a change to the game crate rebuilds in about a minute on 12 threads.

Day to day, build with the `fast` profile instead: the same optimisation, but
incremental and without link-time optimisation, so a change rebuilds in about 7 s.
Frames cost the same, because the GPU sets the pace, and world generation is about 1.5%
slower. Its first build takes a few minutes and it keeps its own `target/fast`. Bench,
take screenshots for a comparison, and ship with `--release`.

```sh
cargo build --profile fast -p game
./target/fast/game.exe
```

If `cargo` is not found in Git Bash on Windows, add it to the path for that shell:

```sh
export PATH="$USERPROFILE/.cargo/bin:$PATH"
```

### On a machine short of memory or cores

The release build uses thin link-time optimisation and can take a lot of memory. Cargo
uses every core by default, which is fastest (a rebuild takes about 1 minute on 12
threads, against nearly 3 with two jobs). To keep the machine usable while it builds, or
when several builds run at once, limit the number of jobs:

```sh
cargo build --release -p game -j 2
cargo test --release -p game -j 2 -- --test-threads=2
```

### Debug builds

World generation and meshing are far too slow unoptimised, so the workspace optimises
even `dev` builds (see the profiles in `Cargo.toml`). Day to day, use `--profile fast`.

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
(where you stand, the hour, the date and the world's year), `pack-N.txt` (what you
carry), `journal-N.txt` (the journal: a line to each find, place, river and note, and
the distances gone) and `map-N/` (what the map has seen). Any of them can be deleted on
its own; a line the game cannot read is left out and the rest still loads.

The world's year counts from 0, the year it began in, and turns each time the date
comes round from winter into spring (`season::advance_year`); the journal's "of the
first year" is year 0. A world saved before the year was kept takes it from its journal,
which counted the springs. Anything drawn that changes over the years should read
`Environment::year`, which the game keeps with the hour and the date.

## Tests

```sh
cargo test --release --workspace
```

The tests check the world as well as the code: drainage reaches the sea, rivers only
grow on the way down, lakes are flat, chunk edges line up. Run them before pushing. The
worldgen tests take about a minute.

Many tests build whole continents, about 600 MB for each test running at once. With a
test thread on every core (12 here) that came to some 7 GB, and on a 16 GB machine with a
browser open it ran out of memory. So `.cargo/config.toml` runs four tests at once
unless told otherwise: `--test-threads=N`, or `RUST_TEST_THREADS` set in the
environment, overrides it.

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
the middle of July. `--year N` sets the world's year (0 is its first), which the journal
shows. PNG files are ignored by git everywhere except under `docs/`, so shots can be
written anywhere.

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

`WH_FRAME_CSV=frames.csv` adds a line per frame: every CPU lap, every GPU pass and what
the streamer did. For a real comparison, run the whole suite of scenes in
`tools/bench/scenes.txt` on an otherwise idle machine, once before a change and once
after: `OUT=before bash tools/bench/bench.sh` (it stops if anything else is running;
`LAPTOP=1` holds the game to 8 CPUs at 960x540, and `tools/bench/take.sh` with
`imgdiff.py` checks that the picture has not changed). A screenshot is drawn at however
long its loading took, which moves the leaves and water; `VOXEL_CLOCK=S` draws it at S
seconds instead, so that two shots of the same build match exactly (take.sh sets it).
Every loading wait gives up after 90 s without progress, prints what each worker is doing
and exits with code 3; `WILLOW_WAIT_LIMIT=S` changes the limit. What was made faster, the
numbers, and the changes still waiting for a yes are in [PERFORMANCE.md](PERFORMANCE.md).

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

**Houses in the editor.** Nobody draws the homes: the house kit
(`worldgen::settlements::kit`) builds them from rules. *Houses* in the editor's list
shows *the kit's own* (`--structure house/temperate/timber`): choose a region, a step
(hut, timber house or stone house) and a variant. Each variant is another household,
and *Household* shows its age, wealth and how many sleep there; move a slider and that
holds for every variant, so the arrows step through the variety of one household
(*As the variant has it* lets go again).

The kit's rules are `assets/structures/kit/<region>.txt`. Run from the repository (or
with `--structures-dir`), the editor builds the kit's homes from those files, and reads
a file again within half a second of its being saved, or at *Read the kit files again*,
so a change to the rules shows without a rebuild. A line the kit cannot read is named in
red in the panel, and until it reads the homes are built with the rules last read from
the file (or those compiled in). The world always builds with the rules compiled into
the game: it takes up a change when the game is built again, and the panel says when
the files and the game differ.

*Copy this house to edit* (or `--copy`) turns the home shown into a house of your own,
saved as `assets/structures/house/<name>.txt`. Its file is a structure file with three
more things: the `region:` and `step:` it was copied for, and a `home:` section keeping
where the kit put its door, beds, hearth, furniture places, windows, smoke and anchors
(`crates/worldgen/src/settlements/kit/copy.rs` says how each line reads). Its blocks are roles, as a copied
landmark's are, so built again in the same place it is the kit's home to the voxel. The
floor is not kept: it is found again by walking in from before the door each time the
house is built (`kit::walk`), so it follows the edits. The view outlines the beds (cut
the layers above away to see them) and marks the way in; if an edit shuts the door on
any bed they turn red and the panel says so. It still saves, but
`every_saved_structure_reads` fails until the way is open again, since every home in the
world must be walked into.

A saved house is one of the homes `kit::build` makes for its region and step: each home
is the kit's or one of the saved ones, chosen by its seed in proportion to their weights,
and a saved house only for a household it has a bed for each of. It is built as drawn,
whatever the age and wealth asked for. The kit's weight is in `house/generator.txt`, as
a generator's is, written from *How often in the world*; left out, it is 10, not 0, so
one saved house of weight 1 is one home in eleven of its region and step rather than
every one of them. With no houses saved, `kit::build` is the kit and the world is as it
was. `--structure house/<name>` opens a saved house. To try a hand edit on a house's file
without the editor, `KIT_COPY="temperate hut 5" cargo test --release -p worldgen --lib
kit::copy::tests::print -- --ignored --nocapture` prints a copy of one.

**Components.** What makes a region's homes its own (#203), set on the kit's homes at
the anchors the kit marks: a desert house's cloth awning and water jars, a lowland
cottage's porch, window boxes and climbing rose, a northern woodpile to the eaves and
flowers on the turf roof, a marsh hut's rope rails, boardwalk and punt. Each is a small
generator in `crates/worldgen/src/settlements/kit/components/` (its module says how they
go on and what the kit already builds itself), and each region's kit file says which go
at which kind of anchor and how often, under `components:` (a name in the wrong place is
named in red as any wrong line is). The editor shows the homes with theirs on; untick
*Components* in the Houses panel, or add `/bare` (`--structure
house/temperate/timber/bare`), to see one as the kit first makes it, and the panel lists
what went where. *Components* in the list has a gallery of every one, or a region's, each
on a stretch of a home it goes on, new and let go (`--structure components/desert`), and
each component's own (`--structure component/porch`), whose variants set it on other
homes. Every awning, sail, banner and hide is cloth and moves in the wind in the game, as
washing does; the editor draws it still.

*Copy this variant to edit* turns one into a component of your own, saved as
`assets/structures/component/<name>.txt` with a `component:` line saying which it is. It
is drawn in its anchor's frame: the anchor's voxel is the origin, out from the house is
east (+x), and the wall is at x = -1, so whatever home it is chosen for it is turned to
fit. Wherever its component goes it is chosen against the generator's own by weight
(`component/generator.txt`, 10 if left out, as for houses), and if it does not fit there
(it would stand in a wall, hang in the air or block the way in) the generator's goes
instead; the panel says when a saved one fits on none of the homes tried. To look into a
component that never goes on, `COMPONENT_SHOW="rope-rail wetland" cargo test --release
-p worldgen --lib components::tests::trace -- --ignored --nocapture` tries it on that
region's homes and says what stopped it.

**Settlement buildings.** The tavern, smithy, sawmill, granary, market hall, guild house,
well head and gate are kinds of their own (`designs::Special`), listed under *Settlement
buildings*, each with its folder (`tavern/`, `market-hall/`, `guild-house/`,
`well-head/`...), so they can be drawn before stage 3 (#203) brings their generators. Until
then the generator's own is a stand-in: a plain home of the kit, of the country in *Seen
in*, for a building people go into, which can be copied with its door and beds as a
house can; and an empty plot for the well head and the gate, which are started from
nothing. The tavern stands in as a timber house with six beds and the guild house as a
stone house with two; the smithy and the market hall as stone houses, and the sawmill
and the granary as timber houses, with none (`editor/houses.rs`, `stand_in`). The world
does not build any of them yet.

**Small delights** live in `crates/worldgen/src/trails/delights.rs`. Each kind has a
reason to stand where it does and a chance of being taken up when the reason holds;
`APART` and `APART_SAME` keep any two from standing near each other. They are meant to
be a surprise, so add to the chances with care. Nothing makes a sound yet:
`GameWorld::delights_near(x, z, reach)` lists the ones about a place, which is where
the wind chimes' sound should start from when there are recordings.

A string of prayer flags is a line on two poles (`delights::flags`), often with a stay
run down from a pole to a cairn. Every box on a line carries how far the line gives
there (`PlantBox::flap`), and `flap_motion` in terrain.wgsl swings the line and
flutters each flag's free edge in the wind, each at its own phase. Motion does not show
in one shot: take two with different `VOXEL_CLOCK` values and compare them.

Two other kinds of cloth move in the wind (`features/cloth.rs`), made of fine boxes in
cells like the flags. A ribbon tied by one end (`cloth::ribbon`) hangs from its knot in
still air, leans out downwind the harder it blows and flutters most at its free end. A
sheet tied at its corners (`cloth::sheet`, for sail shades and awnings) billows up and
down between them and ripples at its free edges; it lies smoothest tied level, since a
slope is built in steps a cell high. Cloth is told from flags and plants by the ground's
plant kind, which nothing that sways otherwise has, with a code in the colour's tag:
for a ribbon how far below its knot that corner hangs, for a sheet how far it gives
there and how big it is (`mesh::cloth_code`, read by `cloth_motion` in terrain.wgsl).
What it was made as is kept in `PlantBox::flap` and `season` (`PlantBox::cloth`), and a
sheet's corners turn with the model it belongs to (`parts::place_fine`). Ribbons hang
from the tie beam of a shrine's stone gate, and a camp in desert or savanna has a sail
shade over its bench. Cloth casts a shadow that stays still, as the flags' do.

**Bees' and wasps' nests** (`crates/worldgen/src/features/nests.rs`) belong to the trees
and boulders they are in, chosen by a hash of where each stands: a colony of bees in a
hollow of an old tree or a crevice in a boulder, a wasps' globe under a branch.
`GameWorld::nests_near(x, z, reach)` lists them, kind, host and door, without building
any chunk; the bees and wasps in the air (`crates/game/src/insects.rs`) fly from it, and
an apiary should look there for colonies. With `RUST_LOG=warn,game=debug` a screenshot
logs the nests within 200 m of the camera, to aim a shot at one.

**Starlings.** On about one evening in five of late autumn, over open country or reeds, a
murmuration gathers at dusk (`crates/game/src/starlings.rs`). `--murmuration` puts one
ahead of the camera whatever the date, hour and country; give it `--ambient 20` or more
to see the cloud a while after it has gathered.

**Butterflies** rise from flowers in bloom near the player (`particles.rs`), now and then,
so a still seldom has one close. `--butterflies M` sets out one of each colour M metres
ahead of the camera; `--ambient 0.3` keeps them there.

**The northern lights** come out on some nights in the cold country: the cold biomes, and
the cold north of a continent or its high ground (`crates/game/src/aurora.rs`; drawn in
`sky.wgsl` from a uniform of their own). `--aurora` has them out tonight wherever the
camera is; north is toward -z, `--look 270,18`. A screenshot logs how bright they are.

```sh
./target/release/game.exe --seed 1 --screenshot aurora.png --over 42186,30679,4 --look 270,18 --time 23 --season winter --aurora --no-audio
./target/release/game.exe --seed 1 --screenshot starlings.png --over 42186,30679,4 --look 180,8 --time 16.6 --date november --murmuration --ambient 20 --no-audio
```

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
to see its smoke from far off; give it `--ambient 60` or so for the column to rise. Add `--rest` for the
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

## The console

`` ` `` or `/` opens the console in a running game: a line to type commands into, with what
they said above it. Enter runs a line, Tab fills out a command's name or its first word
(listing the choices when there are several), Up and Down go back through what was run,
and Esc closes it. While it is open the keyboard is its own and the mouse is free, so
nothing typed moves the player. A leading `/` is allowed and ignored. A command that
cannot be done says why and how it is used.

| Command | What it does |
|---|---|
| `help [command]` | List every command with how it is used, or say more of one |
| `clear` | Empty the console |
| `weather [kind\|auto]` | Hold a weather: `clear`, `fair`, `broken`, `overcast`, `rain`, `snow`, `storm` or `fog`, there at once; `auto` lets it go its own way again, easing into it. Alone, says what the weather is doing |
| `time [hour\|+h\|-h]` | Set the hour (`18`, `18:30`, or a word: `dawn`, `noon`, `dusk`, `midnight`...), or move it on or back (`+3`, `-1.5`). Alone, says the time |
| `date [month\|0-1]` | Set the time of year: a month (`may`, `oct`) or a fraction of the year from the spring equinox, as `--date` takes |
| `season <name>` | The middle of `spring`, `summer`, `autumn` or `winter` |
| `year [n\|+n\|-n]` | Set which year of the world it is, counting its first as 0, as `--year` takes, or move it on or back. Alone, says the year. The date stays as it is |
| `tp <x> <z> [h]` | Go to voxel x, z, the eye h voxels above the ground or water there (eye height when not given). Also `tp x,z,h`, as `pos` prints it |
| `tp <place>` | Go to the named place on this continent whose name has these words in it, nearest first (`tp hollin tarn`) |
| `tp home`, `tp back`, `back` | Go to the world's start; go back to where the last tp left from |
| `pos` | Say where you are, as `x,z,h` for `tp` and as `--over` and `--look` for a screenshot |
| `places [name]` | The ten named places nearest, or those with this in their name, with how far, which way and the `tp` to each |
| `people [here\|all]` | Who lives in the settlement nearest, how many households, how many of them are where at this hour (in bed, at the well, at work, at the tavern...) and where its well, market, tavern and tree are, with the nearest few by name; `all` names every one of them and where each is |
| `greet` | Greet whoever of a settlement is within 3 m, as E does. On a screenshot, the one nearest the figure (or the camera) turns to it, and what they say is the shot's prompt |
| `read` | Read the notice board or guild house's board looked at, within 2.4 m, as E does. On a screenshot (or with `--read`), the board the camera looks at, its page drawn over the shot. `people` says where each board of the settlement nearest is read from, and when its next market day is |
| `fly [on\|off]` | Fly free of the ground, or walk again (F) |
| `view` | Through the eyes, or from behind the figure (V) |
| `spawn camp [laid\|lit\|out\|burnt]` | Make camp a few steps ahead, as `--camp` does: a fire (lit if not said) and a few things set down |
| `spawn murmuration` | Starlings gathering ahead, whatever the date, the hour and the country |
| `spawn crows [how]` | Crows near a point ahead: `ground`, `perch`, `tree`, `rock`, `fly`, `land`, `roost` or `any`. They are wild: they take fright, and leave after a minute or two. In a run, the crows need a few seconds after the start (or a long tp) to look over the country first |
| `spawn boat` | A boat afloat on the nearest water ahead, within 80 m; E by it to board |
| `spawn <thing> [n]` | Set down n of a thing at your feet (three sticks if not said): `sticks`, a flower or mushroom by its id or name (`poppy`, `water lily`, `penny_bun`), `holly-berries`, `reed-mace`... Tab lists them |
| `aurora [on\|off\|auto]` | The northern lights out every night wherever you are, never, or as the night and the country have them. They show only in the dark |
| `give <thing> [n]` | Put n of a thing in the pack (the same things as `spawn`, and the `lantern`) |
| `speed [times]` | Walk, run and swim this many times faster than usual, 0.1 to 20. Alone, says the speed |
| `seed` | Say which world this is |
| `boon [second-well\|guild-hall]` | Grant the settlement nearest a boon in this year, kept in `boons-N.txt` with the world's save: a `second-well` sunk on its green (its empty homes are soon built again), or a great town's `guild-hall` opened, hung with its banners. Only where it was laid out with room for one. Alone, says which it has and could have. On a screenshot it holds for the shot |

`--command "LINE"` runs a line at the start, as if typed, and can be given as often as
needed; what each says goes in the log and in the console. On a screenshot the commands
set the shot up as their flags would: `weather`, `time`, `date`, `season`, `year`, `tp` (as
`--over`), `pos`, `places`, `people`, `greet`, `read`, `spawn camp`, `spawn murmuration`,
`spawn crows`, `spawn boat`, `aurora` and `boon`; any other, or one that fails, stops the shot with what was
wrong. `--console` draws the console open on a screenshot, with what the commands said,
and in a run starts with it open.

```sh
./target/release/game.exe --seed 1 --screenshot storm.png --command "tp 42186,30679,5" --command "weather storm" --command "time 15" --look 180,-5 --no-audio
./target/release/game.exe --seed 1 --screenshot camp.png --over 42186,30679,5 --look 180,-14 --command "time 22" --command "spawn camp lit" --console --no-audio
```

The commands are the table `COMMANDS` in `crates/game/src/commands.rs`: a name, other
names it answers to, how it is used, a line for `help`, the words Tab offers for its
first argument, and a function from its words to what it does to a `Game`. The game in
a window is one `Game`, and a screenshot being set up another (both in `main.rs`); a
`Game` that cannot do something says so by leaving that method as it is. A new command
is one entry in the table, and a method on `Game` if it needs the game to do something
it cannot yet. The parsing and the console's history and completion have tests there.

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
- **What people build is generated for the year, and the fields for the season** (#207),
  the one exception to the rule above. Settlements are still laid out once, from the
  seed; each building and yard stamp carries a tag (`settlements::years::Tag`), and
  `years::Lives`, a pure function of the seed, the year, the season and the boons
  granted, says what each home is that year (lived in, freshly thatched, standing
  empty, being taken down, rising again in scaffolding) and `Lives::view` makes the
  stamp as it stands. Every reader of stamps (chunks, the far rings, the far boxes, the
  map, the climbers) goes through `view`, so they agree, and chunk borders with them.
  Which crop a field is in is `farmland::rotation`. Year 0 is today's world in every
  season, so the checksums and layout tests hold. `GameWorld::set_now` sets the moment;
  when it changes anything the world's `epoch` goes up, and the streamer makes again
  every loaded column a settlement or its fields reach (`changes_with_years`), showing
  the old meshes until the new are up, while the far boxes and the villagers are made
  again on their own threads. Nothing of a chunk is ever saved, so there is nothing to
  undo: the save keeps the year and the boons (`boons-N.txt`), and the rest follows.
  A settlement that lives by logging works a camp out in the woods beyond its fields
  (`settlements::logging`, raised after the barns and kept in `Network::years`), whose
  clearing is felled back a ring a year as far as the land lets it.
  `logging::fell` turns the trees felled by the year to stumps. Every reader of the
  trees calls it: chunks, climbers, nests, the coarse rings, the far trees and the
  map. The far trees a clearing reaches are made again when the year turns
  (`GameWorld::fells_with_years`).

## How work gets in

- Work happens on a branch (currently `roadmap-68`) and is merged into `master` with a
  merge commit once a piece is finished.
- Each piece of work has a GitHub issue saying what was wrong or wanted, and its commit
  message ends with the issue number, for example `(#107)`.
- One commit per piece where possible, with a message that says what changed for
  someone playing the game.
