//! Footsteps, made as they play from noise and a low tone.
//!
//! [`Mixer`] is plain arithmetic with no audio device behind it, so it can be tested,
//! and it never allocates, so it is safe on the audio thread.

use std::f32::consts::{PI, TAU};

/// What a foot lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    Grass,
    Earth,
    Stone,
    Sand,
    Gravel,
    Snow,
    Wood,
    Shallows,
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// White noise, -1 to 1.
    fn noise(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// A state-variable filter.
#[derive(Clone, Copy, Default)]
struct Filter {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl Filter {
    fn tune(&mut self, rate: f32, freq: f32, q: f32) {
        let g = (PI * (freq / rate).min(0.45)).tan();
        let k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Returns (low-pass, band-pass).
    fn run(&mut self, x: f32) -> (f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1)
    }
}

/// A short filtered burst of noise with a low thump under it.
#[derive(Clone, Copy, Default)]
struct Footfall {
    filter: Filter,
    noise: f32,
    noise_decay: f32,
    thump: f32,
    thump_decay: f32,
    thump_phase: f32,
    thump_step: f32,
    /// Gravel crunches: the noise switches on and off in grains.
    grain: f32,
    grain_chance: f32,
}

impl Footfall {
    fn next(&mut self, rng: &mut Rng) -> f32 {
        if self.grain_chance > 0.0 && rng.unit() < self.grain_chance {
            self.grain = rng.unit();
        }
        let grain = if self.grain_chance > 0.0 { self.grain } else { 1.0 };
        let (low, band) = self.filter.run(rng.noise() * grain);
        let hiss = (band * 1.4 + low * 0.4) * self.noise;
        self.thump_phase = (self.thump_phase + self.thump_step).rem_euclid(TAU);
        let s = hiss + self.thump_phase.sin() * self.thump;
        self.noise *= self.noise_decay;
        self.thump *= self.thump_decay;
        s
    }

    fn done(&self) -> bool {
        self.noise < 1e-3 && self.thump < 1e-3
    }
}

/// Plays footsteps, one buffer at a time.
pub struct Mixer {
    rate: f32,
    rng: Rng,
    volume: f32,
    steps: [Footfall; 4],
}

impl Mixer {
    pub fn new(rate: f32, seed: u32, volume: f32) -> Self {
        Self {
            rate,
            rng: Rng(seed.wrapping_mul(0x9e37_79b9) | 1),
            volume,
            steps: [Footfall::default(); 4],
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    /// Starts a footstep; `loud` is 0 to 1.
    pub fn step(&mut self, ground: Ground, loud: f32) {
        // Cutoff, resonance, length of the scuff, weight of the heel, and heel pitch.
        let (freq, q, seconds, heel, pitch) = match ground {
            Ground::Grass => (640.0, 0.6, 0.075, 0.18, 80.0),
            Ground::Earth => (850.0, 0.7, 0.05, 0.45, 85.0),
            Ground::Stone => (3400.0, 1.3, 0.028, 0.06, 240.0),
            Ground::Sand => (2300.0, 0.6, 0.09, 0.1, 80.0),
            Ground::Gravel => (3000.0, 0.8, 0.11, 0.18, 110.0),
            Ground::Snow => (1700.0, 0.6, 0.13, 0.12, 80.0),
            Ground::Wood => (760.0, 2.6, 0.05, 0.6, 190.0),
            Ground::Shallows => (1050.0, 0.7, 0.14, 0.05, 90.0),
        };
        // No two steps quite alike, or walking sounds like a machine.
        let vary = self.rng.range(0.82, 1.2);
        let loud = loud * self.rng.range(0.8, 1.0) * 0.34;
        let mut foot = Footfall {
            noise: loud,
            noise_decay: (-1.0 / (seconds * self.rate)).exp(),
            thump: loud * heel,
            thump_decay: (-1.0 / (0.04 * self.rate)).exp(),
            thump_step: pitch * vary / self.rate * TAU,
            grain_chance: if ground == Ground::Gravel { 0.01 } else { 0.0 },
            grain: 1.0,
            ..Default::default()
        };
        foot.filter.tune(self.rate, freq * vary, q);
        if let Some(slot) = self.steps.iter_mut().find(|s| s.done()) {
            *slot = foot;
        }
    }

    /// Fills a buffer of interleaved stereo samples.
    pub fn render(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<2>().0 {
            let mut feet = 0.0;
            for s in self.steps.iter_mut().filter(|s| !s.done()) {
                feet += s.next(&mut self.rng);
            }
            let sample = (feet * self.volume).tanh();
            *frame = [sample, sample];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
    }

    /// Power in a band of frequencies, summed over evenly spaced bins.
    fn band(samples: &[f32], lo: f32, hi: f32) -> f32 {
        (0..48)
            .map(|k| {
                let freq = lo + (hi - lo) * k as f32 / 47.0;
                let w = TAU * freq / RATE;
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (n, s) in samples.iter().enumerate() {
                    re += s * (w * n as f32).cos();
                    im += s * (w * n as f32).sin();
                }
                re * re + im * im
            })
            .sum()
    }

    fn footstep(ground: Ground, volume: f32) -> Vec<f32> {
        let mut mixer = Mixer::new(RATE, 9, volume);
        mixer.step(ground, 1.0);
        let mut out = vec![0.0; 2 * (0.5 * RATE) as usize];
        mixer.render(&mut out);
        out.as_chunks::<2>().0.iter().map(|f| f[0]).collect()
    }

    #[test]
    fn nothing_plays_without_a_step() {
        let mut mixer = Mixer::new(RATE, 3, 1.0);
        let mut out = vec![1.0; 9600];
        mixer.render(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn stone_sounds_harder_than_grass_and_steps_die_away() {
        let (stone, grass) = (footstep(Ground::Stone, 1.0), footstep(Ground::Grass, 1.0));
        let brightness = |s: &[f32]| band(s, 2500.0, 6000.0) / band(s, 100.0, 800.0);
        assert!(brightness(&stone) > brightness(&grass) * 1.5);
        assert!(rms(&grass[..2400]) > 5e-3, "too quiet to hear");
        assert!(rms(&grass[(0.35 * RATE) as usize..]) < 1e-3, "still ringing");
    }

    #[test]
    fn many_steps_at_once_stay_in_range() {
        let mut mixer = Mixer::new(RATE, 5, 1.0);
        let mut out = vec![0.0; 4096];
        for i in 0..400 {
            mixer.step(if i % 2 == 0 { Ground::Wood } else { Ground::Gravel }, 1.0);
            mixer.render(&mut out);
            assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "buffer {i}");
        }
    }

    #[test]
    fn volume_scales_the_output() {
        let (quiet, loud) = (rms(&footstep(Ground::Earth, 0.2)), rms(&footstep(Ground::Earth, 0.8)));
        assert!(loud > quiet * 3.0, "{quiet} against {loud}");
    }

    #[test]
    fn the_same_seed_makes_the_same_sound() {
        assert_eq!(footstep(Ground::Gravel, 1.0), footstep(Ground::Gravel, 1.0));
    }
}
