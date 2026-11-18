//! Homes copied out of the kit into designs the owner can edit by hand (see
//! [`crate::features::designs`]), and built again from them.
//!
//! A copy keeps what the kit said of the home under `home:` in its file, one line to a
//! thing, so that a hand-edited home is still walked into, slept in and furnished.
//! Places are voxels in the home's model space (door to the east, +X); a way is a word,
//! `east` (+X), `west`, `south` (+Z), `north`, `up` or `down`, two joined for a corner
//! (`south-east`), or three numbers with commas (`1,0,1`).
//!
//! ```text
//! home:
//!   style: rect frame thatch gable 0     # footprint, walls, roof, roof shape, how raised
//!   walls: -7 -9  7 9                    # the columns inside the outer walls, x z to x z
//!   door: 7 0 0  east  3 5               # the floor in the doorway, out, width, height
//!   smoke: 0 17 -6                       # where the smoke comes out
//!   hearth: 0 1 -7  south  wall          # the fire, into the room, `middle` or `wall`
//!   bed: -5 1 -7  east  south  5 2       # at, along, into the room, long, wide
//!   furniture: 3 1 -7  east  south  2 1
//!   window: -7 3 0  -7 4 1  west         # the opening, corner to corner, out
//!   anchor: door  10 0 0  east           # a kind of anchor, where, out
//! ```
//!
//! The floor is not kept: it is found again by walking in each time the home is built
//! ([`super::walk`]), so it follows the edits.

use std::fmt::Write as _;

use glam::{IVec2, IVec3};
use voxel::{AIR, Voxel, mesh::CELLS};

use super::{
    Anchor, AnchorKind, Area, Building, Door, Hearth, Inside, Opening, Spec, Spot, Style, generate,
    rules::{Cover, Form, RoofShape, Rules, WallKind},
    walk::walk,
};
use crate::features::{
    designs::{self, Cell, Design, Fine, Kind, Role},
    landmarks::Palette,
};

/// What the kit said of a home besides its blocks: everything in a [`Building`] but its
/// voxels, fine parts, floor and the boxes kept clear (which a design keeps itself).
#[derive(Clone, PartialEq, Debug)]
pub struct Home {
    pub door: Door,
    pub walls: Area,
    pub smoke: IVec3,
    pub style: Style,
    pub hearth: Option<Hearth>,
    pub beds: Vec<Spot>,
    pub furniture: Vec<Spot>,
    pub windows: Vec<Opening>,
    pub anchors: Vec<Anchor>,
}

impl Home {
    /// What `b` says of itself.
    pub fn of(b: &Building) -> Home {
        Home {
            door: b.door,
            walls: b.walls,
            smoke: b.smoke,
            style: b.style,
            hearth: b.inside.hearth,
            beds: b.inside.beds.clone(),
            furniture: b.inside.furniture.clone(),
            windows: b.inside.windows.clone(),
            anchors: b.anchors.clone(),
        }
    }

    /// Nothing yet, to be read into: a plain cottage with no door, which
    /// [`Home::check`] turns away until a `door:` line is read.
    pub fn blank() -> Home {
        Home {
            door: Door { floor: IVec3::ZERO, facing: IVec3::X, width: 0, height: 0 },
            walls: Area { lo: IVec2::ZERO, hi: IVec2::ZERO },
            smoke: IVec3::ZERO,
            style: Style { form: Form::Rect, walls: WallKind::Plank, roof: Cover::Thatch, shape: RoofShape::Gable, raised: 0 },
            hearth: None,
            beds: Vec::new(),
            furniture: Vec::new(),
            windows: Vec::new(),
            anchors: Vec::new(),
        }
    }

    /// Whether what was read is enough to build from.
    pub fn check(&self) -> Result<(), String> {
        if self.door.width <= 0 || self.door.height <= 0 {
            return Err("the `home:` section has no `door:` line, which says where the way in is".into());
        }
        Ok(())
    }

    /// The lines under `home:` in a design's file, each indented and ended.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let s = &self.style;
        let _ = writeln!(out, "  style: {} {} {} {} {}", s.form.word(), s.walls.word(), s.roof.word(), s.shape.word(), s.raised);
        let _ = writeln!(out, "  walls: {} {}  {} {}", self.walls.lo.x, self.walls.lo.y, self.walls.hi.x, self.walls.hi.y);
        let d = &self.door;
        let _ = writeln!(out, "  door: {}  {}  {} {}", at(d.floor), way(d.facing), d.width, d.height);
        let _ = writeln!(out, "  smoke: {}", at(self.smoke));
        if let Some(h) = &self.hearth {
            let _ = writeln!(out, "  hearth: {}  {}  {}", at(h.at), way(h.facing), if h.central { "middle" } else { "wall" });
        }
        for (word, spots) in [("bed", &self.beds), ("furniture", &self.furniture)] {
            for s in spots {
                let _ = writeln!(out, "  {word}: {}  {}  {}  {} {}", at(s.at), way(s.along), way(s.facing), s.len, s.wide);
            }
        }
        for w in &self.windows {
            let _ = writeln!(out, "  window: {}  {}  {}", at(w.lo), at(w.hi), way(w.normal));
        }
        for a in &self.anchors {
            let _ = writeln!(out, "  anchor: {}  {}  {}", a.kind.word(), at(a.at), way(a.normal));
        }
        out
    }

    /// Reads one line from under `home:`, as `name: value`.
    pub fn read(&mut self, line: &str) -> Result<(), String> {
        let (key, value) = line.split_once(':').ok_or_else(|| format!("expected `name: value` under `home:`, found `{}`", line.trim()))?;
        let mut w = value.split_whitespace();
        let mut next = |what: &str| w.next().ok_or_else(|| format!("`{}:` is missing its {what}", key.trim()));
        match key.trim().to_ascii_lowercase().as_str() {
            "style" => {
                let form = next("footprint")?;
                let walls = next("walls")?;
                let roof = next("roof")?;
                let shape = next("roof shape")?;
                let raised = next("height above the ground")?;
                self.style = Style {
                    form: Form::by_word(form).ok_or_else(|| format!("`{form}` is not a footprint (rect, l or round)"))?,
                    walls: WallKind::by_word(walls).ok_or_else(|| format!("`{walls}` is not a kind of wall"))?,
                    roof: Cover::by_word(roof).ok_or_else(|| format!("`{roof}` is not a roof covering"))?,
                    shape: RoofShape::by_word(shape).ok_or_else(|| format!("`{shape}` is not a roof shape"))?,
                    raised: whole(raised)?,
                };
            }
            "walls" => {
                let a = IVec2::new(whole(next("first x")?)?, whole(next("first z")?)?);
                let b = IVec2::new(whole(next("second x")?)?, whole(next("second z")?)?);
                self.walls = Area { lo: a.min(b), hi: a.max(b) };
            }
            "door" => {
                let floor = place([next("x")?, next("y")?, next("z")?])?;
                let facing = parse_way(next("way out")?)?;
                let width = whole(next("width")?)?;
                let height = whole(next("height")?)?;
                if width < 1 || height < 1 {
                    return Err("a door is at least one voxel wide and high".into());
                }
                self.door = Door { floor, facing, width, height };
            }
            "smoke" => self.smoke = place([next("x")?, next("y")?, next("z")?])?,
            "hearth" => {
                let at = place([next("x")?, next("y")?, next("z")?])?;
                let facing = parse_way(next("way into the room")?)?;
                let central = match next("`middle` or `wall`")? {
                    "middle" => true,
                    "wall" => false,
                    other => return Err(format!("a hearth is in the `middle` of the floor or in a `wall`, not `{other}`")),
                };
                self.hearth = Some(Hearth { at, facing, central });
            }
            k @ ("bed" | "furniture") => {
                let at = place([next("x")?, next("y")?, next("z")?])?;
                let along = parse_way(next("way along the wall")?)?;
                let facing = parse_way(next("way into the room")?)?;
                let len = whole(next("length")?)?;
                let wide = whole(next("width")?)?;
                let spot = Spot { at, along, facing, len, wide };
                if k == "bed" { self.beds.push(spot) } else { self.furniture.push(spot) }
            }
            "window" => {
                let a = place([next("x")?, next("y")?, next("z")?])?;
                let b = place([next("x")?, next("y")?, next("z")?])?;
                let normal = parse_way(next("way out")?)?;
                self.windows.push(Opening { lo: a.min(b), hi: a.max(b), normal });
            }
            "anchor" => {
                let word = next("kind")?;
                let kind = AnchorKind::ALL
                    .into_iter()
                    .find(|k| k.word() == word)
                    .ok_or_else(|| format!("`{word}` is not a kind of anchor ({})", AnchorKind::ALL.map(|a| a.word()).join(", ")))?;
                let at = place([next("x")?, next("y")?, next("z")?])?;
                let normal = parse_way(next("way out")?)?;
                self.anchors.push(Anchor { kind, at, normal });
            }
            other => return Err(format!("`{other}` is not something a home keeps (style, walls, door, smoke, hearth, bed, furniture, window, anchor)")),
        }
        if let Some(extra) = w.next() {
            return Err(format!("`{extra}` is one word too many on a `{}:` line", key.trim()));
        }
        Ok(())
    }
}

fn whole(w: &str) -> Result<i32, String> {
    w.parse().map_err(|_| format!("`{w}` should be a whole number"))
}

fn place(w: [&str; 3]) -> Result<IVec3, String> {
    Ok(IVec3::new(whole(w[0])?, whole(w[1])?, whole(w[2])?))
}

fn at(q: IVec3) -> String {
    format!("{} {} {}", q.x, q.y, q.z)
}

/// A way as a file gives it: `east`, `south-west`, `up`, or `x,y,z` if it is none of those.
fn way(v: IVec3) -> String {
    let ns = match v.z {
        1 => Some("south"),
        -1 => Some("north"),
        0 => None,
        _ => return format!("{},{},{}", v.x, v.y, v.z),
    };
    let ew = match v.x {
        1 => Some("east"),
        -1 => Some("west"),
        0 => None,
        _ => return format!("{},{},{}", v.x, v.y, v.z),
    };
    match (v.y, ns, ew) {
        (1, None, None) => "up".into(),
        (-1, None, None) => "down".into(),
        (0, Some(a), Some(b)) => format!("{a}-{b}"),
        (0, Some(a), None) | (0, None, Some(a)) => a.into(),
        _ => format!("{},{},{}", v.x, v.y, v.z),
    }
}

fn parse_way(w: &str) -> Result<IVec3, String> {
    let bad = || format!("`{w}` is not a way: east, west, south, north, up, down, two of them joined (south-east), or x,y,z");
    if w.contains(',') {
        let v: Option<Vec<i32>> = w.split(',').map(|n| n.trim().parse().ok()).collect();
        return match v.as_deref() {
            Some(&[x, y, z]) => Ok(IVec3::new(x, y, z)),
            _ => Err(bad()),
        };
    }
    let mut v = IVec3::ZERO;
    for part in w.split('-') {
        v += match part.to_ascii_lowercase().as_str() {
            "east" => IVec3::X,
            "west" => IVec3::NEG_X,
            "south" => IVec3::Z,
            "north" => IVec3::NEG_Z,
            "up" => IVec3::Y,
            "down" => IVec3::NEG_Y,
            _ => return Err(bad()),
        };
    }
    Ok(v)
}

/// The kit's own home for `spec`, built of `pal` by `rules`, as a design of `kind` that
/// builds the same voxels again for the same palette: its stone, timber and roof as roles,
/// so it takes the materials of wherever it stands, and what the kit said of it kept
/// under `home:`.
pub fn copy(spec: &Spec, pal: &Palette, rules: &Rules, kind: Kind, name: &str) -> Design {
    let b = generate(spec, pal, rules);
    // The same home again with every role in a material of its own, which says which role
    // each voxel came from where the place uses one material for two of them. The kit
    // decides nothing by what a material is, so the two are built alike; if ever they are
    // not, the tags are let go and roles are guessed from the materials alone.
    let tags = designs::tag_palette(pal);
    let t = generate(spec, &tags, rules);
    let same = b.voxels.len() == t.voxels.len() && b.voxels.iter().zip(&t.voxels).all(|(a, c)| a.0 == c.0);
    // A role is only believed where it gives back the very material the kit laid, so the
    // copy builds what the kit built (mossy stone in a place too dry for moss stays mossy).
    let cell = |v: Voxel, tag: Option<Voxel>, at: IVec3| -> Cell {
        if v == AIR {
            return Cell::Air;
        }
        let gives = |r: Role| r.plain(pal) == Some(v) && r.resolve(pal, spec.seed, at) == v;
        if let Some(t) = tag {
            if let Some(r) = Role::ALL.into_iter().find(|&r| r.plain(&tags) == Some(t) && gives(r)) {
                return Cell::Role(r);
            }
            if t == v {
                return Cell::Material(v);
            }
        }
        match Role::ALL.into_iter().find(|&r| gives(r)) {
            Some(r) => Cell::Role(r),
            None => Cell::Material(v),
        }
    };
    let mut d = Design::new(kind, name);
    d.note = format!(
        "copied from the house kit's {} {}, variant {} (age {:.2}, wealth {:.2}, {} sleeping)",
        spec.region.word(),
        spec.step.word(),
        spec.seed,
        spec.age,
        spec.wealth,
        b.inside.beds.len()
    );
    for (k, &(q, v)) in b.voxels.iter().enumerate() {
        d.cells.insert(q, cell(v, same.then(|| t.voxels[k].1), q));
    }
    let same_fine = b.fine.len() == t.fine.len();
    for (k, f) in b.fine.iter().enumerate() {
        let tag = same_fine.then(|| t.fine[k]).filter(|g| g.lo == f.lo && g.hi == f.hi).map(|g| g.voxel);
        let lo = IVec3::from_array(f.lo);
        d.fine.push(Fine::of(f, cell(f.voxel, tag, lo.div_euclid(IVec3::splat(CELLS)))));
    }
    d.clear = b.clear.clone();
    d.home = Some(Home::of(&b));
    d
}

/// The home a saved design builds of palette `pal`, with its floor found again by walking
/// in. None for a design that keeps no `home:` (one drawn from nothing), which has no door
/// to walk in by.
pub fn from_design(d: &Design, pal: &Palette, seed: u32) -> Option<Building> {
    let home = d.home.as_ref()?;
    let mut s = d.shape(pal, seed);
    let fine = s.take_fine();
    let voxels = s.into_voxels();
    // Every column it covers above the ground, its walls at least, as the kit counts it.
    let footprint = super::footprint(home.walls, &voxels, &fine);
    let mut b = Building {
        voxels,
        fine,
        footprint,
        walls: home.walls,
        door: home.door,
        anchors: home.anchors.clone(),
        inside: Inside {
            floor: Vec::new(),
            beds: home.beds.clone(),
            hearth: home.hearth,
            furniture: home.furniture.clone(),
            windows: home.windows.clone(),
        },
        clear: d.clear.clone(),
        smoke: home.smoke,
        style: home.style,
    };
    b.inside.floor = walk(&b).floor;
    Some(b)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{
        materials as mat,
        settlements::{
            Region,
            kit::{Step, rules::rules},
        },
    };

    fn palette(region: Region, seed: u32) -> Palette {
        let biome = region.biome();
        Palette::local(biome, mat::bedrock(biome).0).varied(seed)
    }

    /// Copied, saved, read back and built again, a home is the kit's own to the voxel,
    /// and is walked into to every bed.
    #[test]
    fn a_copied_home_survives_its_file_and_builds_what_the_kit_built() {
        for region in Region::ALL {
            for step in Step::ALL {
                for seed in [4, 19] {
                    let spec = Spec { region, step, age: 0.6, wealth: (seed % 7) as f32 / 6.0, sleepers: step.max_sleepers() / 2, seed };
                    let pal = palette(region, seed);
                    let want = generate(&spec, &pal, rules(region));
                    let d = copy(&spec, &pal, rules(region), Kind::House(region, step), "Test copy");
                    // Built of roles, so it takes the materials of wherever it stands.
                    assert!(d.cells.values().any(|c| matches!(c, Cell::Role(Role::Timber | Role::Stone))), "{region:?} {step:?} {seed}: roles");
                    let text = d.to_text();
                    let back = Design::parse(&text).unwrap_or_else(|e| panic!("{region:?} {step:?} {seed}: {e}\n{text}"));
                    assert_eq!(back, d, "{region:?} {step:?} {seed}: changed on the way through its file");
                    let got = from_design(&back, &pal, seed).expect("a copy keeps its home");
                    let differ = want.voxels.iter().zip(&got.voxels).filter(|(a, b)| a != b).count();
                    assert_eq!((want.voxels.len(), differ), (got.voxels.len(), 0), "{region:?} {step:?} {seed}: the same voxels");
                    // Everything else but the floor, which is found again by walking in.
                    let floor = got.inside.floor.clone();
                    let got = Building { inside: Inside { floor: want.inside.floor.clone(), ..got.inside }, ..got };
                    assert_eq!(got, want, "{region:?} {step:?} {seed}");
                    let walked = walk(&got);
                    assert!(walked.ok(), "{region:?} {step:?} {seed}: {:?}", walked.problems);
                    // Where a person stands inside takes in all the kit's floor.
                    let found: HashSet<IVec3> = floor.into_iter().collect();
                    let missed = want.inside.floor.iter().filter(|f| !found.contains(f)).count();
                    assert_eq!(missed, 0, "{region:?} {step:?} {seed}: kit floor not found again");
                    // And it builds of another place's materials.
                    let elsewhere = palette(Region::Desert, 3);
                    assert_eq!(from_design(&back, &elsewhere, seed).unwrap().voxels.len(), want.voxels.len());
                }
            }
        }
    }

    /// A wall built across the doorway is found out: the door is blocked and every bed
    /// cut off.
    #[test]
    fn a_blocked_door_is_found() {
        let (region, step) = (Region::Temperate, Step::Timber);
        let spec = Spec { region, step, age: 0.2, wealth: 0.5, sleepers: 3, seed: 7 };
        let pal = palette(region, 7);
        let mut d = copy(&spec, &pal, rules(region), Kind::House(region, step), "Shut");
        let door = d.home.as_ref().unwrap().door;
        for y in 1..=door.height {
            for dz in -3..=3 {
                d.cells.insert(door.floor + IVec3::new(0, y, dz), Cell::Role(Role::Planks));
            }
        }
        let b = from_design(&d, &pal, 7).unwrap();
        let walked = walk(&b);
        assert!(!walked.ok() && !walked.door, "{:?}", walked.problems);
        assert_eq!(walked.cut_off.len(), b.inside.beds.len());
    }

    /// Prints a kit home as the file of a saved house, to try a hand edit on without the
    /// editor: `KIT_COPY="temperate timber 7" cargo test ... kit::copy::tests::print --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print() {
        let Ok(want) = std::env::var("KIT_COPY") else { return };
        let words: Vec<&str> = want.split_whitespace().collect();
        let region = Region::ALL.into_iter().find(|r| r.word() == words[0]).expect("a region");
        let step = Step::ALL.into_iter().find(|s| s.word() == words[1]).expect("a step");
        let seed = words[2].parse().expect("a seed");
        let spec = Spec { region, step, age: 0.5, wealth: 0.5, sleepers: step.max_sleepers() / 2, seed };
        print!("{}", copy(&spec, &palette(region, seed), rules(region), Kind::House(region, step), &format!("{} {} {seed}", region.word(), step.word())).to_text());
    }
}
