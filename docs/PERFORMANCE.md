# Performance: what was done, what was learnt, what is left

The work ran in November 2026, from `30e2e41` to the merge of `roadmap-68`, under issues
#181–#189. The target is a laptop GTX 1650 Ti at 60 fps at 1080p. The dev machine is a
GTX 1060 6GB with 12 threads.

The rule throughout: every change had to show a measured gain and give the same picture.
Screenshots of the same scene before and after had to match to within 1 in a channel.
A change that was flat or slower was taken out again, however sound it looked.

## How it was measured

- **The bench:** `tools/bench/bench.sh` runs every scene in `tools/bench/scenes.txt`, twice
  by default.
  - Static scenes give 6000 frames, of which the last 30 s are scored once the view is
    loaded. Flights give 3000 frames, all scored.
  - It refuses to run, or stops, if anything else is using the machine: see
    [DEVELOPMENT.md](DEVELOPMENT.md#checking-a-change).
  - Numbers from a shared machine mean nothing. Run-to-run spread on an idle one is under
    0.3 ms.
- **Hitches** (`dt8` in the table): frames more than 8 ms slower than the median of the 60
  frames around them. Unlike a fixed threshold, this sees a stutter in a fast scene as
  well as a slow one.
- **The picture:**
  - `tools/bench/take.sh DIR` shoots every scene. `VOXEL_CLOCK=10` holds the renderer's
    clock still, so two shots of one build match exactly. Pass `AMBIENT=0` as well.
  - `tools/bench/imgdiff.py BEFORE AFTER` compares two such directories.
  - One pixel in `cloud` flickers by 9 even between two runs of one build. Every other
    scene repeats exactly.

## Results (GTX 1060, 1080p; mean frame ms, 1% low ms, hitches per run)

| Scene | 30e2e41 | After | 1% low | First 30 s, 1% low | Hitches |
|---|---|---|---|---|---|
| wood_noon | 13.98 (72 fps) | 11.75 (85 fps) | 15.2 → 13.1 | 25.9 → 13.1 | 0 → 0 |
| wood_night | 12.12 | 9.88 | 13.3 → 11.1 | 21.0 → 11.3 | 0 → 0 |
| keep | 12.21 | 9.99 | 13.4 → 11.1 | 22.4 → 11.1 | 0 → 0 |
| falls | 13.37 | 11.29 | 15.0 → 12.9 | 21.2 → 12.8 | 0 → 0 |
| crows | 13.99 | 11.64 | 15.3 → 12.9 | 22.1 → 12.7 | 0.5 → 0 |
| meadow | 8.54 | 7.50 | 9.6 → 8.5 | 17.9 → 8.4 | 0 → 0 |
| murmuration | 10.02 | 8.93 | 11.3 → 10.1 | 19.8 → 10.1 | 0 → 0 |
| aurora | 16.07 (62 fps) | 11.67 (86 fps) | 17.1 → 12.9 | 25.1 → 12.6 | 0 → 0.5 |
| cloud | 7.72 | 7.07 | 8.5 → 7.9 | 16.8 → 7.7 | 0 → 0 |
| overcast | 8.91 | 7.78 | 9.8 → 8.7 | 18.5 → 8.7 | 0 → 0 |
| overcast_high | 6.22 | 5.05 | 7.2 → 6.0 | 15.9 → 6.1 | 0 → 0 |
| overcast_above | 4.70 | 4.32 | 13.2 → 5.1 | 15.4 → 5.3 | 30 → 0 |
| drift_fast | 13.27 | 10.32 | 25.1 → 15.7 | 25.9 → 15.7 | 25.5 → 1 |
| drift_low | 7.64 | 6.15 | 20.4 → 12.1 | 20.4 → 12.1 | 26.5 → 5.5 |
| border | 6.15 | 5.52 | 19.2 → 12.9 | 19.2 → 12.9 | 37.5 → 8 |

Before the work, the game was bound by the GPU everywhere. The main thread worked for about
3.3 ms a frame. Terrain was 60–72% of the GPU's time. The aurora alone cost 6.5 ms, and
the worst stutters came in the first 30 s and at continent borders.

## What each package did

**#181: measurement**
- The per-frame CSV (`WH_FRAME_CSV`), which times every CPU lap and every GPU pass. The
  GPU's own uploads are timed apart from its passes (`upload` and `pre`).
- The bench itself, with its idle-machine check, laptop mode (`LAPTOP=1`), and the
  overcast, cloud-deck and continent-border scenes.
- The detrended hitch count.
- `VOXEL_CLOCK`, for screenshots that repeat exactly.
- `ColumnResult::new`, so a new field is added in one place.
- `.gitattributes`, so the shell scripts keep LF endings.

**#182: the screenshot hang, a deadlock**
- **The cause:**
  - `Skylines::get` held its cache lock while computing with rayon `par_iter`.
  - Inside the chunk pool, a worker waiting on its stolen half could pick up another
    chunk job.
  - That job asked for skylines, and took the same lock on the same thread.
- **The fix:**
  - Skylines are now computed serially, on the thread that made the continent, or on a
    `skylines` thread of their own. They are never computed inside a pool job.
  - A job that finds them not ready hands its column back (`Streamer::waiting`) and
    tries again later.
  - The cache evicts by last use, never the continents around the camera.
  - A regression test puts pool jobs that ask for skylines beside idle ones. It deadlocks
    on the old code.
- **Deadlines on every loading wait:** `Streamer::wait_idle`, `Impostors::wait_idle`,
  `FarView::wait_ready` and `MiniMap::settle` give up after 90 s with no progress, with a
  report of what each worker is doing.
  - The process exits with code 3.
  - `WILLOW_WAIT_LIMIT=S` changes the limit.
  - In play, a stall is logged once and the game goes on.
- **Continents** can be asked for without waiting (`continent_soon`), and are prefetched
  ahead of the camera.

**#183: aurora and clouds**
- **The aurora:** curtains that cannot be within 4 widths of a ray's step are skipped,
  using exact bounds on the fbm term. Pixels facing south return at once.
  - Sky pass: 6.5 → 3.1 ms.
  - The picture is identical.
- **The cloud slowdown:** the 0.7 ms rise in broken cloud seen from above was traced to
  #150 (`b0c6842`).
  - An exact fix (baking each square's highest cloud mix) costs as much in its own pass
    as it saves, so it is not in.
  - It is kept on the local branch `p2-cloud-bake-experiment`.

**#184: terrain shader**
- **Per-frame values moved to the CPU:** the screen size of a pixel at unit distance is
  worked out on the CPU once a frame (`G.cam_up.w`), and so is the height below which no
  snow can lie (`G.cam_fwd.w`), which lets the shader skip the snow lookups there.
- **Constants and small savings:** the shadow taps are constant tables, and the water's
  fresnel term squares instead of calling `pow`.
- **The main gain, about 1.4 ms:** the opaque vertex output went from 40 scalars to 18.
  What is the same over a whole chunk is read in the fragment shader from the instance
  buffer, through `instance_index`. This was checked on DX12 as well as Vulkan.

**#185: the crows' survey and thread priority**
- **The survey:**
  - It reads each column once into what doesn't depend on the date. When the season
    turns, only the leaf test is redone from that.
  - It uses one river `Sampler` per patch, not one per cell.
  - It now takes about 2.8 s of CPU per survey while travelling. Before, the thread was
    busy for a whole flight.
- **Thread priority:** all background threads run below normal priority
  (`threads::background()`): the chunk, impostor, far-view, crows, scanner, collision
  and map threads.

**#186: draw lists and far trees**
- Far-tree boxes are drawn as the three sides that face the camera, with no bare winter
  twin. The bare colour rides in the vertex data, so winter and autumn look the same as
  before. In the woods there are 25% fewer boxes.
- The voxel terrain is culled a column at a time before chunk by chunk.
- The draw lists are kept from frame to frame.
- `band_scale` looks only at the columns near the camera.

**#187: world access on the main thread**
- Measured first: the main thread generates no columns of its own while moving. The
  collision lookups already fetch the columns around the camera.
- So only the counter went in: `sync_cols` and `sync_ms` in the CSV.

**#188: upload spikes and distant terrain**
- **Streamer uploads:** the upload budget is also checked against the next column, and
  each column goes to the GPU as one span and one write.
- **Far view:** a continent is put up about 1 ms a frame and shown only when it is whole.
  Its memory is freed a little at a time as well.
- **Far trees:** they upload within a budget of their own.
- **Re-sorting the wanted columns:** each sort key is worked out once. A sort went from
  5–15 ms to about 0.4–1.1 ms, in the same order.
- **Coarse columns:** they look only for the trees they draw. A test checks their meshes
  are byte-identical.

## Lessons

- **Fewer instructions is not faster.** Splitting the light-shaft weights into two small
  arrays read by index cut the `exp` calls from 25 to 10, and made the pass 50% slower on
  the GTX 1060. Indexed local arrays can end up in slow memory. Time every shader change;
  a pixel match is not enough.
- **Measure before building.** The plan expected main-thread world generation to cause the
  6–15 ms spikes in `sub_near`, but it never generated a column while moving. Those spikes
  were the thread being pre-empted, and they went away with the lower background priority.
- **Never hold a lock across rayon.** A pool worker that waits can run another job on the
  same thread. Any lock it holds is then taken again by that thread, and it deadlocks.
- **Every wait needs a deadline.** One stuck job hung a screenshot for 2.5 hours before
  this.
- **A clock is part of a screenshot.** Without `VOXEL_CLOCK`, two shots of one build
  differed by up to 200 in a channel, which hid any real difference.

## Left open

- **P8, GPU occlusion culling (Hi-Z):** not started, by choice. Estimated 1–2 ms in woods
  and valleys.
  - It needs a frame-by-frame diff along a fixed camera path, with culling on and off.
  - It is the riskiest item.
- **Single-write upload stalls:** a single column's `write_buffer` sometimes stalls for
  6–9 ms, probably the driver allocating a staging buffer. A staging belt or a mapped
  staging buffer would be the next step.
- **The cloud slowdown from #150:** 0.7 ms, above broken cloud. See #183.
- **`acquire` stalls in flights:** on a shared machine these came from other processes'
  GPU work. They have not been seen on an idle machine.
- **Laptop mode is not exact:** Rust's `available_parallelism` ignores the affinity mask
  on Windows, so the chunk pool is still sized for all the machine's cores.

## Waiting for the owner's yes

None of these is done. Each one changes the picture, or trades away a safety check.

| Id | Change | Gain (est.) | What you'd notice |
|---|---|---|---|
| A1 | Aurora at half resolution, or from a lookup table | 1–2 ms more | Slightly softer curtains |
| A2 | Render at 85–90% on slow GPUs | about 10% in woods | Softer image |
| A3 | Plants drawn to 60 m, not 100 m | 1–3 ms | Less plant detail mid-distance |
| A4 | Delay a forced cascade redraw by a frame | smaller spikes | Far shadows a frame late while turning |
| A5 | No far cascade by moonlight | about 0.3 ms at night | No moon shadows beyond 128 m |
| A6 | Coarser light shafts | 0.2–0.4 ms | Grainier shafts |
| A7 | Clouds at half resolution | 0.5–1 ms | Softer cloud edges |
| A8 | Dithered cascade seams | 0.2–0.5 ms | Faint dither at seams |
| A9 | Graphics settings menu, defaults unchanged | up to 3 ms | Only what the player changes |
| A10 | Rg11b10 HDR buffer | 0.1–0.3 ms | Possible banding in dark skies |
| A11 | wgpu's indirect-draw validation off | up to 0.7 ms CPU | A bad draw faults instead of erroring |
| A12 | Border columns wait for all four corner continents | correctness | Border lighting and marks change |
| A13 | One fewer chunk worker | steadier frames | Terrain loads a little later |
| A14 | Coarser `lod::SAMPLE_STRIDE` | up to half of coarse build time | Distant hills shift slightly |
