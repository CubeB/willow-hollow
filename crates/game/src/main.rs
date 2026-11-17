mod arena;
mod audio;
mod aurora;
mod camera;
mod camp;
mod character;
mod commands;
mod crows;
mod editor;
mod env;
mod fallen;
mod farbuilt;
mod farview;
mod gfx;
mod impostors;
mod insects;
mod inventory;
mod journal;
mod lod;
mod minimap;
mod overlay;
mod particles;
mod placed;
mod player;
mod profile;
mod render;
mod season;
mod starlings;
mod settings;
mod weather;
mod terrain;
mod threads;
mod vehicles;

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    time::Instant,
};

use anyhow::{Context, Result, bail};
use audio::{Audio, Ground};
use glam::DVec3;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};
use worldgen::{
    World,
    scale::{OLD_VOXEL, m, m64},
    world::GameWorld,
};

use camera::{Camera, FlyController};
use env::Environment;
use farview::FarView;
use gfx::{Gpu, WindowSurface};
use impostors::Impostors;
use minimap::Minimap;
use overlay::Overlay;
use particles::{Particles, Surroundings};
use player::{Controls, Mode, Player, WorldVoxels};
use profile::CpuTimer;
use render::Renderer;
use settings::{Overrides, Progress, Settings, Store, Travel};
use terrain::Streamer;
use vehicles::Fleet;

struct Options {
    /// Choices that take precedence over the saved settings, for this run.
    overrides: Overrides,
    /// Camera start: world position, then yaw and pitch in degrees.
    pos: DVec3,
    /// If set, start at this (x, z) and this far above the ground or water there.
    over: Option<(i32, i32, f64)>,
    /// Start flying instead of on foot. Implied by --pos and --over.
    fly: bool,
    /// On a screenshot, stand the player's figure at the start and look from behind it,
    /// or from this many degrees round it.
    third: Option<f32>,
    /// What the figure on a screenshot is doing: running at this speed, or gliding with
    /// the wing this far open.
    figure_speed: f32,
    figure_glide: Option<f32>,
    /// With --figure-glide, the wing banked this many degrees (right wing down) and the
    /// figure rising this many metres a second (negative diving).
    figure_bank: f32,
    figure_rise: f32,
    /// How far the figure on a screenshot has swung its furled glider up from its back.
    figure_furl: f32,
    look: (f32, f32),
    /// Time of day in hours, if not the one left off at.
    time: Option<f32>,
    /// Time of year, 0 to 1 from the spring equinox, if not the one left off at.
    date: Option<f32>,
    /// The world's year, from 0 for its first, if not the one left off in.
    year: Option<u32>,
    /// Weather to hold for this run (clear, overcast, rain, snow, storm, fog).
    weather: Option<weather::Forced>,
    /// Start from the spawn point, ignoring any saved position.
    fresh: bool,
    /// A murmuration of starlings ahead, whatever the date, hour and country.
    murmuration: bool,
    /// Butterflies set out this many metres ahead of a screenshot's camera.
    shot_butterflies: Option<f64>,
    /// The northern lights out every night, wherever, or never; as the night has them
    /// when not given.
    aurora: Option<bool>,
    /// Console commands to run at the start, in order.
    commands: Vec<String>,
    /// Show the console on a screenshot, with what those commands said; in a run, open it.
    shot_console: bool,
    /// Boats left afloat on a screenshot (by `spawn boat`), and their headings.
    shot_boats: Vec<(DVec3, f32)>,
    /// Render one frame to this PNG without opening a window, then exit.
    screenshot: Option<PathBuf>,
    /// Write the map of the continent around the start to this PNG, then exit.
    map: Option<PathBuf>,
    audio: bool,
    size: (u32, u32),
    /// Exit after this many frames (smoke tests).
    frames: Option<u64>,
    /// Fly straight ahead at this many metres a second, to measure streaming.
    drift: Option<f32>,
    /// Turn while drifting, so as to fly a circle of this radius in metres.
    circle: Option<f32>,
    /// Seconds of ambient life (leaves, birds, spray) simulated before a screenshot.
    ambient: f32,
    /// Draw the map on a screenshot: the corner map, or the large map this many voxels
    /// across.
    shot_map: Option<Option<f64>>,
    /// The large map on a screenshot turned and tilted, in degrees, and moved off the
    /// player, in metres east and south.
    shot_turn: Option<(f32, f32)>,
    shot_pan: Option<glam::DVec2>,
    /// Show the pack's window on a screenshot, with a few things in it.
    shot_pack: bool,
    /// Show the hotbar on a screenshot, with a few things in it.
    shot_hotbar: bool,
    /// A line of words to show along the foot of a screenshot, as a prompt is shown.
    shot_prompt: Option<String>,
    /// Make camp ahead of the camera before a screenshot: a fire in this state.
    shot_camp: Option<u8>,
    /// How far ahead that camp is made, in metres.
    camp_ahead: f64,
    /// With a camp made: lying by its fire on a screenshot, or resting by it from the
    /// first frame of a run.
    rest: bool,
    /// Hold a lit lantern on a screenshot.
    shot_lantern: bool,
    /// Set crows out on a screenshot: feeding on the ground, perched, flying, landing
    /// or at the rookery, at the place of that sort nearest to a point ahead.
    shot_crows: Option<String>,
    /// How far ahead that point is, in metres.
    crows_ahead: f64,
    /// Pick every flower within this many metres of the camera before a screenshot.
    picked: Option<f32>,
    /// Show the journal's window on a screenshot, open at this page, written up as if
    /// after a walk to this many of the places nearest the camera.
    shot_journal: Option<journal::Page>,
    journal_fill: usize,
    /// Where the player is taken to have walked before a screenshot, world x and z.
    walked: Vec<glam::DVec2>,
    /// Open the structure editor instead of the game, starting as these say.
    structures: Option<editor::Options>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            overrides: Overrides::default(),
            pos: DVec3::new(0.0, 90.0, 0.0),
            over: None,
            fly: false,
            third: None,
            figure_speed: 4.16,
            figure_glide: None,
            figure_bank: 0.0,
            figure_rise: -1.5,
            figure_furl: 0.0,
            look: (35.0, -18.0),
            time: None,
            date: None,
            year: None,
            weather: None,
            fresh: false,
            murmuration: false,
            shot_butterflies: None,
            aurora: None,
            commands: Vec::new(),
            shot_console: false,
            shot_boats: Vec::new(),
            screenshot: None,
            map: None,
            audio: true,
            size: (1280, 720),
            frames: None,
            drift: None,
            circle: None,
            ambient: 8.0,
            shot_map: None,
            shot_turn: None,
            shot_pan: None,
            shot_pack: false,
            shot_hotbar: false,
            shot_prompt: None,
            shot_camp: None,
            camp_ahead: CAMP_AHEAD,
            rest: false,
            shot_lantern: false,
            shot_crows: None,
            crows_ahead: 12.0,
            picked: None,
            shot_journal: None,
            journal_fill: 16,
            walked: Vec::new(),
            structures: None,
        }
    }
}

const USAGE: &str = "\
usage: game [options]
Options hold for one run. Settings and your place in each world are saved between runs.
  --seed N              play world N, from where you left off (default: a new random
                        world each run, starting on foot at its spawn point)
  --fresh               start from the spawn point instead of where you left off
  --pos X,Y,Z           camera start position in voxels
  --over X,Z,H          start H voxels above the ground (or water) at X,Z
  --fly                 start in free-fly mode (F toggles in game)
  --third               on a screenshot, show the player's figure from behind (V in game)
  --figure DEG          as --third, with the figure turned DEG degrees from the view
  --figure-speed V      the figure runs at V metres a second (default 4.16)
  --figure-glide T      the figure glides, its wing T of the way open (0-1)
  --figure-bank DEG     with --figure-glide, the wing banked DEG degrees, right wing down
  --figure-rise V       with --figure-glide, rising V metres a second (default -1.5)
  --figure-furl T       the figure has its furled glider T of the way up from its back (0-1)
  --look YAW,PITCH      camera direction in degrees
  --time H              time of day, 0-24
  --season S            spring, summer, autumn or winter
  --date F              time of year, 0-1 from the spring equinox, or a month (may, october):
                        each flower has its own months
  --year N              the world's year, counting its first as 0 (the journal's first year)
  --weather W           hold the weather: clear, fair, broken, overcast, rain, snow, storm or fog
  --radius N            chunk load radius
  --no-audio            run without sound
  --screenshot FILE     render one frame to a PNG without a window, then exit
  --map FILE            write a map of the continent around the start to a PNG, then exit
  --minimap             draw the corner map on a screenshot
  --pack                show the pack's window on a screenshot, with a few things in it
  --hotbar              show the hotbar on a screenshot, with a few things in it
  --prompt TEXT         show TEXT along the foot of a screenshot, as a prompt is
  --camp STATE          make camp a few steps ahead before a screenshot: a fire that is
                        laid, lit, out (put out a moment ago) or burnt, and a few
                        things set down
  --camp-ahead M        make that camp M metres ahead instead (default 4.2), to see its
                        smoke from far off
  --rest                with --camp: on a screenshot, the view lying by the fire, as
                        high as --look says; in a run, rest by the fire at once
  --lantern             hold a lit lantern on a screenshot (in the figure's hand with --third)
  --crows HOW           set crows out on a screenshot near a point ahead: ground, perch,
                        tree, rock, fly, land, roost or any (they never take fright)
  --crows-ahead M       that point M metres ahead (default 12)
  --picked M            pick every flower within M metres before a screenshot
  --journal [PAGE]      show the journal on a screenshot, at finds, places, walk or days,
                        written up as if after a walk round the places near the camera
  --journal-fill N      with --journal, a walk to the N nearest places (default 16; 0 for
                        a journal with nothing in it yet)
  --bigmap KM           draw the large map, KM across, on a screenshot
  --bigmap-turn YAW,PITCH  with --bigmap, the large map turned YAW and tilted PITCH degrees
                        (as a right drag would; the map opens at about -17,49)
  --bigmap-pan X,Z      with --bigmap, the large map moved X metres east and Z south
  --walked X,Z;X,Z..    on a screenshot's map, as if walked from X,Z through the rest
                        to the start
  --ambient S           seconds of ambient life before a screenshot (default 8)
  --murmuration         starlings gathering ahead of the camera, whatever the date and hour
  --butterflies M       on a screenshot, a few butterflies M metres ahead of the camera
  --aurora              the northern lights out tonight, wherever the camera is
  --command LINE        run a console command at the start (` or / opens the console in
                        game; 'help' lists the commands); give it as often as you like.
                        On a screenshot: weather, time, date, season, year, tp, spawn camp,
                        murmuration, crows and boat, and aurora
  --console             show the console on a screenshot, with what --command said; in
                        a run, start with it open
  --size WxH            screenshot size, or window size with --frames (default 1280x720)
  --day-minutes M       real minutes per day, 0 to freeze time
  --frames N            exit after N frames, logging where the time went (with --bigmap,
                        the large map is open throughout)
  --drift V             fly straight ahead at V metres a second
  --circle R            with --drift, fly a circle of radius R metres instead
  --no-vsync            uncapped frame rate

The structure editor, for looking at the landmarks and drawing your own:
  --structures          open it instead of the game
  --structure WHAT      start at a kind (shrine), a saved one (shrine/high-gate), landing,
                        lantern/N or signpost
  --variant N           the generator's variant N
  --place N             seen in place N of its list (0 to 8)
  --layer L             cut away everything above layer L
  --copy                start drawing on a copy of the variant
  --orbit YAW,PITCH[,D] the view, in degrees, D voxels off
  --hover X,Y           on a screenshot, as if the cursor were at X,Y
  --structures-dir DIR  read and save structures in DIR, not the repository's
  With --screenshot FILE and --size, draws the editor once to a PNG.";

fn list<T: std::str::FromStr>(s: &str, sep: char, n: usize) -> Result<Vec<T>> {
    let v: Vec<T> = s
        .split(sep)
        .map(|p| p.trim().parse::<T>().ok())
        .collect::<Option<_>>()
        .with_context(|| format!("could not parse '{s}'"))?;
    if v.len() != n {
        bail!("expected {n} values separated by '{sep}' in '{s}'");
    }
    Ok(v)
}

fn parse_args() -> Result<Options> {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        let mut value = || args.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--seed" => o.overrides.seed = Some(value()?.parse()?),
            "--fresh" => o.fresh = true,
            "--murmuration" => o.murmuration = true,
            "--butterflies" => o.shot_butterflies = Some(value()?.parse::<f64>()?.clamp(0.5, 50.0)),
            "--aurora" => o.aurora = Some(true),
            "--command" => o.commands.push(value()?),
            "--console" => o.shot_console = true,
            "--pos" => {
                let v = list::<f64>(&value()?, ',', 3)?;
                o.pos = DVec3::new(v[0], v[1], v[2]);
                o.over = None;
                o.fly = true;
                o.overrides.placed = true;
            }
            "--fly" => o.fly = true,
            "--third" => o.third = Some(0.0),
            "--figure" => o.third = Some(value()?.parse()?),
            "--figure-speed" => o.figure_speed = value()?.parse()?,
            "--figure-glide" => o.figure_glide = Some(value()?.parse()?),
            "--figure-bank" => o.figure_bank = value()?.parse()?,
            "--figure-rise" => o.figure_rise = value()?.parse()?,
            "--figure-furl" => o.figure_furl = value()?.parse()?,
            "--over" => {
                let v = list::<f64>(&value()?, ',', 3)?;
                o.over = Some((v[0] as i32, v[1] as i32, v[2]));
                o.fly = true;
                o.overrides.placed = true;
            }
            "--look" => {
                let v = list::<f32>(&value()?, ',', 2)?;
                o.look = (v[0], v[1]);
            }
            "--time" => o.time = Some(value()?.parse()?),
            "--ambient" => o.ambient = value()?.parse::<f32>()?.clamp(0.0, 600.0),
            "--date" => {
                let v = value()?;
                let date = match season::month(&v) {
                    Some(date) => date,
                    None => v.parse::<f32>().map_err(|_| anyhow::anyhow!("--date takes a number from 0 to 1 or a month, not {v}"))?,
                };
                o.date = Some(date.rem_euclid(1.0));
            }
            "--year" => o.year = Some(value()?.parse().map_err(|_| anyhow::anyhow!("--year takes a whole number from 0"))?),
            "--weather" => {
                let name = value()?;
                o.weather = Some(weather::Forced::named(&name).ok_or_else(|| anyhow::anyhow!("unknown weather {name}"))?);
            }
            "--season" => {
                let name = value()?;
                o.date = Some(season::named(&name).ok_or_else(|| anyhow::anyhow!("unknown season {name}"))?);
            }
            "--radius" => o.overrides.radius = Some(value()?.parse::<i32>()?.clamp(1, 64)),
            "--no-audio" => o.audio = false,
            "--screenshot" => o.screenshot = Some(value()?.into()),
            "--map" => o.map = Some(value()?.into()),
            "--minimap" => o.shot_map = Some(None),
            "--pack" => o.shot_pack = true,
            "--hotbar" => o.shot_hotbar = true,
            "--prompt" => o.shot_prompt = Some(value()?),
            "--rest" => o.rest = true,
            "--lantern" => o.shot_lantern = true,
            "--crows" => {
                let how = value()?;
                if !commands::CROWS.contains(&how.as_str()) {
                    bail!("unknown crows '{how}': ground, perch, tree, rock, fly, land, roost or any");
                }
                o.shot_crows = Some(how);
            }
            "--crows-ahead" => o.crows_ahead = value()?.parse::<f64>()?.clamp(0.0, 500.0),
            "--camp-ahead" => o.camp_ahead = value()?.parse::<f64>()?.clamp(2.0, 1000.0),
            "--camp" => {
                let word = value()?;
                o.shot_camp = Some(commands::fire_state(&word).ok_or_else(|| anyhow::anyhow!("unknown fire '{word}': laid, lit, out or burnt"))?);
            }
            "--picked" => o.picked = Some(value()?.parse()?),
            "--journal" => {
                // The page is optional: without one the journal opens at its first.
                let page = args.next_if(|v| !v.starts_with("--"));
                o.shot_journal = Some(match page {
                    Some(word) => journal::Page::named(&word).ok_or_else(|| anyhow::anyhow!("unknown journal page {word}"))?,
                    None => journal::Page::default(),
                });
            }
            "--journal-fill" => o.journal_fill = value()?.parse()?,
            "--bigmap" => o.shot_map = Some(Some(m64(value()?.parse::<f64>()? * 1000.0))),
            "--bigmap-turn" => {
                let v = list::<f32>(&value()?, ',', 2)?;
                o.shot_turn = Some((v[0], v[1]));
            }
            "--bigmap-pan" => {
                let v = list::<f64>(&value()?, ',', 2)?;
                o.shot_pan = Some(glam::DVec2::new(m64(v[0]), m64(v[1])));
            }
            "--walked" => {
                for p in value()?.split(';') {
                    let v = list::<f64>(p, ',', 2)?;
                    o.walked.push(glam::DVec2::new(v[0], v[1]));
                }
            }
            "--size" => {
                let v = list::<u32>(&value()?, 'x', 2)?;
                o.size = (v[0].max(1), v[1].max(1));
            }
            "--day-minutes" => o.overrides.day_minutes = Some(value()?.parse()?),
            "--frames" => o.frames = Some(value()?.parse()?),
            "--drift" => o.drift = Some(value()?.parse()?),
            "--circle" => o.circle = Some(value()?.parse::<f32>()?.max(1.0)),
            "--no-vsync" => o.overrides.vsync = Some(false),
            "--structures" => drop(o.structures.get_or_insert_default()),
            "--structure" => o.structures.get_or_insert_default().show = Some(value()?),
            "--variant" => o.structures.get_or_insert_default().variant = Some(value()?.parse()?),
            "--place" => o.structures.get_or_insert_default().place = Some(value()?.parse()?),
            "--layer" => o.structures.get_or_insert_default().layer = Some(value()?.parse()?),
            "--copy" => o.structures.get_or_insert_default().copy = true,
            "--structures-dir" => o.structures.get_or_insert_default().dir = Some(value()?.into()),
            "--orbit" => {
                let v: Vec<f32> = value()?.split(',').map(|w| w.trim().parse()).collect::<Result<_, _>>()?;
                anyhow::ensure!(v.len() == 2 || v.len() == 3, "--orbit takes YAW,PITCH or YAW,PITCH,DISTANCE");
                o.structures.get_or_insert_default().orbit = Some((v[0], v[1], v.get(2).copied()));
            }
            "--hover" => {
                let v = list::<f32>(&value()?, ',', 2)?;
                o.structures.get_or_insert_default().hover = Some((v[0], v[1]));
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}'\n{USAGE}"),
        }
    }
    // A plain run is a new game: a random world, on foot at its spawn point. Runs that
    // only look at a place (screenshots, maps, smoke tests, --pos/--over) keep the
    // saved seed so they stay repeatable.
    if o.overrides.seed.is_none() && !o.overrides.placed && o.screenshot.is_none() && o.map.is_none() && o.frames.is_none() {
        use std::hash::{BuildHasher, Hasher};
        let seed = std::collections::hash_map::RandomState::new().build_hasher().finish() as u32;
        o.overrides.seed = Some(seed);
        o.overrides.random = true;
        o.fresh = true;
    }
    Ok(o)
}

/// Height of the walkable or swimmable surface at a column.
fn surface_height(world: &GameWorld, x: i32, z: i32) -> f64 {
    let col = world.column(x, z);
    col.water_surface().unwrap_or(col.top() + 1).max(col.top() + 1) as f64
}

/// The starting view: an explicit position if one was given, else where the player
/// left off, else the spawn point.
fn make_camera(opts: &Options, world: &GameWorld, saved: Option<&Progress>) -> Camera {
    if let Some(p) = saved {
        let mut pos = p.pos;
        if p.travel != Travel::Flying {
            if p.travel == Travel::Walking {
                // In case the terrain here has changed since the save was made.
                pos.y = pos.y.max(surface_height(world, pos.x.floor() as i32, pos.z.floor() as i32));
            }
            pos.y += player::EYE_HEIGHT;
        }
        let mut camera = Camera::new(pos, 0.0, 0.0);
        (camera.yaw, camera.pitch) = (p.yaw, p.pitch);
        return camera;
    }
    if let Some((x, z, h)) = opts.over {
        let pos = DVec3::new(x as f64 + 0.5, surface_height(world, x, z) + h, z as f64 + 0.5);
        return Camera::new(pos, opts.look.0, opts.look.1);
    }
    if opts.fly {
        return Camera::new(opts.pos, opts.look.0, opts.look.1);
    }
    let spawn = world.spawn_point();
    let feet = surface_height(world, spawn.x, spawn.z);
    let pos = DVec3::new(spawn.x as f64 + 0.5, feet + player::EYE_HEIGHT, spawn.z as f64 + 0.5);
    Camera::new(pos, spawn.yaw, -4.0)
}

fn fullscreen(on: bool) -> Option<winit::window::Fullscreen> {
    on.then_some(winit::window::Fullscreen::Borderless(None))
}

/// How far round the camera (metres) flowers are looked for, for butterflies to rise from.
const BUTTERFLY_REACH: f32 = 12.0;

/// How close a boat must be to climb aboard.
const BOARD_REACH: f64 = m64(4.8);

/// How far ahead the hands reach to set a thing down.
const PLACE_REACH: f64 = m64(3.0);
/// How long a fire burns once lit, in days: from dusk until well into the morning.
const FIRE_BURNS: f32 = 14.0 / 24.0;
/// How long a fire smoulders once it has gone out, in days.
const SMOULDERS: f32 = 1.5 / 24.0;
/// How far ahead a camp asked for (`--camp`, `spawn camp`) is made, in metres: a few steps.
const CAMP_AHEAD: f64 = 4.2;
/// How long the name of what has come to hand stays up, in seconds.
const HAND_WORD: f32 = 2.6;

/// The ground under `(x, z)` nearest to height `y`: the top of the highest solid voxel
/// with open air over it, looked for a little way up and down. None in water, or where
/// there is no such ground.
fn ground_near(voxels: &mut WorldVoxels, x: f64, y: f64, z: f64) -> Option<f64> {
    use player::Voxels;
    let (vx, vz, vy) = (x.floor() as i32, z.floor() as i32, y.floor() as i32);
    for top in (vy - 5..=vy + 3).rev() {
        if voxels.kind(vx, top, vz) == voxel::Kind::Solid {
            let over = voxels.kind(vx, top + 1, vz);
            return (over != voxel::Kind::Solid && over != voxel::Kind::Water).then_some(top as f64 + 1.0);
        }
    }
    None
}

/// The fires of the camps that were in the world already, near `pos`: the top of each
/// bed of embers, once.
fn embers_near(streamer: &Streamer, voxels: &mut WorldVoxels, pos: DVec3) -> Vec<DVec3> {
    let mut embers: Vec<DVec3> = Vec::new();
    for p in streamer.lamps_near(pos, m(24.0)) {
        let top = p.as_dvec3() + DVec3::new(0.5, 1.0, 0.5);
        if voxels.voxel(p.x, p.y, p.z) == worldgen::materials::EMBER && embers.iter().all(|e| e.distance(top) > m64(1.6)) {
            embers.push(top);
        }
    }
    embers
}

/// Where a fire's light comes from, and how brightly this moment: it never burns steady.
fn firelight(heart: DVec3, clock: f32) -> (DVec3, f32) {
    let phase = (heart.x * 0.7 + heart.z * 1.3) as f32;
    let flicker = 0.16 * (clock * 11.0 + phase).sin() + 0.09 * (clock * 23.0 + phase * 2.0).sin() + 0.05 * (clock * 4.1).sin();
    (heart + DVec3::new(0.0, m64(0.25), 0.0), 1.25 + flicker)
}

/// How brightly a lantern's candle burns at `clock` seconds: steady, with a small
/// unsettled flicker, less than a fire's.
fn candle(clock: f32) -> f32 {
    1.15 + 0.05 * (clock * 7.3).sin() + 0.035 * (clock * 17.9 + 1.3).sin() + 0.03 * (clock * 2.3).sin()
}

/// Where someone resting by `fire` lies, coming to it from `from`: on the ground a step
/// short of its ring, feet to the warmth.
fn by_the_fire(voxels: &mut WorldVoxels, fire: &placed::Thing, from: DVec3) -> DVec3 {
    let toward = DVec3::new(from.x - fire.pos.x, 0.0, from.z - fire.pos.z).normalize_or(DVec3::X);
    let p = fire.pos + toward * m64(1.3);
    DVec3::new(p.x, ground_near(voxels, p.x, fire.pos.y + 1.0, p.z).unwrap_or(fire.pos.y), p.z)
}

/// Makes camp `far` metres ahead of someone whose eye is at `eye`, looking along `yaw`:
/// a fire in `state` and a few things set down about it. Behind them or to one side
/// instead, if there is water or a cliff ahead. For showing a camp without having made
/// one. False if there was no ground about to make it on.
fn sample_camp(camp: &mut placed::Placed, voxels: &mut WorldVoxels, eye: DVec3, yaw: f32, state: u8, far: f64) -> bool {
    use worldgen::features::plants::Flower;
    let foot = eye.y - player::EYE_HEIGHT;
    for turned in [0.0, std::f32::consts::PI, std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2] {
        let yaw = yaw + turned;
        let (ahead, across) = (DVec3::new(yaw.cos() as f64, 0.0, yaw.sin() as f64), DVec3::new(-yaw.sin() as f64, 0.0, yaw.cos() as f64));
        let mut spot = |along: f64, aside: f64| {
            let along = along + far - 4.2;
            let p = eye + ahead * m64(along) + across * m64(aside);
            let y = ground_near(voxels, p.x, foot + 2.0, p.z).or_else(|| ground_near(voxels, p.x, foot - 4.0, p.z));
            // Far off, the ground may be well above or below the feet.
            let y = y.or_else(|| (along > 10.0).then(|| (-30..30).rev().find_map(|k| ground_near(voxels, p.x, foot + 8.0 * k as f64, p.z))).flatten())?;
            Some(DVec3::new(p.x, y, p.z))
        };
        let Some(fire) = spot(4.2, 0.0) else { continue };
        log::info!("camp made at {:.0} {:.0} {:.0}, {:.0} m from the eye", fire.x, fire.y, fire.z, fire.distance(eye) / m64(1.0));
        camp.put(placed::Thing::new(inventory::campfire(), 1, fire, yaw));
        if let Some(at) = camp.within_reach(fire).filter(|_| state != placed::LAID) {
            camp.set_state(at, state);
        }
        // A fire burnt down has long since stopped smoking; one put out has only just.
        if state == placed::BURNT {
            camp.pass(1.0, f64::MAX);
        }
        for (item, count, along, aside, turn) in [
            (inventory::sticks(), 3, 3.6, -1.3, 0.4),
            (inventory::of_flower(Flower::Poppy), 3, 3.0, 0.9, 2.0),
            (inventory::of_flower(Flower::PennyBun), 2, 3.3, 1.5, 0.0),
            (inventory::of_flower(Flower::Bluebell), 1, 2.6, -0.5, 1.0),
        ] {
            if let Some(at) = spot(along, aside) {
                camp.put(placed::Thing::new(item, count, at, yaw + turn));
            }
        }
        return true;
    }
    log::warn!("there is no ground here to make camp on");
    false
}

/// The nearest water ahead of `eye` along `yaw`, within 80 m, deep enough to float a
/// boat on: where on its surface.
fn water_ahead(world: &GameWorld, eye: DVec3, yaw: f32) -> Option<DVec3> {
    let ahead = DVec3::new(yaw.cos() as f64, 0.0, yaw.sin() as f64);
    let afloat = |metres: f64| {
        let p = eye + ahead * m64(metres);
        let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
        let col = world.column(x, z);
        let surface = col.water_surface().filter(|w| w - col.top() >= 2)?;
        Some(DVec3::new(x as f64 + 0.5, surface as f64, z as f64 + 0.5))
    };
    let shore = (2..80).find(|&metres| afloat(metres as f64).is_some())? as f64;
    // Out from the edge, clear of the reeds, as far as the water goes.
    (0..4).rev().find_map(|out| afloat(shore + out as f64))
}

/// The named places on the continent under `at`, nearest first: those whose names have
/// every word of `query` in them, or all of them for an empty one; at most `n`.
fn named_places(world: &GameWorld, at: DVec3, query: &str, n: usize) -> Vec<commands::Named> {
    let Some((c, local)) = journal::continent_at(world, at, true) else { return Vec::new() };
    let origin = glam::DVec2::new(at.x - local.x as f64, at.z - local.y as f64);
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found: Vec<&worldgen::names::Place> = c.trails.names.places.iter().filter(|p| {
        let name = p.name.to_lowercase();
        words.iter().all(|w| name.contains(w.as_str()))
    }).collect();
    // A name given in full goes before those it is only part of.
    let whole = query.trim().to_lowercase();
    found.sort_by(|a, b| (a.name.to_lowercase() != whole).cmp(&(b.name.to_lowercase() != whole)).then(a.at.distance(local).total_cmp(&b.at.distance(local))));
    found.truncate(n);
    found.into_iter().map(|p| commands::Named { name: p.name.clone(), kind: p.kind.key(), x: origin.x + p.at.x as f64, z: origin.y + p.at.y as f64 }).collect()
}

/// Voxel positions with their linear colours.
type Spots = Vec<(DVec3, [f32; 3])>;
type Leaves = Vec<particles::LeafSite>;

/// Which shape of leaf a tree's foliage sheds; plants and vines shed none, and nor do
/// holly, yew and the other winter evergreens, which are left out here.
fn leaf_shape(v: voxel::Voxel) -> Option<particles::LeafShape> {
    use particles::LeafShape::*;
    use worldgen::materials as mat;
    Some(match v {
        mat::NEEDLES | mat::NEEDLES_REDWOOD => Needle,
        mat::LEAVES_WILLOW | mat::LEAVES_ACACIA | mat::LEAVES_PALM => Slip,
        mat::LEAVES_MAPLE | mat::LEAVES_MAPLE_RED => Autumn,
        mat::LEAVES_CHERRY => Petal,
        mat::LEAVES_OAK | mat::LEAVES_BIRCH | mat::LEAVES_JUNGLE | mat::LEAVES_BAOBAB => Broad,
        _ => return None,
    })
}

/// Samples voxels around the camera for leaves (to shed), asks the world which flowers
/// stand near and are out on `date` (for butterflies), and reads the biome. Cheap enough
/// to do every couple of seconds.
fn scan_surroundings(
    world: &GameWorld,
    voxels: &mut WorldVoxels,
    camera: DVec3,
    salt: u32,
    date: f32,
) -> Scan {
    let (cx, cz) = (camera.x.floor() as i32, camera.z.floor() as i32);
    let biome = world.column(cx, cz).biome;
    let (mut leaves, mut flowers) = (Vec::new(), Vec::new());
    let mats = world.materials();
    let to_linear = |c: [u8; 3]| c.map(|v| (v as f32 / 255.0).powf(2.2));
    let mut rng = salt.wrapping_mul(0x9e37_79b9) ^ (cx as u32).wrapping_mul(31) ^ (cz as u32).wrapping_mul(17) | 1;
    let half = (30.0 * OLD_VOXEL) as i32;
    let span = 2 * half as u32 + 1;
    let (under, above) = ((40.0 * OLD_VOXEL) as i32, (40.0 * OLD_VOXEL) as i32);
    // Enough looks to find most of the trees in reach, not just a few of them.
    for _ in 0..1100 {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let x = cx + (rng % span) as i32 - half;
        let z = cz + ((rng >> 8) % span) as i32 - half;
        let top = camera.y as i32 + above;
        let mut below = voxel::AIR;
        for y in ((camera.y as i32 - under)..=top).rev() {
            let v = voxels.voxel(x, y, z);
            let kind = mats.get(v).kind;
            if kind == voxel::Kind::Foliage && mats.get(below).kind == voxel::Kind::Air {
                if let Some(shape) = leaf_shape(v) {
                    // Leaves fall from under the canopy as well as off its top: find its
                    // underside, where a falling leaf comes into view.
                    let mut under = y;
                    while under > y - 40 && mats.get(voxels.voxel(x, under - 1, z)).kind == voxel::Kind::Foliage {
                        under -= 1;
                    }
                    let m = mats.get(v);
                    let open = mats.get(voxels.voxel(x, under - 1, z)).kind == voxel::Kind::Air;
                    leaves.push(particles::LeafSite {
                        pos: DVec3::new(x as f64, y as f64, z as f64),
                        under: open.then_some(under as f64),
                        color: to_linear(m.color),
                        shape,
                        plant: m.plant,
                        tag: voxels.leaf_tag(x, y, z),
                    });
                }
                break;
            }
            if kind == voxel::Kind::Solid {
                break;
            }
            below = v;
        }
    }
    // The flowers are plants standing on the ground, not voxels of it: the world names
    // them. Mushrooms draw no butterflies, nor does a flower out of bloom, and a picked
    // flower is no longer there.
    let reach = m(BUTTERFLY_REACH) as i32;
    for (at, flower) in world.flowers_standing(cx - reach, cz - reach, 2 * reach + 1) {
        use worldgen::features::plants::Flower;
        let out = flower.blooming_at(date, at.x, at.z);
        if out && !matches!(flower, Flower::FlyAgaric | Flower::PennyBun) && (at.y as f64 - camera.y).abs() < m64(12.0) {
            flowers.push((at.as_dvec3(), to_linear(flower.color())));
        }
    }
    // And the sticks fallen under the trees about.
    let sticks = fallen::near(world, voxels, camera);
    // The nests in sight, and the flowers in bloom a bee from each could fly to: a few
    // nests at most, so it is a few small squares of the world to look over.
    let nests = world.nests_near(cx, cz, m(insects::NEST_SIGHT as f32) as i32);
    let mut forage = Vec::new();
    for n in nests.iter().filter(|n| n.kind == worldgen::features::nests::NestKind::Bees && insects::on_the_wing(n.kind, date)).take(3) {
        let reach = m(insects::FORAGE as f32) as i32;
        for (at, flower) in world.flowers_standing(n.door.x - reach, n.door.z - reach, 2 * reach + 1) {
            use worldgen::features::plants::Flower;
            if flower.blooming_at(date, at.x, at.z) && !matches!(flower, Flower::FlyAgaric | Flower::PennyBun) {
                forage.push((at.as_dvec3(), to_linear(flower.color())));
            }
        }
    }
    (biome, leaves, flowers, low_ground(world, cx, cz), sticks, (nests, forage))
}

/// The lowest ground or water within a short walk: where mist gathers. Standing in a
/// valley that is underfoot; on a hill it is the valley below.
fn low_ground(world: &GameWorld, cx: i32, cz: i32) -> f32 {
    let mut low = surface_height(world, cx, cz);
    for reach in [m(50.0) as i32, m(140.0) as i32] {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            low = low.min(surface_height(world, cx + dx * reach, cz + dz * reach));
        }
    }
    low as f32
}

/// The biome, leaves and flowers round the camera, the low ground near it, and the
/// sticks lying under the trees.
type Scan = (worldgen::biome::Biome, Leaves, Spots, f32, Vec<fallen::Stick>, Bees);

/// The nests of bees and wasps near, and the flowers in bloom about them.
type Bees = (Vec<worldgen::features::nests::Nest>, Spots);

/// Runs [`scan_surroundings`] on its own thread. The scan can touch dozens of chunk
/// columns that aren't cached, which would stall a frame.
struct Scanner {
    ask: Sender<(DVec3, u32, f32)>,
    answer: Receiver<Scan>,
    busy: bool,
}

impl Scanner {
    fn new(world: Arc<GameWorld>) -> Self {
        let (ask, questions) = channel::<(DVec3, u32, f32)>();
        let (reply, answer) = channel();
        std::thread::Builder::new()
            .name("scanner".into())
            .spawn(move || {
                crate::threads::background();
                let mut voxels = WorldVoxels::new(world.clone());
                // Ends when the game drops `ask`.
                while let Ok((camera, salt, date)) = questions.recv() {
                    voxels.trim(camera);
                    let found = scan_surroundings(&world, &mut voxels, camera, salt, date);
                    if reply.send(found).is_err() {
                        break;
                    }
                }
            })
            .expect("scanner thread");
        Self { ask, answer, busy: false }
    }

    fn request(&mut self, camera: DVec3, salt: u32, date: f32) {
        if !self.busy {
            self.busy = self.ask.send((camera, salt, date)).is_ok();
        }
    }

    fn poll(&mut self) -> Option<Scan> {
        let found = self.answer.try_recv().ok()?;
        self.busy = false;
        Some(found)
    }
}

struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    surface: WindowSurface,
    overlay: Overlay,
    renderer: Renderer,
    streamer: Streamer,
    far: FarView,
    impostors: Impostors,
    env: Environment,
    particles: Particles,
    /// The crows about the player, and their nests.
    crows: crows::Crows,
    /// Leaf and flower voxels near the player, refreshed now and then.
    leaves: Leaves,
    flowers: Vec<(DVec3, [f32; 3])>,
    /// Sticks lying on the ground near the player, refreshed with them.
    fallen: Vec<fallen::Stick>,
    /// The bees' and wasps' nests near, and the flowers about them.
    nests: Bees,
    scan_timer: f32,
    scanner: Scanner,
    biome: worldgen::biome::Biome,
    /// How warm the biome underfoot is, eased as the player crosses into another.
    warmth: f32,
    /// The haze of the air here (season::humidity), eased as the biome changes.
    humidity: f32,
    /// Northern lights every night, wherever, or never (`--aurora`, the console's
    /// `aurora`); as the night has them when None.
    aurora_forced: Option<bool>,
    /// The console, its scrollback and what was typed into it.
    console: commands::Console,
    /// How many times faster than usual the player goes on foot (`speed`).
    pace: f32,
    /// Where the eye was before the last `tp`, for `back`.
    before_tp: Option<DVec3>,
    /// The low ground mist lies on near the camera, eased toward the last scan's answer.
    /// None until the first scan is in.
    mist_floor: Option<f32>,
    mist_target: f32,
    weather: weather::Weather,
    /// How high the clouds stand here (clear of the hills).
    cloud_base: weather::CloudBase,
    audio: Option<Audio>,
    /// The settings in force.
    settings: Settings,
    /// The settings as they stand on disk.
    saved: Settings,
    /// Where to save, if anywhere: smoke tests leave no trace.
    store: Option<Store>,
    save_timer: f32,
    show_settings: bool,
    /// What the player carries, its window, and whether that is open.
    pack: inventory::Pack,
    pack_view: inventory::View,
    pack_open: bool,
    /// What could be gathered from where the player stands, and from which column;
    /// looked for every so often.
    within_reach: Option<((i32, i32), inventory::Item)>,
    reach_timer: f32,
    /// The record of the walk, its window, whether that is open, and how long until the
    /// places round about are next looked over.
    journal: journal::Journal,
    journal_view: journal::View,
    journal_open: bool,
    journal_timer: f32,
    /// A word about what was just gathered, and how long it has left to show.
    notice: Option<(String, f32)>,
    /// What has been set down in this world: flowers, sticks, fires.
    placed: placed::Placed,
    /// The thing set down that is within reach, if one is; looked for with `within_reach`.
    thing_near: Option<placed::Handle>,
    /// The embers of camps that were here already, near the player: fires to rest by.
    embers: Vec<DVec3>,
    /// The rest being taken, if one is, and how far the eye is lowered for it, metres.
    rest: Option<camp::Rest>,
    lowered: f32,
    /// How long the name of what is held has left to show.
    hand_word: f32,
    /// Game years gone by since what was gathered was last looked over for regrowth,
    /// and how long until it is looked over again.
    grown: f64,
    grow_timer: f32,
    quit: bool,
    /// Distance walked since the last footstep, and how many there have been.
    stride: f32,
    steps: u32,
    /// How far out the glider is: 0 a rod on the back, to [`GLIDER_UP`] swung overhead,
    /// to 1 spread and flying.
    glider: f32,
    last_fall_speed: f32,
    was_grounded: bool,
    was_swimming: bool,
    /// Seconds since the player was last swimming: only going in after a while out
    /// splashes, not the chest bobbing in and out at the top of a swim.
    dry_for: f32,
    fleet: Fleet,
    /// Solid models to draw this frame.
    props: Vec<particles::ParticleInstance>,
    /// As `props`, but casting shadows only: the player's figure, seen through its eyes.
    shades: Vec<particles::ParticleInstance>,
    /// Glider key pressed since the last frame.
    deploy: bool,
    /// Space was pressed: opens the glider too, but only well clear of the ground.
    deploy_clear: bool,
    last_yaw: f32,
    /// The view is from behind the player's figure, not through its eyes.
    third: bool,
    /// The lantern's shutter is closed: held, it gives no light.
    shuttered: bool,
    gait: character::Gait,
    world: Arc<GameWorld>,
    camera: Camera,
    /// On foot, or flying free of collision.
    walking: bool,
    player: Player,
    voxels: WorldVoxels,
    /// How shut in by rock the eye has grown used to being (`player::enclosure`).
    shut_in: f32,
    fly: FlyController,
    keys: HashSet<KeyCode>,
    /// Whether the mouse is currently captured for looking around.
    grabbed: bool,
    /// Whether it should be: grabbing can fail until the window has focus, so it is
    /// retried every frame while this is set.
    want_grab: bool,
    /// Raw mouse motion has arrived at least once; until then, cursor position deltas
    /// (with the cursor re-centred each frame) drive the look instead.
    raw_motion_seen: bool,
    last_cursor: Option<(f64, f64)>,
    /// Where the cursor was last seen, for dragging the map.
    last_cursor_seen: Option<(f64, f64)>,
    last_frame: Instant,
    start: Instant,
    frame: u64,
    frame_ms: f32,
    loaded: bool,
    cpu: CpuTimer,
    /// Every frame's length in milliseconds, kept only when measuring.
    frame_log: Option<Vec<f32>>,
    sub: profile::SubTimer,
    last_dt_ms: f32,
    csv: Option<std::io::BufWriter<std::fs::File>>,
    /// Columns the main thread generated itself for a lookup, and the milliseconds that
    /// took: in the last frame, and in all frames (see `player::take_sync_loads`).
    sync_frame: (u32, f32),
    sync_total: (u32, f32),
    drift: Option<f32>,
    circle: Option<f32>,
    map: Minimap,
    /// The large map is being dragged with the mouse, from this cursor position.
    drag: Option<(f64, f64)>,
    /// Where the cursor was when the map was last turned by it, while it is.
    turning: Option<(f64, f64)>,
}

/// How much of bringing the glider out is swinging it up from the back.
const GLIDER_UP: f32 = 0.5;
/// Distance between footsteps.
const STRIDE: f32 = m(1.7);
/// How long each frame may spend handing finished terrain to the GPU.
const UPLOAD_MS: f32 = 3.0;
/// Frames left out of the measurements, while the first terrain loads in.
const SETTLE_FRAMES: u64 = 120;
/// Seconds between saves, so a crash loses little.
const AUTOSAVE: f32 = 30.0;

struct App {
    opts: Options,
    run: Option<Running>,
}

impl Running {
    fn new(el: &ActiveEventLoop, opts: &Options) -> Result<Self> {
        let began = Instant::now();
        let store = if opts.frames.is_some() { None } else { Store::find() };
        let saved = store.as_ref().map_or_else(Settings::default, Store::settings);
        let settings = opts.overrides.applied_to(saved);
        let progress = store
            .as_ref()
            .filter(|_| opts.overrides.keeps_position() && !opts.fresh)
            .and_then(|s| s.progress(settings.seed));
        // The pack goes with the world, wherever in it this run begins.
        let mut pack = store.as_ref().filter(|_| opts.overrides.keeps_position()).map_or_else(inventory::Pack::new, |s| inventory::Pack::from_text(&s.pack(settings.seed)));
        // Every wanderer has a lantern, those who set out before there were lanterns too.
        pack.give_kit();
        // As does what has been set down in it.
        let placed = store.as_ref().filter(|_| opts.overrides.keeps_position()).map_or_else(placed::Placed::new, |s| placed::Placed::from_text(&s.placed(settings.seed)));

        let journal = store.as_ref().filter(|_| opts.overrides.keeps_position()).map_or_else(journal::Journal::new, |s| journal::Journal::from_text(&s.journal(settings.seed)));

        // Simulating the continent takes seconds, so it starts now, alongside the window
        // and the GPU, instead of waiting its turn after them.
        log::info!("world seed {0} (run with --seed {0} to come back to it)", settings.seed);
        let world = Arc::new(GameWorld::new(settings.seed));
        // What was picked before is still gone.
        for (x, z) in pack.spots() {
            world.pick(x, z);
        }
        let first_column = opts.over.map(|(x, z, _)| (x, z)).or(progress.map(|p| (p.pos.x.floor() as i32, p.pos.z.floor() as i32)));
        let warming = world.clone();
        std::thread::spawn(move || {
            match first_column {
                Some((x, z)) => drop(warming.column(x, z)),
                None => drop(warming.spawn_point()),
            }
            warming.library();
        });

        let attrs = Window::default_attributes().with_title("Willow Hollow").with_fullscreen(fullscreen(settings.fullscreen));
        // Measurements always use the size asked for, not the player's window.
        let size = if opts.frames.is_some() { Some(opts.size) } else { settings.window };
        let attrs = match size {
            Some((w, h)) => attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h)),
            None => attrs.with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
        };
        let window = Arc::new(el.create_window(attrs)?);
        let (gpu, surface) = Gpu::for_window(window.clone(), settings.vsync)?;
        log::info!("startup: window and GPU after {:.2} s", began.elapsed().as_secs_f32());
        let overlay = Overlay::new(&window, &gpu, surface.config.format);
        let renderer = Renderer::new(
            &gpu,
            surface.config.format,
            surface.config.width,
            surface.config.height,
        );
        log::info!("startup: pipelines after {:.2} s", began.elapsed().as_secs_f32());
        let mut camera = make_camera(opts, &world, progress.as_ref());
        log::info!("startup: world after {:.2} s", began.elapsed().as_secs_f32());
        camera.fov_y = settings.fov.to_radians();
        camera.sensitivity = settings.sensitivity;
        let looking = camera.forward();
        let feet = camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
        let mut player = Player::new(feet);
        if let Some(p) = progress.filter(|p| p.travel == Travel::Sailing) {
            player.board(p.pos, p.heading);
        }
        let flying = opts.fly || progress.is_some_and(|p| p.travel == Travel::Flying);
        let time = opts.time.or(progress.map(|p| p.time)).unwrap_or(10.0);
        let date = opts.date.or(progress.map(|p| p.date)).unwrap_or(season::DEFAULT_DATE);
        // A world saved before it kept its year takes it from the journal, which counted
        // the springs; a new one begins in year 0.
        let year = opts.year.or(progress.and_then(|p| p.year.or(journal.year()))).unwrap_or(0);
        // What has been seen of this world is kept with the rest of its save; a visit to
        // a place given by hand keeps nothing.
        // A camp asked for by hand, to look at or to time: made where the run begins.
        let mut placed = placed;
        let mut voxels = WorldVoxels::new(world.clone());
        let mut rest = None;
        if let Some(state) = opts.shot_camp {
            sample_camp(&mut placed, &mut voxels, camera.pos, camera.yaw, state, opts.camp_ahead);
            if opts.rest
                && !flying
                && let Some((_, fire)) = placed.near(camera.pos, m64(30.0)).find(|(_, t)| t.is_fire())
            {
                player.pos = by_the_fire(&mut voxels, fire, camera.pos);
                rest = Some(camp::Rest::begin(&Environment::at(time, date)));
                log::info!("resting by the fire from {time:.2} h");
            }
        }
        let map_dir = store.as_ref().filter(|_| opts.overrides.keeps_position()).map(|s| s.map_dir(settings.seed));
        let mut map = Minimap::new(&gpu, surface.config.format, world.clone(), map_dir);
        map.corner = settings.minimap;
        Ok(Self {
            map,
            drag: None,
            turning: None,
            window,
            overlay,
            renderer,
            last_yaw: camera.yaw,
            third: false,
            shuttered: false,
            gait: character::Gait::new(camera.yaw),
            camera,
            walking: !flying,
            player,
            voxels,
            shut_in: 0.0,
            far: FarView::new(&gpu, world.clone()),
            impostors: Impostors::new(&gpu, world.clone()),
            streamer: Streamer::new(&gpu, world.clone(), settings.radius),
            env: Environment::at(time, date).in_year(year),
            particles: {
                let mut p = Particles::new();
                // Asked for: ahead of where the camera first looks.
                if opts.murmuration {
                    p.starlings.forced = Some(looking);
                }
                p
            },
            crows: crows::Crows::new(world.clone()),
            leaves: Vec::new(),
            flowers: Vec::new(),
            fallen: Vec::new(),
            nests: (Vec::new(), Vec::new()),
            scan_timer: 0.0,
            scanner: Scanner::new(world.clone()),
            biome: worldgen::biome::Biome::Grassland,
            warmth: season::warmth(worldgen::biome::Biome::Grassland),
            humidity: season::humidity(worldgen::biome::Biome::Grassland),
            aurora_forced: opts.aurora,
            console: commands::Console::default(),
            pace: 1.0,
            before_tp: None,
            mist_floor: None,
            mist_target: 0.0,
            weather: weather::Weather::new(opts.weather),
            cloud_base: weather::CloudBase::default(),
            audio: if opts.audio { Audio::start(settings.volume) } else { None },
            settings,
            saved,
            store,
            save_timer: AUTOSAVE,
            show_settings: false,
            pack,
            pack_view: inventory::View::default(),
            pack_open: false,
            journal,
            journal_view: journal::View::new(world.materials()),
            journal_open: false,
            journal_timer: 0.0,
            within_reach: None,
            reach_timer: 0.0,
            notice: None,
            placed,
            thing_near: None,
            embers: Vec::new(),
            rest,
            lowered: 0.0,
            hand_word: HAND_WORD,
            grown: 0.0,
            grow_timer: 0.0,
            quit: false,
            stride: 0.0,
            steps: 0,
            glider: 0.0,
            last_fall_speed: 0.0,
            was_grounded: false,
            was_swimming: false,
            dry_for: 0.0,
            fleet: Fleet::new(world.clone()),
            props: Vec::new(),
            shades: Vec::new(),
            deploy: false,
            deploy_clear: false,
            world,
            fly: FlyController { speed: m(16.0) },
            keys: HashSet::new(),
            grabbed: false,
            // Measurements must not be disturbed by whatever the mouse is doing.
            want_grab: opts.frames.is_none(),
            raw_motion_seen: false,
            last_cursor: None,
            last_cursor_seen: None,
            last_frame: Instant::now(),
            start: Instant::now(),
            frame: 0,
            frame_ms: 16.0,
            loaded: false,
            cpu: CpuTimer::new(),
            frame_log: opts.frames.map(|n| Vec::with_capacity(n as usize)),
            sub: profile::SubTimer::new(),
            last_dt_ms: 0.0,
            csv: open_csv(),
            sync_frame: (0, 0.0),
            sync_total: (0, 0.0),
            drift: opts.drift,
            circle: opts.circle,
            gpu,
            surface,
        })
    }

    /// Captures or releases the mouse. Capturing can fail (no focus yet); the caller
    /// keeps `want_grab` set and tries again later.
    fn set_grab(&mut self, grab: bool) {
        if grab {
            if !self.window.has_focus() {
                return;
            }
            let ok = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            if !ok {
                return;
            }
            self.last_cursor = None;
            self.centre_cursor();
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.keys.clear();
        }
        self.window.set_cursor_visible(!grab);
        self.grabbed = grab;
    }

    /// Opens or closes the settings menu, which needs the mouse.
    fn set_menu(&mut self, open: bool) {
        self.show_settings = open;
        self.want_grab = !open;
        self.set_grab(!open);
        if !open {
            self.save_timer = 0.0;
        }
    }

    /// Opens or closes the pack, which needs the mouse.
    fn set_pack(&mut self, open: bool) {
        self.pack_open = open;
        self.want_grab = !open;
        self.set_grab(!open);
    }

    /// Opens or closes the journal, which needs the mouse.
    fn set_journal(&mut self, open: bool) {
        self.journal_open = open;
        self.want_grab = !open;
        self.set_grab(!open);
    }

    /// Opens or closes the console. While it is open the keyboard is its own: letting go
    /// of the mouse lets go of every key held, so nothing typed moves the player.
    fn set_console(&mut self, open: bool) {
        self.console.set_open(open);
        self.want_grab = !open;
        self.set_grab(!open);
    }

    /// Runs a console command, as if typed, logging what it said.
    fn command(&mut self, line: &str) -> commands::Reply {
        let mut console = std::mem::take(&mut self.console);
        let reply = console.run(line, self);
        self.console = console;
        Self::log_reply(line, &reply);
        reply
    }

    /// Runs what is typed in the console (Enter).
    fn submit_command(&mut self) {
        let mut console = std::mem::take(&mut self.console);
        let reply = console.submit(self);
        self.console = console;
        if let Some(reply) = reply {
            Self::log_reply("console", &reply);
        }
    }

    fn log_reply(line: &str, reply: &commands::Reply) {
        match reply {
            Ok(said) => log::info!("{line}: {}", said.replace('\n', "; ")),
            Err(wrong) => log::warn!("{line}: {wrong}"),
        }
    }

    /// Puts the eye at `eye`, on foot or flying as the player is: out of any boat and
    /// up from any rest. Remembers where it was, for `back`.
    fn go_to(&mut self, eye: DVec3) {
        let was = if self.walking { self.player.eye() } else { self.camera.pos };
        if self.walking && self.player.mode == Mode::Boat {
            let (pos, heading) = self.player.leave_boat(&mut self.voxels);
            self.fleet.leave(pos, heading);
        }
        self.wake();
        if self.walking {
            self.player = Player::new(eye - DVec3::new(0.0, player::EYE_HEIGHT, 0.0));
        }
        self.camera.pos = eye;
        self.before_tp = Some(was);
        // What was within reach there is not here.
        self.reach_timer = 0.0;
        self.scan_timer = 0.0;
    }

    /// Writes up the journal: the time gone by (`days`, of the game's), the way gone,
    /// and, every second or so, the named places come to.
    fn keep_journal(&mut self, dt: f32, days: f32) {
        self.journal.tick(days, self.env.date, self.env.year, self.env.time, self.weather.now.snow > 0.3);
        let way = match (self.walking, self.player.mode, self.player.swimming) {
            (false, ..) => None,
            (true, Mode::Boat, _) => Some(journal::Way::Sail),
            (true, Mode::Glide, _) => Some(journal::Way::Glide),
            (true, Mode::Walk, true) => Some(journal::Way::Swim),
            (true, Mode::Walk, false) => Some(journal::Way::Foot),
        };
        self.journal.moved(self.player.pos, way, self.player.on_ground);
        self.journal_timer -= dt;
        if self.journal_timer <= 0.0 && self.walking {
            self.journal_timer = 1.0;
            if let Some((c, at)) = journal::continent_at(&self.world, self.player.pos, false) {
                self.journal.survey(&c, at, self.player.mode != Mode::Glide);
            }
        }
    }

    /// Where the hands are: a little ahead of the feet, the way the player looks.
    fn hands(&self) -> DVec3 {
        let ahead = self.camera.forward();
        let ahead = DVec3::new(ahead.x as f64, 0.0, ahead.z as f64).normalize_or_zero();
        self.player.pos + ahead * m64(0.6)
    }

    /// What grows within arm's reach ahead of the player that can be gathered and has
    /// not been already: the nearest, with the column it stands in.
    fn find_within_reach(&mut self) -> Option<((i32, i32), inventory::Item)> {
        if !self.walking || self.player.mode != Mode::Walk {
            return None;
        }
        let centre = self.hands();
        let reach = m64(1.2);
        let (cx, cz, feet) = (centre.x.floor() as i32, centre.z.floor() as i32, self.player.pos.y.floor() as i32);
        let span = reach.ceil() as i32;
        let mats = self.world.materials();
        let mut best: Option<(f64, (i32, i32), inventory::Item)> = None;
        // What the loaded ground really has growing on it here: the world's word for
        // where the flowers stand leaves out a boulder sat on one, or a stone step.
        let growing: Vec<(glam::IVec3, voxel::Voxel)> = self.streamer.growing_in(cx - span, cz - span, 2 * span + 1).collect();
        // The flowers and mushrooms, which stand on the ground; those on ground far above
        // or below the feet are out of reach. Only one that is out today can be picked:
        // a bud or a seed head is left where it is.
        for ((x, z), flower) in self.world.flowers_in(cx - span, cz - span, 2 * span + 1) {
            let d = DVec3::new(x as f64 + 0.5 - centre.x, 0.0, z as f64 + 0.5 - centre.z).length();
            if d > reach || self.pack.gathered(x, z) || best.as_ref().is_some_and(|b| b.0 <= d) || !flower.blooming_at(self.env.date, x, z) {
                continue;
            }
            if !growing.iter().any(|(p, v)| p.x == x && p.z == z && *v == flower.petals()) {
                continue;
            }
            let ground = (feet - 4..=feet + 3).rev().find(|&y| mats.get(self.voxels.voxel(x, y, z)).kind == voxel::Kind::Solid);
            if ground.is_some_and(|y| (y - feet).abs() <= 3) {
                best = Some((d, (x, z), inventory::of_flower(flower)));
            }
        }
        // What the bushes, the ivy and the reeds bear, from a step below the feet to as
        // high as a hand goes.
        for &(p, v) in &growing {
            let d = DVec3::new(p.x as f64 + 0.5 - centre.x, 0.0, p.z as f64 + 0.5 - centre.z).length();
            if d > reach || !(feet - 3..=feet + 5).contains(&p.y) || self.pack.gathered(p.x, p.z) || best.as_ref().is_some_and(|b| b.0 <= d) {
                continue;
            }
            if let Some(item) = inventory::of_voxel(v, mats) {
                best = Some((d, (p.x, p.z), item));
            }
        }
        // And the sticks lying on the ground, until they are picked up.
        for stick in &self.fallen {
            let (x, z) = stick.spot();
            let d = DVec3::new(stick.pos.x - centre.x, 0.0, stick.pos.z - centre.z).length();
            if d > reach || (stick.pos.y - feet as f64).abs() > 3.0 || self.pack.gathered(x, z) || best.as_ref().is_some_and(|b| b.0 <= d) {
                continue;
            }
            best = Some((d, (x, z), inventory::sticks()));
        }
        best.map(|b| (b.1, b.2))
    }

    /// Gathers what is within reach into the pack.
    fn gather(&mut self) {
        let Some(((x, z), item)) = self.find_within_reach() else { return };
        let word = if self.pack.add(&item) {
            // It is gone from where it grew, until it grows back.
            if let Some((ox, oz)) = self.pack.mark(x, z) {
                self.world.regrow(ox, oz);
                self.streamer.replant(ox, oz);
            }
            if item.id == "sticks" {
                // All that lay there, which is not part of the world: nothing to build again.
                let more = self.fallen.iter().find(|s| s.spot() == (x, z)).map_or(0, |s| s.count.saturating_sub(1));
                self.pack.add_many(&item, more);
            } else {
                self.world.pick(x, z);
                self.streamer.replant(x, z);
            }
            // The first of its kind goes in the journal.
            let new = if self.journal.found(&item) { "   \u{b7}   written in the journal" } else { "" };
            format!("{}  \u{d7} {}{new}", item.name, self.pack.count(&item.id))
        } else {
            "Your pack is full".to_string()
        };
        self.say(word);
        self.within_reach = None;
        self.reach_timer = 0.0;
    }

    /// Shows a word or two for a moment.
    fn say(&mut self, word: impl Into<String>) {
        self.notice = Some((word.into(), 2.5));
    }

    /// Where the player is looking on the ground, within reach, to set a thing down
    /// on: the first ground the eye's line meets, or failing that the ground a step ahead.
    fn spot_ahead(&mut self) -> Option<DVec3> {
        use player::Voxels;
        let (eye, dir) = (self.player.eye(), self.camera.forward().as_dvec3());
        let mut d = m64(0.5);
        while d < PLACE_REACH {
            let p = eye + dir * d;
            let at = p.floor().as_ivec3();
            match self.voxels.kind(at.x, at.y, at.z) {
                voxel::Kind::Solid => return ground_near(&mut self.voxels, p.x, at.y as f64 + 1.0, p.z).map(|y| DVec3::new(p.x, y, p.z)),
                voxel::Kind::Water => return None,
                _ => d += 0.2,
            }
        }
        let p = self.player.pos + DVec3::new(dir.x, 0.0, dir.z).normalize_or_zero() * m64(1.5);
        ground_near(&mut self.voxels, p.x, self.player.pos.y + 1.0, p.z).map(|y| DVec3::new(p.x, y, p.z))
    }

    /// Uses what is held: lays a fire of sticks, or sets the thing down, where the
    /// player is looking.
    fn use_held(&mut self) {
        if !self.walking || self.player.mode != Mode::Walk || self.rest.is_some() {
            return;
        }
        let Some(stack) = self.pack.in_hand().cloned() else { return };
        // The lantern is not set down: it is carried, lit or shuttered.
        if stack.item.id == "lantern" {
            self.shuttered = !self.shuttered;
            self.say(if self.shuttered { "The lantern is shuttered" } else { "The lantern is lit" });
            return;
        }
        let Some(at) = self.spot_ahead() else {
            self.say("There is nowhere here to put it");
            return;
        };
        let yaw = self.camera.yaw;
        match stack.item.id.as_str() {
            "sticks" => {
                if !self.pack.spend("sticks", inventory::FIRE_STICKS) {
                    self.say(format!("A fire wants {} sticks", inventory::FIRE_STICKS));
                    return;
                }
                self.placed.put(placed::Thing::new(inventory::campfire(), 1, at, yaw));
                self.say("A fire is laid");
            }
            id => {
                self.pack.spend(id, 1);
                self.placed.put(placed::Thing::new(stack.item, 1, at, yaw + std::f32::consts::FRAC_PI_2));
            }
        }
        self.thing_near = None;
        self.reach_timer = 0.0;
    }

    /// Sets down by the player's feet what was let go of from the pack's window.
    fn set_down(&mut self, stack: inventory::Stack) {
        let hands = self.hands();
        // Each lands a little apart from the last.
        let n = self.placed.near(hands, m64(1.0)).count() as f64;
        let p = hands + DVec3::new((n * 2.4).cos(), 0.0, (n * 2.4).sin()) * m64(0.12 * n.min(4.0));
        let y = ground_near(&mut self.voxels, p.x, self.player.pos.y + 1.0, p.z).unwrap_or(self.player.pos.y);
        self.placed.put(placed::Thing::new(stack.item, stack.count, DVec3::new(p.x, y, p.z), self.camera.yaw + std::f32::consts::FRAC_PI_2));
    }

    /// Takes up the thing set down that is within reach, back into the pack. A fire
    /// burning is put out instead, and one that is not is taken apart.
    fn take_up(&mut self) {
        if !self.walking || self.player.mode != Mode::Walk || self.rest.is_some() {
            return;
        }
        let Some(at) = self.placed.within_reach(self.hands()) else { return };
        let Some(thing) = self.placed.get(at).cloned() else { return };
        if thing.burns() {
            // Smothered and stamped out, it smoulders for a while.
            self.placed.set_state(at, placed::OUT);
            self.say("The fire is put out");
        } else if thing.is_fire() {
            // It comes apart into what of its sticks is not burnt through, and its stones
            // are scattered.
            let back = thing.salvage();
            let left = self.pack.add_many(&inventory::sticks(), back);
            if left > 0 {
                self.pack.spend("sticks", back - left);
                self.say("Your pack is full");
                return;
            }
            self.placed.take(at);
            self.map.camps = self.placed.camps();
            self.say(if back > 0 { format!("Sticks  \u{d7} {}", self.pack.count("sticks")) } else { "The fire is cleared away".into() });
        } else {
            let left = self.pack.add_many(&thing.item, thing.count);
            if left == thing.count {
                self.say("Your pack is full");
                return;
            }
            if left == 0 {
                self.placed.take(at);
            } else {
                self.placed.set_count(at, left);
            }
            self.say(format!("{}  \u{d7} {}", thing.item.name, self.pack.count(&thing.item.id)));
        }
        self.thing_near = None;
        self.reach_timer = 0.0;
    }

    /// The fire of a camp that was here already, if the player is by one.
    fn ember_near(&self) -> Option<DVec3> {
        let hands = self.hands();
        self.embers.iter().copied().find(|e| (e.x - hands.x).hypot(e.z - hands.z) <= m64(2.4) && (e.y - hands.y).abs() < m64(2.0))
    }

    /// What E would do with the thing set down within reach, or with the fire of a
    /// camp that was here already, in words; and what a left click would.
    fn camp_prompt(&self) -> Option<String> {
        if let Some(rest) = &self.rest {
            return Some(format!("Resting until {}   \u{b7}   any step to get up", rest.until.word()));
        }
        let until = camp::Until::from(&self.env).word();
        let Some(thing) = self.thing_near.and_then(|at| self.placed.get(at)) else {
            return self.ember_near().map(|_| format!("E to rest by the fire until {until}"));
        };
        Some(if thing.burns() {
            format!("E to rest by the fire until {until}   \u{b7}   left click to put it out")
        } else if thing.is_fire() && thing.state == placed::LAID {
            "E to light the fire   \u{b7}   left click to take the sticks back".into()
        } else if thing.is_fire() && thing.salvage() > 0 {
            "E to light the fire again   \u{b7}   left click to take it apart for its sticks".into()
        } else if thing.is_fire() {
            "E to light the fire again   \u{b7}   left click to clear it away".into()
        } else if thing.count > 1 {
            format!("Left click to pick up  {}  \u{d7} {}", thing.item.name, thing.count)
        } else {
            format!("Left click to pick up  {}", thing.item.name)
        })
    }

    /// Lights the fire within reach, or settles down to rest by the one burning there.
    /// Whether there was anything of the kind to do.
    fn make_camp(&mut self) -> bool {
        let thing = self.thing_near.and_then(|at| Some((at, self.placed.get(at)?.clone())));
        match thing {
            Some((at, thing)) if thing.is_fire() && !thing.burns() => {
                self.placed.set_state(at, placed::BURNING);
                self.map.camps = self.placed.camps();
                self.say("The fire catches");
                true
            }
            Some((_, thing)) if thing.is_fire() => {
                self.player.vel = glam::Vec3::ZERO;
                self.rest = Some(camp::Rest::begin(&self.env));
                true
            }
            Some(_) => false,
            None if self.ember_near().is_some() => {
                self.player.vel = glam::Vec3::ZERO;
                self.rest = Some(camp::Rest::begin(&self.env));
                true
            }
            None => false,
        }
    }

    /// Gets up from a rest.
    fn wake(&mut self) {
        self.rest = None;
    }

    /// Holds slot `k` of the hotbar, or the one `by` along from the one held, and says
    /// what is now in the hand.
    fn hold(&mut self, k: Option<usize>, by: i32) {
        let changed = match k {
            Some(k) => self.pack.hold(k),
            None => self.pack.cycle(by),
        };
        if changed {
            self.hand_word = HAND_WORD;
        }
    }

    /// Opens or closes the large map, which needs the mouse to move it.
    fn set_map(&mut self, open: bool) {
        self.map.set_open(open);
        self.drag = None;
        self.turning = None;
        // Where the cursor was before it was let go of is not where it is now.
        self.last_cursor_seen = None;
        self.want_grab = !open;
        self.set_grab(!open);
    }

    /// Puts into effect whatever changed in the settings since `was`.
    fn apply_settings(&mut self, was: Settings) {
        let now = self.settings;
        if now.radius != was.radius {
            self.streamer.set_radius(now.radius);
        }
        self.camera.fov_y = now.fov.to_radians();
        self.camera.sensitivity = now.sensitivity;
        if now.volume != was.volume
            && let Some(audio) = &self.audio
        {
            audio.set_volume(now.volume);
        }
        if now.vsync != was.vsync {
            self.surface.set_vsync(&self.gpu, now.vsync);
        }
        if now.fullscreen != was.fullscreen {
            self.window.set_fullscreen(fullscreen(now.fullscreen));
        }
        if now.window != was.window
            && let Some((w, h)) = now.window
        {
            let _ = self.window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
        }
    }

    fn progress(&self) -> Progress {
        let flying = !self.walking;
        Progress {
            pos: if flying { self.camera.pos } else { self.player.pos },
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            time: self.env.time,
            date: self.env.date,
            year: Some(self.env.year),
            travel: match (flying, self.player.mode) {
                (true, _) => Travel::Flying,
                (false, Mode::Boat) => Travel::Sailing,
                (false, _) => Travel::Walking,
            },
            heading: self.player.heading,
        }
    }

    /// Writes the settings, if changed, and where the player is.
    fn persist(&mut self, overrides: &Overrides) {
        let Some(store) = &self.store else { return };
        let keep = overrides.worth_keeping(self.settings, self.saved);
        if keep != self.saved && store.write_settings(keep) {
            self.saved = keep;
        }
        if overrides.keeps_position() {
            store.write_progress(self.settings.seed, self.progress());
            if self.pack.unsaved && store.write_pack(self.settings.seed, &self.pack.to_text()) {
                self.pack.unsaved = false;
            }
            if self.journal.unsaved && store.write_journal(self.settings.seed, &self.journal.to_text()) {
                self.journal.unsaved = false;
            }
            if self.placed.unsaved && store.write_placed(self.settings.seed, &self.placed.to_text()) {
                self.placed.unsaved = false;
            }
        }
        self.map.save(false);
    }

    fn centre_cursor(&mut self) {
        let size = self.window.inner_size();
        let centre = winit::dpi::PhysicalPosition::new(size.width as f64 / 2.0, size.height as f64 / 2.0);
        if self.window.set_cursor_position(centre).is_ok() {
            self.last_cursor = Some((centre.x, centre.y));
        }
    }

    /// Mouse look from cursor positions, used only if raw motion events never arrive.
    fn cursor_moved(&mut self, x: f64, y: f64) {
        if !self.grabbed || self.raw_motion_seen {
            return;
        }
        if let Some((lx, ly)) = self.last_cursor {
            let (dx, dy) = (x - lx, y - ly);
            if dx != 0.0 || dy != 0.0 {
                self.camera.add_look(dx as f32, dy as f32);
            }
        }
        self.last_cursor = Some((x, y));
        // Keep the cursor away from the window edges.
        let size = self.window.inner_size();
        let (cx, cy) = (size.width as f64 / 2.0, size.height as f64 / 2.0);
        if (x - cx).abs() > cx * 0.5 || (y - cy).abs() > cy * 0.5 {
            self.centre_cursor();
        }
    }

    fn toggle_flying(&mut self) {
        self.walking = !self.walking;
        if self.walking {
            // Drop in wherever the camera is.
            let feet = self.camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
            self.player = Player::new(feet);
        }
    }

    /// Boards the nearest boat, or goes ashore from the one being sailed.
    fn interact(&mut self) {
        if !self.walking {
            return;
        }
        if self.player.mode == Mode::Boat {
            let (pos, heading) = self.player.leave_boat(&mut self.voxels);
            self.fleet.leave(pos, heading);
            self.camera.pos = self.player.eye();
        } else if let Some(i) = self.fleet.nearest(self.player.pos, BOARD_REACH) {
            let boat = self.fleet.take(i);
            self.player.board(boat.pos, boat.heading);
        } else if !self.make_camp() {
            self.gather();
        }
    }

    /// Counts the steps taken, one each stride, playing a footstep for each and a
    /// heavier one on landing. The figure's legs keep time by the same count.
    fn update_footsteps(&mut self, dt: f32) {
        let on_foot = self.walking && self.player.mode == Mode::Walk && !self.player.swimming;
        let grounded = on_foot && self.player.on_ground;
        if grounded {
            let landed = !self.was_grounded && self.last_fall_speed < -m(6.0);
            if landed {
                self.stride = 0.0;
            }
            self.stride += glam::Vec2::new(self.player.vel.x, self.player.vel.z).length() * dt;
            let stepped = self.stride >= STRIDE;
            if stepped {
                self.stride -= STRIDE;
                self.steps = self.steps.wrapping_add(1);
            }
            if let Some(audio) = &self.audio
                && (landed || stepped)
            {
                let (x, y, z) = (self.player.pos.x.floor() as i32, self.player.pos.y.floor() as i32, self.player.pos.z.floor() as i32);
                let feet_in_water = self.world.materials().get(self.voxels.voxel(x, y, z)).kind == voxel::Kind::Water;
                let ground = if feet_in_water { Ground::Shallows } else { audio::ground_of(self.voxels.voxel(x, y - 1, z)) };
                if landed {
                    audio.step(ground, (-self.last_fall_speed / m(16.0)).min(1.0));
                }
                if stepped {
                    audio.step(ground, 0.7);
                }
            }
        }
        self.was_grounded = grounded;
        self.last_fall_speed = self.player.vel.y;
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(&self.gpu, width, height);
        self.renderer.resize(&self.gpu, width, height);
    }

    fn redraw(&mut self) {
        if self.gpu.has_failed() {
            // There is nothing left to draw with. Saving still works, so leave cleanly.
            log::error!("the graphics device failed, closing the game");
            self.quit = true;
            return;
        }
        if self.want_grab && !self.grabbed {
            self.set_grab(true);
        }
        let now = Instant::now();
        let raw_dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_ms += (raw_dt * 1000.0 - self.frame_ms) * 0.05;
        let dt = raw_dt.min(0.1);
        if let Some(log) = &mut self.frame_log
            && self.frame > SETTLE_FRAMES
        {
            log.push(raw_dt * 1000.0);
        }
        if self.frame == SETTLE_FRAMES {
            self.cpu.reset_worst();
        }
        self.cpu.start();
        self.sub.start();
        self.last_dt_ms = raw_dt * 1000.0;
        if let Some(speed) = self.drift {
            if let Some(radius) = self.circle {
                self.camera.yaw = (self.camera.yaw + speed / radius * dt).rem_euclid(std::f32::consts::TAU);
            }
            let ahead = self.camera.forward() * glam::Vec3::new(1.0, 0.0, 1.0);
            self.camera.pos += (ahead.normalize_or_zero() * m(speed) * dt).as_dvec3();
        }

        if self.walking {
            let fwd = self.camera.forward();
            let fwd = glam::Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
            let right = self.camera.right();
            let axis = |pos: KeyCode, neg: KeyCode| {
                self.keys.contains(&pos) as i32 as f32 - self.keys.contains(&neg) as i32 as f32
            };
            let pi = std::f32::consts::PI;
            let turned = (self.camera.yaw - self.last_yaw + pi).rem_euclid(pi * 2.0) - pi;
            self.last_yaw = self.camera.yaw;
            // Resting, the player stays where they lie.
            let up = if self.rest.is_some() { 0.0 } else { 1.0 };
            let controls = Controls {
                wish: (fwd * axis(KeyCode::KeyW, KeyCode::KeyS) + right * axis(KeyCode::KeyD, KeyCode::KeyA)) * up,
                jump: self.keys.contains(&KeyCode::Space) && self.rest.is_none(),
                sprint: self.keys.contains(&KeyCode::ShiftLeft),
                deploy: std::mem::take(&mut self.deploy),
                deploy_clear: std::mem::take(&mut self.deploy_clear),
                look: self.camera.forward(),
                turn: if dt > 0.0 { turned / dt } else { 0.0 },
            };
            self.player.pace = self.pace;
            self.player.update(&mut self.voxels, controls, dt);
            self.camera.pos = self.player.eye();
            // Lying down to rest, and getting up again, the eye sinks and rises.
            let low = self.rest.as_ref().map_or(0.0, camp::Rest::lowered);
            self.lowered += (low - self.lowered) * (1.0 - (-3.0 * dt).exp());
            self.camera.pos.y -= m64(self.lowered as f64);
            // The figure: its stride from how fast it goes over the ground, and facing the
            // way it goes, or the way its glider or boat does.
            let over_ground = glam::Vec2::new(self.player.vel.x, self.player.vel.z);
            let speed = over_ground.length() / worldgen::scale::VOXELS_PER_METRE;
            let heading = match self.player.mode {
                Mode::Glide => Some(self.camera.yaw),
                Mode::Boat => Some(self.player.heading),
                Mode::Walk if speed > 0.5 => Some(over_ground.y.atan2(over_ground.x)),
                Mode::Walk if !self.third => Some(self.camera.yaw),
                Mode::Walk => None,
            };
            let doing = character::Doing::of(self.player.mode, self.player.on_ground, self.player.swimming, speed);
            // Two steps to a turn of the legs, so only whether the count is odd matters.
            let stride = (self.steps % 2) as f32 + self.stride / STRIDE;
            // Taking the glider from the back, or putting it away.
            let gliding = self.player.mode == Mode::Glide;
            let stow = ((gliding || self.glider > 0.0) && self.glider <= GLIDER_UP).then_some(self.glider / GLIDER_UP);
            let rise = self.player.vel.y / worldgen::scale::VOXELS_PER_METRE;
            self.gait.update(doing, stride, heading, gliding, stow, rise, dt);
            if self.third {
                // Behind and looking past the figure, as far back as the ground and walls allow.
                use player::Voxels;
                let back = -self.camera.forward().as_dvec3();
                let eye = self.camera.pos + DVec3::new(0.0, m64(0.25), 0.0);
                let (reach, step) = (m64(3.6), 0.25);
                let mut d = 0.0;
                while d < reach {
                    let ahead = (eye + back * (d + 0.75)).floor().as_ivec3();
                    if self.voxels.kind(ahead.x, ahead.y, ahead.z) == voxel::Kind::Solid {
                        break;
                    }
                    d += step;
                }
                self.camera.pos = eye + back * d;
            }
            self.camera.roll = self.player.bank;
        } else {
            self.camera.roll = 0.0;
            self.fly.update(&mut self.camera, &self.keys, dt);
        }
        self.fleet.update(self.player.pos, dt);
        // What can be gathered here is looked for a few times a second, not every frame.
        self.reach_timer -= dt;
        if self.reach_timer <= 0.0 {
            self.reach_timer = 0.2;
            self.within_reach = self.find_within_reach();
            let on_foot = self.walking && self.player.mode == Mode::Walk;
            self.thing_near = if on_foot { self.placed.within_reach(self.hands()) } else { None };
            self.embers = embers_near(&self.streamer, &mut self.voxels, self.player.pos);
        }
        self.hand_word = (self.hand_word - dt).max(0.0);
        if let Some((_, left)) = &mut self.notice {
            *left -= dt;
            if *left <= 0.0 {
                self.notice = None;
            }
        }
        self.cpu.lap(0);
        self.sub.lap(0);
        // Stamped on the GPU on either side, so that the copies of what these upload, and
        // of an arena that grows, show apart from the frame's passes.
        if let Some(t) = &self.renderer.timer {
            t.mark_alone(&self.gpu, profile::UPLOAD_BEGIN);
        }
        self.streamer.update(&self.gpu, self.camera.pos, UPLOAD_MS);
        self.sub.lap(1);
        self.far.update(&self.gpu, self.camera.pos);
        self.sub.lap(2);
        self.impostors.update(&self.gpu, self.camera.pos);
        if let Some(t) = &self.renderer.timer {
            t.mark_alone(&self.gpu, profile::UPLOAD_END);
        }
        self.sub.lap(3);
        self.cpu.lap(1);

        // The day turns.
        let mut days = if self.settings.day_minutes > 0.0 { dt / (self.settings.day_minutes * 60.0) } else { 0.0 };
        // And faster for someone resting, until the dawn or the dark they wait for.
        if let Some(rest) = &mut self.rest {
            let on_foot = self.walking && self.player.mode == Mode::Walk;
            match rest.hours(&self.env, dt).filter(|_| on_foot) {
                Some(hours) => days += hours / 24.0,
                None => {
                    let word = if rest.until == camp::Until::Dawn { "Morning" } else { "Night has come" };
                    log::info!("the rest ended at {:.2} h, on frame {}", self.env.time, self.frame);
                    self.wake();
                    if on_foot {
                        self.say(word);
                    }
                }
            }
        }
        if days > 0.0 {
            let (date, year) = season::advance_year(self.env.date, self.env.year, days, self.settings.year_days);
            self.env = Environment::at(self.env.time + days * 24.0, date).in_year(year);
        }
        self.keep_journal(dt, days);
        // What was gathered a year or two ago has grown back: looked over now and then.
        self.grown += (days / self.settings.year_days.max(1.0)) as f64;
        self.grow_timer -= dt;
        if self.grow_timer <= 0.0 {
            self.grow_timer = 1.0;
            let years = std::mem::take(&mut self.grown);
            for (x, z) in self.pack.pass(years) {
                self.world.regrow(x, z);
                self.streamer.replant(x, z);
            }
            // The fires burn down in their time; where one has burned is a camp.
            self.placed.pass(years, (FIRE_BURNS / self.settings.year_days.max(1.0)) as f64);
            self.map.camps = self.placed.camps();
        }
        self.save_timer -= dt;
        // On white ground what falls is snow.
        let white = self.weather.ground(self.env.date, self.warmth).depth(self.camera.pos.y as f32 / worldgen::scale::VOXELS_PER_METRE);
        // (And in the cold biomes all winter, whatever lies.)
        let cold = self.warmth < 0.0 && season::snow_cover(self.env.date) > 0.5;
        self.weather.snowy = if cold { 1.0 } else { ((white - 0.1) / 0.1).clamp(0.0, 1.0) };
        self.weather.update(dt, days, self.env.date, self.settings.year_days, self.warmth, self.settings.seed);
        self.warmth += (season::warmth(self.biome) - self.warmth) * (1.0 - (-0.4 * dt).exp());
        // The air changes more slowly than the ground: haze does not end at a biome's edge.
        self.humidity += (season::humidity(self.biome) - self.humidity) * (1.0 - (-0.08 * dt).exp());
        if let Some(floor) = &mut self.mist_floor {
            *floor += (self.mist_target - *floor) * (1.0 - (-0.25 * dt).exp());
        }

        self.sub.lap(4);
        // Ambient life. Every so often, look around for leaves and flowers to spawn from.
        self.scan_timer -= dt;
        if self.scan_timer <= 0.0 {
            self.scan_timer = 2.0;
            self.scanner.request(self.camera.pos, self.frame as u32, self.env.date);
        }
        if let Some(found) = self.scanner.poll() {
            let low;
            (self.biome, self.leaves, self.flowers, low, self.fallen, self.nests) = found;
            self.mist_floor.get_or_insert(low);
            self.mist_target = low;
        }
        self.sub.lap(5);
        let entered_water = (self.player.swimming && !self.was_swimming && self.walking && self.dry_for > 0.75)
            .then(|| self.player.pos + DVec3::new(0.0, m64(0.8), 0.0));
        self.was_swimming = self.player.swimming;
        self.dry_for = if self.player.swimming { 0.0 } else { self.dry_for + dt };
        let cascades: Vec<_> = self.streamer.cascades_near(self.camera.pos, m(120.0)).copied().collect();
        let springs: Vec<_> = self.streamer.springs_near(self.camera.pos, m(40.0)).copied().collect();
        let surf: Vec<_> = self.streamer.surf_near(self.camera.pos, m(60.0)).copied().collect();
        // Every fire burning near: the player's own, and the embers of older camps.
        let burning: Vec<DVec3> = self.placed.near(self.camera.pos, m64(90.0)).filter(|(_, t)| t.burns()).map(|(_, t)| t.heart()).collect();
        let mut fires = self.placed.smoking(self.camera.pos, m64(particles::SMOKE_SIGHT), (SMOULDERS / self.settings.year_days.max(1.0)) as f64);
        fires.extend(self.embers.iter().map(|&e| (e, 1.0)));
        let surroundings = Surroundings {
            camera: self.camera.pos,
            biome: self.biome,
            sun_elevation: self.env.sun_elevation,
            cascades: &cascades,
            springs: &springs,
            surf: &surf,
            leaves: &self.leaves,
            flowers: &self.flowers,
            fires: &fires,
            player_entered_water: entered_water,
            leaf_fall: season::leaf_fall(self.env.date),
            date: self.env.date,
            rain: self.weather.now.rain,
            snow: self.weather.now.snow,
            storm: self.weather.now.storm,
            time: self.last_frame.duration_since(self.start).as_secs_f64(),
            land: &|x, z| particles::land_under(&self.world, x, z),
            nests: &self.nests.0,
            forage: &self.nests.1,
            hour: self.env.time,
        };
        // Leaves and rain can wait a frame for ground that is not generated yet.
        self.sub.lap(6);
        self.voxels.trim(self.camera.pos);
        self.particles.update(dt, &surroundings, &mut self.voxels.lazily());
        self.sub.lap(7);
        let around = crows::Around {
            camera: self.camera.pos,
            sun_elevation: self.env.sun_elevation,
            hour: self.env.time,
            date: self.env.date,
            rain: self.weather.now.rain,
            land: &|x, z| particles::land_under(&self.world, x, z),
        };
        self.crows.update(dt, &around);
        self.sub.lap(20);
        self.update_footsteps(dt);
        self.sub.lap(8);

        let clock = self.last_frame.duration_since(self.start).as_secs_f32();
        self.props.clear();
        self.shades.clear();
        for boat in self.fleet.boats() {
            vehicles::draw_boat(boat.pos, boat.heading, clock, self.camera.pos, &mut self.props);
        }
        self.placed.draw(self.camera.pos, clock, &mut self.props);
        let mut held_light = None;
        fallen::draw(&self.fallen, &self.pack, self.camera.pos, &mut self.props);
        self.crows.draw(self.camera.pos, &mut self.props);
        if self.walking {
            match self.player.mode {
                Mode::Boat => vehicles::draw_boat(self.player.pos, self.player.heading, clock, self.camera.pos, &mut self.props),
                Mode::Glide | Mode::Walk => {}
            }
            // The glider swings open as it is taken to the air and folds away on landing.
            let flying = self.player.mode == Mode::Glide;
            self.glider = (self.glider + if flying { dt / 0.7 } else { -dt / 0.8 }).clamp(0.0, 1.0);
            if self.glider > GLIDER_UP {
                let open = (self.glider - GLIDER_UP) / (1.0 - GLIDER_UP);
                let rise = self.player.vel.y / worldgen::scale::VOXELS_PER_METRE;
                let flight = vehicles::Flight::of(self.player.bank, rise, clock);
                vehicles::draw_glider(self.player.pos, self.camera.yaw, open, flight, self.camera.pos, &mut self.props);
            }
            {
                // Sitting, the hips are where the thwart is.
                // On foot they rise and fall over steps as smoothly as the eye does.
                let feet = match self.player.mode {
                    Mode::Boat => self.player.pos - DVec3::new(0.0, m64(0.43), 0.0),
                    _ => self.player.eye() - DVec3::new(0.0, player::EYE_HEIGHT, 0.0),
                };
                // Seen from behind, or through its own eyes only by its shadow.
                let into = if self.third { &mut self.props } else { &mut self.shades };
                // The glider is a rod on the back until the wing itself is out.
                let furled = (self.glider <= GLIDER_UP).then_some(self.glider / GLIDER_UP);
                character::draw(&character::WANDERER, feet, self.gait.facing, &self.gait.pose(), furled, self.camera.pos, into);
                // A lantern in hand lights the way, unless it is shuttered; not with both
                // hands on the glider's bar.
                if self.pack.in_hand().is_some_and(|s| s.item.id == "lantern") && self.player.mode != Mode::Glide {
                    let lit = if self.shuttered { 0.0 } else { candle(clock) };
                    let flame = character::draw_lantern(feet, self.gait.facing, &self.gait.pose(), lit, self.camera.pos, into);
                    held_light = (lit > 0.0).then_some((flame, lit));
                }
            }
        }

        self.sub.lap(9);
        self.map.corner = self.settings.minimap;
        self.map.hour = self.env.time;
        let here = if self.walking { self.player.eye() } else { self.camera.pos };
        self.map.update(&self.gpu, here, self.camera.forward(), self.walking, dt);
        self.sub.lap(10);

        self.cpu.lap(2);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let mut lamps: Vec<(DVec3, f32)> = self.streamer.lamps_near(self.camera.pos, m(24.0)).map(|p| (p.as_dvec3() + DVec3::splat(0.5), 1.0)).collect();
        lamps.extend(burning.iter().map(|&heart| firelight(heart, clock)));
        lamps.extend(held_light);
        let mut lit = self.env.clone();
        self.weather.apply(&mut lit);
        // The northern lights, on the nights they are out where the camera is.
        let lights = match self.aurora_forced {
            Some(out) => out as u8 as f32,
            None => aurora::cold(self.warmth, aurora::climate_at(&self.world, self.camera.pos.x, self.camera.pos.z)) * aurora::activity(self.env.date, self.settings.year_days, self.settings.seed),
        };
        aurora::light(&mut lit, lights);
        lit.cloud_base = self.cloud_base.update(&self.world, self.camera.pos, dt);
        lit.air(self.humidity, self.weather.wet * (1.0 - (self.weather.now.rain + self.weather.now.snow).min(1.0)));
        // The eye takes a couple of seconds to open up in the dark, and less to close
        // again in the light.
        let shut_in = player::enclosure(self.camera.pos, &mut self.voxels.lazily());
        let rate = if shut_in > self.shut_in { 0.6 } else { 2.0 };
        self.shut_in += (shut_in - self.shut_in) * (1.0 - (-rate * dt).exp());
        lit.adapt(self.shut_in);
        lit.mist_floor = self.mist_floor.unwrap_or(self.camera.pos.y as f32 - m(12.0));
        let ground = self.weather.places(self.env.date, self.settings.year_days, self.warmth, self.settings.seed);
        self.map.set_season(self.env.date, ground);
        let scene = render::Scene {
            camera: &self.camera,
            env: &lit,
            terrain: &self.streamer,
            far: &self.far,
            impostors: &self.impostors,
            particles: self.particles.instances(),
            props: &self.props,
            shades: &self.shades,
            wind: self.particles.wind.lagged * OLD_VOXEL,
            ground,
            lamps: &lamps,
            shafts: self.settings.shafts,
        };
        // The scene is drawn into the renderer's own textures and handed to the GPU
        // before waiting for the window's next image, which is not free until the last
        // frame is done with: the GPU then draws this frame through the wait, rather
        // than standing idle until the frame has been put together after it.
        self.sub.lap(11);
        self.renderer.draw(&self.gpu, &mut encoder, &scene);
        self.sub.lap(12);
        self.gpu.queue.submit([encoder.finish()]);
        self.sub.lap(13);
        self.cpu.lap(4);
        let Some(frame) = self.surface.acquire(&self.gpu) else {
            return;
        };
        self.cpu.lap(3);
        self.sub.lap(14);
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer.finish(&self.gpu, &mut encoder, &view);
        self.sub.lap(15);
        self.map.draw(&self.gpu, &mut encoder, &view, (self.surface.config.width, self.surface.config.height));
        self.sub.lap(16);

        let size = [self.surface.config.width, self.surface.config.height];
        let cam = &self.camera;
        let (frame_ms, speed) = (self.frame_ms, self.fly.speed);
        let gpu_name = &self.gpu.adapter_name;
        let (s, d) = (self.streamer.stats, self.renderer.stats);
        let (particle_count, biome, seed) = (self.particles.len(), self.biome, self.settings.seed);
        let (gpu_line, cpu_line) = (self.renderer.timer.as_ref().map(profile::GpuTimer::summary), self.cpu.summary());
        let mode = match (self.walking, self.player.mode, self.player.swimming, self.player.on_ground) {
            (false, ..) => "flying",
            (true, Mode::Boat, ..) => "sailing",
            (true, Mode::Glide, ..) => "gliding",
            (true, Mode::Walk, true, _) => "swimming",
            (true, Mode::Walk, false, true) => "walking",
            (true, Mode::Walk, false, false) => "in the air",
        };
        let prompt: Option<String> = match self.player.mode {
            _ if !self.walking => None,
            Mode::Boat => Some("E to go ashore   W to sail   Shift for full sail".into()),
            Mode::Walk if self.fleet.nearest(self.player.pos, BOARD_REACH).is_some() => Some("E to board the boat".into()),
            _ if self.notice.is_some() => self.notice.as_ref().map(|n| n.0.clone()),
            _ => self.camp_prompt().or_else(|| self.within_reach.as_ref().map(|(_, item)| format!("E to gather  {}", item.name))),
        };
        // What has just come to hand, and what it is for, fading out.
        let hand_word = self.pack.in_hand().filter(|_| self.hand_word > 0.0).map(|s| (format!("{}   \u{b7}   {}", s.item.name, inventory::use_of(&s.item.id)), (self.hand_word / 0.6).min(1.0)));
        let mut let_go = Vec::new();
        let (pack, pack_view, pack_open) = (&mut self.pack, &mut self.pack_view, self.pack_open);
        let (journal, journal_view, journal_open) = (&self.journal, &mut self.journal_view, self.journal_open);
        let mut time = self.env.time;
        let mut date = self.env.date;
        let mut settings = self.settings;
        let mut choice = overlay::MenuChoice::default();
        let held = Some(self.streamer.radius()).filter(|r| *r < settings.radius);
        let (show_debug, show_menu, grabbed) = (settings.show_debug, self.show_settings, self.grabbed);
        let (labels, map_open) = (self.map.labels(), self.map.open);
        let console = &mut self.console;
        self.overlay
            .draw(&self.window, &self.gpu, &mut encoder, &view, size, |ctx| {
                overlay::map_labels(ctx, labels);
                if show_menu {
                    choice = overlay::settings_menu(ctx, &mut settings, held, (size[0], size[1]));
                } else if pack_open {
                    let_go = inventory::show(ctx, pack, pack_view);
                } else if journal_open {
                    journal::show(ctx, journal, journal_view);
                } else if console.open {
                    console.show(ctx);
                } else if !grabbed && !map_open {
                    overlay::bottom_line(ctx, "hint", "Click the view (or press Tab) to look around", 150.0, egui::Color32::from_gray(220));
                }
                // The hotbar is always there, but for the menu, the map and the pack
                // (which shows it among its own slots).
                if !show_menu && !pack_open && !map_open {
                    inventory::hotbar(ctx, pack, hand_word.as_ref().map(|(text, shown)| (text.as_str(), *shown)));
                }
                if let Some(text) = prompt {
                    overlay::bottom_line(ctx, "prompt", &text, 112.0, egui::Color32::from_gray(235));
                }
                if !show_debug {
                    return;
                }
                egui::Window::new("Debug")
                    .default_pos([8.0, 8.0])
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "{:.0} fps ({frame_ms:.1} ms)  |  {gpu_name}",
                            1000.0 / frame_ms
                        ));
                        ui.label(format!(
                            "pos {:.1} {:.1} {:.1}   yaw {:.0} pitch {:.0}   speed {speed:.0}",
                            cam.pos.x,
                            cam.pos.y,
                            cam.pos.z,
                            cam.yaw.to_degrees(),
                            cam.pitch.to_degrees()
                        ));
                        ui.label(format!(
                            "chunks {} in {} columns, drawn {} (culled {}), far tiles {}",
                            s.chunks, s.columns, d.chunks_drawn, d.chunks_culled, d.far_tiles
                        ));
                        ui.label(format!(
                            "quads {:.2} M loaded, {:.2} M drawn   mesh {:.0} MB",
                            s.quads as f64 / 1e6,
                            d.quads_drawn as f64 / 1e6,
                            s.mesh_bytes as f64 / 1048576.0
                        ));
                        ui.label(format!(
                            "per column: gen {:.2} ms, mesh {:.2} ms   queued {} (+{} running)",
                            s.gen_ms, s.mesh_ms, s.queued, s.in_flight
                        ));
                        if let Some(line) = &gpu_line {
                            ui.label(line);
                        }
                        ui.label(&cpu_line);
                        ui.add(egui::Slider::new(&mut time, 0.0..=24.0).text("time of day"));
                        let month = season::month_name(date);
                        ui.add(egui::Slider::new(&mut date, 0.0..=1.0).text(format!("time of year ({month})")));
                        ui.label(format!("mode: {mode}   particles {particle_count}   biome {biome:?}   seed {seed}"));
                        ui.label("Esc = settings. Click or Tab to look. WASD, Space = jump/swim up, Shift = sprint");
                        ui.label("F = toggle flying (Space/Ctrl up/down, wheel = speed). F3 = hide this panel, F11 = fullscreen");
                        ui.label("G in the air, or Space when high up = glider. E = board or leave a boat at a landing");
                        ui.label("M = the map (wheel zooms, drag moves it). N = show or hide the corner map");
                        ui.label("I = the pack. J = the journal. 1-7 or the wheel = what is held. Right click = set it down, left click = pick it up");
                        ui.label("` or / = the console (type help)");
                    });
            });
        if time != self.env.time || date != self.env.date {
            self.env = Environment::at(time, date).in_year(self.env.year);
        }
        if choice.resume {
            self.set_menu(false);
        }
        for stack in let_go {
            self.set_down(stack);
        }
        self.quit |= choice.quit;
        self.cpu.lap(5);
        self.sub.lap(17);
        if let Some(t) = &mut self.renderer.timer {
            t.mark(&mut encoder, 15);
            t.resolve(&mut encoder);
        }

        self.gpu.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        self.sub.lap(18);
        // Only now: the surface can't be reconfigured while a frame from it is held.
        let was = std::mem::replace(&mut self.settings, settings);
        self.apply_settings(was);
        if let Some(timer) = &mut self.renderer.timer {
            timer.after_submit(&self.gpu);
        }
        self.cpu.lap(6);
        self.sub.lap(19);
        // Columns the main thread had to generate itself this frame, for the log.
        self.sync_frame = player::take_sync_loads();
        self.sync_total.0 += self.sync_frame.0;
        self.sync_total.1 += self.sync_frame.1;
        self.write_csv_row();
        self.frame += 1;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.run.is_some() {
            return;
        }
        match Running::new(el, &self.opts) {
            Ok(mut r) => {
                for line in &self.opts.commands {
                    let _ = r.command(line);
                }
                if self.opts.shot_console {
                    r.set_console(true);
                }
                self.run = Some(r);
            }
            Err(e) => {
                log::error!("startup failed: {e:#}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(r) = self.run.as_mut() else { return };
        // Esc always reaches the game, even while a slider has the keyboard.
        if let WindowEvent::KeyboardInput { event: key, .. } = &event
            && key.state == ElementState::Pressed
            && !key.repeat
            && key.physical_key == PhysicalKey::Code(KeyCode::Escape)
        {
            if r.console.open {
                r.set_console(false);
            } else if r.map.open {
                r.set_map(false);
            } else if r.pack_open {
                r.set_pack(false);
            } else if r.journal_open {
                r.set_journal(false);
            } else {
                r.set_menu(!r.show_settings);
            }
            return;
        }
        // While the console is open every key is its own, and none reaches the game. It
        // runs, completes and goes through its history itself; the rest is typing.
        if r.console.open
            && let WindowEvent::KeyboardInput { event: key, .. } = &event
        {
            let pressed = key.state == ElementState::Pressed;
            match key.physical_key {
                PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) if pressed => r.submit_command(),
                PhysicalKey::Code(KeyCode::Tab) if pressed => r.console.complete(),
                PhysicalKey::Code(KeyCode::ArrowUp) if pressed => r.console.older(),
                PhysicalKey::Code(KeyCode::ArrowDown) if pressed => r.console.newer(),
                PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab | KeyCode::ArrowUp | KeyCode::ArrowDown) => {}
                _ => drop(r.overlay.on_event(&r.window, &event)),
            }
            return;
        }
        // While the map is up the mouse is the map's, wherever on it the cursor is: egui
        // still hears of it, but may not keep it.
        let for_map = r.map.open
            && !r.show_settings
            && matches!(event, WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. });
        if !r.grabbed && r.overlay.on_event(&r.window, &event) && !for_map {
            return;
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                r.resize(s.width, s.height);
                // Remember only a size the player would want the window to come back at.
                if s.width > 0 && s.height > 0 && r.window.fullscreen().is_none() && !r.window.is_maximized() {
                    r.settings.window = Some((s.width, s.height));
                }
            }
            WindowEvent::Focused(false) => {
                r.set_grab(false);
                r.drag = None;
                r.turning = None;
            }
            WindowEvent::Focused(true) => {
                if r.want_grab {
                    r.set_grab(true);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // A drag begun before the cursor was ever seen starts from here.
                if let Some((x, y)) = r.drag {
                    if x.is_finite() {
                        r.map.drag((position.x - x) as f32, (position.y - y) as f32);
                    }
                    r.drag = Some((position.x, position.y));
                }
                if let Some((x, y)) = r.turning {
                    if x.is_finite() {
                        r.map.turn((position.x - x) as f32, (position.y - y) as f32);
                    }
                    r.turning = Some((position.x, position.y));
                }
                r.last_cursor_seen = Some((position.x, position.y));
                r.cursor_moved(position.x, position.y);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if r.map.open && !r.show_settings => {
                r.drag = (state == ElementState::Pressed).then(|| r.last_cursor_seen.unwrap_or((f64::NAN, f64::NAN)));
            }
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } if r.map.open && !r.show_settings => {
                r.turning = (state == ElementState::Pressed).then(|| r.last_cursor_seen.unwrap_or((f64::NAN, f64::NAN)));
            }
            WindowEvent::MouseWheel { delta, .. } if r.map.open => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                r.map.zoom(lines);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !r.grabbed && !r.show_settings && !r.pack_open && !r.journal_open && !r.console.open => {
                r.want_grab = true;
                r.set_grab(true);
            }
            // The hands: the left takes up what was set down, the right sets down (or
            // pitches, or lays) what is held.
            WindowEvent::MouseInput { state: ElementState::Pressed, button, .. } if r.grabbed && !r.show_settings && !r.pack_open => match button {
                MouseButton::Left => r.take_up(),
                MouseButton::Right => r.use_held(),
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } if r.grabbed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                // Flying, the wheel sets the speed; on foot it goes along the hotbar.
                if r.walking {
                    if lines != 0.0 {
                        r.hold(None, if lines > 0.0 { -1 } else { 1 });
                    }
                } else {
                    r.fly.scroll(lines);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        if code == KeyCode::F3 && !event.repeat {
                            r.settings.show_debug = !r.settings.show_debug;
                        } else if code == KeyCode::F11 && !event.repeat {
                            r.settings.fullscreen = !r.settings.fullscreen;
                            r.window.set_fullscreen(fullscreen(r.settings.fullscreen));
                        } else if r.show_settings {
                            // The rest of the keyboard is for playing.
                        } else if code == KeyCode::KeyM && !event.repeat && !r.pack_open && !r.journal_open {
                            r.set_map(!r.map.open);
                        } else if code == KeyCode::KeyN && !event.repeat {
                            r.settings.minimap = !r.settings.minimap;
                        } else if r.map.open {
                            // The rest waits until the map is put away.
                        } else if code == KeyCode::KeyI && !event.repeat && !r.journal_open {
                            r.set_pack(!r.pack_open);
                        } else if let Some(k) = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7].iter().position(|d| *d == code) {
                            r.hold(Some(k), 0);
                        } else if r.pack_open {
                            // And until the pack is shut.
                        } else if code == KeyCode::KeyJ && !event.repeat {
                            r.set_journal(!r.journal_open);
                        } else if r.journal_open {
                            // Or the journal.
                        } else if matches!(code, KeyCode::Backquote | KeyCode::Slash) && !event.repeat {
                            r.set_console(true);
                        } else if r.rest.is_some() && !event.repeat && matches!(code, KeyCode::KeyW | KeyCode::KeyA | KeyCode::KeyS | KeyCode::KeyD | KeyCode::Space | KeyCode::KeyE | KeyCode::KeyF) {
                            // Any step gets the player up from a rest.
                            r.wake();
                            if code == KeyCode::KeyF {
                                r.toggle_flying();
                            } else if code != KeyCode::KeyE {
                                r.keys.insert(code);
                            }
                        } else if code == KeyCode::Tab && !event.repeat {
                            r.want_grab = !r.grabbed;
                            r.set_grab(r.want_grab);
                        } else if code == KeyCode::KeyV && !event.repeat {
                            r.third = !r.third;
                        } else if code == KeyCode::KeyF && !event.repeat {
                            r.toggle_flying();
                        } else if code == KeyCode::KeyE && !event.repeat {
                            r.interact();
                        } else if r.grabbed {
                            // G opens the glider anywhere in the air. A fresh press of Space
                            // does too, but only high up: Space is also the jump key, and
                            // hopping up a hill must not open it.
                            if !event.repeat && code == KeyCode::KeyG {
                                r.deploy = true;
                            }
                            if !event.repeat && code == KeyCode::Space {
                                r.deploy_clear = true;
                            }
                            r.keys.insert(code);
                        }
                    } else {
                        r.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                r.redraw();
                if !r.loaded && r.streamer.is_idle() {
                    r.loaded = true;
                    log::info!("startup: all terrain in view {:.2} s after the first frame", r.start.elapsed().as_secs_f32());
                }
                if r.quit {
                    el.exit();
                }
                if r.save_timer <= 0.0 {
                    r.save_timer = AUTOSAVE;
                    r.persist(&self.opts.overrides);
                }
                // A timed run can have the large map open, as a screenshot can, to see
                // what drawing it costs.
                if self.opts.frames.is_some()
                    && r.frame == 2
                    && let Some(large) = self.opts.shot_map
                {
                    let mut path = self.opts.walked.clone();
                    path.push(glam::DVec2::new(r.camera.pos.x, r.camera.pos.z));
                    r.map.walk(&path);
                    if let Some(span) = large {
                        r.set_map(true);
                        r.map.set_span(span);
                        if let Some((yaw, pitch)) = self.opts.shot_turn {
                            r.map.set_turn(yaw.to_radians(), pitch.to_radians());
                        }
                        if let Some(by) = self.opts.shot_pan {
                            r.map.shift(by);
                        }
                    }
                }
                if self.opts.frames.is_some() && r.frame % 120 == 0 {
                    log::info!("frame {}: {}; {}; {}", r.frame, r.streamer.progress(), r.impostors.progress(), r.far.progress());
                    // A timed run whose terrain has stopped loading fails, rather than
                    // timing a world that is not there.
                    if r.streamer.stalled().is_some() {
                        log::error!("timed run hung: the terrain stopped loading (see above)");
                        std::process::exit(3);
                    }
                }
                if self.opts.frames.is_some_and(|n| r.frame >= n) {
                    let (s, d) = (r.streamer.stats, r.renderer.stats);
                    let mut times = r.frame_log.take().unwrap_or_default();
                    times.sort_unstable_by(f32::total_cmp);
                    log::info!(
                        "frame ms: median {:.1}, p95 {:.1}, p99 {:.1}, worst {:.1}",
                        profile::quantile(&times, 0.5),
                        profile::quantile(&times, 0.95),
                        profile::quantile(&times, 0.99),
                        profile::quantile(&times, 1.0)
                    );
                    log::info!("{}", r.cpu.summary());
                    log::info!("{}", r.cpu.worst_summary());
                    if let Some(timer) = &r.renderer.timer {
                        log::info!("{}", timer.summary());
                    }
                    for (level, (n, generate, mesh)) in s.built.iter().enumerate().filter(|(_, b)| b.0 > 0) {
                        let n = *n as f32;
                        log::info!("level {level}: {n} columns built, {:.2} ms to generate and {:.2} ms to mesh each", generate / n, mesh / n);
                    }
                    log::info!("main thread generated {} columns itself, in {:.1} ms", r.sync_total.0, r.sync_total.1);
                    log::info!(
                        "after {} frames: {:.1} ms/frame ({:.0} fps), {} chunks, {:.2} M quads drawn, {} far tiles, {} far tree boxes, arena {} MiB",
                        r.frame,
                        r.frame_ms,
                        1000.0 / r.frame_ms,
                        s.chunks,
                        d.quads_drawn as f64 / 1e6,
                        d.far_tiles,
                        d.impostor_boxes,
                        s.arena_bytes >> 20
                    );
                    el.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(r) = self.run.as_mut() else { return };
        if let DeviceEvent::MouseMotion { delta } = event
            && r.grabbed
        {
            r.raw_motion_seen = true;
            r.camera.add_look(delta.0 as f32, delta.1 as f32);
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if let Some(r) = self.run.as_mut() {
            r.map.save(true);
            r.persist(&self.opts.overrides);
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(r) = &self.run {
            r.window.request_redraw();
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("wgpu_core", log::LevelFilter::Warn)
        .filter_module("wgpu_hal", log::LevelFilter::Warn)
        // Vulkan loader chatter about other programs' broken overlay layers; not ours to fix.
        .filter_module("wgpu_hal::vulkan::instance", log::LevelFilter::Off)
        .filter_module("naga", log::LevelFilter::Warn)
        .init();
    let mut opts = parse_args()?;

    if let Some(mut structures) = opts.structures.clone() {
        structures.screenshot = opts.screenshot.clone();
        structures.size = opts.size;
        return editor::run(structures);
    }

    if let Some(path) = &opts.map {
        let settings = opts.overrides.applied_to(Settings::default());
        let world = GameWorld::new(settings.seed);
        let camera = make_camera(&opts, &world, None);
        minimap::build(&world, camera.pos).save(path)?;
        log::info!("map {}", path.display());
        return Ok(());
    }

    if let Some(path) = opts.screenshot.clone() {
        let path = &path;
        // Screenshots neither read nor write anything saved, so they are repeatable.
        let settings = opts.overrides.applied_to(Settings::default());
        let gpu = Gpu::headless()?;
        let world = Arc::new(GameWorld::new(settings.seed));
        // The console's commands set the shot up as the flags would, before anything is
        // made of them. One that fails stops the shot: it would not show what was asked.
        let mut console = commands::Console::default();
        for line in std::mem::take(&mut opts.commands) {
            let reply = console.run(&line, &mut Shot { opts: &mut opts, world: &world, seed: settings.seed });
            Running::log_reply(&line, &reply);
            if let Err(wrong) = reply {
                bail!("--command '{line}': {wrong}");
            }
        }
        console.open = opts.shot_console;
        let opts = opts;
        let mut camera = make_camera(&opts, &world, None);
        // The figure stands where the camera was, mid stride, and the camera steps back.
        let figure = opts.third.map(|turned| {
            let feet = camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
            camera.pos += -camera.forward().as_dvec3() * m64(3.6) + DVec3::new(0.0, m64(0.25), 0.0);
            let facing = camera.yaw + turned.to_radians();
            let gait = if opts.figure_glide.is_some() {
                character::Gait::gliding(facing)
            } else if opts.figure_furl > 0.0 {
                character::Gait::reaching(facing, opts.figure_furl)
            } else {
                character::Gait::striding(facing, opts.figure_speed)
            };
            (feet, gait)
        });
        let mut far = FarView::new(&gpu, world.clone());
        let mut streamer = Streamer::new(&gpu, world.clone(), settings.radius);
        let mut impostors = Impostors::new(&gpu, world.clone());
        let t = Instant::now();
        let mut env = Environment::at(opts.time.unwrap_or(10.0), opts.date.unwrap_or(season::DEFAULT_DATE)).in_year(opts.year.unwrap_or(0));
        // Screenshots are clear unless a weather is asked for.
        let mut weather = weather::Weather::new(opts.weather);
        weather.apply(&mut env);
        env.cloud_base = weather::CloudBase::default().update(&world, camera.pos, 0.0);
        if let Some(metres) = opts.picked {
            let (reach, x, z) = (m(metres) as i32, camera.pos.x.floor() as i32, camera.pos.z.floor() as i32);
            let all = world.flowers_in(x - reach, z - reach, 2 * reach + 1);
            log::info!("picking {} flowers", all.len());
            for ((x, z), _) in all {
                world.pick(x, z);
            }
        }
        // A few seconds of ambient life, so stills show spray, leaves and fireflies.
        streamer.wait_idle(&gpu, camera.pos);
        let mut voxels = WorldVoxels::new(world.clone());
        let (biome, leaves, flowers, low, sticks, nests) = scan_surroundings(&world, &mut voxels, camera.pos, 1, env.date);
        env.mist_floor = low;
        // The snow and ice the season has left lying here, as the game would have them.
        let warmth = season::warmth(biome);
        // The northern lights, if they are out tonight here (or asked for).
        let lights = if let Some(out) = opts.aurora { out as u8 as f32 } else { aurora::cold(warmth, aurora::climate_at(&world, camera.pos.x, camera.pos.z)) * aurora::activity(env.date, settings.year_days, settings.seed) };
        aurora::light(&mut env, lights);
        log::info!("northern lights {:.2}", env.aurora);
        weather.settle(env.date, settings.year_days, warmth, settings.seed);
        env.air(season::humidity(biome), weather.wet * (1.0 - (weather.now.rain + weather.now.snow).min(1.0)));
        env.adapt(player::enclosure(camera.pos, &mut voxels));
        let cascades: Vec<_> = streamer.cascades_near(camera.pos, m(120.0)).copied().collect();
        let springs: Vec<_> = streamer.springs_near(camera.pos, m(40.0)).copied().collect();
        let surf: Vec<_> = streamer.surf_near(camera.pos, m(60.0)).copied().collect();
        let mut fleet = Fleet::new(world.clone());
        fleet.refresh(camera.pos);
        for &(at, heading) in &opts.shot_boats {
            fleet.leave(at, heading);
        }
        let mut props = Vec::new();
        for boat in fleet.boats() {
            vehicles::draw_boat(boat.pos, boat.heading, 0.0, camera.pos, &mut props);
        }
        if let Some((feet, gait)) = &figure {
            let furled = opts.figure_glide.is_none().then_some(opts.figure_furl);
            character::draw(&character::WANDERER, *feet, gait.facing, &gait.pose(), furled, camera.pos, &mut props);
            if let Some(open) = opts.figure_glide {
                let flight = vehicles::Flight::of(opts.figure_bank.to_radians(), opts.figure_rise, 0.0);
                vehicles::draw_glider(*feet, gait.facing, open, flight, camera.pos, &mut props);
            }
        }
        // A camp a few steps ahead: a fire, and a few things set down.
        let mut camp = placed::Placed::new();
        if let Some(state) = opts.shot_camp {
            sample_camp(&mut camp, &mut voxels, camera.pos, camera.yaw, state, opts.camp_ahead);
        }
        // Lying by its fire, facing it, looking up as high as --look says.
        if opts.rest
            && let Some((_, fire)) = camp.near(camera.pos, m64(30.0)).find(|(_, t)| t.is_fire())
        {
            let bed = by_the_fire(&mut voxels, fire, camera.pos);
            let to = fire.pos - bed;
            camera.pos = bed + DVec3::new(0.0, player::EYE_HEIGHT - m64(camp::Rest::begin(&env).lowered() as f64), 0.0);
            camera.yaw = (to.z.atan2(to.x) as f32).rem_euclid(std::f32::consts::TAU);
        }
        camp.draw(camera.pos, 0.6, &mut props);
        fallen::draw(&sticks, &inventory::Pack::new(), camera.pos, &mut props);
        let fires: Vec<DVec3> = camp.near(camera.pos, m64(90.0)).filter(|(_, t)| t.burns()).map(|(_, t)| t.heart()).collect();
        // Smoke rises from them, and from the embers of the camps that were here already.
        let mut smoking = camp.smoking(camera.pos, m64(particles::SMOKE_SIGHT), 1.0);
        smoking.extend(embers_near(&streamer, &mut voxels, camera.pos).into_iter().map(|e| (e, 1.0)));
        log::info!("{} fires burning near, {} of them the player's", smoking.len(), fires.len());
        let mut particles = Particles::new();
        if opts.murmuration {
            particles.starlings.forced = Some(camera.forward());
        }
        if let Some(ahead) = opts.shot_butterflies {
            particles.set_out_butterflies(camera.pos + camera.forward().as_dvec3() * m64(ahead));
        }
        for step in 0..(opts.ambient * 60.0) as u32 {
            let surroundings = Surroundings {
                camera: camera.pos,
                biome,
                sun_elevation: env.sun_elevation,
                cascades: &cascades,
                springs: &springs,
                surf: &surf,
                leaves: &leaves,
                flowers: &flowers,
                fires: &smoking,
                player_entered_water: None,
                leaf_fall: season::leaf_fall(env.date),
                date: env.date,
                rain: weather.now.rain,
                snow: weather.now.snow,
                storm: weather.now.storm,
                time: step as f64 / 60.0,
                land: &|x, z| particles::land_under(&world, x, z),
                nests: &nests.0,
                forage: &nests.1,
                hour: env.time,
            };
            particles.update(1.0 / 60.0, &surroundings, &mut voxels);
        }
        log::info!("season {:.2} ({}), biome {biome:?}, ground {:?}", env.date, season::month_name(env.date), weather.ground(env.date, warmth));
        // Crows on a map made at once, and any set out as --crows says, given the same
        // seconds of life as the rest.
        let mut crows = crows::Crows::without_lookout();
        crows.set_sites(crows::survey::Survey::new(world.clone()).look(camera.pos, env.date));
        if let Some(how) = &opts.shot_crows {
            let ahead = camera.pos + camera.forward().as_dvec3().with_y(0.0).normalize_or(DVec3::X) * m64(opts.crows_ahead);
            crows.stage(how, ahead, camera.pos);
        }
        for _ in 0..(opts.ambient * 60.0) as u32 {
            let around = crows::Around {
                camera: camera.pos,
                sun_elevation: env.sun_elevation,
                hour: env.time,
                date: env.date,
                rain: weather.now.rain,
                land: &|x, z| particles::land_under(&world, x, z),
            };
            crows.update(1.0 / 60.0, &around);
        }
        crows.report(camera.pos);
        crows.draw(camera.pos, &mut props);
        for (at, n) in particles.flocks() {
            let off = at - camera.pos;
            log::info!("flock of {n} at {:.0} {:.0} {:.0} from the camera", off.x, off.y, off.z);
        }
        if let Some((at, n)) = particles.starlings.now() {
            let off = at - camera.pos;
            log::info!("murmuration of {n} starlings at {:.0} {:.0} {:.0} from the camera", off.x, off.y, off.z);
        }
        // Where the nests further about are, to aim a shot at one (RUST_LOG=game=debug).
        if log::log_enabled!(log::Level::Debug) {
            for n in world.nests_near(camera.pos.x as i32, camera.pos.z as i32, m(200.0) as i32) {
                log::debug!("nest of {:?} in a {:?} at {} {} {}, facing {}", n.kind, n.host, n.door.x, n.door.y, n.door.z, n.out);
            }
        }
        let mut lamps: Vec<(DVec3, f32)> = streamer.lamps_near(camera.pos, m(24.0)).map(|p| (p.as_dvec3() + DVec3::splat(0.5), 1.0)).collect();
        lamps.extend(fires.iter().map(|&heart| firelight(heart, 0.6)));
        // A lit lantern in the hand: of the figure if there is one, else of whoever's eyes
        // the camera is, standing (seen only by its light).
        if opts.shot_lantern {
            let (mut unseen, standing) = (Vec::new(), character::Gait::new(camera.yaw));
            let (feet, gait, into) = match &figure {
                Some((feet, gait)) => (*feet, gait, &mut props),
                None => (camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0), &standing, &mut unseen),
            };
            lamps.push((character::draw_lantern(feet, gait.facing, &gait.pose(), candle(0.6), camera.pos, into), candle(0.6)));
        }
        // The map, as the player would have it after walking here.
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut map = opts.shot_map.map(|large| {
            let mut map = Minimap::new(&gpu, format, world.clone(), None);
            map.set_season(env.date, weather.places(env.date, settings.year_days, warmth, settings.seed));
            map.settle(&gpu, camera.pos, camera.forward());
            let mut path = opts.walked.clone();
            path.push(glam::DVec2::new(camera.pos.x, camera.pos.z));
            map.walk(&path);
            if let Some(span) = large {
                map.set_open(true);
                map.set_span(span);
                if let Some((yaw, pitch)) = opts.shot_turn {
                    map.set_turn(yaw.to_radians(), pitch.to_radians());
                }
                if let Some(by) = opts.shot_pan {
                    map.shift(by);
                }
            }
            map.camps = camp.camps();
            map.settle(&gpu, camera.pos, camera.forward());
            map
        });
        // A pack with something of everything that can be gathered, some of it in plenty.
        let mut sample = opts.shot_pack.then(|| {
            use worldgen::materials as mat;
            let mut pack = inventory::Pack::new();
            use worldgen::features::plants::Flower;
            for (k, f) in Flower::ALL.iter().enumerate() {
                for _ in 0..[12, 3, 1, 7, 2, 31, 1, 5, 4][k % 9] {
                    pack.add(&inventory::of_flower(*f));
                }
            }
            for v in [mat::HOLLY_BERRY, mat::REED_HEAD] {
                pack.add(&inventory::of_voxel(v, world.materials()).expect("a thing to gather"));
            }
            // The first of them went to the hotbar: most of it back to the pack, to
            // leave room for what a camp is made with.
            for k in 3..inventory::HOTBAR {
                pack.shift(inventory::SLOTS + k, 20 + k - 3);
            }
            for _ in 0..5 {
                pack.add(&inventory::sticks());
            }
            pack.shift(10, 14);
            let mut view = inventory::View::default();
            view.select(1);
            (pack, view)
        });
        // A journal as it might stand after a walk round the places near here.
        let mut written = opts.shot_journal.map(|page| {
            let mut view = journal::View::new(world.materials());
            view.page = page;
            let (c, at) = journal::continent_at(&world, camera.pos, true).expect("the continent, having waited for it");
            (journal::Journal::sample(&c, at, opts.journal_fill, view.catalogue(), env.year), view)
        });
        // A hotbar as it might be an hour into a walk: sticks, things picked.
        let hotbar = opts.shot_hotbar.then(|| {
            use worldgen::features::plants::Flower;
            let mut pack = inventory::Pack::new();
            pack.give_kit();
            for (item, n) in [(inventory::sticks(), 5), (inventory::of_flower(Flower::Poppy), 12), (inventory::of_flower(Flower::PennyBun), 2), (inventory::of_flower(Flower::Bluebell), 7)] {
                pack.add_many(&item, n);
            }
            pack.shift(inventory::SLOTS + 3, inventory::SLOTS + 5);
            pack.hold(0);
            pack
        });
        let mut hud = |encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, text: bool| {
            if text && let Some((journal, journal_view)) = &mut written {
                overlay::draw_headless(&gpu, encoder, view, format, opts.size, |ctx| journal::show(ctx, journal, journal_view));
            }
            if text && let Some((pack, pack_view)) = &mut sample {
                overlay::draw_headless(&gpu, encoder, view, format, opts.size, |ctx| drop(inventory::show(ctx, pack, pack_view)));
            } else if text && let Some(pack) = &hotbar {
                let word = pack.in_hand().map(|s| format!("{}   \u{b7}   {}", s.item.name, inventory::use_of(&s.item.id)));
                overlay::draw_headless(&gpu, encoder, view, format, opts.size, |ctx| inventory::hotbar(ctx, pack, word.as_deref().map(|w| (w, 1.0))));
            }
            if text && let Some(line) = &opts.shot_prompt {
                overlay::draw_headless(&gpu, encoder, view, format, opts.size, |ctx| overlay::bottom_line(ctx, "prompt", line, 112.0, egui::Color32::from_gray(235)));
            }
            if text && console.open {
                overlay::draw_headless(&gpu, encoder, view, format, opts.size, |ctx| console.show(ctx));
            }
            let Some(map) = &mut map else { return };
            map.hour = env.time;
            map.update(&gpu, camera.pos, camera.forward(), true, 0.0);
            map.draw(&gpu, encoder, view, opts.size);
            if text {
                overlay::draw_headless(&gpu, encoder, view, format, opts.size, |ctx| overlay::map_labels(ctx, map.labels()));
            }
        };
        render::screenshot(
            &gpu,
            path,
            opts.size,
            &camera,
            &env,
            &mut streamer,
            &mut far,
            &mut impostors,
            particles.instances(),
            &props,
            particles.wind.lagged * OLD_VOXEL,
            weather.places(env.date, settings.year_days, warmth, settings.seed),
            &lamps,
            &mut hud,
        )?;
        let s = streamer.stats;
        log::info!(
            "{} chunks in {} columns, {:.2} M quads, {:.0} MB mesh, {:.1} s total; per column gen {:.2} ms, mesh {:.2} ms",
            s.chunks,
            s.columns,
            s.quads as f64 / 1e6,
            s.mesh_bytes as f64 / 1048576.0,
            t.elapsed().as_secs_f32(),
            s.gen_ms,
            s.mesh_ms
        );
        return Ok(());
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { opts, run: None };
    event_loop.run_app(&mut app)?;
    Ok(())
}


/// Measurement build: a per-frame CSV, to the file named by WH_FRAME_CSV.
fn open_csv() -> Option<std::io::BufWriter<std::fs::File>> {
    use std::io::Write;
    let path = std::env::var("WH_FRAME_CSV").ok()?;
    let mut w = std::io::BufWriter::new(std::fs::File::create(path).ok()?);
    let mut head = vec!["frame".to_string(), "dt_ms".to_string()];
    head.extend(profile::CPU_SPANS.iter().map(|n| format!("cpu_{n}")));
    head.extend(profile::SUB_SPANS.iter().map(|n| format!("sub_{n}")));
    head.push("gpu_fresh".into());
    head.extend(profile::GPU_SPANS.iter().map(|n| format!("gpu_{}", n.replace(' ', ""))));
    for n in ["queued", "in_flight", "rebuild_ms", "upload_ms", "received", "upload_bytes", "grown_bytes", "columns", "chunks_drawn", "quads_drawn", "shadow1_draws", "shadow2_draws", "shadow3_draws", "far_tiles", "impostor_boxes", "particles", "props", "shades", "x", "y", "z", "sync_cols", "sync_ms"] {
        head.push(n.into());
    }
    let _ = writeln!(w, "{}", head.join(","));
    Some(w)
}

impl Running {
    fn write_csv_row(&mut self) {
        use std::io::Write;
        let Some(w) = &mut self.csv else { return };
        let mut row: Vec<String> = vec![self.frame.to_string(), format!("{:.3}", self.last_dt_ms)];
        row.extend(self.cpu.raw.iter().map(|v| format!("{v:.3}")));
        row.extend(self.sub.ms.iter().map(|v| format!("{v:.3}")));
        match &self.renderer.timer {
            Some(t) => {
                row.push((t.fresh as u8).to_string());
                row.extend(t.raw.iter().map(|v| format!("{v:.3}")));
            }
            None => {
                row.push("0".into());
                row.extend(profile::GPU_SPANS.iter().map(|_| "0".to_string()));
            }
        }
        let (s, d) = (self.streamer.stats, self.renderer.stats);
        row.extend([
            s.queued.to_string(),
            s.in_flight.to_string(),
            format!("{:.3}", s.f_rebuild_ms),
            format!("{:.3}", s.f_upload_ms),
            s.f_received.to_string(),
            s.f_bytes.to_string(),
            s.f_grown.to_string(),
            s.columns.to_string(),
            d.chunks_drawn.to_string(),
            d.quads_drawn.to_string(),
            d.shadow_draws[0].to_string(),
            d.shadow_draws[1].to_string(),
            d.shadow_draws[2].to_string(),
            d.far_tiles.to_string(),
            d.impostor_boxes.to_string(),
            self.particles.len().to_string(),
            self.props.len().to_string(),
            self.shades.len().to_string(),
            format!("{:.1}", self.camera.pos.x),
            format!("{:.1}", self.camera.pos.y),
            format!("{:.1}", self.camera.pos.z),
            self.sync_frame.0.to_string(),
            format!("{:.3}", self.sync_frame.1),
        ]);
        let _ = writeln!(w, "{}", row.join(","));
    }
}

/// Where world `(x, z)` given to `tp` is: the middle of the voxel for a whole number.
fn centred(v: f64) -> f64 {
    if v.fract() == 0.0 { v + 0.5 } else { v }
}

/// How a `tp` reads back.
fn arrived(eye: DVec3, ground: f64) -> String {
    format!("at {:.0}, {:.0}, the eye {:.1} m over the ground", eye.x.floor(), eye.z.floor(), (eye.y - ground) / worldgen::scale::VOXELS_PER_METRE as f64)
}

/// The weather, in words: held at a kind, or going its own way, and what it is doing.
fn weather_words(w: &weather::Weather) -> String {
    let s = w.now;
    let doing = format!("rain {:.0}%, snow {:.0}%, storm {:.0}%, cloud {:.0}%, overcast {:.0}%, fog {:.0}%", s.rain * 100.0, s.snow * 100.0, s.storm * 100.0, s.cloud * 100.0, s.overcast * 100.0, s.fog * 100.0);
    match w.forced() {
        Some(f) => format!("the weather is held at {} ({doing})", f.name()),
        None => format!("the weather is going its own way ({doing})"),
    }
}

impl commands::Game for Running {
    fn seed(&self) -> u32 {
        self.settings.seed
    }

    fn here(&self) -> commands::Here {
        let eye = if self.walking { self.player.eye() } else { self.camera.pos };
        commands::Here { eye, ground: surface_height(&self.world, eye.x.floor() as i32, eye.z.floor() as i32), yaw: self.camera.yaw.to_degrees(), pitch: self.camera.pitch.to_degrees(), flying: !self.walking }
    }

    fn clock(&self) -> (f32, f32) {
        (self.env.time, self.env.date)
    }

    fn set_clock(&mut self, hour: f32, date: f32) {
        self.env = Environment::at(hour, date).in_year(self.env.year);
    }

    fn year(&self) -> u32 {
        self.env.year
    }

    fn set_year(&mut self, year: u32) -> commands::Reply {
        self.env.year = year;
        Ok(String::new())
    }

    fn weather(&self) -> String {
        weather_words(&self.weather)
    }

    fn set_weather(&mut self, weather: Option<weather::Forced>) -> commands::Reply {
        self.weather.force(weather);
        Ok(match weather {
            Some(f) => format!("the weather is held at {} ('weather auto' lets it go)", f.name()),
            None => "the weather goes its own way again".into(),
        })
    }

    fn teleport(&mut self, x: f64, z: f64, h: Option<f64>) -> commands::Reply {
        let (x, z) = (centred(x), centred(z));
        // Finding the ground can mean making that part of the world first, which takes a
        // moment far from here.
        let ground = surface_height(&self.world, x.floor() as i32, z.floor() as i32);
        let eye = DVec3::new(x, ground + h.unwrap_or(player::EYE_HEIGHT).max(0.0), z);
        self.go_to(eye);
        Ok(arrived(eye, ground))
    }

    fn home(&self) -> (f64, f64) {
        let spawn = self.world.spawn_point();
        (spawn.x as f64 + 0.5, spawn.z as f64 + 0.5)
    }

    fn back(&mut self) -> commands::Reply {
        let eye = self.before_tp.ok_or("there is nowhere to go back to: no tp yet")?;
        self.go_to(eye);
        Ok(format!("back {}", arrived(eye, surface_height(&self.world, eye.x.floor() as i32, eye.z.floor() as i32))))
    }

    fn places(&self, query: &str, n: usize) -> Vec<commands::Named> {
        let eye = if self.walking { self.player.eye() } else { self.camera.pos };
        named_places(&self.world, eye, query, n)
    }

    fn fly(&mut self, on: Option<bool>) -> commands::Reply {
        if on.is_none_or(|on| on == self.walking) {
            if self.walking && self.player.mode == Mode::Boat {
                return Err("go ashore first (E)".into());
            }
            self.wake();
            self.toggle_flying();
        }
        Ok(if self.walking { "on foot".into() } else { "flying: WASD, Space and Ctrl, the wheel for speed (F lands)".into() })
    }

    fn spawn(&mut self, what: commands::Spawn) -> commands::Reply {
        use commands::Spawn;
        let eye = if self.walking { self.player.eye() } else { self.camera.pos };
        match what {
            Spawn::Camp(state) => {
                if !sample_camp(&mut self.placed, &mut self.voxels, eye, self.camera.yaw, state, CAMP_AHEAD) {
                    return Err("there is no ground near here to make camp on".into());
                }
                self.map.camps = self.placed.camps();
                self.thing_near = None;
                self.reach_timer = 0.0;
                Ok(format!("camp made a few steps ahead, its fire {}", commands::fire_word(state)))
            }
            Spawn::Murmuration => {
                self.particles.starlings.summon(self.camera.forward());
                Ok("starlings gathering ahead, a couple of hundred metres off".into())
            }
            Spawn::Crows(how) => {
                if !self.crows.have_sites() {
                    return Err("the crows have not looked over the country here yet: try again in a moment".into());
                }
                let ahead = self.camera.pos + self.camera.forward().as_dvec3().with_y(0.0).normalize_or(DVec3::X) * m64(12.0);
                if !self.crows.summon(&how, ahead, self.camera.pos) {
                    return Err(format!("there is nowhere near for crows to {how}"));
                }
                Ok(format!("crows called in to {how}"))
            }
            Spawn::Boat => {
                let at = water_ahead(&self.world, eye, self.camera.yaw).ok_or("there is no water ahead to put a boat on (look toward it)")?;
                self.fleet.leave(at, self.camera.yaw);
                Ok(format!("a boat afloat {:.0} m ahead (E by it to board)", at.with_y(eye.y).distance(eye) / m64(1.0)))
            }
            Spawn::Thing(name, n) => {
                let item = commands::item_named(&name, self.world.materials()).ok_or_else(|| format!("there is no thing called '{name}' (Tab lists them)"))?;
                if !self.walking || self.player.mode != Mode::Walk {
                    return Err("things are set down on foot: 'fly off' first".into());
                }
                let word = format!("{} \u{d7} {} set down", item.name, n.min(inventory::STACK));
                self.set_down(inventory::Stack { item, count: n.min(inventory::STACK) });
                self.thing_near = None;
                self.reach_timer = 0.0;
                Ok(word)
            }
        }
    }

    fn aurora(&mut self, on: Option<bool>) -> commands::Reply {
        self.aurora_forced = on;
        Ok(match on {
            Some(true) => "the northern lights are out every night (they show only in the dark)".into(),
            Some(false) => "no northern lights".into(),
            None => "the northern lights come out as the night has them".into(),
        })
    }

    fn give(&mut self, name: &str, n: u32) -> commands::Reply {
        let item = commands::item_named(name, self.world.materials()).ok_or_else(|| format!("there is no thing called '{name}' (Tab lists them)"))?;
        let left = self.pack.add_many(&item, n);
        if left == n {
            return Err("the pack is full".into());
        }
        let short = if left > 0 { format!(" (no room for {left} more)") } else { String::new() };
        Ok(format!("{} \u{d7} {} in the pack{short}", item.name, self.pack.count(&item.id)))
    }

    fn pace(&mut self, times: Option<f32>) -> commands::Reply {
        if let Some(t) = times {
            self.pace = t;
        }
        Ok(format!("walking at {} times the usual speed", self.pace))
    }

    fn third(&mut self) -> commands::Reply {
        self.third = !self.third;
        Ok(if self.third { "the view from behind".into() } else { "the view through the eyes".into() })
    }
}

/// A screenshot being set up, for the console's commands: each sets what its flag would.
struct Shot<'a> {
    opts: &'a mut Options,
    world: &'a GameWorld,
    seed: u32,
}

impl Shot<'_> {
    fn eye(&self) -> DVec3 {
        match self.opts.over {
            Some((x, z, h)) => DVec3::new(x as f64 + 0.5, surface_height(self.world, x, z) + h, z as f64 + 0.5),
            None => self.opts.pos,
        }
    }
}

impl commands::Game for Shot<'_> {
    fn seed(&self) -> u32 {
        self.seed
    }

    fn here(&self) -> commands::Here {
        let eye = self.eye();
        commands::Here { eye, ground: surface_height(self.world, eye.x.floor() as i32, eye.z.floor() as i32), yaw: self.opts.look.0, pitch: self.opts.look.1, flying: true }
    }

    fn clock(&self) -> (f32, f32) {
        (self.opts.time.unwrap_or(10.0), self.opts.date.unwrap_or(season::DEFAULT_DATE))
    }

    fn set_clock(&mut self, hour: f32, date: f32) {
        (self.opts.time, self.opts.date) = (Some(hour), Some(date));
    }

    fn year(&self) -> u32 {
        self.opts.year.unwrap_or(0)
    }

    fn set_year(&mut self, year: u32) -> commands::Reply {
        self.opts.year = Some(year);
        Ok(String::new())
    }

    fn weather(&self) -> String {
        match self.opts.weather {
            Some(f) => format!("the weather is held at {}", f.name()),
            None => "the weather is a screenshot's own: fair".into(),
        }
    }

    fn set_weather(&mut self, weather: Option<weather::Forced>) -> commands::Reply {
        self.opts.weather = weather;
        Ok(format!("the weather is {}", weather.map_or("a screenshot's own, fair", |f| f.name())))
    }

    fn teleport(&mut self, x: f64, z: f64, h: Option<f64>) -> commands::Reply {
        let h = h.unwrap_or(player::EYE_HEIGHT).max(0.0);
        self.opts.over = Some((x.floor() as i32, z.floor() as i32, h));
        self.opts.fly = true;
        let eye = self.eye();
        Ok(arrived(eye, eye.y - h))
    }

    fn home(&self) -> (f64, f64) {
        let spawn = self.world.spawn_point();
        (spawn.x as f64 + 0.5, spawn.z as f64 + 0.5)
    }

    fn places(&self, query: &str, n: usize) -> Vec<commands::Named> {
        named_places(self.world, self.eye(), query, n)
    }

    fn spawn(&mut self, what: commands::Spawn) -> commands::Reply {
        use commands::Spawn;
        Ok(match what {
            Spawn::Camp(state) => {
                self.opts.shot_camp = Some(state);
                format!("camp made a few steps ahead, its fire {}", commands::fire_word(state))
            }
            Spawn::Murmuration => {
                self.opts.murmuration = true;
                "starlings gathering ahead, a couple of hundred metres off".into()
            }
            Spawn::Crows(how) => {
                let said = format!("crows called in to {how}");
                self.opts.shot_crows = Some(how);
                said
            }
            Spawn::Boat => {
                let (eye, yaw) = (self.eye(), self.opts.look.0.to_radians());
                let at = water_ahead(self.world, eye, yaw).ok_or("there is no water ahead to put a boat on (look toward it)")?;
                self.opts.shot_boats.push((at, yaw));
                format!("a boat afloat {:.0} m ahead", at.with_y(eye.y).distance(eye) / m64(1.0))
            }
            Spawn::Thing(..) => return Err("things can't be set down on a screenshot: spawn camp sets a few down".into()),
        })
    }

    fn aurora(&mut self, on: Option<bool>) -> commands::Reply {
        self.opts.aurora = on;
        Ok(match on {
            Some(true) => "the northern lights are out tonight".into(),
            Some(false) => "no northern lights".into(),
            None => "the northern lights come out as the night has them".into(),
        })
    }
}
