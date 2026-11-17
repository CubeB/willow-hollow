//! The map (#26): a small round minimap in the corner and a large map on a key, both
//! north up, showing the land and water around the player as far as they have seen
//! it, the faint trail of where they have walked, and soft marks for the places worth
//! finding that come to light as the map fills in.
//!
//! Each continent gets a picture, one texel per grid cell, and a list of the places on
//! it worth walking to ([`marks`]), both made once on a worker thread. A reveal mask per
//! continent ([`reveal`]) records what has been seen and where the player walked; it
//! is kept per world on disk. The map is drawn over the finished frame with its own
//! shader, from texture arrays holding the two by two continents around its middle, so
//! it reaches over the edge of a continent as soon as the next one has been simulated.
//! Text (names on the large map) goes through egui.
//!
//! Marks are hints, not objectives: no distances, no lines to them. A place not yet
//! visited has no mark on the land once that has been seen (a natural one has its
//! name); one still out of sight shows as a faint glow at the corner map's rim in its
//! direction, for the nearest few. A visited place turns into its icon, or close in, where the
//! model has the place itself standing on it, is left to that.
//!
//! The large map is a model of the land seen so far ([`relief`]): a slab of blocks with
//! its water, paths and woods, and close in its trees, buildings, bridges and tors,
//! looked down on at a slant, which the mouse moves, turns and zooms. It is lit by a
//! sun of its own that never moves ([`light`]), with the shadows worked into the model
//! as it is built. The corner map is a small one of the same, of the ground about the
//! player, turned so that the way they face is always up.
//!
//! The large map has the names of the places on it ([`names`]): the land's own (regions,
//! ranges, lakes, rivers, peaks, tors) wherever the land they belong to has been seen,
//! and those of what people built once they have been visited. The corner map has one
//! name, along its foot: the place stood at, or the region.
//!
//! [`build`] is the older still image of a whole continent, for `--map FILE`.

pub mod gpu;
pub mod light;
pub mod marks;
pub mod names;
pub mod picture;
pub mod relief;
pub mod reveal;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
};

use glam::{DVec2, DVec3, Mat4, Vec2, Vec3, Vec4};
use worldgen::{
    continent::Continent,
    macro_layout::MACRO_SIZE,
    names::PlaceKind,
    scale::{OLD_VOXEL, VOXELS_PER_METRE, m, m64},
    world::GameWorld,
};

use crate::gfx::Gpu;
use gpu::{Instance, MapGpu, ReliefGpu, Uniforms};
use marks::{Kind, Mark};
use names::Name;
use relief::{Relief, Request, Season};
use reveal::Reveal;

/// A continent, by macro-cell.
pub type Key = (i32, i32);

/// How near and how far the large map zooms, edge to edge, in voxels.
const LARGE_SPAN: (f64, f64) = (m64(160.0), MACRO_SIZE as f64);
const LARGE_START: f64 = m64(2400.0);
/// How the large map is first looked at: turned a little from north, and from how high.
const YAW: f32 = -0.3;
const PITCH: f32 = 0.86;
const PITCH_RANGE: (f32, f32) = (0.45, 1.45);
/// The large map's field of view, top to bottom.
const FOV: f32 = 0.5;
/// How many pixels wide a column of the large map's model is, about, on a window 720 high.
const COLUMN_PX: f64 = 5.0;
/// How far, in columns, the map's middle may stray from the model's before a new one is made.
const STRAY: i64 = 40;
/// The corner map: how much ground it spans across its middle, how steeply it is looked
/// down on, how far the player may walk from the middle of its model, in columns, and
/// how long it waits, in seconds, before a new one is made for newly seen ground.
const SMALL_SPAN: f64 = m64(130.0);
const SMALL_PITCH: f32 = 0.78;
const SMALL_STRAY: i64 = 16;
const SMALL_EVERY: f32 = 1.5;
/// Which model an answer from the relief thread is.
const LARGE: u8 = 0;
const SMALL: u8 = 1;
/// How far off a place may be for its glow to show at the rim.
const HINT: f64 = m64(2500.0);
/// Most places hinted at the rim at once.
const HINTS: usize = 3;
/// How far the player moves between updates of what they have seen.
const SEE_STEP: f64 = m64(6.0);
/// Seconds before asking again for a neighbouring continent that was not ready.
const RETRY: f32 = 3.0;
/// How near a named place the player must stand for the corner map to name it rather
/// than the region.
const NAMED: f32 = m(30.0);
/// How finely a continent is looked over for where to write its regions' names: this
/// many points each way.
const REGION_GRID: usize = 72;
/// The colours names are written in: land, water, and a region's.
const INK: [u8; 3] = [250, 248, 240];
const WATER_INK: [u8; 3] = [200, 228, 252];
const REGION_INK: [u8; 3] = [255, 246, 222];

/// What the worker makes of one continent.
pub struct Sheet {
    pub picture: Arc<Vec<u8>>,
    pub marks: Vec<Mark>,
    pub continent: Arc<Continent>,
}

/// Builds the sheet for continent `key`.
pub fn sheet(world: &GameWorld, c: Arc<Continent>) -> Sheet {
    Sheet { picture: Arc::new(picture::flat(world, &c)), marks: marks::find(&c, world.seed()), continent: c }
}

struct Known {
    sheet: Sheet,
    /// What kind of place each mark with a name is, and how much it matters on a map.
    places: Vec<Option<(PlaceKind, f32)>>,
    visited: Vec<bool>,
    /// Whether `visited` has been read back from the walked trail in the saved mask.
    checked: bool,
}

enum Answer {
    Sheet(Key, Option<Sheet>),
    Reveal(Key, Option<Reveal>),
    Relief(Box<Relief>),
}

enum Disk {
    Load(Key, PathBuf),
    Save(PathBuf, Vec<u8>),
    /// Answers once everything asked before it is done.
    Flush(Sender<()>),
}

/// Text to draw over the map, in pixels.
pub struct Label {
    pub pos: [f32; 2],
    pub text: String,
    pub size: f32,
    pub color: [u8; 4],
    /// Placed by its left end rather than its middle.
    pub left: bool,
    /// Placed by its right end.
    pub right: bool,
}

/// How the large map is looked at this frame.
#[derive(Clone, Copy)]
struct View {
    /// From voxels east, up and south of the map's middle to the screen.
    transform: Mat4,
    look: DVec2,
    /// The part of the screen it fills: its corner and size, in pixels.
    corner: Vec2,
    screen: Vec2,
    /// Which way east and north run across the screen, a unit each on level ground.
    east: Vec2,
    north: Vec2,
}

impl View {
    /// Where the point `height` voxels up at world `at` falls on the screen, if in front.
    fn px(&self, at: DVec2, height: f32) -> Option<Vec2> {
        let o = at - self.look;
        let p = self.transform * Vec4::new(o.x as f32, height, o.y as f32, 1.0);
        (p.w > 1e-3).then(|| self.corner + Vec2::new((p.x / p.w * 0.5 + 0.5) * self.screen.x, (0.5 - p.y / p.w * 0.5) * self.screen.y))
    }
}

/// A view down onto the ground at `look`, from `yaw` round it (0 is from the south,
/// looking north) and `pitch` above the level, aimed `aim` voxels up, drawn at `per_px`
/// voxels a pixel there, in the rectangle of the screen at `corner`, `size` pixels.
fn look_down(look: DVec2, per_px: f32, yaw: f32, pitch: f32, aim: f32, corner: Vec2, size: Vec2) -> View {
    let dist = size.y * 0.5 * per_px / (FOV * 0.5).tan();
    let (sin, cos) = yaw.sin_cos();
    let target = Vec3::new(0.0, aim, 0.0);
    let eye = target + dist * Vec3::new(sin * pitch.cos(), pitch.sin(), cos * pitch.cos());
    // Written out by hand, as the camera's is: right-handed, looking down -Z, depth 0..1.
    let ahead = (target - eye).normalize();
    let right = ahead.cross(Vec3::Y).normalize();
    let up = right.cross(ahead);
    let looking = Mat4::from_cols(
        Vec4::new(right.x, up.x, -ahead.x, 0.0),
        Vec4::new(right.y, up.y, -ahead.y, 0.0),
        Vec4::new(right.z, up.z, -ahead.z, 0.0),
        Vec4::new(-right.dot(eye), -up.dot(eye), ahead.dot(eye), 1.0),
    );
    let (near, far) = (dist * 0.05, dist * 40.0);
    let t = 1.0 / (FOV * 0.5).tan();
    let lens = Mat4::from_cols(
        Vec4::new(t * size.y / size.x, 0.0, 0.0, 0.0),
        Vec4::new(0.0, t, 0.0, 0.0),
        Vec4::new(0.0, 0.0, far / (near - far), -1.0),
        Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
    );
    View {
        transform: lens * looking,
        look,
        corner,
        screen: size,
        east: Vec2::new(cos, sin * pitch.sin()),
        north: Vec2::new(sin, -cos * pitch.sin()),
    }
}

/// The width of the model's columns, in voxels, for a map drawn at `voxels_per_px`.
fn step_for(voxels_per_px: f64, s: f32) -> i32 {
    let want = (voxels_per_px * COLUMN_PX * s as f64).max(1.0);
    (1 << want.log2().round() as i32).clamp(relief::FINE_STEP, relief::MAX_STEP)
}

/// Where on the screen the map is drawn this frame, and what it shows.
#[derive(Clone, Copy)]
struct Frame {
    centre: Vec2,
    half: Vec2,
    round: bool,
    corner: f32,
    voxels_per_px: f64,
    /// Scale for sizes, from the window height.
    s: f32,
}

impl Frame {
    /// Distance in pixels from the map's edge, negative inside (as the shader has it).
    fn edge(&self, p: Vec2) -> f32 {
        let o = p - self.centre;
        if self.round {
            return o.length() - self.half.x;
        }
        let q = o.abs() - self.half + Vec2::splat(self.corner);
        q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - self.corner
    }

    /// Where a ray from `from` (inside) toward `dir` leaves the map, `inset` pixels in.
    fn rim(&self, from: Vec2, dir: Vec2, inset: f32) -> Vec2 {
        if self.round {
            let o = from - self.centre;
            // Solve |o + t dir| = r for the positive root.
            let r = self.half.x - inset;
            let b = o.dot(dir);
            let t = -b + (b * b - o.length_squared() + r * r).max(0.0).sqrt();
            return from + dir * t;
        }
        let h = self.half - Vec2::splat(inset);
        let lo = self.centre - h;
        let hi = self.centre + h;
        let tx = if dir.x > 0.0 { (hi.x - from.x) / dir.x } else if dir.x < 0.0 { (lo.x - from.x) / dir.x } else { f32::INFINITY };
        let tz = if dir.y > 0.0 { (hi.y - from.y) / dir.y } else if dir.y < 0.0 { (lo.y - from.y) / dir.y } else { f32::INFINITY };
        from + dir * tx.min(tz).max(0.0)
    }
}

fn origin(key: Key) -> DVec2 {
    DVec2::new(key.0 as f64, key.1 as f64) * MACRO_SIZE as f64
}

fn key_of(at: DVec2) -> Key {
    ((at.x / MACRO_SIZE as f64).floor() as i32, (at.y / MACRO_SIZE as f64).floor() as i32)
}

/// The two by two continents nearest `at`: together they hold everything within half a
/// continent of it.
fn block_of(at: DVec2) -> [Key; 4] {
    let (bx, bz) = ((at.x / MACRO_SIZE as f64 - 0.5).floor() as i32, (at.y / MACRO_SIZE as f64 - 0.5).floor() as i32);
    [(bx, bz), (bx + 1, bz), (bx, bz + 1), (bx + 1, bz + 1)]
}

fn xz(v: DVec3) -> DVec2 {
    DVec2::new(v.x, v.z)
}

pub struct Minimap {
    world: Arc<GameWorld>,
    sheets_ask: Sender<(Key, bool)>,
    disk: Sender<Disk>,
    answers: Receiver<Answer>,
    pending: HashSet<Key>,
    asked: HashMap<Key, f32>,
    known: HashMap<Key, Known>,
    reveals: HashMap<Key, Reveal>,
    /// Masks being read from disk, with what was seen meanwhile, to add once they are in.
    loading: HashMap<Key, Vec<(Vec2, f32, bool)>>,
    /// Where this world's masks are saved, if anywhere.
    dir: Option<PathBuf>,
    last_seen: Option<DVec3>,
    /// The corner map is shown.
    pub corner: bool,
    /// The large map is open.
    pub open: bool,
    /// The large map's middle, in world voxels, and its width.
    look: DVec2,
    span: f64,
    /// Its width on the screen last frame, in pixels, and the scale of its sizes.
    side: f32,
    scale: f32,
    /// How the large map is turned and tilted, in radians.
    yaw: f32,
    pitch: f32,
    /// The height of the ground the large map looks at, in voxels, once it is known.
    aim: Option<f32>,
    /// The time of day to show on the large map, in hours.
    pub hour: f32,
    /// Where the player has camped, in world voxels: each is marked.
    pub camps: Vec<DVec2>,
    /// The time of year the models are made for, once it is known.
    season: Option<Season>,
    /// The large map's model, the one being made, and a count of what has changed the
    /// land shown since the game began.
    relief: Option<Relief>,
    relief_ask: Sender<Request>,
    relief_pending: bool,
    version: u64,
    relief_gpu: ReliefGpu,
    /// The corner map's model, as the large map's.
    small: Option<Relief>,
    small_pending: bool,
    small_asked: f32,
    small_aim: Option<f32>,
    small_gpu: ReliefGpu,
    gpu: MapGpu,
    player: DVec3,
    heading: Vec2,
    region: Option<String>,
    /// The named place the player stands at, if any.
    near: Option<String>,
    region_timer: f32,
    /// Where each region's name is written on each continent, in world voxels (with the
    /// place it is in the gazetteer), and what had been seen when that was worked out.
    anchors: HashMap<Key, (u64, Vec<(usize, DVec2)>)>,
    clock: f32,
    labels: Vec<Label>,
}

impl Minimap {
    /// `dir` is where this world's masks live; None keeps nothing.
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, world: Arc<GameWorld>, dir: Option<PathBuf>) -> Self {
        let (reply, answers) = channel();
        let (sheets_ask, sheets) = channel::<(Key, bool)>();
        let (w, to_main) = (world.clone(), reply.clone());
        std::thread::Builder::new()
            .name("map".into())
            .spawn(move || {
                crate::threads::background();
                while let Ok((key, wait)) = sheets.recv() {
                    let c = if wait { Some(w.continent(key.0, key.1)) } else { w.continent_if_ready(key.0, key.1) };
                    let t = std::time::Instant::now();
                    let made = c.map(|c| sheet(&w, c));
                    if let Some(s) = &made {
                        log::info!("map of continent {key:?}: {} places, made in {:.2} s", s.marks.len(), t.elapsed().as_secs_f32());
                    }
                    if to_main.send(Answer::Sheet(key, made)).is_err() {
                        break;
                    }
                }
            })
            .expect("map thread");
        let (relief_ask, reliefs) = channel::<Request>();
        let (w, to_main) = (world.clone(), reply.clone());
        std::thread::Builder::new()
            .name("map relief".into())
            .spawn(move || {
                crate::threads::background();
                while let Ok(req) = reliefs.recv() {
                    let t = std::time::Instant::now();
                    let made = relief::build(&w, &req);
                    log::debug!(
                        "map model at step {}: {} corners, {} trees, {} buildings and tors, made in {:.2} s",
                        made.step,
                        made.vertices.len(),
                        made.trees.len(),
                        made.models,
                        t.elapsed().as_secs_f32()
                    );
                    if to_main.send(Answer::Relief(Box::new(made))).is_err() {
                        break;
                    }
                }
            })
            .expect("map relief thread");
        let (disk, jobs) = channel::<Disk>();
        std::thread::Builder::new()
            .name("map disk".into())
            .spawn(move || {
                crate::threads::background();
                while let Ok(job) = jobs.recv() {
                    match job {
                        Disk::Load(key, path) => {
                            if reply.send(Answer::Reveal(key, Reveal::load(&path))).is_err() {
                                break;
                            }
                        }
                        Disk::Save(path, data) => {
                            reveal::write(&path, &data);
                        }
                        Disk::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .expect("map disk thread");
        Self {
            world,
            sheets_ask,
            disk,
            answers,
            pending: HashSet::new(),
            asked: HashMap::new(),
            known: HashMap::new(),
            reveals: HashMap::new(),
            loading: HashMap::new(),
            dir,
            last_seen: None,
            corner: true,
            open: false,
            look: DVec2::ZERO,
            span: LARGE_START,
            side: 600.0,
            scale: 1.0,
            yaw: YAW,
            pitch: PITCH,
            aim: None,
            hour: 12.0,
            camps: Vec::new(),
            season: None,
            relief: None,
            relief_ask,
            relief_pending: false,
            version: 0,
            relief_gpu: ReliefGpu::new(gpu, format),
            small: None,
            small_pending: false,
            small_asked: -1.0e9,
            small_aim: None,
            small_gpu: ReliefGpu::new(gpu, format),
            gpu: MapGpu::new(gpu, format),
            player: DVec3::ZERO,
            heading: Vec2::X,
            region: None,
            near: None,
            region_timer: 0.0,
            anchors: HashMap::new(),
            clock: 0.0,
            labels: Vec::new(),
        }
    }

    /// Tells the map the time of year (0 to 1 from the spring equinox) and what the
    /// ground carries in each biome, so that its woods turn and its hills whiten with
    /// the world's. The models are made again when either has changed enough to see.
    pub fn set_season(&mut self, date: f32, places: crate::season::Places) {
        let moved = self.season.is_none_or(|s| {
            let snow = |p: &crate::season::Places| p.biomes.iter().map(|g| g.snow + g.highland).sum::<f32>();
            (s.date - date).abs() > 0.008 || (snow(&s.places) - snow(&places)).abs() > 0.5
        });
        if moved {
            self.season = Some(Season { date, places });
            self.version += 1;
        }
    }

    /// Opens or closes the large map, opening on the player.
    pub fn set_open(&mut self, open: bool) {
        if open && !self.open {
            self.look = xz(self.player);
            self.aim = None;
            // The regions' names are placed as soon as it is open.
            self.region_timer = 0.0;
        }
        self.open = open;
    }

    /// Zooms the large map in by `lines` of the mouse wheel.
    pub fn zoom(&mut self, lines: f32) {
        self.span = (self.span * 0.85f64.powf(lines as f64)).clamp(LARGE_SPAN.0, LARGE_SPAN.1);
    }

    /// Sets how much the large map spans, edge to edge, in voxels.
    pub fn set_span(&mut self, span: f64) {
        self.span = span.clamp(LARGE_SPAN.0, LARGE_SPAN.1);
    }

    /// Moves the large map with the mouse, by pixels: the ground follows the cursor.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        let per_px = self.span / self.side.max(1.0) as f64;
        let (sin, cos) = (self.yaw.sin() as f64, self.yaw.cos() as f64);
        let (right, ahead) = (DVec2::new(cos, -sin), DVec2::new(-sin, -cos));
        // The ground is seen at a slant, so a pixel up the screen is further over it.
        self.look += (ahead * dy as f64 / (self.pitch.sin() as f64) - right * dx as f64) * per_px;
    }

    /// Turns and tilts the large map with the mouse, by pixels.
    pub fn turn(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw - dx * 0.006).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + dy * 0.005).clamp(PITCH_RANGE.0, PITCH_RANGE.1);
    }

    /// Turns and tilts the large map to `yaw` and `pitch`, in radians, as right drags
    /// would, for screenshots.
    pub fn set_turn(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw.rem_euclid(std::f32::consts::TAU);
        self.pitch = pitch.clamp(PITCH_RANGE.0, PITCH_RANGE.1);
    }

    /// Moves the large map's middle by `by` world voxels east and south, as left drags
    /// would, for screenshots.
    pub fn shift(&mut self, by: DVec2) {
        self.look += by;
        self.aim = None;
    }

    /// How the large map is looked at on a screen `w` by `h`.
    fn view(&self, w: f32, h: f32) -> View {
        let side = (w.min(h) * 0.86).floor();
        let per_px = (self.span / side as f64) as f32;
        look_down(self.look, per_px, self.yaw, self.pitch, self.aim.unwrap_or(0.0), Vec2::ZERO, Vec2::new(w, h))
    }

    /// Asks the relief thread for the model `tag`, `n` columns of `step` voxels each way
    /// about `centre` (in columns), of what has been seen. False if it cannot be asked
    /// for yet.
    fn ask_relief(&mut self, tag: u8, n: usize, step: i32, centre: (i64, i64)) -> bool {
        let origin = ((centre.0 - n as i64 / 2) as i32, (centre.1 - n as i64 / 2) as i32);
        let corner = |c: usize, o: i32| (o as f64 + c as f64 + 0.5) * step as f64;
        let (lo, hi) = (key_of(DVec2::new(corner(0, origin.0), corner(0, origin.1))), key_of(DVec2::new(corner(n - 1, origin.0), corner(n - 1, origin.1))));
        let mut sheets = Vec::new();
        for kz in lo.1..=hi.1 {
            for kx in lo.0..=hi.0 {
                if let Some(k) = self.known.get(&(kx, kz)) {
                    sheets.push(((kx, kz), k.sheet.continent.clone(), k.sheet.picture.clone()));
                }
            }
        }
        if sheets.is_empty() {
            return false;
        }
        let mut mask = vec![[0u8; 2]; n * n];
        for z in 0..n {
            for x in 0..n {
                let at = DVec2::new(corner(x, origin.0), corner(z, origin.1));
                let key = key_of(at);
                if let Some(r) = self.reveals.get(&key) {
                    mask[z * n + x] = r.at((at - self::origin(key)).as_vec2());
                }
            }
        }
        // The large map does not show what a landmark is until it has been come to: it
        // could be zoomed in on and made out from far off. The corner map shows only the
        // ground about the player, where it stands in plain sight.
        let mut hidden = Vec::new();
        if tag == LARGE {
            for k in self.known.values() {
                for (mark, _) in k.sheet.marks.iter().zip(&k.visited).filter(|(m, v)| m.kind == Kind::Landmark && !**v) {
                    hidden.push(glam::IVec2::new(mark.at.x.floor() as i32, mark.at.y.floor() as i32));
                }
            }
        }
        self.relief_ask.send(Request { tag, n, step, origin, mask, sheets, version: self.version, season: self.season, hidden }).is_ok()
    }

    /// Asks for a new model for the large map if the one it has no longer fits: the map
    /// has moved or zoomed too far from it, or more of the land has been seen.
    fn want_relief(&mut self) {
        if !self.open || self.relief_pending {
            return;
        }
        let step = step_for(self.span / self.side.max(1.0) as f64, self.scale);
        let centre = ((self.look.x / step as f64).round() as i64, (self.look.y / step as f64).round() as i64);
        let n = relief::N as i64;
        if let Some(r) = &self.relief {
            let mid = (r.origin.0 as i64 + n / 2, r.origin.1 as i64 + n / 2);
            if r.step == step && r.version == self.version && (centre.0 - mid.0).abs() <= STRAY && (centre.1 - mid.1).abs() <= STRAY {
                return;
            }
        }
        self.relief_pending = self.ask_relief(LARGE, relief::N, step, centre);
    }

    /// Asks for a new model for the corner map once the player has walked too far from
    /// the middle of the one it has, or has seen more ground, though not too often.
    fn want_small(&mut self) {
        if self.open || !self.corner || self.small_pending {
            return;
        }
        let step = relief::FINE_STEP;
        let centre = ((self.player.x / step as f64).round() as i64, (self.player.z / step as f64).round() as i64);
        let n = relief::SMALL as i64;
        if let Some(r) = &self.small {
            let mid = (r.origin.0 as i64 + n / 2, r.origin.1 as i64 + n / 2);
            let near = (centre.0 - mid.0).abs() <= SMALL_STRAY && (centre.1 - mid.1).abs() <= SMALL_STRAY;
            if near && (r.version == self.version || self.clock - self.small_asked < SMALL_EVERY) {
                return;
            }
        }
        self.small_pending = self.ask_relief(SMALL, relief::SMALL, step, centre);
        if self.small_pending {
            self.small_asked = self.clock;
        }
    }

    fn path(&self, key: Key) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(format!("{}_{}.png", key.0, key.1)))
    }

    fn want(&mut self, key: Key, wait: bool) {
        if self.known.contains_key(&key) || self.pending.contains(&key) {
            return;
        }
        if self.asked.get(&key).is_some_and(|&t| self.clock - t < RETRY) {
            return;
        }
        if self.sheets_ask.send((key, wait)).is_ok() {
            self.pending.insert(key);
            self.asked.insert(key, self.clock);
        }
    }

    /// The mask of continent `key`, or None while it is read from disk.
    fn reveal_mut(&mut self, key: Key) -> Option<&mut Reveal> {
        if !self.reveals.contains_key(&key) {
            if self.loading.contains_key(&key) {
                return None;
            }
            match self.path(key) {
                Some(path) if path.exists() && self.disk.send(Disk::Load(key, path.clone())).is_ok() => {
                    self.loading.insert(key, Vec::new());
                    return None;
                }
                _ => {
                    self.reveals.insert(key, Reveal::new());
                }
            }
        }
        self.reveals.get_mut(&key)
    }

    fn take(&mut self, gpu: &Gpu, answer: Answer) {
        match answer {
            Answer::Sheet(key, sheet) => {
                self.pending.remove(&key);
                if let Some(sheet) = sheet {
                    let n = sheet.marks.len();
                    let gazetteer = &sheet.continent.trails.names;
                    let places = sheet.marks.iter().map(|m| gazetteer.named(m.name.as_deref()?).map(|p| (p.kind, p.importance))).collect();
                    self.known.insert(key, Known { sheet, places, visited: vec![false; n], checked: false });
                    self.reveal_mut(key);
                    self.check_visits(key);
                    self.version += 1;
                }
            }
            Answer::Relief(mut r) => {
                let (model, pending, on_gpu) = match r.tag {
                    LARGE => (&mut self.relief, &mut self.relief_pending, &mut self.relief_gpu),
                    _ => (&mut self.small, &mut self.small_pending, &mut self.small_gpu),
                };
                *pending = false;
                on_gpu.upload(gpu, &r.vertices, &r.indices);
                r.vertices = Vec::new();
                r.indices = Vec::new();
                *model = Some(*r);
            }
            Answer::Reveal(key, loaded) => {
                let mut r = loaded.unwrap_or_default();
                for (at, radius, walked) in self.loading.remove(&key).unwrap_or_default() {
                    r.see(at, radius, walked);
                }
                self.reveals.insert(key, r);
                self.check_visits(key);
                self.version += 1;
            }
        }
    }

    /// Reads which of a continent's places were visited from the trail in its mask.
    fn check_visits(&mut self, key: Key) {
        let (Some(k), Some(r)) = (self.known.get_mut(&key), self.reveals.get(&key)) else { return };
        if k.checked {
            return;
        }
        k.checked = true;
        let o = origin(key);
        for (mark, seen) in k.sheet.marks.iter().zip(&mut k.visited) {
            *seen |= r.walked_near((mark.at - o).as_vec2(), mark.reach);
        }
    }

    /// Keeps the map up to date with where the player is: `eye` the camera, `forward`
    /// its direction, `walked` unless flying free.
    pub fn update(&mut self, gpu: &Gpu, eye: DVec3, forward: Vec3, walked: bool, dt: f32) {
        self.clock += dt;
        self.player = eye;
        let h = Vec2::new(forward.x, forward.z);
        if h.length_squared() > 1e-6 {
            self.heading = h.normalize();
        }
        while let Ok(a) = self.answers.try_recv() {
            self.take(gpu, a);
        }
        let here = key_of(xz(eye));
        self.want(here, true);
        let look = if self.open { self.look } else { xz(eye) };
        let block = block_of(look);
        for k in block {
            self.want(k, false);
        }
        if self.last_seen.is_none_or(|p| p.distance(eye) >= SEE_STEP)
            && let Some(c) = self.known.get(&here).map(|k| k.sheet.continent.clone())
        {
            self.see(&c, eye, walked);
        }
        self.want_relief();
        self.want_small();
        // Each map looks at the ground under its middle, coming to it gently, or walking
        // over the blocks of the corner map's model would jolt it.
        if let Some(r) = &self.small {
            let ground = r.top_at(self.player.x, self.player.z).unwrap_or(r.level * r.step as f32);
            self.small_aim = Some(match self.small_aim {
                Some(aim) if dt > 0.0 => aim + (ground - aim) * (1.0 - (-dt * 4.0).exp()),
                _ => ground,
            });
        }
        // The large map looks at the ground under its middle, coming to it gently as it
        // is moved over hills.
        if let Some(r) = self.relief.as_ref().filter(|_| self.open) {
            let ground = r.top_at(self.look.x, self.look.y).unwrap_or(r.level * r.step as f32);
            self.aim = Some(match self.aim {
                Some(aim) if dt > 0.0 => aim + (ground - aim) * (1.0 - (-dt * 6.0).exp()),
                _ => ground,
            });
        }
        self.region_timer -= dt;
        if self.region_timer <= 0.0
            && let Some(k) = self.known.get(&here)
        {
            let names = &k.sheet.continent.trails.names;
            let at = (xz(eye) - origin(here)).as_vec2();
            self.region = names.region_at(at).and_then(|i| names.places.get(i)).map(|p| p.name.clone());
            // The place stood at: the smallest that reaches here, a thing people built
            // only once it has been visited (but a village's name is known to all). Islands
            // and ranges are too large to stand at.
            let visited = |name: &str| k.sheet.marks.iter().zip(&k.visited).any(|(m, &v)| v && m.name.as_deref() == Some(name));
            self.near = names
                .places
                .iter()
                .filter(|p| matches!(p.kind, PlaceKind::Lake | PlaceKind::River | PlaceKind::Peak | PlaceKind::Falls | PlaceKind::Tor | PlaceKind::Landmark(_)) || p.kind.is_settlement())
                .filter(|p| p.distance(at) <= NAMED && (p.kind.is_natural() || p.kind.is_settlement() || visited(&p.name)))
                .min_by(|a, b| a.radius.total_cmp(&b.radius))
                .map(|p| p.name.clone());
            self.region_timer = 1.0;
            if self.open {
                self.anchor_regions();
            }
        }
    }

    /// Works out where each region's name goes on each continent: in the middle of what
    /// has been seen of its land, so that a region is named as soon as some of it is on
    /// the map, and its name moves in as more of it is. Done again only when more has
    /// been seen.
    fn anchor_regions(&mut self) {
        for (key, k) in &self.known {
            let Some(r) = self.reveals.get(key) else { continue };
            if self.anchors.get(key).is_some_and(|a| a.0 == self.version) {
                continue;
            }
            let c = &k.sheet.continent;
            let names = &c.trails.names;
            let cell = MACRO_SIZE as f32 / REGION_GRID as f32;
            let mut seen: Vec<(usize, Vec2)> = Vec::new();
            for gz in 0..REGION_GRID {
                for gx in 0..REGION_GRID {
                    let at = Vec2::new(gx as f32 + 0.5, gz as f32 + 0.5) * cell;
                    let grid = |v: f32| ((v / c.cell_size()) as usize).min(c.size - 1);
                    if r.seen(at)
                        && !c.ocean[grid(at.y) * c.size + grid(at.x)]
                        && let Some(i) = names.region_at(at)
                    {
                        seen.push((i, at));
                    }
                }
            }
            let mut middles: HashMap<usize, (Vec2, f32)> = HashMap::new();
            for &(i, at) in &seen {
                let m = middles.entry(i).or_insert((Vec2::ZERO, 0.0));
                *m = (m.0 + at, m.1 + 1.0);
            }
            let o = origin(*key);
            let mut anchors: Vec<(usize, DVec2)> = middles
                .into_iter()
                .filter_map(|(i, (sum, count))| {
                    let middle = sum / count;
                    let nearest = seen.iter().filter(|s| s.0 == i).min_by(|a, b| a.1.distance(middle).total_cmp(&b.1.distance(middle)))?;
                    Some((i, o + nearest.1.as_dvec2()))
                })
                .collect();
            anchors.sort_by_key(|a| a.0);
            self.anchors.insert(*key, (self.version, anchors));
        }
    }

    /// Records what can be seen from `eye`, on continent `c` (the one it stands over).
    fn see(&mut self, c: &Continent, eye: DVec3, walked: bool) {
        let here = (c.desc.mx, c.desc.mz);
        let at = xz(eye);
        let radius = reveal::sight(c, (at - origin(here)).as_vec2(), eye.y as f32);
        self.last_seen = Some(eye);
        let (lo, hi) = (key_of(at - radius as f64), key_of(at + radius as f64));
        let mut changed = false;
        for kz in lo.1..=hi.1 {
            for kx in lo.0..=hi.0 {
                let key = (kx, kz);
                let local = (at - origin(key)).as_vec2();
                match self.reveal_mut(key) {
                    Some(r) => {
                        let before = r.changes;
                        r.see(local, radius, walked);
                        changed |= r.changes != before;
                    }
                    None => {
                        if let Some(q) = self.loading.get_mut(&key) {
                            q.push((local, radius, walked));
                        }
                    }
                }
            }
        }
        if changed {
            self.version += 1;
        }
        if walked {
            let mut found = false;
            for k in self.known.values_mut() {
                for (mark, seen) in k.sheet.marks.iter().zip(&mut k.visited) {
                    if !*seen && mark.at.distance(at) <= mark.reach as f64 {
                        *seen = true;
                        found = true;
                    }
                }
            }
            // A landmark come to is shown on the large map's model from now on.
            if found {
                self.version += 1;
            }
        }
    }

    /// Writes what has changed of the masks on the disk thread; if `wait` (when the game
    /// is closing), waits until that and any earlier save are done.
    pub fn save(&mut self, wait: bool) {
        let Some(dir) = self.dir.clone() else { return };
        for (key, r) in &mut self.reveals {
            if r.unsaved {
                let path = dir.join(format!("{}_{}.png", key.0, key.1));
                let _ = self.disk.send(Disk::Save(path, r.data.clone()));
                r.unsaved = false;
            }
        }
        if wait {
            let (done, finished) = channel();
            if self.disk.send(Disk::Flush(done)).is_ok() {
                let _ = finished.recv_timeout(std::time::Duration::from_secs(10));
            }
        }
    }

    /// Waits until everything the map needs around `eye` has arrived, for screenshots.
    /// If nothing arrives for [`crate::terrain::wait_limit`], says what it was waiting
    /// for and exits the process with a failure, rather than hang or go on without it.
    pub fn settle(&mut self, gpu: &Gpu, eye: DVec3, forward: Vec3) {
        let (began, limit) = (std::time::Instant::now(), crate::terrain::wait_limit());
        let mut progressed = began;
        let mut was = None;
        log::info!("stage: waiting for the map");
        loop {
            self.update(gpu, eye, forward, false, 0.0);
            if self.pending.is_empty() && self.loading.is_empty() && !self.relief_pending && !self.small_pending {
                break;
            }
            // `update` takes what has come in too: anything changing is progress.
            let now = Some((self.pending.len(), self.loading.len(), self.relief_pending, self.small_pending, self.version));
            if now != was {
                (was, progressed) = (now, std::time::Instant::now());
            }
            match self.answers.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(a) => {
                    self.take(gpu, a);
                    progressed = std::time::Instant::now();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if progressed.elapsed() <= limit => {}
                Err(e) => {
                    log::error!(
                        "hung waiting for the map ({e:?}) after {:.0} s: sheets {:?}, masks loading {:?}, relief {}, corner relief {}",
                        began.elapsed().as_secs_f32(),
                        self.pending,
                        self.loading.keys().collect::<Vec<_>>(),
                        self.relief_pending,
                        self.small_pending
                    );
                    std::process::exit(3);
                }
            }
            // Whatever comes in is taken by `update` too, so a pause keeps this from
            // spinning should answers keep coming and never finish it.
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        log::info!("stage: map ready after {:.2} s", began.elapsed().as_secs_f32());
        self.last_seen = None;
        self.update(gpu, eye, forward, false, 0.0);
    }

    /// As if the player had walked along `path` (world x, z), for screenshots.
    pub fn walk(&mut self, path: &[DVec2]) {
        for pair in path.windows(2) {
            let steps = (pair[0].distance(pair[1]) / SEE_STEP).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let at = pair[0].lerp(pair[1], k as f64 / steps as f64);
                let key = key_of(at);
                let c = self.world.continent(key.0, key.1);
                let local = (at - origin(key)).as_vec2();
                let (cs, n) = (c.cell_size(), c.size);
                let cell = |v: f32| ((v / cs) as usize).min(n - 1);
                let ground = (c.height[cell(local.y) * n + cell(local.x)] * OLD_VOXEL).max(0.0) as f64;
                self.see(&c, DVec3::new(at.x, ground + crate::player::EYE_HEIGHT, at.y), true);
            }
        }
        self.last_seen = None;
    }

    /// The labels drawn with the map last frame.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Where on the screen the corner map is.
    fn frame(&self, w: f32, h: f32) -> Frame {
        let s = (h / 720.0).max(0.75);
        let d = (h * 0.27).clamp(140.0, 360.0).floor();
        let margin = (h * 0.035).max(12.0);
        Frame {
            centre: Vec2::new(w - margin - d * 0.5, margin + d * 0.5),
            half: Vec2::splat(d * 0.5),
            round: false,
            corner: 9.0 * s,
            voxels_per_px: SMALL_SPAN / d as f64,
            s,
        }
    }

    /// Draws the map over `target`, `size` pixels.
    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32)) {
        self.labels.clear();
        if !self.open && !self.corner {
            return;
        }
        let (w, h) = (size.0 as f32, size.1 as f32);
        if self.open {
            self.draw_large(gpu, encoder, target, size);
            return;
        }
        let f = self.frame(w, h);
        let u = Uniforms {
            screen: [w, h],
            centre: f.centre.into(),
            half: f.half.into(),
            voxels_per_px: f.voxels_per_px as f32,
            time: self.clock,
            corner: f.corner,
            ..Default::default()
        };
        // The way the player faces is up: the map is looked at from behind them.
        let yaw = (-self.heading.x).atan2(-self.heading.y);
        let inset = Vec2::splat(2.0);
        let (corner, side) = (f.centre - f.half + inset, (f.half - inset) * 2.0);
        let view = look_down(xz(self.player), f.voxels_per_px as f32, yaw, SMALL_PITCH, self.small_aim.unwrap_or(0.0), corner, side);
        let mut out = Vec::with_capacity(64);
        let mut panel = Instance::new(gpu::PANEL, f.centre.into(), 0.0, [0.075, 0.095, 0.125, 0.78]);
        panel.size = (f.half + Vec2::splat(12.0)).into();
        out.push(panel);

        let model = self.small.as_ref();
        let level = self.small_aim.unwrap_or(0.0);
        let place = |at: DVec2| match model.and_then(|r| Some((r.top_at(at.x, at.y)?, r.step))) {
            Some((top, step)) => view.px(at, top + step as f32).map(|p| (p, true)),
            None => view.px(at, level).map(|p| (p, false)),
        };
        let mut labels = Vec::new();
        let models = model.is_some_and(|r| r.step <= relief::MODEL_STEP);
        self.marks(&f, &place, Some((view.east, view.north)), models, &mut out, None);
        // Where this is: the place stood at, or the region, small along the foot.
        if let Some(name) = self.near.as_ref().or(self.region.as_ref()) {
            let size = (12.5 * f.s).min(f.half.x * 1.8 / names::width(name, 1.0).max(1.0));
            labels.push(Label { pos: [f.centre.x, f.centre.y + f.half.y - 10.0 * f.s], text: name.clone(), size, color: [INK[0], INK[1], INK[2], 235], left: false, right: false });
        }

        // The player, always facing up the map, and where north has got to.
        if let Some((me, _)) = place(xz(self.player)) {
            let mut arrow = Instance::new(gpu::ARROW, me.into(), 7.5 * f.s, [0.99, 0.98, 0.95, 1.0]);
            arrow.dir = [0.0, -1.0];
            out.push(arrow);
        }
        // Clear of the name along the foot.
        let mut north = f.rim(f.centre, view.north.normalize_or_zero(), 10.0 * f.s);
        north.y = north.y.min(f.centre.y + f.half.y - 26.0 * f.s);
        labels.push(Label { pos: north.into(), text: "N".into(), size: 12.0 * f.s, color: [250, 248, 240, 235], left: false, right: false });

        let transform = model.map(|r| {
            let from = (DVec2::new(r.origin.0 as f64, r.origin.1 as f64) * r.step as f64 - xz(self.player)).as_vec2();
            view.transform * Mat4::from_translation(Vec3::new(from.x, 0.0, from.y)) * Mat4::from_scale(Vec3::splat(r.step as f32))
        });
        self.labels = labels;
        self.gpu.load(gpu, &u, &out);
        self.gpu.pass(encoder, target, 0..1);
        if let Some(transform) = transform {
            let within = [corner.x.round() as u32, corner.y.round() as u32, side.x.round() as u32, side.y.round() as u32];
            self.small_gpu.draw(gpu, encoder, target, size, transform, Some(within), self.hour, self.clock);
        }
        self.gpu.pass(encoder, target, 1..out.len() as u32);
    }

    /// Draws the large map: the land seen so far as a model looked down on at a slant,
    /// with the places on it, the player, a compass and where and when this is.
    fn draw_large(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32)) {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let s = (h / 720.0).max(0.75);
        self.side = (w.min(h) * 0.86).floor();
        self.scale = s;
        let view = self.view(w, h);
        let screen = Vec2::new(w, h);
        // The whole screen is the map: marks keep a little in from its edge.
        let f = Frame {
            centre: screen * 0.5,
            half: screen * 0.5 - Vec2::splat(8.0 * s),
            round: false,
            corner: 0.0,
            voxels_per_px: self.span / self.side as f64,
            s,
        };
        let u = Uniforms {
            screen: [w, h],
            centre: f.centre.into(),
            half: (screen * 0.5 + Vec2::splat(4.0)).into(),
            voxels_per_px: f.voxels_per_px as f32,
            time: self.clock,
            ..Default::default()
        };
        let mut out = Vec::with_capacity(64);
        let mut veil = Instance::new(gpu::VEIL, f.centre.into(), 0.0, [0.075, 0.095, 0.125, 0.9]);
        veil.size = [w * 0.5 + 1.0, h * 0.5 + 1.0];
        out.push(veil);

        let relief = self.relief.as_ref();
        let level = relief.map_or(0.0, |r| r.level * r.step as f32);
        // A place stands on its column of the model, a block clear of the ground; one the
        // model does not show is taken to lie at the model's middle height.
        let place = |at: DVec2| match relief.and_then(|r| Some((r.top_at(at.x, at.y)?, r.step))) {
            Some((top, step)) => view.px(at, top + step as f32).map(|p| (p, true)),
            None => view.px(at, level).map(|p| (p, false)),
        };
        let mut labels = Vec::new();
        let models = relief.is_some_and(|r| r.step <= relief::MODEL_STEP);
        let mut named: Vec<Name> = Vec::new();
        self.marks(&f, &place, None, models, &mut out, Some(&mut named));
        self.names(&f, &place, &mut named);

        let white = [250, 248, 240, 255];
        let me = xz(self.player);
        if let Some((p, _)) = place(me)
            && f.edge(p) < -2.0
        {
            let ahead = place(me + self.heading.as_dvec2() * relief.map_or(64.0, |r| r.step as f64 * 3.0));
            let dir = ahead.map_or(Vec2::ZERO, |(q, _)| (q - p).normalize_or_zero());
            let mut arrow = Instance::new(gpu::ARROW, p.into(), 10.0 * s, [0.99, 0.98, 0.95, 1.0]);
            arrow.dir = (if dir == Vec2::ZERO { self.heading } else { dir }).into();
            out.push(arrow);
        }

        // The compass, lying on the ground as the map does.
        let rose = Vec2::new(w - 56.0 * s, 58.0 * s);
        let reach = 26.0 * s;
        for (axis, letters) in [(view.north, ["N", "S"]), (view.east, ["E", "W"])] {
            let mut bar = Instance::new(gpu::LINE, rose.into(), 0.0, [0.98, 0.97, 0.94, 0.95]);
            bar.size = [axis.length() * reach, 1.6 * s];
            bar.dir = axis.normalize_or_zero().into();
            out.push(bar);
            for (letter, side) in letters.into_iter().zip([1.0, -1.0]) {
                let tip = rose + axis * side * (reach + 10.0 * s);
                let size = if letter == "N" { 15.0 } else { 11.0 } * s;
                labels.push(Label { pos: tip.into(), text: letter.into(), size, color: white, left: false, right: false });
            }
        }

        if let Some(region) = &self.region {
            labels.push(Label { pos: [w - 112.0 * s, 44.0 * s], text: region.clone(), size: 30.0 * s, color: white, left: false, right: true });
        }
        let metres = |v: f64| v / worldgen::scale::VOXELS_PER_METRE as f64;
        let hour = self.hour.rem_euclid(24.0);
        let zoom = relief.map_or(1, |r| r.step / relief::FINE_STEP);
        // The names give way to what else is written on the map, and to the player.
        let mut taken: Vec<(Vec2, Vec2)> = Vec::new();
        let about = |mid: Vec2, half: Vec2| (mid - half, mid + half);
        taken.push(about(rose, Vec2::splat(reach + 22.0 * s)));
        if let Some(region) = &self.region {
            let wide = names::width(region, 30.0 * s);
            taken.push((Vec2::new(w - 112.0 * s - wide, 22.0 * s), Vec2::new(w - 112.0 * s, 66.0 * s)));
        }
        taken.push((Vec2::new(0.0, 0.0), Vec2::new(360.0 * s, 34.0 * s)));
        taken.push(about(Vec2::new(w * 0.5, h - 20.0 * s), Vec2::new(300.0 * s, 12.0 * s)));
        if let Some((p, _)) = place(me) {
            taken.push(about(p, Vec2::splat(9.0 * s)));
        }
        let edge = 6.0 * s;
        for n in names::declutter(named, (Vec2::splat(edge), screen - Vec2::splat(edge)), &taken, 5.0 * s) {
            labels.push(Label { pos: n.at.into(), text: n.text, size: n.size, color: n.color, left: false, right: false });
        }
        labels.push(Label {
            pos: [16.0 * s, 20.0 * s],
            text: format!(
                "X {:.0}   Z {:.0}      {}:{:02}      1:{zoom}",
                metres(self.player.x),
                metres(self.player.z),
                hour as u32,
                (hour.fract() * 60.0) as u32
            ),
            size: 14.0 * s,
            color: white,
            left: true,
            right: false,
        });
        labels.push(Label {
            pos: [w * 0.5, h - 20.0 * s],
            text: "M or Esc to close      wheel to zoom      drag to move      right drag to turn".into(),
            size: 12.0 * s,
            color: [225, 222, 214, 200],
            left: false,
            right: false,
        });

        let transform = relief.map(|r| {
            let step = r.step as f64;
            let corner = (DVec2::new(r.origin.0 as f64, r.origin.1 as f64) * step - self.look).as_vec2();
            view.transform * Mat4::from_translation(Vec3::new(corner.x, 0.0, corner.y)) * Mat4::from_scale(Vec3::splat(r.step as f32))
        });
        self.labels = labels;
        self.gpu.load(gpu, &u, &out);
        self.gpu.pass(encoder, target, 0..1);
        if let Some(transform) = transform {
            self.relief_gpu.draw(gpu, encoder, target, size, transform, None, self.hour, self.clock);
        }
        self.gpu.pass(encoder, target, 1..out.len() as u32);
    }

    /// The places on the map: icons for those visited, nothing for those seen but not
    /// visited, and at the rim, the nearest few still out of sight. `place` gives
    /// where a point of the world falls on the screen, and whether the map shows it;
    /// `rim`, the ways east and north run across the screen, for the hints at the rim,
    /// or None for none.
    ///
    /// The large map has no hints at its rim. It fills the screen, so its rim is the
    /// screen's edge, and the places hinted at, a few kilometres off, mostly lie within
    /// it on ground not yet seen: a glow at the edge of the screen pointing at the middle
    /// of it, wandering round the edge as the map was turned, read as something wrong.
    ///
    /// With `models`, the map's model has the buildings, bridges and tors standing on it
    /// as themselves, and those have no icon laid over them. `named` takes the names of
    /// the marked places that have one to show: any the land shows that is the land's
    /// own (a lake, a peak, a tor, a fall), and what people built once it is visited.
    #[allow(clippy::too_many_arguments)]
    fn marks(
        &self,
        f: &Frame,
        place: &dyn Fn(DVec2) -> Option<(Vec2, bool)>,
        rim: Option<(Vec2, Vec2)>,
        models: bool,
        out: &mut Vec<Instance>,
        mut named: Option<&mut Vec<Name>>,
    ) {
        let span = (self.span / VOXELS_PER_METRE as f64) as f32;
        let me = xz(self.player);
        let me_px = place(me).map(|p| p.0);
        let icon = if self.open { 6.5 * f.s } else { 4.5 * f.s };
        let mut hints: Vec<(f64, DVec2)> = Vec::new();
        let mut icons = Vec::new();
        for key in self.known.keys() {
            let (Some(k), Some(r)) = (self.known.get(key), self.reveals.get(key)) else { continue };
            let o = origin(*key);
            for ((mark, &visited), known) in k.sheet.marks.iter().zip(&k.visited).zip(&k.places) {
                let at = place(mark.at);
                let on_map = at.is_some_and(|(p, shown)| shown && f.edge(p) < -3.0);
                let seen = r.seen((mark.at - o).as_vec2());
                // What stands there as a model of itself is its own mark.
                let modelled = models && matches!(mark.kind, Kind::Landmark | Kind::Tor);
                let mut on_water = false;
                if let Some((p, _)) = at
                    && on_map
                    && (visited || seen)
                    && let Some(named) = named.as_deref_mut()
                    && let (Some(name), Some((kind, importance))) = (&mark.name, known)
                    && (visited || kind.is_natural() || kind.is_settlement())
                {
                    let rank = names::rank(*kind);
                    let fade = names::fade(rank, *importance, span);
                    if fade > 0.05 {
                        let ink = if names::watery(*kind) { WATER_INK } else { INK };
                        let strength = if visited { 255.0 } else { 225.0 };
                        // Over its place, as a pin's flag is; a lake's lies on its water.
                        on_water = *kind == PlaceKind::Lake;
                        let lift = if on_water { 0.0 } else { icon + 9.0 * f.s + if modelled { 8.0 * f.s } else { 0.0 } };
                        let size = names::size(rank, *importance) * f.s;
                        let (at, or) = clear_of_player(Vec2::new(p.x, p.y - lift), Vec2::new(p.x, p.y + icon + 10.0 * f.s), me_px, size, f.s);
                        named.push(Name {
                            text: name.clone(),
                            at: if on_water { Vec2::new(p.x, p.y - lift) } else { at },
                            // Under its place, if there is no room over it.
                            or: (!on_water).then_some(or),
                            size,
                            weight: rank as u8 as f32 * 100.0 + importance + if visited { 40.0 } else { 0.0 },
                            color: [ink[0], ink[1], ink[2], (strength * fade) as u8],
                        });
                    }
                }
                if let Some((p, _)) = at
                    && on_map
                    && visited
                {
                    if !modelled && !on_water {
                        icons.push(icon_of(mark, p, icon));
                    }
                } else if !mark.hint {
                    // Left for the player to come on unawares.
                } else if on_map && seen {
                    // Seen but not yet come to: nothing is laid over it, as the little
                    // glowing blocks that stood here looked like toy cubes, not places
                    // (#196). A natural place still has its name, above.
                } else if !visited {
                    let d = mark.at.distance(me);
                    if d <= HINT {
                        hints.push((d, mark.at));
                    }
                }
            }
        }
        // Only on the corner map, and from where the player is on it: a hint points away
        // from them. Its way across the screen is worked out on the level ground, not from
        // where the place itself would fall: a place a few kilometres off is far outside
        // the view, or behind the eye, where a projected point swings about or flips over.
        if let Some((east, north)) = rim
            && let Some(me_px) = me_px
            && f.edge(me_px) < -2.0
        {
            hints.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (d, at) in hints.into_iter().take(HINTS) {
                let off = (at - me).as_vec2();
                let dir = (east * off.x - north * off.y).normalize_or_zero();
                if dir == Vec2::ZERO {
                    continue;
                }
                let p = f.rim(me_px, dir, 5.0 * f.s);
                let near = (1.0 - d / HINT) as f32;
                let mut g = Instance::new(gpu::GLOW, p.into(), 0.0, [1.0, 0.95, 0.82, 0.25 + 0.5 * near.powf(0.7)]);
                g.size = [5.0 * f.s, 14.0 * f.s];
                g.dir = dir.into();
                out.push(g);
            }
        }
        // The player's own camps: where a fire of theirs has burned.
        for &camp in &self.camps {
            if let Some((p, true)) = place(camp)
                && f.edge(p) < -3.0
            {
                icons.push(Instance::new(gpu::TRIANGLE, p.into(), icon * 0.9, [0.93, 0.50, 0.16, 1.0]));
                if let Some(named) = named.as_deref_mut() {
                    let (at, or) = clear_of_player(Vec2::new(p.x, p.y - icon - 9.0 * f.s), Vec2::new(p.x, p.y + icon + 9.0 * f.s), me_px, 13.5 * f.s, f.s);
                    named.push(Name { text: "Camp".into(), at, or: Some(or), size: 13.5 * f.s, weight: 1.5, color: [255, 226, 184, 255] });
                }
            }
        }
        out.extend(icons);
    }

    /// The names of the places that are not marks: the regions, and the ranges, islands,
    /// rivers and bays, each where the land shows it. `place` is as for [`Self::marks`].
    fn names(&self, f: &Frame, place: &dyn Fn(DVec2) -> Option<(Vec2, bool)>, named: &mut Vec<Name>) {
        let span = (self.span / VOXELS_PER_METRE as f64) as f32;
        for (key, k) in &self.known {
            let Some(r) = self.reveals.get(key) else { continue };
            let o = origin(*key);
            let gazetteer = &k.sheet.continent.trails.names;
            let anchors = self.anchors.get(key).map_or(&[][..], |a| &a.1[..]);
            for (i, p) in gazetteer.places.iter().enumerate() {
                let rank = names::rank(p.kind);
                let at = match p.kind {
                    PlaceKind::Region => anchors.iter().find(|a| a.0 == i).map(|a| a.1),
                    PlaceKind::Range | PlaceKind::Fault | PlaceKind::Rift | PlaceKind::Island | PlaceKind::River | PlaceKind::Bay | PlaceKind::Inlet => {
                        // Where the gazetteer has it, or along its course where that
                        // has been seen.
                        let near = |a: &&Vec2, b: &&Vec2| a.distance(p.at).total_cmp(&b.distance(p.at));
                        let local = if r.seen(p.at) { Some(p.at) } else { p.line.iter().filter(|q| r.seen(**q)).min_by(near).copied() };
                        local.map(|q| o + q.as_dvec2())
                    }
                    // The rest are marks, and named with them.
                    _ => None,
                };
                let Some(at) = at else { continue };
                let fade = names::fade(rank, p.importance, span);
                if fade <= 0.05 {
                    continue;
                }
                let Some((px, _)) = place(at).filter(|(px, _)| f.edge(*px) < -3.0) else { continue };
                let (text, ink, strength) = match p.kind {
                    PlaceKind::Region => (names::spaced(&p.name), REGION_INK, 215.0),
                    kind if names::watery(kind) => (p.name.clone(), WATER_INK, 240.0),
                    _ => (p.name.clone(), INK, 240.0),
                };
                // A river's name lies beside its water, not over it.
                let lift = if p.kind == PlaceKind::River { 11.0 * f.s } else { 0.0 };
                named.push(Name {
                    text,
                    at: Vec2::new(px.x, px.y - lift),
                    or: (p.kind == PlaceKind::River).then(|| Vec2::new(px.x, px.y + lift)),
                    size: names::size(rank, p.importance) * f.s,
                    weight: rank as u8 as f32 * 100.0 + p.importance,
                    color: [ink[0], ink[1], ink[2], (strength * fade) as u8],
                });
            }
        }
    }
}

/// Where the name of a place goes, over it or else under it, moved clear of the player
/// where they stand on it. Names give way to the player (see `draw`), so the name of
/// the very place they stand in, written just over or under its icon, would have no
/// room at all: a great town's, the largest, was never written while they were there.
fn clear_of_player(mut over: Vec2, mut under: Vec2, me: Option<Vec2>, size: f32, s: f32) -> (Vec2, Vec2) {
    if let Some(me) = me
        && (me - (over + under) * 0.5).abs().max_element() < 30.0 * s
    {
        // The player's own rectangle, 9 either way, and the gap names keep, 5, and a
        // little more; a name is 1.2 of its size tall.
        let clear = 15.0 * s + size * 0.6;
        over.y = over.y.min(me.y - clear);
        under.y = under.y.max(me.y + clear);
    }
    (over, under)
}

/// The icon a visited place shows as.
fn icon_of(mark: &Mark, p: Vec2, size: f32) -> Instance {
    let water = [0.24, 0.50, 0.74, 1.0];
    let (kind, color, scale) = match mark.kind {
        Kind::Lake => (gpu::RING, water, 1.1),
        Kind::Falls => (gpu::CIRCLE, water, 1.0),
        Kind::Spring => (gpu::CIRCLE, [0.30, 0.62, 0.66, 1.0], 0.8),
        Kind::Mouth | Kind::Confluence => (gpu::CIRCLE, [0.36, 0.58, 0.78, 1.0], 0.75),
        Kind::Peak => (gpu::TRIANGLE, [0.46, 0.39, 0.33, 1.0], 1.1),
        // Granite grey, and smaller than a peak.
        Kind::Tor => (gpu::TRIANGLE, [0.50, 0.50, 0.48, 1.0], 0.85),
        Kind::Landmark => (gpu::DIAMOND, [0.74, 0.53, 0.26, 1.0], 1.1),
        // Brick red, as the map in the shops would show one.
        Kind::Settlement => (gpu::CIRCLE, [0.66, 0.30, 0.22, 1.0], 1.2),
    };
    Instance::new(kind, p.into(), size * scale, color)
}

/// The whole continent under `pos` (world voxels) as the map draws it, all of it seen:
/// `picture::TEX` square, north up. For `--map FILE`.
pub fn build(world: &GameWorld, pos: DVec3) -> image::RgbaImage {
    let key = key_of(xz(pos));
    let c = world.continent(key.0, key.1);
    let side = picture::TEX as u32;
    image::RgbaImage::from_raw(side, side, picture::picture(world, &c)).expect("a picture is TEX square")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(round: bool) -> Frame {
        Frame {
            centre: Vec2::new(500.0, 300.0),
            half: Vec2::splat(100.0),
            round,
            corner: 8.0,
            voxels_per_px: 10.0,
            s: 1.0,
        }
    }

    #[test]
    fn a_view_puts_what_it_looks_at_in_its_middle_and_what_lies_ahead_above_it() {
        let look = DVec2::new(5000.0, -3000.0);
        let (corner, size) = (Vec2::new(900.0, 20.0), Vec2::new(200.0, 200.0));
        for heading in [Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(-0.6, 0.8)] {
            let yaw = (-heading.x).atan2(-heading.y);
            let v = look_down(look, 2.0, yaw, SMALL_PITCH, 40.0, corner, size);
            let mid = v.px(look, 40.0).expect("in front");
            assert!((mid - (corner + size * 0.5)).length() < 0.01, "{mid}");
            let ahead = v.px(look + heading.as_dvec2() * 100.0, 40.0).expect("in front");
            assert!((ahead.x - mid.x).abs() < 0.01 && ahead.y < mid.y - 10.0, "{heading}: {ahead} from {mid}");
            // The ground to the right of the way faced is to the right.
            let right = v.px(look + DVec2::new(-heading.y as f64, heading.x as f64) * 100.0, 40.0).expect("in front");
            assert!(right.x > mid.x + 10.0, "{heading}: {right}");
        }
    }

    /// The way a hint at the corner map's rim points, worked out on the level ground, is
    /// the way the place lies on the map, however the map is turned.
    #[test]
    fn a_hint_points_the_way_its_place_lies_however_the_map_is_turned() {
        let look = DVec2::new(5000.0, -3000.0);
        let (corner, size) = (Vec2::new(900.0, 20.0), Vec2::new(200.0, 200.0));
        for yaw in [0.0f32, 0.8, 1.6, 2.4, 3.1, 4.7] {
            let v = look_down(look, 2.0, yaw, SMALL_PITCH, 40.0, corner, size);
            let me = v.px(look, 40.0).expect("in front");
            for off in [DVec2::new(60.0, 0.0), DVec2::new(-40.0, 50.0), DVec2::new(10.0, -70.0)] {
                let seen = (v.px(look + off, 40.0).expect("in front") - me).normalize();
                let way = |off: DVec2| (v.east * off.x as f32 - v.north * off.y as f32).normalize();
                assert!(way(off).dot(seen) > 0.97, "yaw {yaw} {off}: {} against {seen}", way(off));
            }
        }
    }

    #[test]
    fn the_model_has_coarser_columns_the_further_out_the_map_is() {
        assert_eq!(step_for(0.2, 1.0), relief::FINE_STEP);
        assert_eq!(step_for(1.0e6, 1.0), relief::MAX_STEP);
        let steps: Vec<i32> = (0..40).map(|k| step_for(1.5 * 1.15f64.powi(k), 1.0)).collect();
        assert!(steps.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] * 2), "{steps:?}");
        // Whatever the zoom, what a wide window shows fits within the model.
        for k in 0..44 {
            let per_px = 1.5 * 1.1f64.powi(k);
            let columns = 1280.0 * per_px / step_for(per_px, 1.0) as f64;
            assert!(columns < relief::N as f64, "{per_px} voxels a pixel shows {columns} columns");
        }
    }

    #[test]
    fn the_edge_of_the_map_is_nought_and_inside_is_less() {
        for round in [true, false] {
            let f = frame(round);
            assert!(f.edge(f.centre) < 0.0 && f.edge(f.centre + Vec2::new(120.0, 0.0)) > 0.0);
            assert!(f.edge(f.centre + Vec2::new(100.0, 0.0)).abs() < 1e-3);
        }
    }

    #[test]
    fn hints_sit_just_inside_the_rim_in_their_direction() {
        for round in [true, false] {
            let f = frame(round);
            for dir in [Vec2::X, Vec2::NEG_Y, Vec2::new(0.6, 0.8), Vec2::new(-0.8, 0.6)] {
                let from = f.centre + Vec2::new(10.0, -20.0);
                let p = f.rim(from, dir, 5.0);
                assert!((f.edge(p) + 5.0).abs() < 0.6 || !round, "{round} {dir} {p} {}", f.edge(p));
                assert!(f.edge(p) < 0.0 && f.edge(p) > -6.0, "{round} {dir} edge {}", f.edge(p));
                assert!((p - from).normalize().dot(dir) > 0.999);
            }
        }
    }

    #[test]
    fn a_continent_sheet_has_its_land_and_water_and_places_on_it() {
        let world = GameWorld::with_config(3, worldgen::continent::SimConfig { size: 256 });
        let c = world.continent(0, 0);
        let s = sheet(&world, c.clone());
        assert_eq!(s.picture.len(), picture::TEX * picture::TEX * 4);
        // The sea is blue and the land is not.
        let texel = |i: usize| {
            let (x, z) = (i % c.size * picture::TEX / c.size, i / c.size * picture::TEX / c.size);
            let t = (z * picture::TEX + x) * 4;
            [s.picture[t] as i32, s.picture[t + 1] as i32, s.picture[t + 2] as i32]
        };
        let sea = (0..c.size * c.size).filter(|&i| c.ocean[i]).min_by(|&a, &b| c.height[a].total_cmp(&c.height[b])).expect("sea");
        let [r, _, b] = texel(sea);
        assert!(b > r + 40, "sea {:?}", texel(sea));
        let land = (0..c.size * c.size).filter(|&i| !c.ocean[i]).count();
        let greenish = (0..c.size * c.size).filter(|&i| !c.ocean[i] && texel(i)[1] > texel(i)[2]).count();
        assert!(greenish * 2 > land, "{greenish} of {land} land texels are green over blue");

        assert!(!s.marks.is_empty());
        let size = MACRO_SIZE as f64;
        for m in &s.marks {
            assert!((0.0..size).contains(&m.at.x) && (0.0..size).contains(&m.at.y), "{m:?}");
            assert!(m.reach > 0.0);
        }
        let named = s.marks.iter().filter(|m| m.name.is_some()).count();
        assert!(named > 0);
        assert!(s.marks.iter().any(|m| m.kind == Kind::Landmark));
        assert!(s.marks.iter().any(|m| matches!(m.kind, Kind::Falls | Kind::Lake | Kind::Spring | Kind::Mouth | Kind::Confluence)));
        // A bridge is walked over, not sought out: none has a mark of its own.
        let arches: Vec<DVec2> = c.trails.crossings.iter().filter(|x| x.kind == worldgen::trails::CrossingKind::Arch).map(|x| DVec2::new(x.x as f64 + 0.5, x.z as f64 + 0.5)).collect();
        for m in &s.marks {
            assert!(!arches.iter().any(|a| a.distance(m.at) < 1.0), "{m:?} marks a bridge");
        }
        // The same world finds the same places.
        let again = marks::find(&c, world.seed());
        assert_eq!(again.len(), s.marks.len());
        assert!(again.iter().zip(&s.marks).all(|(a, b)| a.at == b.at && a.kind == b.kind));
    }

    #[test]
    fn the_block_around_a_point_holds_everything_within_half_a_continent() {
        let size = MACRO_SIZE as f64;
        for at in [DVec2::new(10.0, 10.0), DVec2::new(size * 0.7, -size * 0.2), DVec2::new(-size * 3.4, size * 1.5)] {
            let block = block_of(at);
            for (dx, dz) in [(-0.49, -0.49), (0.49, -0.49), (-0.49, 0.49), (0.49, 0.49), (0.0, 0.0)] {
                let p = at + DVec2::new(dx, dz) * size;
                assert!(block.contains(&key_of(p)), "{at} {p}");
            }
        }
    }
}
