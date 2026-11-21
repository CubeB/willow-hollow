<p align="center"><img src="docs/images/logo.png" alt="Willow Hollow" width="720"></p>

Willow Hollow is a first-person game about wandering around a huge, quiet world made of coloured cubes. There's nothing to fight and nothing to build. You follow a river to see where it goes, climb a watchtower, jump off it with a glider, and borrow a boat when you run out of land. Now and then the river brings you to a village, where people are getting on with their day. If you like, you can pick the wild flowers on the way.

There are no textures anywhere in it. Every cube is a flat colour, and the rest comes from shape, sunlight, mist and water.

![A mountain stream cascading through a valley](docs/images/valley_cascade.png)

## The rivers

Rivers are the heart of this project. Each landmass is run through an erosion simulation before you ever see it, so the valleys are there because water cut them. A stream starts high in the hills, picks up tributaries on the way down, and ends in a lake or the sea. If you pick any trickle and follow it downhill, you will get somewhere.

The simulation is much coarser than the voxels, so rivers aren't read straight off its grid. They're smooth curves traced through the drainage network and carved into the ground afterwards, with the water stepping down in flat pools. Lowland rivers run long and calm. Hill streams come down in little waterfalls. Water never runs uphill and never leaks out of its bed, and the tests check both.

## Villages and towns

People live along the rivers. Each continent has a handful of villages, a town or two and a scatter of hamlets, about one village for every quarter of an hour's walk. Each stands where a village would: at a confluence, a ford or a bend, by a spring or on the coast, and it has a history, so an ancient town still has its old walls, half tumbled, and a young village is only a few homes. Its name, the names of its people and the names of its fields all sound like the region they're in.

Nobody draws the houses by hand. They come from a kit of rules for each region, so a temperate village is stone, timber and thatch, a desert one is mud brick under sail shades, and a marsh village stands on stilts joined by boardwalks. Every building can be walked into, and every one is furnished for whoever lives there, with beds, a hearth, lit windows after dark and smoke from the chimney when the fire is in. A village is laid out round its green, with a well and a great old tree hung with ribbons. Towns have a paved market square, gates on the roads in, a smithy, a sawmill, a granary and a market hall, and the biggest has a guild house. Read a tavern's notice board and what's asked for goes in your journal; read a guild's roll and the places it knows are marked on your map. A town on the coast has a harbour wall with a beacon. Stop at a tavern and you can sleep the night in a guest bed.

Everyone in a village has a name, a home, a household and a trade, and keeps to it through the day. Say hello and they stop, turn and say a word back, and the journal remembers who you've met. Farmers walk out in the morning to work the fields, sowing, hoeing, reaping and carrying the sheaves as the season has it. Herders drive the cattle and sheep out to pasture and fold them at night, loggers stay out at the camp through the working week, masons cut stone at the quarry, and fishers stand along the bank. On market day the hamlets come in to buy at the stalls, and there are lanterns at midwinter and garlands at midsummer. Boats carry goods down the rivers between villages that trade, and carts go along the roads.

The land round a village is worked. Fields with hedges, orchards, terraces on the slopes and paddies on wet warm valley floors change with the year and the crop rotation, with haystacks, scarecrows, troughs, dovecotes and hives about them, and pollarded willows along the ditches. Beyond an ancient place lie the fields it gave up long ago, gone back to rough grass. Hens, ducks, pigs, sheep, goats and cattle are kept in yards and pens, oxen and donkeys pull the carts, and out in the wild their shy cousins, such as the wild boar, wild sheep and junglefowl, keep their distance. The years pass as well, so a new house goes up in scaffolding and is lived in a year or two later.

| | |
|---|---|
| ![A street in Brynell, a coastal town](docs/images/town.png) | ![The village tree over a market square](docs/images/village_tree.png) |

![Fields at harvest by a river](docs/images/fields.png)

## What else is out there

**An endless ocean of islands.** The world is an infinite grid of cells about 26 km across. Each one holds a continent, a scatter of islands or open sea, and gets generated in the background as you head towards it.

**The land.** There are twenty-two kinds of country, from marsh and birch wood to dune sea, mangrove and black volcanic islands. On the open moor, granite tors stand on the summits with their fallen blocks strewn down the slope below. Hillsides break into cliffs that show the rock in layers, small caves run back into the slopes, narrow ravines cut through rocky ground, and streams start from springs. Every region, river, lake, peak and landmark has a name, and the names in one region sound as if they belong together.

**Forests.** There are nineteen kinds of tree, from birches and cherries through holly, yew and Scots pine up to redwoods big enough to feel silly standing next to. They grow in groves that suit the climate, with willows on the riverbanks and palms on warm beaches. Moss grows in the shade on trunks, logs and boulders, and ivy climbs trees and walls.

**Small things.** Grass, flowers, reeds, mushrooms and lily pads are little models built from blocks a quarter of a voxel wide. There are six kinds of grass, from short lawn to seed-headed meadow to the sedge along the rivers, and they grow in drifts that suit the climate. Fallen logs lie along the slope they landed on, and pushing through a bush slows you down. Keep an eye out for butterflies during the day and fireflies at dusk.

**Wild flowers.** There are twenty-nine kinds, mushrooms among them, each in the places it would really grow: bluebells carpeting the broadleaf woods, poppies and buttercups in the meadows, heather and foxgloves on the moor, kingcups and flag irises by the streams, thrift on the dunes, gentians high up. Each keeps its own months, as the real plant does. Primroses are out in the woods at winter's end, and the bluebells follow in April and May, before the leaves close over them. Poppies, buttercups and foxgloves belong to the summer, gentians to the short summer of the heights, and the heather turns the moor purple in August and September. St George's mushrooms come up in the grass in spring, chanterelles in the moss from July, and penny buns and fly agarics with the autumn. The trees keep their own year too: cherries come out pink in April and are green in leaf by May, with their fruit dark red in July; oaks carry acorns, green in summer and brown in September and October, all down before their leaves; and holly berries ripen red in October and last into February. Out of its season a plant is only its leaves, a green bud or a brown seed head, and each plant comes out on its own day, so a drift fills in over a week or two. The same wood is a different place in March, May and July. You can pick whatever is in flower, and mushrooms, ripe berries, gorse blossom and reed mace too, and carry them in your pack. A flower you pick stays picked, and grows back a year or two later.

**Camping.** Sticks lie fallen under the trees, plenty of them in a wood and few under a tree on its own; one picked up is gone until more have fallen there, a year or two on. Three sticks lay a fire in a ring of stones: light it and it burns through the night, lighting the ground around it, with sparks and a column of smoke you can see from far off. Put it out and it smoulders for a while; take it apart and you get back whatever sticks aren't burnt through. Rest by a fire and the clock runs fast until dark, or until morning (after dark you lie down by it to sleep), with the stars wheeling overhead; nothing is skipped, and any step gets you up. A fire that has burned leaves its ring of stones for good and is marked on the map as a camp. A night's sleep by a fire, or in a tavern bed, leaves you rested: for the next day you walk a little faster and the glider floats a little longer, and a small word under the corner map says so until it wears off. Light a fire on high ground and the map opens out around it, further the higher the camp stands above the land about. Lie back by the fire on a clear night and look up: while you look well up the night goes by at its usual pace, and among the stars are a few figures of this world's own, named by its people, that wheel over with the night and change with the seasons. Look down again to sleep on until morning. Anything in your pack can be set down where you like, a flower on a stone or a row of mushrooms, and it stays there.

**Places to find.** Every landmass has a network of trails linking shrines, lookout towers, camps, ruins, standing stones and boat landings. They're built of the rock in the ground where they stand and the trees that grow there, so a tower on the moor is granite, a desert shrine is sandstone, and a shrine among the redwoods is mossy rock and red timber. The paths are worn into the ground, climb steep slopes in rough steps, and are marked by cairns, waymarks and milestones. They wander a little where the land gives them room, and cross brooks on stepping stones and streams on timber footbridges; a stone bridge took a summer to build, so a trail will go a long way round to avoid one, and only the big rivers have them. The lanterns come on at dusk. Once in a long while there's something small that somebody left beside the way: wind chimes on the brow of a hill, a candle burning on a wayside shelf where two paths meet, a log seat turned to a view, prayer flags over a high pass, stones balanced by a ford, a face carved in a boulder.

**Incense.** Every shrine has a small burner beside its altar, built like the shrine of what is to hand: a post of the local timber in the woods, a little pillar of the local stone in open country, with a clay bowl, or an iron one in the cold north; the wayside shelves have their dish. Pines, spruces, junipers, acacias and the great trees of the rainforest sometimes weep a bead of resin on the bark at about chest height, which you can pick, and which grows back like the sticks do. Drag resin onto a sweet herb in the pack (violets, heather, gorse blossom, marigolds or orchids) and you have two sticks of incense. Light one in a burner with E, or by using it with it in your hand: a soft chime, a thin curl of smoke that drifts off with the wind for a couple of hours, and then ash in the bowl until the next. The smoke tells of somewhere: the map gains a faint ring at the nearest spring or shrine you haven't been to, and the journal keeps the shrine.

**The journal.** Press J for a record of the walk that writes itself. The first of each kind of thing you gather is entered with where and when you found it and the sort of country it grows in; what you haven't found yet is only an outline. Each named place is entered when you first come to it, region by region. There is a page for how far you have walked, glided, sailed and swum, the highest ground you have stood on and the rivers you have followed down to the sea, one for each camp you have made, with where and when, the country, the weather and the nights slept there, how far off and which way it lies now, and the figures you have made out in the stars; and one that sets down the notable moments a day at a time: a new region, a summit at dusk, the first snow, a night by the fire.

**Getting around.** Open the glider in mid-air and ride it off anything tall. Every landing has a boat tied up, which you can take down a river or across the strait to the next island. Press V to see yourself from behind: a small figure that walks, swims and carries the glider folded on its back.

**The map.** The map fills in with what you've seen, which is further from high ground, and shows a faint line where you've walked. A small one sits in the corner, turned the way you face, with the name of the place or the region you're in along its foot. The large one is a model of the land you've seen, made of blocks like the world, which you can move, turn and zoom, from the whole continent down to a single clearing. Both are lit by an afternoon sun, so hills shade their valleys and trees throw shadows, and close in they show the world's own trees, shrines, ruins, bridges, jetties and tors as small models of themselves, in the colours of the season. Regions, ranges, lakes, rivers and peaks are named on the large map once you've seen them; the things people built keep their names until you've been there.

**The sky and the year.** The day turns in real time, with mist in the valleys at dawn, and a moon and stars at night. The clouds are built of blocks too. They cast blocky shadows on the ground, and you can fly up through them and look down on them. Each day brings its own weather, from clear to overcast, with rain, snow, storms and fog, and rain splashes where it lands. The year turns as well: flowers come out in their season, the leaves colour and fall in autumn, and in winter snow lies on the high ground and in cold country, and now and then on the lowlands too. Past the edge of the loaded terrain, the continent carries on to the horizon in a simpler form, so you can see where a river is heading long before you reach it.

| | |
|---|---|
| ![Dawn mist in a valley](docs/images/dawn_mist.png) | ![Forest floor in dappled shade](docs/images/forest.png) |
| ![Redwoods](docs/images/redwoods.png) | ![A lake among the hills](docs/images/mountain_lake.png) |
| ![Fireflies at dusk](docs/images/fireflies.png) | ![River bank with willows and poppies](docs/images/river_bank.png) |
| ![A bluebell wood in spring](docs/images/bluebells.png) | ![A shrine at dusk with its lantern lit](docs/images/shrine_dusk.png) |
| ![Blocky clouds over the hills](docs/images/clouds.png) | ![Looking down on the clouds](docs/images/above_clouds.png) |
| ![A broadleaf wood in autumn](docs/images/autumn.png) | ![A tor on the moor in winter](docs/images/winter_tor.png) |
| ![Rain on a lake](docs/images/rain.png) | ![Gliding over the moor towards a tor](docs/images/glider.png) |
| ![The player's figure in an autumn wood](docs/images/third_person.png) | ![The pack, with the corner map](docs/images/pack.png) |

![The large map: a model of the land seen so far](docs/images/map_3d.png)

Here's the same world from above, as `mapgen` draws it: the biomes of the starting continent, and the islands in the cell next door.

| | |
|---|---|
| ![Biome map of the origin continent](docs/images/map_biomes.png) | ![An archipelago cell](docs/images/map_archipelago.png) |

## Playing it

You need stable Rust (edition 2024), plus the MSVC build tools on Windows. There's nothing to download beyond that, because the game has no asset files (the landmarks drawn in its structure editor are text, compiled in). So far it has only been run on Windows. To work on the game rather than just play it, see [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md); for what has been done to make it run faster, and what is left, see [docs/PERFORMANCE.md](docs/PERFORMANCE.md).

```sh
cargo run --release -p game                  # a new world, starting on a river bank
cargo run --release -p game -- --seed 1      # world 1, from where you left off
cargo run --release -p game -- --fly         # free camera
cargo run --release -p game -- --help        # every option
```

The first launch takes a few seconds, because the starting continent has to be simulated before the window opens. On a laptop GTX 1650 Ti it runs at around 135 fps at 1080p.

| Key | What it does |
|---|---|
| Mouse | Look around (**Tab** frees or recaptures the mouse) |
| Esc | Settings, or close the map, the pack, the journal or the console |
| W A S D | Walk |
| Shift | Sprint |
| Space | Jump, or swim up |
| G, or Space in mid-air | Open or fold the glider. Look down to dive, up to slow down |
| E | Pick what's in front of you (a flower, sticks lying under the trees, or resin on a pine), light a fire or a stick of incense at a shrine, rest by a fire or sleep in a tavern bed, greet someone, read a notice board, or get in or out of a boat at a landing. W sails, Shift for full sail |
| 1 to 7, or the wheel | Choose which slot of the hotbar is in your hand |
| Right click | Use what's in your hand where you're looking: lay a fire with three sticks, light incense at a shrine's burner, or set a thing down |
| Left click | Pick up what was set down, such as a flower. Put out a fire that's burning, or take apart one that isn't for what's left of its sticks |
| I | Open the pack. Drag one thing onto another to move, join or swap them, or down to the hotbar; drag resin onto a sweet herb to make incense. Click one to read about it or set it down |
| J | Open the journal. Click a page's name, or use the left and right arrows, to turn to it |
| M | The large map. Drag to move it, right drag to turn it, the wheel to zoom |
| N | Show or hide the corner map |
| V | First or third person |
| F | Toggle flying. Space and Ctrl go up and down, the wheel sets the speed |
| F3 | Hide the debug panel |
| F11 | Fullscreen |
| `` ` `` or / | The console, for typed commands: `weather storm`, `time dusk`, `tp 42186 30679`, `spawn camp lit` and more. `help` lists them all; Esc closes it. See [the console](docs/DEVELOPMENT.md#the-console) |

The settings menu covers your figure's looks, field of view, mouse sensitivity, volume, how far terrain loads, the length of a day and of a year, window size, fullscreen, vsync, light shafts and the corner map. If you push the load radius further than your graphics card can hold, the game pulls it back in by itself.

A plain run starts a new, random world each time. The game remembers your settings and where you were standing in each world, boat included, along with the time of year, the map you've drawn, what's in your pack, what you've set down and your journal, so `--seed N` takes you back to world N where you left it. Use `--fresh` to go back to the start of a world. `--time H` picks the hour and `--season` the time of year for one run, and the debug panel has sliders for both.

### Looking around without playing

`mapgen` draws relief, biome and rainfall maps of a cell and prints the coordinates of anything interesting. The game can also render a single frame without opening a window, which is handy for checking a change. `--weather` and `--date` set the sky and the season for the shot.

```sh
cargo run --release -p mapgen -- --seed 1 --poi
cargo run --release -p game -- --seed 1 --screenshot shot.png --over 31776,43030,30 --look 280,-6 --time 17.5 --weather clear
```

## How it's put together

```
crates/worldgen   continents, climate, biomes, rivers, trees, plants, trails, landmarks, names,
                  and settlements: siting, the house kit, layouts, farmland, trade and the years
crates/voxel      chunk storage and the mesher
crates/game       renderer, chunk streaming, player, villagers, animals, weather and seasons,
                  map, pack, journal, sound
tools/mapgen      draws a continent's simulation as PNG maps
tools/bench       times chunk generation and checksums what it made
```

If you're curious about the details:

- The erosion is stream-power erosion using the FastScape implicit solver, with priority-flood drainage and rainfall that leaves rain shadows behind mountains. A 1024×1024 continent takes about three seconds.
- A voxel is 40 cm. Each 32³ chunk is generated with a one-voxel border, so chunks can be built on any thread in any order without waiting for their neighbours.
- Land never crosses the border of a world cell. That's what makes it possible to simulate a whole watershed at once and still have an endless world.
- All chunk meshes live in one big GPU buffer and go out as a multi-draw, with faces grouped by direction so the ones pointing away from you get skipped.
- Positions are 64-bit on the CPU and camera-relative on the GPU, so nothing gets jittery a long way from the start.
- Collision doesn't keep any voxel data around. A column without its small plants takes about a millisecond to generate, so the few under the player are just rebuilt when needed. Falling leaves and rain get theirs from a background thread, so they never hold up a frame.
- Moss, ivy, bark and twigs are most of the cost of a column, so they're only grown for columns close enough to show them. `cargo run --release -p bench` times all this and prints a checksum of everything generated, which is how a speed-up gets shown not to have changed the world.
- The whole world comes from the seed. The tests check that drainage reaches the sea, rivers only grow on the way down, lakes are flat, and chunk edges line up.

The logo is drawn by `docs/logo.py`, using Pillow and Georgia. Run `python docs/logo.py docs/images` to redraw it.

## Where the ideas came from

Cube World, for the scale and the feeling of it. Veloren, for showing that erosion-based world generation works. A handful of Minecraft mods also shaped particular pieces: Geophilic and Blooming Biosphere for ground detail and flower drifts, Wilder Wild for wind and fallen trees, Particular and Subtle Effects for waterfall spray, fireflies and falling leaves, and RoadWeaver for the trail network.

## Still to do

The issues tab has the full list. Footsteps and the chime of incense are the only sounds for now, and the ambience will come from real recordings later.

## Licence

Not decided yet, so all rights reserved for now.
