use glam::Vec3;
use worldgen::scale::{VOXELS_PER_METRE, m};

/// Lighting and sky state for a moment in the day. All colours are linear.
#[derive(Clone)]
pub struct Environment {
    /// Hours, 0..24.
    pub time: f32,
    /// Unit vector toward the light (sun by day, moon by night).
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub sky_zenith: Vec3,
    /// The horizon away from the sun, and toward it: at dusk the warm colour gathers
    /// around where the sun went down.
    pub sky_horizon: Vec3,
    pub sky_glow: Vec3,
    /// How far into dusk or dawn the sky is, 0..1.
    pub glow: f32,
    /// Unit vector toward the sun itself, even at night when the light is the moon.
    pub sun_pos: Vec3,
    /// Clear air, per voxel: it takes the blue out of the distance first.
    pub fog_density: f32,
    /// Haze, per voxel at sea level, and the height over which it thins to about a third.
    pub haze: f32,
    pub haze_height: f32,
    /// True by day; at night the light is the moon.
    pub daylight: bool,
    /// Ground mist strength (dawn and dusk), and the cloud cover 0..1.
    pub mist: f32,
    /// The height of the low ground the mist lies on, in voxels above sea level.
    pub mist_floor: f32,
    pub clouds: f32,
    /// The height of the clouds' flat bases above sea level, in voxels: `CLOUD_HEIGHT`
    /// over low country, and higher where the hills round about would stand up into them.
    pub cloud_base: f32,
    /// How far round the clouds' slow cycle of forming and clearing the sky has come, in
    /// radians (see `weather::CHURN`), and how tall the heaps stand for their cover, 0..1
    /// with a half the usual: they tower ahead of rain and sink as it clears.
    pub cloud_churn: f32,
    pub cloud_swell: f32,
    /// Sine of the sun's elevation; negative at night.
    pub sun_elevation: f32,
    /// Time of year, 0 to 1 from the spring equinox.
    pub date: f32,
    /// How bright the northern lights are, 0 to 1 (see aurora.rs); 0 unless set.
    pub aurora: f32,
}

/// Height of the cloud base above sea level over low country, in voxels.
pub const CLOUD_HEIGHT: f32 = m(240.0);
/// How far the cloud base keeps above the highest ground round about (the thick of a
/// cloud hangs a block or two below its base).
pub const CLOUD_CLEARANCE: f32 = m(60.0);

/// Haze at sea level in temperate air, per metre.
const HAZE: f32 = 0.00030;

/// How many times brighter everything is to an eye fully used to the dark of a cave.
const SHUT_IN_GAIN: f32 = 6.0;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Environment {
    /// The air of a place: `humidity` is its biome's (season::humidity, 1 for temperate
    /// woodland), and ground still `wet` from rain that has stopped has washed the haze out.
    pub fn air(&mut self, humidity: f32, wet: f32) {
        self.haze *= humidity * (1.0 - 0.45 * wet.clamp(0.0, 1.0));
    }

    /// The eye opening up in the dark of a cave: `shut_in` is how enclosed by rock the
    /// viewer is (`player::enclosure`). Every light is brighter for it, so the daylight
    /// outside the mouth burns out as the walls come into view.
    pub fn adapt(&mut self, shut_in: f32) {
        let gain = 1.0 + (SHUT_IN_GAIN - 1.0) * smoothstep(0.4, 0.95, shut_in);
        self.sun_color *= gain;
        self.sky_zenith *= gain;
        self.sky_horizon *= gain;
        self.sky_glow *= gain;
    }

    /// How hazy the air is beside a temperate clear day, 0..1 (for the sun's glow in it).
    pub fn haziness(&self) -> f32 {
        (self.haze * VOXELS_PER_METRE / HAZE).clamp(0.0, 1.5) / 1.5
    }

    /// The environment at an hour of the day and a time of year (0..1).
    pub fn at(hours: f32, date: f32) -> Self {
        let time = hours.rem_euclid(24.0);
        // The sun stands higher and rides the sky longer in summer, lower and shorter in winter.
        let decl = crate::season::declination(date);
        // Sun rises at 6, peaks at 12, sets at 18, on a tilted arc.
        let angle = (time - 6.0) / 12.0 * std::f32::consts::PI;
        let sun = Vec3::new(angle.cos() * 0.85, angle.sin() + 0.3 * decl, 0.42 - 0.25 * decl).normalize();
        let elev = sun.y;

        let day = smoothstep(-0.02, 0.35, elev);
        let night = smoothstep(0.05, -0.22, elev);
        let dusk = (1.0 - day - night).max(0.0);
        let dusk_sky = dusk;

        // The night sky is a deep blue-black overhead and a little paler toward the
        // horizon: dark enough for the stars, and a fire, to stand out against it.
        let zenith = Vec3::new(0.14, 0.30, 0.68) * day
            + Vec3::new(0.10, 0.14, 0.36) * dusk
            + Vec3::new(0.0035, 0.0060, 0.016) * night;
        let horizon = Vec3::new(0.52, 0.68, 0.88) * day
            + Vec3::new(0.40, 0.36, 0.52) * dusk
            + Vec3::new(0.010, 0.014, 0.028) * night;
        let glow = Vec3::new(0.60, 0.72, 0.88) * day
            + Vec3::new(0.95, 0.46, 0.22) * dusk
            + Vec3::new(0.010, 0.014, 0.028) * night;

        let daylight = elev > -0.06;
        let (sun_dir, sun_color) = if daylight {
            let warm = smoothstep(0.6, 0.03, elev);
            let col = Vec3::new(1.0, 0.95, 0.86).lerp(Vec3::new(1.0, 0.50, 0.22), warm);
            (sun, col * 1.35 * smoothstep(-0.06, 0.12, elev))
        } else {
            // Moonlight from the opposite side of the sky: faint, cold and grey, enough to
            // make out the lie of the land by and little of its colour.
            let moon = Vec3::new(-sun.x, (-sun.y).max(0.15), -sun.z).normalize();
            (moon, Vec3::new(0.40, 0.50, 0.78) * 0.095 * smoothstep(-0.06, -0.2, elev))
        };

        // Eye adaptation: a high sun puts far more light on the ground than a low one, so
        // expose down toward midday. Every light source is one of these three colours,
        // so scaling them here exposes the whole scene.
        let exposure = 1.0 / (0.75 + 1.1 * elev.max(0.0)) * (1.0 + 0.5 * smoothstep(-0.02, -0.22, elev));
        // Mist gathers in the valleys around dawn, and a little at dusk.
        let dawn = smoothstep(3.5, 6.0, time) * smoothstep(9.5, 7.0, time);
        let dusk = smoothstep(17.0, 18.5, time) * smoothstep(21.0, 19.5, time);
        let mist = (dawn * 0.004375 + dusk * 0.0015) / VOXELS_PER_METRE * exposure.min(1.0);

        Self {
            time,
            sun_dir,
            sun_color: sun_color * exposure * (1.0 + 0.25 * (-decl).max(0.0)),
            sky_zenith: zenith * exposure,
            sky_horizon: horizon * exposure,
            sky_glow: glow * exposure,
            glow: dusk_sky,
            sun_pos: sun,
            fog_density: 0.00007 / VOXELS_PER_METRE,
            haze: HAZE / VOXELS_PER_METRE,
            haze_height: 160.0 * VOXELS_PER_METRE,
            daylight,
            mist,
            mist_floor: 0.0,
            clouds: 0.45,
            cloud_base: CLOUD_HEIGHT,
            cloud_churn: 0.0,
            cloud_swell: 0.5,
            sun_elevation: elev,
            date,
            aurora: 0.0,
        }
    }
}
