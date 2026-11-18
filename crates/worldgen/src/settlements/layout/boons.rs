//! What the player's boons add to a settlement (#207): a second well on its green, and
//! a great town's guild hall hung with banners when it is opened. Each is laid with the
//! settlement, last of all so that nothing else is moved by it, tagged with its boon,
//! and unseen until the boon is granted (see `settlements::years`). The boons granted
//! are the game's to keep; the layout only knows where each would go.

use glam::{IVec2, IVec3, Vec2};
use voxel::mesh::{CELLS, PlantBox};

use super::{GREEN, KEEP, PLOT, Purpose, THING, Town, WAY, facing};
use crate::{
    features::fittings::{self, Fitting},
    materials as mat,
    noise::{hash2, unit},
    settlements::{
        Reason, Region,
        years::{Boon, Site, Tag},
    },
    surface::Sampler,
    trails::Network,
};

impl Town<'_> {
    /// Lays where each boon would build in this settlement, unseen.
    pub(super) fn boon_sites(&mut self, net: &mut Network, fin: &Sampler) {
        self.second_well(net, fin);
        self.guild_banners(net);
    }

    /// A second well on the green, off the ways across it and clear of the first, or
    /// nowhere if there is no room.
    fn second_well(&mut self, net: &mut Network, fin: &Sampler) {
        let kind = match self.s.region {
            Region::Desert if self.s.reason == Reason::Oasis => Fitting::OasisWell,
            Region::Desert => Fitting::Shaduf,
            _ => Fitting::StoneWell,
        };
        let seed = self.s.seed ^ 0x3e12;
        let gy = self.green.y as i32 + 1;
        for f in [0.75, 0.6, 0.85, 0.45] {
            for j in 0..24 {
                let a = unit(seed) * std::f32::consts::TAU + j as f32 / 24.0 * std::f32::consts::TAU;
                let p = self.centre + Vec2::from_angle(a) * self.green.radius * f;
                if self.layout.wells.iter().any(|w| Vec2::new(w.x as f32, w.z as f32).distance(p) < 6.0) {
                    continue;
                }
                let at = IVec3::new(p.x.floor() as i32, gy, p.y.floor() as i32);
                let ft = facing(self.centre - p);
                let (lo, hi) = kind.room(at, ft);
                let cols: Vec<IVec2> = (lo.z - 1..=hi.z).flat_map(|z| (lo.x - 1..=hi.x).map(move |x| IVec2::new(x, z))).collect();
                if self.grid.any(&cols, WAY | THING | PLOT | KEEP) || cols.iter().any(|&c| self.grid.get(c) & GREEN == 0) {
                    continue;
                }
                // Level ground under it, as the green is.
                if (lo.z..hi.z).any(|z| (lo.x..hi.x).any(|x| fin.column(x, z).top() != gy - 1 || fin.column(x, z).water_surface().is_some())) {
                    continue;
                }
                for &c in &cols {
                    self.grid.mark(c, THING);
                }
                let f = fittings::build_with(kind, &self.pal, hash2(seed, j, 3), false, fittings::herbs_of(self.s.region)).place(at, ft);
                net.add_stamp_tagged(f.voxels, f.fine, Tag::Boon { settlement: self.k as u32, boon: Boon::SecondWell });
                net.years.sites.push(Site { settlement: self.k as u32, boon: Boon::SecondWell, at });
                return;
            }
        }
    }

    /// Banners either side of the guild house's door, on a pole across above it, for
    /// the day the hall is opened.
    fn guild_banners(&mut self, net: &mut Network) {
        let Some(p) = self.plots.iter().find(|p| p.purpose == Purpose::GuildHouse) else { return };
        let (door, out) = (p.door, p.facing);
        // Across the wall, and a cell into the voxel out from it.
        let (across, depth) = (IVec3::new(out.y.abs(), 0, out.x.abs()), IVec3::new(out.x.abs(), 0, out.y.abs()));
        let top = door.y + 6;
        let mut fine = Vec::new();
        for (n, side) in [-3, 3].into_iter().enumerate() {
            let at = door + IVec3::new(out.x - out.y * side, 0, out.y + out.x * side);
            let a = IVec3::new(at.x, top, at.z) * CELLS + depth;
            // The pole it hangs from, a voxel long along the wall.
            fine.push(PlantBox { lo: a.to_array(), hi: (a + across * CELLS + depth + IVec3::Y).to_array(), voxel: mat::PLANK_DARK, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
            // The banner: a ribbon a cell wide at each cell along the pole, hanging its
            // length from it flat to the wall, red one side and blue the other.
            let colour = if n == 0 { mat::CLOTH_RED } else { mat::CLOTH_BLUE };
            let length = 14;
            for i in 0..CELLS {
                for k in 0..length {
                    let lo = a + across * i - IVec3::new(0, k + 1, 0);
                    fine.push(PlantBox::ribbon(lo.to_array(), (lo + IVec3::ONE).to_array(), colour, k as u8, k == length - 1));
                }
            }
        }
        net.add_stamp_tagged(Vec::new(), fine, Tag::Boon { settlement: self.k as u32, boon: Boon::GuildHall });
        net.years.sites.push(Site { settlement: self.k as u32, boon: Boon::GuildHall, at: door });
    }
}
