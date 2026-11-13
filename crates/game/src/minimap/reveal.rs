//! What the player has seen of a continent, and where they have been.
//!
//! Two channels at a texel per grid cell (about 26 m): how much of the land there has
//! been seen, which fills in around the player as they travel, further from high
//! ground; and the trail they walked, which the map draws faintly and which decides
//! what they have visited. Saved per world as a small PNG for each continent.

use std::path::Path;

use glam::Vec2;
use worldgen::{continent::Continent, macro_layout::MACRO_SIZE, scale::{OLD_VOXEL, m}};

use super::picture::TEX;

/// Voxels per texel.
pub const TEXEL: f32 = MACRO_SIZE as f32 / TEX as f32;
/// How far the player sees on level ground, and at most, in voxels.
pub const NEAR_SIGHT: f32 = m(250.0);
pub const FAR_SIGHT: f32 = m(1500.0);
/// How far around the ground is looked at to judge how high the player stands.
const LOOKOUT: [f32; 2] = [m(500.0), m(1400.0)];
/// Radius of the walked trail, in texels.
const TRAIL: f32 = 0.9;

/// A rectangle of texels, `[x0, x1) x [z0, z1)`.
pub type Rect = (usize, usize, usize, usize);

pub struct Reveal {
    /// Seen, then walked, per texel, row by row.
    pub data: Vec<u8>,
    /// Texels changed since the GPU copy was last brought up to date.
    pub stale: Option<Rect>,
    /// Changed since it was last saved.
    pub unsaved: bool,
    /// How many times it has changed.
    pub changes: u32,
}

impl Default for Reveal {
    fn default() -> Self {
        Self::new()
    }
}

impl Reveal {
    pub fn new() -> Self {
        Self { data: vec![0; TEX * TEX * 2], stale: Some((0, 0, TEX, TEX)), unsaved: false, changes: 0 }
    }

    /// Reads a saved mask, or None if there is none or it is not one.
    pub fn load(path: &Path) -> Option<Self> {
        let img = image::open(path).ok()?.into_luma_alpha8();
        if img.width() as usize != TEX || img.height() as usize != TEX {
            log::warn!("ignoring the map in {}: it is not {TEX} square", path.display());
            return None;
        }
        Some(Self { data: img.into_raw(), stale: Some((0, 0, TEX, TEX)), unsaved: false, changes: 0 })
    }

    fn touch(&mut self, r: Rect) {
        self.stale = Some(match self.stale {
            Some(s) => (s.0.min(r.0), s.1.min(r.1), s.2.max(r.2), s.3.max(r.3)),
            None => r,
        });
        self.unsaved = true;
        self.changes += 1;
    }

    /// Texels within `radius` texels of `centre` (in texels), clipped to the mask.
    fn around(centre: Vec2, radius: f32) -> Option<Rect> {
        let lo = |v: f32| (v - radius - 1.0).floor().max(0.0) as usize;
        let hi = |v: f32| ((v + radius + 1.0).ceil().max(0.0) as usize).min(TEX);
        let r = (lo(centre.x), lo(centre.y), hi(centre.x), hi(centre.y));
        (r.0 < r.2 && r.1 < r.3).then_some(r)
    }

    /// Marks as seen everything within `radius` voxels of continent-local `at`, fading
    /// over its outer quarter; and, if `walked`, the ground underfoot as walked. `at` may
    /// lie outside the continent, so a view across its edge reaches into it.
    pub fn see(&mut self, at: Vec2, radius: f32, walked: bool) {
        let c = at / TEXEL;
        let r = radius / TEXEL;
        if let Some(rect) = Self::around(c, r) {
            let mut changed = false;
            for z in rect.1..rect.3 {
                for x in rect.0..rect.2 {
                    let d = (x as f32 + 0.5 - c.x).hypot(z as f32 + 0.5 - c.y);
                    let t = ((r - d) / (r * 0.25)).clamp(0.0, 1.0);
                    let v = (t * t * (3.0 - 2.0 * t) * 255.0) as u8;
                    let cell = &mut self.data[(z * TEX + x) * 2];
                    if v > *cell {
                        *cell = v;
                        changed = true;
                    }
                }
            }
            if changed {
                self.touch(rect);
            }
        }
        if walked && let Some(rect) = Self::around(c, TRAIL) {
            let mut changed = false;
            for z in rect.1..rect.3 {
                for x in rect.0..rect.2 {
                    let d = (x as f32 + 0.5 - c.x).hypot(z as f32 + 0.5 - c.y);
                    let v = ((TRAIL + 0.5 - d).clamp(0.0, 1.0) * 255.0) as u8;
                    let cell = &mut self.data[(z * TEX + x) * 2 + 1];
                    if v > *cell {
                        *cell = v;
                        changed = true;
                    }
                }
            }
            if changed {
                self.touch(rect);
            }
        }
    }

    fn texel(&self, at: Vec2, channel: usize) -> u8 {
        let t = at / TEXEL;
        if t.x < 0.0 || t.y < 0.0 || t.x >= TEX as f32 || t.y >= TEX as f32 {
            return 0;
        }
        self.data[(t.y as usize * TEX + t.x as usize) * 2 + channel]
    }

    /// How well seen, and how walked, the land at continent-local `at` is.
    pub fn at(&self, at: Vec2) -> [u8; 2] {
        [self.texel(at, 0), self.texel(at, 1)]
    }

    /// Whether the land at continent-local `at` has been seen.
    pub fn seen(&self, at: Vec2) -> bool {
        self.texel(at, 0) >= 128
    }

    /// Whether the walked trail passes within `radius` voxels of continent-local `at`.
    pub fn walked_near(&self, at: Vec2, radius: f32) -> bool {
        let c = at / TEXEL;
        let r = radius / TEXEL + 0.5;
        let Some(rect) = Self::around(c, r) else { return false };
        (rect.1..rect.3).any(|z| {
            (rect.0..rect.2).any(|x| {
                (x as f32 + 0.5 - c.x).hypot(z as f32 + 0.5 - c.y) <= r && self.data[(z * TEX + x) * 2 + 1] >= 128
            })
        })
    }

}

/// Writes a mask's data as a PNG, through a temporary file so that a crash part way
/// leaves the last one whole.
pub fn write(path: &Path, data: &[u8]) -> bool {
    let temp = path.with_extension("tmp");
    let ok = path.parent().is_none_or(|d| std::fs::create_dir_all(d).is_ok())
        && image::save_buffer_with_format(&temp, data, TEX as u32, TEX as u32, image::ColorType::La8, image::ImageFormat::Png).is_ok()
        && std::fs::rename(&temp, path).is_ok();
    if !ok {
        log::warn!("could not save the map to {}", path.display());
    }
    ok
}

/// How far the player sees from continent-local `at`, eye `eye` voxels up: more the
/// higher they stand above the land around them, as from a hill or in the air.
pub fn sight(c: &Continent, at: Vec2, eye: f32) -> f32 {
    let n = c.size;
    let cs = c.cell_size();
    let ground = |p: Vec2| {
        let (x, z) = ((p.x / cs).clamp(0.0, (n - 1) as f32) as usize, (p.y / cs).clamp(0.0, (n - 1) as f32) as usize);
        // The sea is as low as the land around goes.
        (c.height[z * n + x] * OLD_VOXEL).max(0.0)
    };
    let around: f32 = LOOKOUT
        .iter()
        .flat_map(|&r| {
            (0..12).map(move |k| {
                let a = (k as f32 + 0.5 * r / LOOKOUT[1]) / 12.0 * std::f32::consts::TAU;
                at + Vec2::new(a.cos(), a.sin()) * r
            })
        })
        .map(ground)
        .sum::<f32>()
        / (12 * LOOKOUT.len()) as f32;
    let above = (eye - around).max(0.0);
    (NEAR_SIGHT + above * 2.5).min(FAR_SIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeing_fills_a_disc_and_only_walking_leaves_a_trail() {
        let mut r = Reveal::new();
        r.stale = None;
        let at = Vec2::splat(TEXEL * 300.0);
        r.see(at, m(200.0), false);
        assert!(r.seen(at));
        assert!(r.seen(at + Vec2::new(m(140.0), 0.0)));
        assert!(!r.seen(at + Vec2::new(m(260.0), 0.0)));
        assert!(!r.walked_near(at, m(30.0)));
        assert!(r.unsaved && r.stale.is_some());
        r.see(at, m(200.0), true);
        assert!(r.walked_near(at, m(10.0)));
        assert!(r.walked_near(at + Vec2::new(m(60.0), 0.0), m(45.0)));
        assert!(!r.walked_near(at + Vec2::new(m(200.0), 0.0), m(45.0)));
    }

    #[test]
    fn high_ground_sees_further_than_low() {
        let world = worldgen::world::GameWorld::with_config(3, worldgen::continent::SimConfig { size: 256 });
        let c = world.continent(0, 0);
        let n = c.size;
        let cs = c.cell_size();
        let at = |i: usize| Vec2::new((i % n) as f32 + 0.5, (i / n) as f32 + 0.5) * cs;
        let ground = |i: usize| c.height[i] * OLD_VOXEL;
        let top = (0..n * n).max_by(|&a, &b| c.height[a].total_cmp(&c.height[b])).expect("cells");
        let shore = (0..n * n).find(|&i| !c.ocean[i] && c.height[i] < 2.0).expect("a shore");
        let (high, low) = (sight(&c, at(top), ground(top) + 4.0), sight(&c, at(shore), ground(shore) + 4.0));
        assert!(high > low + m(200.0), "summit {high}, shore {low}");
        assert!((NEAR_SIGHT..=FAR_SIGHT).contains(&low) && (NEAR_SIGHT..=FAR_SIGHT).contains(&high));
        // Far up in the air, it is as far as it goes.
        assert_eq!(sight(&c, at(shore), m(3000.0)), FAR_SIGHT);
    }

    #[test]
    fn seeing_never_takes_back_what_was_seen() {
        let mut r = Reveal::new();
        let at = Vec2::splat(TEXEL * 500.0);
        r.see(at, m(400.0), false);
        let before = r.data.clone();
        r.see(at, m(100.0), false);
        assert_eq!(r.data, before);
    }

    #[test]
    fn a_view_across_the_edge_reaches_in_and_one_far_away_does_nothing() {
        let mut r = Reveal::new();
        r.stale = None;
        r.see(Vec2::new(-m(50.0), TEXEL * 100.0), m(200.0), false);
        assert!(r.seen(Vec2::new(m(60.0), TEXEL * 100.0)));
        let mut far = Reveal::new();
        far.stale = None;
        far.see(Vec2::splat(-m(5000.0)), m(200.0), true);
        assert!(far.stale.is_none() && !far.unsaved);
    }

    #[test]
    fn a_saved_mask_reads_back_the_same() {
        let dir = std::env::temp_dir().join(format!("minimap-test-{}", std::process::id()));
        let path = dir.join("0_0.png");
        let mut r = Reveal::new();
        r.see(Vec2::splat(TEXEL * 40.0), m(300.0), true);
        assert!(write(&path, &r.data));
        let back = Reveal::load(&path).expect("reads back");
        assert_eq!(back.data, r.data);
        let _ = std::fs::remove_dir_all(dir);
    }
}
