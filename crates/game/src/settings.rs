//! What the game remembers between runs: the player's settings, and where they were in
//! each world. Both are small `key = value` text files in the user's config folder.

use std::{
    collections::HashMap,
    fmt::Write as _,
    fs, io,
    ops::RangeInclusive,
    path::{Path, PathBuf},
    str::FromStr,
};

use glam::DVec3;

pub const RADIUS: RangeInclusive<i32> = 2..=64;
pub const DAY_MINUTES: RangeInclusive<f32> = 0.0..=120.0;
pub const YEAR_DAYS: RangeInclusive<f32> = crate::season::YEAR_DAYS;
pub const FOV: RangeInclusive<f32> = 50.0..=110.0;
pub const SENSITIVITY: RangeInclusive<f32> = 0.2..=3.0;
pub const VOLUME: RangeInclusive<f32> = 0.0..=1.0;

const SETTINGS_FILE: &str = "settings.txt";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The world last played in.
    pub seed: u32,
    /// Chunk load radius.
    pub radius: i32,
    /// Real minutes per in-game day; 0 freezes the time of day.
    pub day_minutes: f32,
    /// Game days in a year.
    pub year_days: f32,
    /// Vertical field of view in degrees.
    pub fov: f32,
    /// Multiplies the mouse look speed.
    pub sensitivity: f32,
    pub volume: f32,
    pub vsync: bool,
    pub fullscreen: bool,
    /// Sunlight showing in haze and mist as shafts between what shades it.
    pub shafts: bool,
    /// Window size in pixels, once it is anything but the default.
    pub window: Option<(u32, u32)>,
    pub show_debug: bool,
    /// The small map in the corner of the screen.
    pub minimap: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            seed: 1,
            radius: 24,
            day_minutes: 40.0,
            year_days: crate::season::DEFAULT_YEAR_DAYS,
            fov: 70.0,
            sensitivity: 1.0,
            volume: 0.75,
            vsync: true,
            fullscreen: false,
            shafts: true,
            window: None,
            show_debug: true,
            minimap: true,
        }
    }
}

impl Settings {
    fn to_text(self) -> String {
        let mut t = String::new();
        let _ = writeln!(t, "seed = {}", self.seed);
        let _ = writeln!(t, "radius = {}", self.radius);
        let _ = writeln!(t, "day_minutes = {}", self.day_minutes);
        let _ = writeln!(t, "year_days = {}", self.year_days);
        let _ = writeln!(t, "fov = {}", self.fov);
        let _ = writeln!(t, "sensitivity = {}", self.sensitivity);
        let _ = writeln!(t, "volume = {}", self.volume);
        let _ = writeln!(t, "vsync = {}", self.vsync);
        let _ = writeln!(t, "fullscreen = {}", self.fullscreen);
        let _ = writeln!(t, "shafts = {}", self.shafts);
        if let Some((w, h)) = self.window {
            let _ = writeln!(t, "window = {w}x{h}");
        }
        let _ = writeln!(t, "show_debug = {}", self.show_debug);
        let _ = writeln!(t, "minimap = {}", self.minimap);
        t
    }

    /// Reads settings, keeping the default for anything missing or unreadable, so a
    /// hand-edited or older file can never stop the game from starting.
    fn from_text(text: &str) -> Self {
        let f = Fields::parse(text);
        let d = Self::default();
        Self {
            seed: f.get("seed").unwrap_or(d.seed),
            radius: f.get::<i32>("radius").map_or(d.radius, |r| r.clamp(*RADIUS.start(), *RADIUS.end())),
            day_minutes: f.ranged("day_minutes", d.day_minutes, DAY_MINUTES),
            year_days: f.ranged("year_days", d.year_days, YEAR_DAYS),
            fov: f.ranged("fov", d.fov, FOV),
            sensitivity: f.ranged("sensitivity", d.sensitivity, SENSITIVITY),
            volume: f.ranged("volume", d.volume, VOLUME),
            vsync: f.get("vsync").unwrap_or(d.vsync),
            fullscreen: f.get("fullscreen").unwrap_or(d.fullscreen),
            shafts: f.get("shafts").unwrap_or(d.shafts),
            window: f.size("window"),
            show_debug: f.get("show_debug").unwrap_or(d.show_debug),
            minimap: f.get("minimap").unwrap_or(d.minimap),
        }
    }
}

/// Command-line choices, which hold for one run and are never written to the file.
#[derive(Clone, Copy, Debug, Default)]
pub struct Overrides {
    pub seed: Option<u32>,
    pub radius: Option<i32>,
    pub day_minutes: Option<f32>,
    pub vsync: Option<bool>,
    /// The start position was given by hand, so this run is a visit, not a playthrough.
    pub placed: bool,
    /// The seed was picked at random for a new game, so it isn't remembered as the
    /// default world (each world's own progress is still saved under its seed).
    pub random: bool,
}

impl Overrides {
    pub fn applied_to(&self, s: Settings) -> Settings {
        Settings {
            seed: self.seed.unwrap_or(s.seed),
            radius: self.radius.unwrap_or(s.radius),
            day_minutes: self.day_minutes.unwrap_or(s.day_minutes),
            vsync: self.vsync.unwrap_or(s.vsync),
            ..s
        }
    }

    /// Whether this run's position should replace the saved one.
    pub fn keeps_position(&self) -> bool {
        !self.placed
    }

    /// What to write back: this run's settings, minus whatever the command line only
    /// borrowed. The seed is the exception: choosing a world and playing in it is what
    /// "the world I was in" means, so it sticks unless the run was only a visit.
    pub fn worth_keeping(&self, live: Settings, saved: Settings) -> Settings {
        Settings {
            seed: if self.keeps_position() && !self.random { live.seed } else { saved.seed },
            radius: if self.radius.is_some() { saved.radius } else { live.radius },
            day_minutes: if self.day_minutes.is_some() { saved.day_minutes } else { live.day_minutes },
            vsync: if self.vsync.is_some() { saved.vsync } else { live.vsync },
            ..live
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Travel {
    Walking,
    Flying,
    Sailing,
}

/// Where the player left off in one world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// The feet when walking, the boat when sailing, the camera when flying.
    pub pos: DVec3,
    /// Radians.
    pub yaw: f32,
    /// Radians.
    pub pitch: f32,
    /// Hour of the day.
    pub time: f32,
    /// Time of year, 0 to 1 from the spring equinox.
    pub date: f32,
    pub travel: Travel,
    /// Which way the boat points, in radians.
    pub heading: f32,
}

impl Progress {
    fn to_text(self) -> String {
        let travel = match self.travel {
            Travel::Walking => "walking",
            Travel::Flying => "flying",
            Travel::Sailing => "sailing",
        };
        format!(
            "x = {}\ny = {}\nz = {}\nyaw = {}\npitch = {}\ntime = {}\ndate = {}\ntravel = {travel}\nheading = {}\n",
            self.pos.x, self.pos.y, self.pos.z, self.yaw, self.pitch, self.time, self.date, self.heading
        )
    }

    /// None unless the position and view are all there: a save is no use without them.
    fn from_text(text: &str) -> Option<Self> {
        let f = Fields::parse(text);
        let travel = match f.0.get("travel").copied() {
            Some("flying") => Travel::Flying,
            Some("sailing") => Travel::Sailing,
            _ => Travel::Walking,
        };
        Some(Self {
            pos: DVec3::new(f.finite("x")?, f.finite("y")?, f.finite("z")?),
            yaw: f.finite("yaw")? as f32,
            pitch: f.finite("pitch")? as f32,
            time: f.finite("time").map_or(10.0, |t| t.rem_euclid(24.0) as f32),
            date: f.finite("date").map_or(crate::season::DEFAULT_DATE, |d| d.rem_euclid(1.0) as f32),
            travel,
            heading: f.finite("heading").unwrap_or(0.0) as f32,
        })
    }
}

struct Fields<'a>(HashMap<&'a str, &'a str>);

impl<'a> Fields<'a> {
    fn parse(text: &'a str) -> Self {
        let lines = text.lines().map(|l| l.split('#').next().unwrap_or(""));
        Self(lines.filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect())
    }

    fn get<T: FromStr>(&self, key: &str) -> Option<T> {
        self.0.get(key)?.parse().ok()
    }

    fn finite(&self, key: &str) -> Option<f64> {
        self.get::<f64>(key).filter(|v| v.is_finite())
    }

    fn ranged(&self, key: &str, default: f32, range: RangeInclusive<f32>) -> f32 {
        self.finite(key).map_or(default, |v| (v as f32).clamp(*range.start(), *range.end()))
    }

    fn size(&self, key: &str) -> Option<(u32, u32)> {
        let (w, h) = self.0.get(key)?.split_once('x')?;
        let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
        (w >= 320 && h >= 200).then_some((w, h))
    }
}

/// The folder the game's files live in.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The per-user config folder, or None on a machine that doesn't have one.
    pub fn find() -> Option<Self> {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let dir = if cfg!(windows) {
            var("APPDATA")?.join("Willow Hollow")
        } else {
            var("XDG_CONFIG_HOME").or_else(|| Some(var("HOME")?.join(".config")))?.join("willow-hollow")
        };
        Some(Self { dir })
    }

    pub fn settings(&self) -> Settings {
        fs::read_to_string(self.dir.join(SETTINGS_FILE)).map_or_else(|_| Settings::default(), |t| Settings::from_text(&t))
    }

    pub fn progress(&self, seed: u32) -> Option<Progress> {
        Progress::from_text(&fs::read_to_string(self.progress_path(seed)).ok()?)
    }

    pub fn write_settings(&self, settings: Settings) -> bool {
        self.report(write_whole(&self.dir, SETTINGS_FILE.as_ref(), &settings.to_text()))
    }

    pub fn write_progress(&self, seed: u32, progress: Progress) -> bool {
        let path = self.progress_path(seed);
        self.report(write_whole(&self.dir, &path, &progress.to_text()))
    }

    /// The pack carried in a world, as it was saved: empty if it never was.
    pub fn pack(&self, seed: u32) -> String {
        fs::read_to_string(self.dir.join(self.pack_path(seed))).unwrap_or_default()
    }

    pub fn write_pack(&self, seed: u32, text: &str) -> bool {
        let path = self.pack_path(seed);
        self.report(write_whole(&self.dir, &path, text))
    }

    /// What has been set down in a world, as it was saved: nothing if it never was.
    pub fn placed(&self, seed: u32) -> String {
        fs::read_to_string(self.dir.join(format!("placed-{seed}.txt"))).unwrap_or_default()
    }

    pub fn write_placed(&self, seed: u32, text: &str) -> bool {
        self.report(write_whole(&self.dir, format!("placed-{seed}.txt").as_ref(), text))
    }

    fn pack_path(&self, seed: u32) -> PathBuf {
        PathBuf::from(format!("pack-{seed}.txt"))
    }

    /// Where the map of what has been seen of each world is kept.
    pub fn map_dir(&self, seed: u32) -> PathBuf {
        self.dir.join(format!("map-{seed}"))
    }

    /// Each world has its own save, so trying another seed doesn't lose your place.
    fn progress_path(&self, seed: u32) -> PathBuf {
        self.dir.join(format!("world-{seed}.txt"))
    }

    fn report(&self, result: io::Result<()>) -> bool {
        if let Err(e) = &result {
            log::warn!("could not save to {}: {e}", self.dir.display());
        }
        result.is_ok()
    }
}

/// Writes to a temporary file and renames it over the target, so that a crash halfway
/// through a save leaves the previous one intact.
fn write_whole(dir: &Path, path: &Path, text: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let target = dir.join(path);
    let temp = target.with_extension("tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, &target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress() -> Progress {
        Progress {
            pos: DVec3::new(36890.123456789, 41.5, -1234567.0625),
            yaw: 1.25,
            pitch: -0.3,
            time: 17.5,
            date: 0.62,
            travel: Travel::Sailing,
            heading: 2.5,
        }
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let s = Settings {
            seed: 77,
            radius: 40,
            day_minutes: 0.0,
            year_days: 60.0,
            fov: 90.0,
            sensitivity: 1.5,
            volume: 0.25,
            vsync: false,
            fullscreen: true,
            shafts: false,
            window: Some((1920, 1080)),
            show_debug: false,
            minimap: false,
        };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        assert_eq!(Settings::from_text(&Settings::default().to_text()), Settings::default());
    }

    #[test]
    fn a_damaged_settings_file_falls_back_to_defaults() {
        let s = Settings::from_text("fov = lots\nradius = 9000\nvolume = NaN\nwindow = 0x0\nnonsense\nsensitivity = 2 # faster");
        let d = Settings::default();
        assert_eq!(s.fov, d.fov);
        assert_eq!(s.radius, *RADIUS.end());
        assert_eq!(s.volume, d.volume);
        assert_eq!(s.window, None);
        assert_eq!(s.sensitivity, 2.0);
    }

    #[test]
    fn positions_come_back_exact() {
        assert_eq!(Progress::from_text(&progress().to_text()), Some(progress()));
    }

    #[test]
    fn a_save_without_a_usable_position_is_ignored() {
        assert_eq!(Progress::from_text(""), None);
        assert_eq!(Progress::from_text("x = 1\ny = inf\nz = 3\nyaw = 0\npitch = 0"), None);
    }

    #[test]
    fn the_command_line_applies_to_one_run_only() {
        let saved = Settings { radius: 30, ..Default::default() };
        let o = Overrides { radius: Some(8), vsync: Some(false), ..Default::default() };
        let live = o.applied_to(saved);
        assert_eq!((live.radius, live.vsync), (8, false));
        // A change made in the menu during that run is still kept.
        let kept = o.worth_keeping(Settings { fov: 95.0, ..live }, saved);
        assert_eq!((kept.radius, kept.vsync, kept.fov), (30, true, 95.0));
    }

    #[test]
    fn the_seed_sticks_unless_the_run_was_only_a_visit() {
        let saved = Settings::default();
        let played = Overrides { seed: Some(9), ..Default::default() };
        assert_eq!(played.worth_keeping(played.applied_to(saved), saved).seed, 9);
        let visit = Overrides { placed: true, ..played };
        assert_eq!(visit.worth_keeping(visit.applied_to(saved), saved).seed, saved.seed);
        assert!(!visit.keeps_position());
        let new_game = Overrides { random: true, ..played };
        assert_eq!(new_game.worth_keeping(new_game.applied_to(saved), saved).seed, saved.seed);
    }

    #[test]
    fn files_are_written_and_read_back_per_world() {
        let dir = std::env::temp_dir().join(format!("willow-hollow-test-{}", std::process::id()));
        let store = Store { dir: dir.clone() };
        assert!(store.progress(1).is_none());
        assert_eq!(store.settings(), Settings::default());

        let settings = Settings { fov: 85.0, ..Default::default() };
        assert!(store.write_settings(settings));
        assert!(store.write_progress(1, progress()));
        assert_eq!(store.settings(), settings);
        assert_eq!(store.progress(1), Some(progress()));
        assert!(store.progress(2).is_none(), "worlds keep separate saves");
        let _ = fs::remove_dir_all(dir);
    }
}
