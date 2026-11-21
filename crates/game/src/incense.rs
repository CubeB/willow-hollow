//! Incense at the shrines (#40): one of the few things a wanderer does to the world, and
//! one that means something.
//!
//! Every shrine has a burner before it, and every wayside shelf a dish, built of what the
//! place is built of (`worldgen::features::props`, `GameWorld::burners_near`). A stick of
//! incense, made in the pack from resin and a sweet herb, is lit in one with E, or by
//! using it with it in hand: a soft chime, a thin curl of smoke that drifts off with the
//! wind for a couple of hours of the day, and then the ash of it in the bowl until the
//! next is lit. Its smoke tells of somewhere: the map gains a faint mark at the nearest
//! spring or shrine not yet come to, and the journal keeps the shrine.
//!
//! What burns is a thing set down (`placed`), kept by where it stands like a fire, and
//! saved with the world's things; nothing of it is in the world's generation.

use glam::{DVec2, DVec3};
use worldgen::{scale::m64, trails::Burner};

use crate::{Running, inventory, placed, player::Mode};

/// How long a stick burns once lit, in days: two hours.
pub const BURNS: f32 = 2.0 / 24.0;
/// How near the hands must come to a burner to light a stick in it, in metres: across
/// the ground, and up or down.
const REACH: f64 = 1.5;
const REACH_UP: f64 = 1.8;
/// How near a spring or shrine may be and not be told of, in metres: the place stood at.
const HERE: f64 = 60.0;
/// How loud the chime is, 0 to 1.
const CHIME: f32 = 0.8;

/// The burner of `burners` within reach of hands at `hands`, the nearest.
pub fn within_reach(burners: &[Burner], hands: DVec3) -> Option<Burner> {
    let across = |b: &Burner| (b.at.x as f64 - hands.x).hypot(b.at.z as f64 - hands.z);
    burners
        .iter()
        .filter(|b| across(b) <= m64(REACH) && (b.at.y as f64 - hands.y).abs() <= m64(REACH_UP))
        .min_by(|a, b| across(a).total_cmp(&across(b)))
        .copied()
}

/// The nearest spring or shrine of `marks` to `at` (world x and z), not the place stood
/// at: what the smoke of incense lit at `at` tells of when nothing of the world has been
/// come to yet, as on a screenshot. Its place, and whether it is a shrine.
pub fn told_of(marks: &[crate::minimap::marks::Mark], at: DVec2) -> Option<(DVec2, bool)> {
    use crate::minimap::marks::Kind;
    marks
        .iter()
        .filter(|m| (m.kind == Kind::Spring || m.shrine) && m.at.distance(at) > m64(HERE))
        .min_by(|a, b| a.at.distance(at).total_cmp(&b.at.distance(at)))
        .map(|m| (m.at, m.shrine))
}

impl Running {
    /// The burner the player stands at, if they are on foot at one.
    fn burner_near(&self) -> Option<Burner> {
        (self.walking && self.player.mode == Mode::Walk && self.rest.is_none()).then(|| within_reach(&self.burners, self.hands())).flatten()
    }

    /// What can be done at the burner the player stands at, in words, if they are at one.
    pub(crate) fn incense_prompt(&self) -> Option<String> {
        let burner = self.burner_near()?;
        let lit = self.placed.incense_at(burner.at.as_dvec3()).and_then(|h| self.placed.get(h)).is_some_and(placed::Thing::smoulders);
        Some(if lit {
            "Incense is burning here".into()
        } else if self.pack.count("incense") > 0 {
            "E to light a stick of incense".into()
        } else {
            "A burner for incense: resin rolled with a sweet herb makes it".into()
        })
    }

    /// Lights a stick of incense in the burner the player stands at, if they have one and
    /// none burns there already. Whether they were at a burner at all.
    pub(crate) fn light_incense(&mut self) -> bool {
        let Some(burner) = self.burner_near() else { return false };
        let at = burner.at.as_dvec3();
        let there = self.placed.incense_at(at);
        if there.and_then(|h| self.placed.get(h)).is_some_and(placed::Thing::smoulders) {
            self.say("Incense is burning here already");
            return true;
        }
        if !self.pack.spend("incense", 1) {
            self.say("You have no incense: roll resin with a sweet herb in the pack");
            return true;
        }
        // The ash of the last stick is cleared for the new one.
        if let Some(ash) = there {
            self.placed.take(ash);
        }
        self.placed.put(placed::Thing::new(inventory::incense(), 1, at, self.camera.yaw));
        if let Some(lit) = self.placed.incense_at(at) {
            self.placed.set_state(lit, placed::BURNING);
        }
        if let Some(audio) = &self.audio {
            audio.chime(CHIME);
        }
        // The smoke tells of the nearest spring or shrine not come to, and the journal
        // keeps where it was lit.
        let here = DVec2::new(at.x, at.z);
        let told = self.map.unfound_near(here, m64(HERE));
        let named = self.map.shrine_named(here, m64(12.0));
        self.journal.light(here, burner.wayside, named.as_deref(), told);
        self.map.told = self.journal.told();
        // TODO(#37): grant a light rest here, once the rested status is in.
        log::info!("incense lit at {:.1} {:.1} {:.1}; it tells of {:?}", at.x, at.y, at.z, told);
        self.say(match told {
            Some((place, shrine)) => format!("The incense catches   \u{b7}   its smoke drifts away {}, toward {}", crate::journal::compass(here, place), if shrine { "a shrine" } else { "a spring" }),
            None => "The incense catches".into(),
        });
        self.thing_near = None;
        self.reach_timer = 0.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    #[test]
    fn a_burner_is_within_reach_only_from_beside_it() {
        let burners = [Burner { at: Vec3::new(10.0, 20.0, 10.0), wayside: false }, Burner { at: Vec3::new(10.0, 20.0, 40.0), wayside: true }];
        let hands = |x: f64, y: f64, z: f64| within_reach(&burners, DVec3::new(x, y, z));
        assert_eq!(hands(11.0, 21.0, 10.5).map(|b| b.wayside), Some(false));
        assert_eq!(hands(10.0, 19.0, 39.0).map(|b| b.wayside), Some(true));
        assert!(hands(10.0, 21.0, 25.0).is_none(), "between them");
        assert!(hands(10.0, 40.0, 10.0).is_none(), "far above it");
    }
}
