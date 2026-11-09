use glam::Vec3;

/// Lighting and sky state for a moment in the day. All colours are linear.
pub struct Environment {
    /// Hours, 0..24.
    pub time: f32,
    /// Unit vector toward the light (sun by day, moon by night).
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub sky_zenith: Vec3,
    pub sky_horizon: Vec3,
    pub fog_density: f32,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Environment {
    pub fn at_time(hours: f32) -> Self {
        let time = hours.rem_euclid(24.0);
        // Sun rises at 6, peaks at 12, sets at 18, on a tilted arc.
        let angle = (time - 6.0) / 12.0 * std::f32::consts::PI;
        let sun = Vec3::new(angle.cos() * 0.85, angle.sin(), 0.42).normalize();
        let elev = sun.y;

        let day = smoothstep(-0.02, 0.35, elev);
        let night = smoothstep(0.05, -0.22, elev);
        let dusk = (1.0 - day - night).max(0.0);

        let zenith = Vec3::new(0.14, 0.30, 0.68) * day
            + Vec3::new(0.10, 0.14, 0.36) * dusk
            + Vec3::new(0.004, 0.007, 0.020) * night;
        let horizon = Vec3::new(0.52, 0.68, 0.88) * day
            + Vec3::new(0.95, 0.46, 0.22) * dusk
            + Vec3::new(0.012, 0.018, 0.040) * night;

        let (sun_dir, sun_color) = if elev > -0.06 {
            let warm = smoothstep(0.45, 0.0, elev);
            let col = Vec3::new(1.0, 0.95, 0.86).lerp(Vec3::new(1.0, 0.50, 0.22), warm);
            (sun, col * 1.35 * smoothstep(-0.06, 0.12, elev))
        } else {
            // Moonlight from the opposite side of the sky.
            let moon = Vec3::new(-sun.x, (-sun.y).max(0.15), -sun.z).normalize();
            (moon, Vec3::new(0.35, 0.45, 0.70) * 0.05 * smoothstep(-0.06, -0.2, elev))
        };

        // Eye adaptation: a high sun puts far more light on the ground than a low one, so
        // expose down toward midday. Every light source is one of these three colours,
        // so scaling them here exposes the whole scene.
        let exposure = 1.0 / (0.75 + 1.1 * elev.max(0.0));

        Self {
            time,
            sun_dir,
            sun_color: sun_color * exposure,
            sky_zenith: zenith * exposure,
            sky_horizon: horizon * exposure,
            fog_density: 0.00013,
        }
    }
}
