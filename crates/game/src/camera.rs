use std::collections::HashSet;

use glam::{DVec3, Mat4, Vec3, Vec4};
use winit::keyboard::KeyCode;

/// World-space camera. Position is f64 so precision holds anywhere in an endless world;
/// everything sent to the GPU is relative to the camera.
pub struct Camera {
    pub pos: DVec3,
    /// Radians. 0 looks along +X, increasing turns right (toward +Z).
    pub yaw: f32,
    /// Radians, positive looks up.
    pub pitch: f32,
    /// Radians, positive leans to the right. Only the glider uses it.
    pub roll: f32,
    pub fov_y: f32,
    pub near: f32,
}

impl Camera {
    pub fn new(pos: DVec3, yaw_deg: f32, pitch_deg: f32) -> Self {
        Self {
            pos,
            yaw: yaw_deg.to_radians(),
            pitch: pitch_deg.to_radians(),
            roll: 0.0,
            fov_y: 70f32.to_radians(),
            near: worldgen::scale::m(0.08),
        }
    }

    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cy * cp, sp, sy * cp)
    }

    pub fn right(&self) -> Vec3 {
        let f = self.forward();
        let level = f.cross(Vec3::Y).normalize();
        if self.roll == 0.0 {
            return level;
        }
        let up = level.cross(f);
        level * self.roll.cos() - up * self.roll.sin()
    }

    /// View-projection with the camera at the origin (rotation only), reverse-Z with an
    /// infinite far plane: depth is 1 at the near plane and approaches 0 at infinity.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        // Written out by hand: right-handed, view space looks down -Z, NDC depth in [0, 1].
        let f = self.forward();
        let s = self.right();
        let u = s.cross(f);
        let view = Mat4::from_cols(
            Vec4::new(s.x, u.x, -f.x, 0.0),
            Vec4::new(s.y, u.y, -f.y, 0.0),
            Vec4::new(s.z, u.z, -f.z, 0.0),
            Vec4::W,
        );
        let g = 1.0 / (self.fov_y * 0.5).tan();
        let proj = Mat4::from_cols(
            Vec4::new(g / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, g, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, self.near, 0.0),
        );
        proj * view
    }

    pub fn add_look(&mut self, dx: f32, dy: f32) {
        const SENS: f32 = 0.0022;
        self.yaw = (self.yaw + dx * SENS).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch - dy * SENS).clamp(-1.55, 1.55);
    }
}

/// Free-flying debug/spectator movement.
pub struct FlyController {
    pub speed: f32,
}

impl FlyController {
    pub fn update(&self, cam: &mut Camera, keys: &HashSet<KeyCode>, dt: f32) {
        let fwd = cam.forward();
        let right = cam.right();
        let mut dir = Vec3::ZERO;
        let down = |k: KeyCode| keys.contains(&k);
        if down(KeyCode::KeyW) {
            dir += fwd;
        }
        if down(KeyCode::KeyS) {
            dir -= fwd;
        }
        if down(KeyCode::KeyD) {
            dir += right;
        }
        if down(KeyCode::KeyA) {
            dir -= right;
        }
        if down(KeyCode::Space) {
            dir += Vec3::Y;
        }
        if down(KeyCode::ControlLeft) {
            dir -= Vec3::Y;
        }
        let boost = if down(KeyCode::ShiftLeft) { 5.0 } else { 1.0 };
        let step = dir.normalize_or_zero() * self.speed * boost * dt;
        cam.pos += step.as_dvec3();
    }

    pub fn scroll(&mut self, lines: f32) {
        self.speed = (self.speed * 1.2f32.powf(lines)).clamp(1.0, 2000.0);
    }
}
