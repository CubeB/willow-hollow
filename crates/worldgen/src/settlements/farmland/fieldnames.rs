//! What the people of a place call its fields (#64): "Long Acre", "Brook Meadow",
//! "Marren's Orchard", in the words of `names/words.txt` and with the given names of the
//! tongue spoken there (see [`crate::names::Gazetteer::field_name`]).
//!
//! A field is named once, for good, by what it was first sown with, whatever it carries
//! this year; the fields nearest the houses are named first, so a name is never the same
//! as one nearer in, and no two fields of a place share one. Strips cut from one field
//! share its name. The names are only known near at hand: the journal has the name of the
//! field the player stands in.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use glam::Vec2;

use super::{Crop, ditch, fields, influence::sample};
use crate::{continent::Continent, noise::hash2, surface::MIN_RIVER_Q};

/// The names of the fields of one place: each field's own number (as
/// [`fields::Found::field`] has it) and its name.
pub type Names = Vec<(u32, String)>;

/// The section of `words.txt` a field first sown with `crop` is named from.
fn kind(crop: Crop) -> &'static str {
    match crop {
        Crop::Hay => "meadow",
        Crop::Pasture => "pasture",
        Crop::Rough => "common",
        Crop::Orchard => "orchard",
        Crop::Vines => "vineyard",
        Crop::Olives => "olives",
        Crop::Coppice => "copse",
        Crop::Paddy => "paddy",
        Crop::Wheat | Crop::Barley | Crop::Rye | Crop::Roots | Crop::Flax | Crop::Ploughed | Crop::Fallow => "field",
    }
}

/// The names of the fields of settlement `owner` of `c`, the nearest its middle named
/// first, no two alike.
pub fn names_of(c: &Continent, owner: usize) -> Names {
    let farm = &c.land.farms[owner];
    let mut all = fields::fields_of(c, owner);
    all.extend(ditch::fields_of(c, owner));
    all.sort_by(|a, b| a.1.distance_squared(farm.at).total_cmp(&b.1.distance_squared(farm.at)).then(a.0.cmp(&b.0)));
    let gazetteer = &c.trails.names;
    let mut out: Names = Vec::new();
    for (field, mid, crop) in all {
        // A brook running by it, as the grid has the streams.
        let by = (sample(c, mid).river >= MIN_RIVER_Q).then(|| ["Brook", "Brook", "Water", "Ford", "Withy"][(hash2(field, 7, 1) % 5) as usize]);
        let name = (0..12).find_map(|k| gazetteer.field_name(farm.at, kind(crop), hash2(field, k, owner as i32), by).filter(|n| !out.iter().any(|(_, m)| m == n)));
        if let Some(name) = name {
            out.push((field, name));
        }
    }
    out
}

/// The names of every place's fields worked out so far: by world, continent (and its size)
/// and place.
type Named = HashMap<(u32, i32, i32, usize, usize), Arc<Names>>;
static NAMED: OnceLock<Mutex<Named>> = OnceLock::new();

/// The name of the field continent-local column `(lx, lz)` of `c` lies in, as the people
/// whose field it is call it; None outside the fields. The first asked about in a place
/// names all its fields, which takes a moment; after that it is a lookup.
pub fn field_name(c: &Continent, lx: i32, lz: i32) -> Option<String> {
    let found = fields::locate(c, c.desc.world_seed, lx, lz)?;
    let owner = found.plot.owner;
    let names = {
        let mut named = NAMED.get_or_init(Default::default).lock().ok()?;
        named.entry((c.desc.world_seed, c.desc.mx, c.desc.mz, c.size, owner)).or_insert_with(|| Arc::new(names_of(c, owner))).clone()
    };
    names.iter().find(|n| n.0 == found.field).map(|n| n.1.clone())
}

/// [`field_name`] at continent-local point `p`.
pub fn field_name_at(c: &Continent, p: Vec2) -> Option<String> {
    field_name(c, p.x.floor() as i32, p.y.floor() as i32)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{continent::SimConfig, world::GameWorld};

    /// Every place names its fields the same every time, no two alike, and a column in a
    /// field has its field's name.
    #[test]
    fn field_names_are_stable_and_no_two_alike_in_a_place() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let again = GameWorld::with_config(3, SimConfig { size: 256 });
        let d = again.continent(0, 0);
        let mut named = 0;
        for k in 0..c.land.farms.len() {
            let a = names_of(&c, k);
            assert_eq!(a, names_of(&d, k), "place {k} names its fields differently the second time");
            let mut seen = HashSet::new();
            for (field, name) in &a {
                assert!(!name.is_empty() && seen.insert(name.clone()), "two fields of place {k} are called {name}");
                assert!(*field & 1 == 1);
            }
            if k < 3 {
                println!("place {k}: {:?}", a.iter().take(12).map(|n| n.1.as_str()).collect::<Vec<_>>());
            }
            named += a.len();
        }
        assert!(named > 100, "only {named} fields named");
        let (mut asked, mut answered) = (0, 0);
        for f in &c.land.farms {
            for k in 0..200 {
                let p = f.at + Vec2::from_angle(k as f32 * 0.61) * (f.radius + 10.0 + k as f32 * 3.0);
                let (x, z) = (p.x as i32, p.y as i32);
                let Some(found) = fields::locate(&c, w.seed(), x, z) else { continue };
                asked += 1;
                let name = field_name(&c, x, z);
                answered += name.is_some() as u32;
                // Its neighbour in the same field has the same name.
                if let Some(next) = fields::locate(&c, w.seed(), x + 1, z)
                    && next.field == found.field
                    && next.plot.owner == found.plot.owner
                {
                    assert_eq!(field_name(&c, x + 1, z), name);
                }
            }
        }
        assert!(answered * 10 > asked * 8, "only {answered} of {asked} columns in fields have a name");
    }
}
