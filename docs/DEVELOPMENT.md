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

The game has no asset files, so there is nothing else to download. It has only been run
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
down: flowers, tents, fires), a line to a thing.

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
`--hotbar` the hotbar. `--camp lit` (or `laid`, `burnt`) makes camp a few steps ahead
of the camera: a fire in that state, a tent and a few things set down. Add `--rest` for
the view out of the tent's door, as high as `--look` says.

```sh
./target/release/game.exe --seed 1 --screenshot camp.png --over 42186,30679,5 --look 180,-14 --time 22 --camp lit --hotbar
./target/release/game.exe --seed 1 --screenshot stars.png --over 42186,30679,5 --look 180,25 --time 23 --camp lit --rest
```

In a window, `--camp lit --rest` starts the run resting in the tent, and logs the hour
the rest ended at: `--seed 1 --time 21 --camp lit --rest --frames 1500` watches a night
go by.

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
