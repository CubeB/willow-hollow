//! The figure on the figure screen: the very boxes the game draws the player with
//! (`character::draw`, the furled glider on its back), drawn here into a picture for the
//! screen rather than into the world, so that it shows the same by day or night,
//! underground or afloat, and on a screenshot. Each box is turned as the props shader
//! turns it, lit from one side and the sky, and kept or hidden by its depth, a pixel at a
//! time; the picture is drawn at twice its size and halved, for soft edges.

use glam::{DVec3, Vec3};
use worldgen::scale::m;

use crate::character::{self, Look, Task};

/// The view: how much of the figure's height it takes in, from a little below the soles
/// to above the tallest hat, in metres, and how wide it looks.
const BELOW: f32 = 0.08;
const ABOVE: f32 = 2.3;
const FOV: f32 = 0.42;
/// Where the light comes from: up to the left of the view, a little in front.
const LIGHT: Vec3 = Vec3::new(0.55, 0.75, -0.38);

/// A picture `size` pixels of `look` standing at ease, turned `turn` radians from facing
/// the view, `clock` seconds into its breathing; and where its soles are in it, and how
/// wide a metre is there, in pixels, for a shadow under them.
pub fn picture(look: &Look, turn: f32, clock: f32, size: [usize; 2]) -> (egui::ColorImage, egui::Pos2, f32) {
    let [w, h] = size.map(|v| v.max(1));
    let (sw, sh) = (2 * w, 2 * h);
    // Facing the view, which looks along +z, and turned from there.
    let facing = -std::f32::consts::FRAC_PI_2 + turn;
    let pose = character::task_pose(Task::Stand, &look.build, clock, 0.0);
    let mut boxes = Vec::new();
    character::draw(look, DVec3::ZERO, facing, &pose, Some(0.0), DVec3::ZERO, &mut boxes);

    // Far enough back for the whole height to fit, looking a little down at it.
    let span = m(ABOVE + BELOW);
    let back = span * 0.5 / (FOV * 0.5).tan() * 1.04;
    let target = Vec3::new(0.0, m((ABOVE - BELOW) * 0.5), 0.0);
    let eye = target + Vec3::new(0.0, back * 0.12, -back);
    let forward = (target - eye).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let focal = sh as f32 * 0.5 / (FOV * 0.5).tan();
    let project = |p: Vec3| {
        let d = p - eye;
        let z = d.dot(forward).max(1e-3);
        Vec3::new(sw as f32 * 0.5 + d.dot(right) / z * focal, sh as f32 * 0.5 - d.dot(up) / z * focal, 1.0 / z)
    };

    // The nearest face at each sample: its nearness (1 / depth) and its colour.
    let mut near = vec![0.0f32; sw * sh];
    let mut colour = vec![[0u8; 3]; sw * sh];
    let light = LIGHT.normalize();
    for b in &boxes {
        let centre = Vec3::from(b.pos);
        let size = Vec3::from(b.size);
        let corner = |x: f32, y: f32, z: f32| centre + turn_box(Vec3::new(x, y, z) * size, b.yaw, b.tilt);
        for axis in 0..3 {
            for sign in [-0.5f32, 0.5] {
                let normal = turn_box(Vec3::from(std::array::from_fn::<f32, 3, _>(|k| if k == axis { sign * 2.0 } else { 0.0 })), b.yaw, b.tilt);
                // The face's corners, round it: the other two axes at each of their ends.
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let at = |a: f32, c: f32| {
                    let mut p = [0.0f32; 3];
                    p[axis] = sign;
                    p[u] = a;
                    p[v] = c;
                    corner(p[0], p[1], p[2])
                };
                let quad = [at(-0.5, -0.5), at(0.5, -0.5), at(0.5, 0.5), at(-0.5, 0.5)];
                let middle = (quad[0] + quad[2]) * 0.5;
                if normal.dot(eye - middle) <= 0.0 {
                    continue;
                }
                // Lit as the props are: the light on the face, and the sky's from above.
                let lit = 0.36 + 0.14 * normal.y + 0.78 * normal.dot(light).max(0.0);
                let c = std::array::from_fn(|k| srgb(b.color[k] * lit));
                let q = quad.map(project);
                fill(&mut near, &mut colour, (sw, sh), [q[0], q[1], q[2]], c);
                fill(&mut near, &mut colour, (sw, sh), [q[0], q[2], q[3]], c);
            }
        }
    }

    // Halved: each pixel the mean of its four samples, as much there as they are covered.
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut covered) = ([0u32; 3], 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let k = (2 * y + dy) * sw + 2 * x + dx;
                if near[k] > 0.0 {
                    covered += 1;
                    for (s, c) in sum.iter_mut().zip(colour[k]) {
                        *s += c as u32;
                    }
                }
            }
            // Colours already weighted by how much is covered, as egui keeps them.
            pixels.push(egui::Color32::from_rgba_premultiplied((sum[0] / 4) as u8, (sum[1] / 4) as u8, (sum[2] / 4) as u8, (covered * 255 / 4) as u8));
        }
    }
    let soles = project(Vec3::ZERO);
    let metre = (project(Vec3::new(m(0.5), 0.0, 0.0)).x - project(Vec3::new(-m(0.5), 0.0, 0.0)).x).abs();
    (egui::ColorImage::new([w, h], pixels), egui::pos2(soles.x * 0.5, soles.y * 0.5), metre * 0.5)
}

/// A box's corner turned as the props shader turns it: rolled about its own z, pitched
/// about its own x, then turned about the upright.
fn turn_box(v: Vec3, yaw: f32, tilt: [f32; 2]) -> Vec3 {
    let (sr, cr) = tilt[1].sin_cos();
    let a = Vec3::new(v.x * cr - v.y * sr, v.x * sr + v.y * cr, v.z);
    let (sp, cp) = tilt[0].sin_cos();
    let b = Vec3::new(a.x, a.y * cp - a.z * sp, a.y * sp + a.z * cp);
    let (s, c) = yaw.sin_cos();
    Vec3::new(b.x * c - b.z * s, b.y, b.x * s + b.z * c)
}

/// From linear light back to the screen's sRGB.
fn srgb(v: f32) -> u8 {
    (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8
}

/// Fills a triangle given in pixels (x, y and nearness), keeping each sample it covers
/// where it is nearer than what is there.
fn fill(near: &mut [f32], colour: &mut [[u8; 3]], (w, h): (usize, usize), t: [Vec3; 3], c: [u8; 3]) {
    let area = (t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[2].x - t[0].x) * (t[1].y - t[0].y);
    if area.abs() < 1e-6 {
        return;
    }
    let lo_x = t.iter().map(|p| p.x).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let hi_x = (t.iter().map(|p| p.x).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(w);
    let lo_y = t.iter().map(|p| p.y).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let hi_y = (t.iter().map(|p| p.y).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(h);
    let edge = |a: Vec3, b: Vec3, x: f32, y: f32| (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
    for y in lo_y..hi_y {
        for x in lo_x..hi_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // How far toward each corner, from the edge opposite it.
            let a = edge(t[1], t[2], px, py) / area;
            let b = edge(t[2], t[0], px, py) / area;
            let g = 1.0 - a - b;
            if a < 0.0 || b < 0.0 || g < 0.0 {
                continue;
            }
            // Nearness is even across the picture, as depth is not.
            let n = a * t[0].z + b * t[1].z + g * t[2].z;
            let k = y * w + x;
            if n > near[k] {
                near[k] = n;
                colour[k] = c;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_figure_stands_in_the_middle_of_its_picture_feet_down() {
        let (img, soles, metre) = picture(&character::WANDERER, 0.4, 0.0, [120, 200]);
        assert_eq!(img.size, [120, 200]);
        let seen = |x: usize, y: usize| img.pixels[y * 120 + x].a() > 0;
        // Something of it in the middle, and nothing at the corners.
        assert!((60..140).any(|y| seen(60, y)));
        assert!(!seen(0, 0) && !seen(119, 199) && !seen(0, 199) && !seen(119, 0));
        // Its feet near the soles, its head above them by its height.
        let rows: Vec<usize> = (0..200).filter(|&y| (0..120).any(|x| seen(x, y))).collect();
        let (top, bottom) = (rows[0] as f32, *rows.last().unwrap() as f32);
        // The toes, nearer and seen from a little above, come lower than the middle of the soles.
        assert!((bottom - soles.y).abs() < 8.0, "the feet at {bottom}, the soles at {}", soles.y);
        let tall = (bottom - top) / metre;
        assert!((tall - 1.76).abs() < 0.2, "{tall} m tall in the picture");
    }

    #[test]
    fn a_taller_figure_stands_taller_in_the_picture() {
        let height = |metres: f32| {
            let mut f = super::super::Figure::default();
            f.choose(super::super::Choice::Height(metres));
            let (img, ..) = picture(&f.look, 0.0, 0.0, [120, 200]);
            (0..200).find(|&y| (0..120).any(|x| img.pixels[y * 120 + x].a() > 0)).unwrap()
        };
        assert!(height(1.95) + 8 < height(1.5), "the crown should be higher");
    }

    /// How long a picture takes to draw, as the screen draws it each frame it is open.
    #[test]
    #[ignore]
    fn time_a_picture() {
        let t = std::time::Instant::now();
        for k in 0..100 {
            std::hint::black_box(picture(&character::WANDERER, k as f32 * 0.1, k as f32 * 0.016, [300, 450]));
        }
        println!("a 300x450 picture in {:.2} ms", t.elapsed().as_secs_f64() * 10.0);
    }
}
