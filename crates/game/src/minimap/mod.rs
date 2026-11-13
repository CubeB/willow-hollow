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
//! visited shows only as a soft glow, and only once the land around it has been seen;
//! one still out of sight shows as a faint glow at the corner map's rim in its direction,
//! for the nearest few. A visited place turns into its icon, and on the large map its name.
//!
//! The large map is a model of the land seen so far ([`relief`]): a slab of blocks with
//! its water, paths and woods, looked down on at a slant, which the mouse moves, turns
//! and zooms. The corner map is a small one of the same, of the ground about the
//! player, turned so that the way they face is always up.
//!
//! [`build`] is the older still image of a whole continent, for `--map FILE`.

pub mod gpu;
pub mod marks;
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
    scale::{OLD_VOXEL, m64},
    world::GameWorld,
};

use crate::gfx::Gpu;
use gpu::{Instance, MapGpu, ReliefGpu, Uniforms};
use marks::{Kind, Mark};
use relief::{Relief, Request};
use reveal::Reveal;

/// A continent, by macro-cell.
pub type Key = (i32, i32);

/// How near and how far the large map zooms, edge to edge, in voxels.
const LARGE_SPAN: (f64, f64) = (m64(700.0), MACRO_SIZE as f64);
const LARGE_START: f64 = m64(2400.0);
/// How the large map is first looked at: turned a little from north, and from how high.
const YAW: f32 = -0.3;
const PITCH: f32 = 0.86;
const PITCH_RANGE: (f32, f32) = (0.45, 1.45);
/// The large map's field of view, top to bottom.
const FOV: f32 = 0.5;
/// How many pixels wide a column of the large map's model is, about, on a window 720 high.
const COLUMN_PX: f64 = 6.0;
/// How far, in columns, the map's middle may stray from the model's before a new one is made.
const STRAY: i64 = 40;
/// The corner map: how much ground it spans across its middle, how steeply it is looked
/// down on, how far the player may walk from the middle of its model, in columns, and
/// how long it waits, in seconds, before a new one is made for newly seen ground.
const SMALL_SPAN: f64 = m64(130.0);
const SMALL_PITCH: f32 = 0.95;
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

/// What the worker makes of one continent.
pub struct Sheet {
    pub picture: Arc<Vec<u8>>,
    pub marks: Vec<Mark>,
    pub continent: Arc<Continent>,
}

/// Builds the sheet for continent `key`.
pub fn sheet(world: &GameWorld, c: Arc<Continent>) -> Sheet {
    Sheet { picture: Arc::new(picture::picture(world, &c)), marks: marks::find(&c, world.seed()), continent: c }
}

struct Known {
    sheet: Sheet,
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
    (1 << want.log2().round() as i32).clamp(relief::MIN_STEP, relief::MAX_STEP)
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
    region_timer: f32,
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
                while let Ok(req) = reliefs.recv() {
                    let t = std::time::Instant::now();
                    let made = relief::build(&w, &req);
                    log::debug!("map model at step {}: {} corners, {} trees, made in {:.2} s", made.step, made.vertices.len(), made.trees.len(), t.elapsed().as_secs_f32());
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
            region_timer: 0.0,
            clock: 0.0,
            labels: Vec::new(),
        }
    }

    /// Opens or closes the large map, opening on the player.
    pub fn set_open(&mut self, open: bool) {
        if open && !self.open {
            self.look = xz(self.player);
            self.aim = None;
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
        self.relief_ask.send(Request { tag, n, step, origin, mask, sheets, version: self.version }).is_ok()
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
                    self.known.insert(key, Known { sheet, visited: vec![false; n], checked: false });
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
            self.region_timer = 1.0;
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
            for k in self.known.values_mut() {
                for (mark, seen) in k.sheet.marks.iter().zip(&mut k.visited) {
                    *seen |= mark.at.distance(at) <= mark.reach as f64;
                }
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
    pub fn settle(&mut self, gpu: &Gpu, eye: DVec3, forward: Vec3) {
        loop {
            self.update(gpu, eye, forward, false, 0.0);
            if self.pending.is_empty() && self.loading.is_empty() && !self.relief_pending && !self.small_pending {
                break;
            }
            match self.answers.recv_timeout(std::time::Duration::from_secs(120)) {
                Ok(a) => self.take(gpu, a),
                Err(_) => break,
            }
        }
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
        self.marks(&f, &place, Some((view.east, view.north)), &mut out, &mut labels);

        // The player, always facing up the map, and where north has got to.
        if let Some((me, _)) = place(xz(self.player)) {
            let mut arrow = Instance::new(gpu::ARROW, me.into(), 7.5 * f.s, [0.99, 0.98, 0.95, 1.0]);
            arrow.dir = [0.0, -1.0];
            out.push(arrow);
        }
        let north = f.rim(f.centre, view.north.normalize_or_zero(), 10.0 * f.s);
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
            self.small_gpu.draw(gpu, encoder, target, size, transform, Some(within));
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
        self.marks(&f, &place, None, &mut out, &mut labels);

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
        let zoom = relief.map_or(1, |r| r.step / relief::MIN_STEP);
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
            self.relief_gpu.draw(gpu, encoder, target, size, transform, None);
        }
        self.gpu.pass(encoder, target, 1..out.len() as u32);
    }

    /// The places on the map: icons for those visited, soft glows for those seen but
    /// not visited, and at the rim, the nearest few still out of sight. `place` gives
    /// where a point of the world falls on the screen, and whether the map shows it;
    /// `rim`, the ways east and north run across the screen, for the hints at the rim,
    /// or None for none.
    ///
    /// The large map has no hints at its rim. It fills the screen, so its rim is the
    /// screen's edge, and the places hinted at, a few kilometres off, mostly lie within
    /// it on ground not yet seen: a glow at the edge of the screen pointing at the middle
    /// of it, wandering round the edge as the map was turned, read as something wrong.
    fn marks(&self, f: &Frame, place: &dyn Fn(DVec2) -> Option<(Vec2, bool)>, rim: Option<(Vec2, Vec2)>, out: &mut Vec<Instance>, labels: &mut Vec<Label>) {
        let me = xz(self.player);
        let me_px = place(me).map(|p| p.0);
        let (icon, glow) = if self.open { (6.5 * f.s, 11.0 * f.s) } else { (4.5 * f.s, 6.5 * f.s) };
        let mut hints: Vec<(f64, DVec2)> = Vec::new();
        let mut icons = Vec::new();
        for key in self.known.keys() {
            let (Some(k), Some(r)) = (self.known.get(key), self.reveals.get(key)) else { continue };
            let o = origin(*key);
            for (mark, &visited) in k.sheet.marks.iter().zip(&k.visited) {
                let at = place(mark.at);
                let on_map = at.is_some_and(|(p, shown)| shown && f.edge(p) < -3.0);
                let seen = r.seen((mark.at - o).as_vec2());
                if let Some((p, _)) = at
                    && on_map
                    && visited
                {
                    icons.push(icon_of(mark, p, icon));
                    if self.open
                        && let Some(name) = &mark.name
                    {
                        // Over its place, as a pin's flag is.
                        labels.push(Label {
                            pos: [p.x, p.y - icon - 9.0 * f.s],
                            text: name.clone(),
                            size: 13.5 * f.s,
                            color: [250, 248, 240, 255],
                            left: false,
                            right: false,
                        });
                    }
                } else if let Some((p, _)) = at
                    && on_map
                    && seen
                {
                    let tint = if mark.kind.watery() { [0.88, 0.96, 1.0] } else { [1.0, 0.97, 0.86] };
                    let phase = (mark.at.x * 0.013 + mark.at.y * 0.007) as f32;
                    let breathe = 0.85 + 0.15 * (self.clock * 1.1 + phase).sin();
                    out.push(Instance::new(gpu::GLOW, p.into(), glow, [tint[0], tint[1], tint[2], 0.8 * breathe]));
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
                if self.open {
                    labels.push(Label { pos: [p.x, p.y - icon - 9.0 * f.s], text: "Camp".into(), size: 13.5 * f.s, color: [255, 226, 184, 255], left: false, right: false });
                }
            }
        }
        out.extend(icons);
    }
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
        Kind::Bridge => (gpu::BAR, [0.52, 0.40, 0.28, 1.0], 1.0),
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
        assert_eq!(step_for(0.5, 1.0), relief::MIN_STEP);
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
        assert!(s.marks.iter().any(|m| m.kind.watery()));
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
