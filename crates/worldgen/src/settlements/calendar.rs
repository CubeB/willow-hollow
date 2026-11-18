//! The settlements' calendar (#207): which days are market days, and when the festivals
//! fall. Both are pure functions of the world's seed and which day it is, so the game,
//! anything later that asks (a quest, a traveller's timetable) and every player agree on
//! them without anything being saved.
//!
//! A day is numbered from the world's first, turning at midnight (the game's
//! `villagers::day_number`). A settlement with stalls holds its market every few days on
//! a beat of its own: a village one day in six, a large village one in five, a town one
//! in three and a great town every other day. Hamlets have none; their people go to the
//! village's.
//!
//! The festivals are midsummer and midwinter, the two days about each solstice: the eve
//! and the day. The date (0 to 1 from the spring equinox, as the game keeps it) puts
//! midsummer at a quarter of the way round the year and midwinter at three quarters.
//!
//! Nothing here is built: the stalls stand bare in the world between market days (their
//! frame and counter only), and the game sets out their awnings, goods and keepers on
//! market day, and the festivals' lanterns and garlands, from what [`Stall`] and the
//! layout record.

use glam::IVec3;
use voxel::Voxel;

use super::{Kind, Settlement};
use crate::noise::hash2;

/// The hours a market keeps on its day: the stalls are dressed and kept from the first to
/// the second.
pub const MARKET_OPENS: f32 = 8.0;
pub const MARKET_CLOSES: f32 = 16.0;

/// How often a settlement of `kind` holds its market: one day in this many, or never
/// (0) for a hamlet.
pub fn market_every(kind: Kind) -> i64 {
    match kind {
        Kind::Hamlet => 0,
        Kind::Village => 6,
        Kind::LargeVillage => 5,
        Kind::Town => 3,
        Kind::GreatTown => 2,
    }
}

/// Which day of its beat a settlement of `kind` whose seed is `seed` holds its market on,
/// so that neighbours' markets fall on different days as often as not.
pub fn market_offset(kind: Kind, seed: u32) -> i64 {
    let every = market_every(kind);
    if every == 0 { 0 } else { (hash2(seed ^ 0x3a7e_d0a1, kind as i32, 7) % every as u32) as i64 }
}

/// Whether day `day` (from the world's first) is market day in a settlement of `kind`
/// whose seed is `seed`.
pub fn is_market_day_of(kind: Kind, seed: u32, day: i64) -> bool {
    let every = market_every(kind);
    every > 0 && day.rem_euclid(every) == market_offset(kind, seed)
}

/// Whether day `day` is market day in `s`.
pub fn is_market_day(s: &Settlement, day: i64) -> bool {
    is_market_day_of(s.kind, s.seed, day)
}

/// The next market day in `s` on or after `day`, if it holds one.
pub fn next_market_day(s: &Settlement, day: i64) -> Option<i64> {
    let every = market_every(s.kind);
    (every > 0).then(|| day + (market_offset(s.kind, s.seed) - day).rem_euclid(every))
}

/// The two festivals of the year.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Festival {
    /// Garlands on the doors, the well and the market; the village tree's ribbons tied
    /// fresh.
    Midsummer,
    /// Lanterns in the village tree and along the lanes, lit through the long nights.
    Midwinter,
}

impl Festival {
    pub const ALL: [Festival; 2] = [Festival::Midsummer, Festival::Midwinter];

    /// The time of year its solstice falls, as the game's date has it (0 to 1 from the
    /// spring equinox).
    pub fn solstice(self) -> f32 {
        match self {
            Festival::Midsummer => 0.25,
            Festival::Midwinter => 0.75,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Festival::Midsummer => "Midsummer",
            Festival::Midwinter => "Midwinter",
        }
    }

    /// How far round the year the date `date` is from its solstice last falling: 0 on
    /// it, to nearly 1 the day before it comes again.
    pub fn since(self, date: f32) -> f32 {
        (date - self.solstice()).rem_euclid(1.0)
    }
}

/// The festival day `day` falls in, with `year_days` days to the year: the day either
/// side of each solstice, the eve and the day, whose middles are within a day of it.
pub fn festival_on(day: i64, year_days: f32) -> Option<Festival> {
    let year = year_days.max(1.0) as f64;
    let middle = ((day as f64 + 0.5) / year).rem_euclid(1.0) as f32;
    let reach = (1.0 / year) as f32;
    Festival::ALL.into_iter().find(|f| {
        let d = (middle - f.solstice()).rem_euclid(1.0);
        d.min(1.0 - d) < reach
    })
}

/// A market stall on a green as the game dresses it on market day. The stall is stamped
/// bare, its frame and counter only; on market day the game stretches its awning over the
/// rails and sets out goods along the counter, and its keeper stands behind it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stall {
    /// Where its model's column 0 stands, continent-local, and its quarter turns (as a
    /// fitting's: it faces `fittings::front(turns)`).
    pub at: IVec3,
    pub turns: u32,
    /// The tops of its back and front rails, as its `Hook::Awning` gives them once
    /// placed: the back left and the front right corner, continent-local cells. None for
    /// a stall fallen into disrepair, which stands empty even on market day.
    pub awning: Option<(IVec3, IVec3)>,
    /// The two corners of its counter's top, continent-local cells, the first the lower:
    /// what goods stand on.
    pub counter: (IVec3, IVec3),
    /// The colour of its cloth.
    pub colour: Voxel,
    pub seed: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_days_come_round_at_each_kinds_rate_and_the_same_every_time() {
        for kind in Kind::ALL {
            for seed in [1u32, 77, 0xdead_beef, 123_456] {
                let every = market_every(kind);
                let days: Vec<i64> = (0..600).filter(|&d| is_market_day_of(kind, seed, d)).collect();
                let again: Vec<i64> = (0..600).filter(|&d| is_market_day_of(kind, seed, d)).collect();
                assert_eq!(days, again);
                if every == 0 {
                    assert!(days.is_empty(), "a hamlet holds no market");
                    continue;
                }
                assert_eq!(days.len() as i64, 600 / every, "{kind:?} {seed}");
                assert!(days.windows(2).all(|w| w[1] - w[0] == every));
                // The world's days before its first come round the same way.
                assert!(is_market_day_of(kind, seed, days[0] - every * 10));
            }
        }
        // Villages are not all on the same beat.
        let offsets: std::collections::HashSet<i64> = (0..40u32).map(|s| market_offset(Kind::Village, s.wrapping_mul(0x9e37_79b9))).collect();
        assert!(offsets.len() >= 4, "only {} offsets among forty villages", offsets.len());
        // Towns hold theirs more often than villages.
        assert!(market_every(Kind::Town) < market_every(Kind::Village) && market_every(Kind::GreatTown) < market_every(Kind::Town));
    }

    #[test]
    fn festivals_fall_about_the_solstices_two_days_each() {
        for year_days in [24.0f32, 12.0, 48.0] {
            let year = year_days as i64;
            let mut seen = Vec::new();
            for day in 0..year * 3 {
                if let Some(f) = festival_on(day, year_days) {
                    seen.push((day, f));
                    let middle = ((day as f32 + 0.5) / year_days).rem_euclid(1.0);
                    assert!((middle - f.solstice()).abs() < 1.5 / year_days, "{f:?} on day {day} of a {year_days}-day year");
                }
            }
            for f in Festival::ALL {
                let n = seen.iter().filter(|s| s.1 == f).count();
                assert_eq!(n, 6, "{f:?} in three {year_days}-day years: {seen:?}");
            }
        }
        // With 24 days to the year, midsummer is the sixth and seventh day of it and
        // midwinter the eighteenth and nineteenth.
        let days: Vec<(i64, Festival)> = (0..24).filter_map(|d| festival_on(d, 24.0).map(|f| (d, f))).collect();
        assert_eq!(days, vec![(5, Festival::Midsummer), (6, Festival::Midsummer), (17, Festival::Midwinter), (18, Festival::Midwinter)]);
        assert_eq!(Festival::Midsummer.since(0.25), 0.0);
        assert!((Festival::Midsummer.since(0.2) - 0.95).abs() < 1e-5);
        assert!((Festival::Midwinter.since(0.0) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn every_stall_is_recorded_for_market_day_and_every_board_hangs_in_its_building() {
        use crate::{
            continent::SimConfig,
            settlements::layout::Purpose,
            world::GameWorld,
        };
        use voxel::mesh::CELLS;
        let (mut stalls, mut boards, mut taverns) = (0, 0, 0);
        for seed in 1..=4 {
            let c = GameWorld::with_config(seed, SimConfig { size: 256 }).continent(0, 0);
            let net = &c.trails;
            for l in &net.layouts {
                // The stalls on the green, each as the game dresses it, in the same order.
                assert_eq!(l.market.len(), l.stalls.len());
                for (m, &at) in l.market.iter().zip(&l.stalls) {
                    assert_eq!(m.at, at);
                    if let Some((back, front)) = m.awning {
                        assert!(back.y > front.y, "an awning sloping the wrong way at {at}");
                    }
                    assert!(m.counter.0.y == m.counter.1.y && m.counter.0.y > at.y * CELLS);
                }
                stalls += l.stalls.len();
            }
            taverns += net.plots.iter().filter(|p| p.purpose == Purpose::Tavern).count();
            for b in &net.boards {
                let p = &net.plots[b.plot];
                assert!(matches!(p.purpose, Purpose::Tavern | Purpose::GuildHouse), "a board in a {}", p.purpose.word());
                // On the inside of a wall, facing into the room, at a reader's height.
                let (lo, hi) = (p.walls.0.as_vec2(), p.walls.1.as_vec2() + 1.0);
                let into = b.at + b.facing.as_vec3();
                let within = |q: glam::Vec3| q.x >= lo.x && q.x <= hi.x && q.z >= lo.y && q.z <= hi.y;
                assert!(within(b.at) && within(into), "a board out of its building's walls: {} facing {}", b.at, b.facing);
                assert!(b.at.y - p.door.y as f32 > 2.0 && b.at.y - (p.door.y as f32) < 6.0, "a board at {} over a floor at {}", b.at.y, p.door.y);
                assert!(b.facing.y == 0 && b.facing.abs().max_element() == 1);
                boards += 1;
            }
            for (plot, guilds) in &net.guild_halls {
                assert_eq!(net.plots[*plot].purpose, Purpose::GuildHouse);
                assert!(!guilds.is_empty() && net.boards.iter().any(|b| b.plot == *plot), "a guild house without its board");
            }
        }
        assert!(stalls > 0 && boards > 0, "{stalls} stalls and {boards} boards in four worlds");
        assert!(boards >= taverns, "{boards} boards for {taverns} taverns");
    }
}
