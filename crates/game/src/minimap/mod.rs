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
//! one still out of sight shows as a faint glow at the map's rim in its direction, for
//! the nearest few. A visited place turns into its icon, and on the large map its name.
//!
//! [`build`] is the older still image of a whole continent, for `--map FILE`.

pub mod gpu;
pub mod marks;
pub mod picture;
pub mod reveal;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
};

use glam::{DVec2, DVec3, Vec2, Vec3};
use worldgen::{
    continent::Continent,
    macro_layout::MACRO_SIZE,
    scale::{OLD_VOXEL, m64},
    world::GameWorld,
};

use crate::gfx::Gpu;
use gpu::{Instance, MapGpu, SLOTS, Uniforms};
use marks::{Kind, Mark};
use reveal::Reveal;

/// A continent, by macro-cell.
pub type Key = (i32, i32);

/// How much of the world the corner map spans, edge to edge, in voxels.
const CORNER_SPAN: f64 = m64(1800.0);
/// How near and how far the large map zooms, edge to edge, in voxels.
const LARGE_SPAN: (f64, f64) = (m64(1200.0), MACRO_SIZE as f64);
const LARGE_START: f64 = m64(7000.0);
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
    pub picture: Vec<u8>,
    pub marks: Vec<Mark>,
    pub continent: Arc<Continent>,
}

/// Builds the sheet for continent `key`.
pub fn sheet(world: &GameWorld, c: Arc<Continent>) -> Sheet {
    Sheet { picture: picture::picture(world, &c), marks: marks::find(&c, world.seed()), continent: c }
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
}

/// Where on the screen the map is drawn this frame, and what it shows.
#[derive(Clone, Copy)]
struct Frame {
    centre: Vec2,
    half: Vec2,
    round: bool,
    corner: f32,
    /// The world at the map's middle, voxels.
    look: DVec2,
    voxels_per_px: f64,
    /// Scale for sizes, from the window height.
    s: f32,
}

impl Frame {
    fn to_px(&self, at: DVec2) -> Vec2 {
        self.centre + ((at - self.look) / self.voxels_per_px).as_vec2()
    }

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
fn block_of(at: DVec2) -> [Key; SLOTS] {
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
    /// Its width on the screen last frame, in pixels.
    side: f32,
    gpu: MapGpu,
    slots: [Option<Key>; SLOTS],
    /// Whether each slot's picture, and its mask, are on the GPU.
    pictured: [bool; SLOTS],
    revealed: [bool; SLOTS],
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
            gpu: MapGpu::new(gpu, format),
            slots: [None; SLOTS],
            pictured: [false; SLOTS],
            revealed: [false; SLOTS],
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

    /// Moves the large map with the mouse, by pixels.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        let per_px = self.span / self.side.max(1.0) as f64;
        self.look -= DVec2::new(dx as f64, dy as f64) * per_px;
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

    fn take(&mut self, answer: Answer) {
        match answer {
            Answer::Sheet(key, sheet) => {
                self.pending.remove(&key);
                if let Some(sheet) = sheet {
                    let n = sheet.marks.len();
                    self.known.insert(key, Known { sheet, visited: vec![false; n], checked: false });
                    self.reveal_mut(key);
                    self.check_visits(key);
                }
            }
            Answer::Reveal(key, loaded) => {
                let mut r = loaded.unwrap_or_default();
                for (at, radius, walked) in self.loading.remove(&key).unwrap_or_default() {
                    r.see(at, radius, walked);
                }
                self.reveals.insert(key, r);
                self.check_visits(key);
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
            self.take(a);
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
        self.fill_slots(gpu, block);
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
        for kz in lo.1..=hi.1 {
            for kx in lo.0..=hi.0 {
                let key = (kx, kz);
                let local = (at - origin(key)).as_vec2();
                match self.reveal_mut(key) {
                    Some(r) => r.see(local, radius, walked),
                    None => {
                        if let Some(q) = self.loading.get_mut(&key) {
                            q.push((local, radius, walked));
                        }
                    }
                }
            }
        }
        if walked {
            for k in self.known.values_mut() {
                for (mark, seen) in k.sheet.marks.iter().zip(&mut k.visited) {
                    *seen |= mark.at.distance(at) <= mark.reach as f64;
                }
            }
        }
    }

    /// Puts the continents of `block` on the GPU, keeping any already there in place.
    fn fill_slots(&mut self, gpu: &Gpu, block: [Key; SLOTS]) {
        for s in 0..SLOTS {
            if self.slots[s].is_some_and(|k| !block.contains(&k)) {
                self.slots[s] = None;
            }
        }
        for key in block {
            if !self.slots.contains(&Some(key))
                && let Some(s) = self.slots.iter().position(Option::is_none)
            {
                self.slots[s] = Some(key);
                self.pictured[s] = false;
                self.revealed[s] = false;
            }
        }
        for s in 0..SLOTS {
            let Some(key) = self.slots[s] else { continue };
            if !self.pictured[s]
                && let Some(k) = self.known.get(&key)
            {
                self.gpu.picture(gpu, s, &k.sheet.picture);
                self.pictured[s] = true;
            }
            if let Some(r) = self.reveals.get_mut(&key) {
                let rect = if self.revealed[s] { r.stale.take() } else { Some((0, 0, picture::TEX, picture::TEX)) };
                r.stale = None;
                if let Some(rect) = rect {
                    self.gpu.reveal(gpu, s, rect, &r.rows(rect));
                }
                self.revealed[s] = true;
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
            if self.pending.is_empty() && self.loading.is_empty() {
                break;
            }
            match self.answers.recv_timeout(std::time::Duration::from_secs(120)) {
                Ok(a) => self.take(a),
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

    fn frame(&self, w: f32, h: f32) -> Frame {
        let s = (h / 720.0).max(0.75);
        if self.open {
            let side = (w.min(h) * 0.86).floor();
            Frame {
                centre: Vec2::new(w, h) * 0.5,
                half: Vec2::splat(side * 0.5),
                round: false,
                corner: 10.0 * s,
                look: self.look,
                voxels_per_px: self.span / side as f64,
                s,
            }
        } else {
            let d = (h * 0.27).clamp(140.0, 360.0).floor();
            let margin = (h * 0.035).max(12.0);
            Frame {
                centre: Vec2::new(w - margin - d * 0.5, margin + d * 0.5),
                half: Vec2::splat(d * 0.5),
                round: true,
                corner: 0.0,
                look: xz(self.player),
                voxels_per_px: CORNER_SPAN / d as f64,
                s,
            }
        }
    }

    /// Draws the map over `target`, `size` pixels.
    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32)) {
        self.labels.clear();
        if !self.open && !self.corner {
            return;
        }
        let (w, h) = (size.0 as f32, size.1 as f32);
        let f = self.frame(w, h);
        if self.open {
            self.side = f.half.x * 2.0;
        }
        let mut u = Uniforms {
            screen: [w, h],
            centre: f.centre.into(),
            half: f.half.into(),
            voxels_per_px: f.voxels_per_px as f32,
            round: if f.round { 1.0 } else { 0.0 },
            time: self.clock,
            corner: f.corner,
            look: [f.look.x.rem_euclid(1.0e6) as f32, f.look.y.rem_euclid(1.0e6) as f32],
            ..Default::default()
        };
        for (s, key) in self.slots.iter().enumerate() {
            if let Some(key) = key {
                let o = (f.look - origin(*key)).as_vec2();
                let ready = self.pictured[s] && self.revealed[s];
                u.slots[s] = [o.x, o.y, if ready { 1.0 } else { 0.0 }, 0.0];
            }
        }
        let mut out = Vec::with_capacity(64);
        if self.open {
            let mut veil = Instance::new(gpu::VEIL, [w * 0.5, h * 0.5], 0.0, [0.03, 0.035, 0.04, 0.45]);
            veil.size = [w * 0.5 + 1.0, h * 0.5 + 1.0];
            out.push(veil);
        }
        let mut map = Instance::new(gpu::MAP, f.centre.into(), 0.0, [1.0; 4]);
        map.size = (f.half + Vec2::splat(12.0)).into();
        out.push(map);
        self.marks(&f, &mut out);

        // The player, and north.
        let me = f.to_px(xz(self.player));
        if f.edge(me) < -2.0 {
            let mut arrow = Instance::new(gpu::ARROW, me.into(), (if self.open { 9.0 } else { 7.5 }) * f.s, [0.99, 0.98, 0.95, 1.0]);
            arrow.dir = self.heading.into();
            out.push(arrow);
        }
        if f.round {
            let top = f.centre - Vec2::new(0.0, f.half.y - 9.0 * f.s);
            self.labels.push(Label { pos: top.into(), text: "N".into(), size: 11.0 * f.s, color: [60, 56, 50, 220], left: false });
        } else {
            let corner = f.centre - f.half + Vec2::splat(14.0 * f.s);
            if let Some(region) = &self.region {
                self.labels.push(Label { pos: [corner.x, corner.y + 4.0 * f.s], text: region.clone(), size: 17.0 * f.s, color: [44, 40, 36, 235], left: true });
            }
            let foot = f.centre + Vec2::new(0.0, f.half.y + 16.0 * f.s);
            self.labels.push(Label {
                pos: foot.into(),
                text: "M or Esc to close    wheel to zoom    drag to move".into(),
                size: 12.0 * f.s,
                color: [225, 222, 214, 200],
                left: false,
            });
        }
        self.gpu.draw(gpu, encoder, target, &u, &out);
    }

    /// The places on the map: icons for those visited, soft glows for those seen but
    /// not visited, and at the rim, the nearest few still out of sight.
    fn marks(&mut self, f: &Frame, out: &mut Vec<Instance>) {
        let me = xz(self.player);
        let me_px = f.to_px(me);
        let (icon, glow) = if self.open { (6.0 * f.s, 10.0 * f.s) } else { (4.5 * f.s, 6.5 * f.s) };
        let mut hints: Vec<(f64, DVec2)> = Vec::new();
        let mut icons = Vec::new();
        for key in self.slots.iter().flatten() {
            let (Some(k), Some(r)) = (self.known.get(key), self.reveals.get(key)) else { continue };
            let o = origin(*key);
            for (mark, &visited) in k.sheet.marks.iter().zip(&k.visited) {
                let p = f.to_px(mark.at);
                let on_map = f.edge(p) < -3.0;
                let seen = r.seen((mark.at - o).as_vec2());
                if visited && on_map {
                    icons.push(icon_of(mark, p, icon));
                    if self.open
                        && let Some(name) = &mark.name
                    {
                        self.labels.push(Label {
                            pos: [p.x + icon + 4.0 * f.s, p.y],
                            text: name.clone(),
                            size: 12.5 * f.s,
                            color: [40, 36, 32, 235],
                            left: true,
                        });
                    }
                } else if !visited && seen && on_map {
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
        // Only from where the player is on the map: a hint points away from them.
        if f.edge(me_px) < -2.0 {
            hints.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (d, at) in hints.into_iter().take(HINTS) {
                let dir = (at - me).normalize_or_zero().as_vec2();
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
            look: DVec2::new(1000.0, 2000.0),
            voxels_per_px: 10.0,
            s: 1.0,
        }
    }

    #[test]
    fn the_map_puts_its_middle_in_the_middle_and_north_up() {
        let f = frame(true);
        assert_eq!(f.to_px(f.look), f.centre);
        // North is toward -z, up the screen.
        assert!(f.to_px(f.look - DVec2::new(0.0, 500.0)).y < f.centre.y);
        assert!(f.edge(f.centre) < 0.0 && f.edge(f.centre + Vec2::new(120.0, 0.0)) > 0.0);
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
