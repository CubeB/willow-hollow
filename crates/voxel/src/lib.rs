//! Voxel chunk storage and meshing. Pure CPU, no GPU types.

pub mod mesh;

/// Chunk edge length in voxels.
pub const CHUNK: i32 = 32;
/// Edge length of a chunk plus a one-voxel border on every side.
pub const PAD: usize = CHUNK as usize + 2;

pub type Voxel = u16;
pub const AIR: Voxel = 0;

/// A chunk's voxels plus a one-voxel border copied from its neighbours.
///
/// The world is a deterministic function of position, so the generator fills the border
/// directly. That makes meshing (face culling and ambient occlusion at chunk edges)
/// self-contained: no chunk ever waits on a neighbour.
///
/// Coordinates here are *padded*: 0 and `PAD - 1` are the border, `1..=CHUNK` the interior.
/// Storage is column-major with y fastest, which suits column-by-column generation.
pub struct PaddedChunk {
    voxels: Box<[Voxel]>,
}

impl Default for PaddedChunk {
    fn default() -> Self {
        Self::new()
    }
}

impl PaddedChunk {
    pub fn new() -> Self {
        Self {
            voxels: vec![AIR; PAD * PAD * PAD].into_boxed_slice(),
        }
    }

    #[inline]
    pub fn idx(x: usize, y: usize, z: usize) -> usize {
        (x * PAD + z) * PAD + y
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> Voxel {
        self.voxels[Self::idx(x, y, z)]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, v: Voxel) {
        self.voxels[Self::idx(x, y, z)] = v;
    }

    /// The `PAD` voxels of one vertical column, indexed by padded y.
    #[inline]
    pub fn column_mut(&mut self, x: usize, z: usize) -> &mut [Voxel] {
        let start = (x * PAD + z) * PAD;
        &mut self.voxels[start..start + PAD]
    }

    pub fn voxels(&self) -> &[Voxel] {
        &self.voxels
    }

    /// True if every voxel (border included) is identical, so no faces can exist.
    pub fn is_uniform(&self) -> bool {
        let first = self.voxels[0];
        self.voxels.iter().all(|&v| v == first)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    Air = 0,
    Solid = 1,
    Water = 2,
    /// Drawn like a solid, but not an obstacle: leaves, vines.
    Foliage = 3,
}

impl Kind {
    /// Blocks light and hides faces behind it.
    pub fn is_opaque(self) -> bool {
        matches!(self, Kind::Solid | Kind::Foliage)
    }
}

/// Vertex flag bits carried to the shader.
pub mod flags {
    pub const WATER: u8 = 1 << 0;
    /// Two bits of per-voxel colour jitter strength (0..=3).
    pub const JITTER_SHIFT: u8 = 1;
    pub const JITTER_MASK: u8 = 0b11 << JITTER_SHIFT;
    /// Leaves and vines: swayed by the wind in the vertex shader.
    pub const FOLIAGE: u8 = 1 << 3;
    /// Lantern glass and embers: shines at night regardless of the light falling on it.
    pub const EMISSIVE: u8 = 1 << 4;
}

#[derive(Clone, Copy, Debug)]
pub struct Material {
    /// sRGB colour.
    pub color: [u8; 3],
    pub kind: Kind,
    /// Per-voxel colour variation strength, 0..=3.
    pub jitter: u8,
    /// Gives off its own light after dark.
    pub emissive: bool,
}

/// Lookup from voxel id to material. Id 0 is always air.
pub struct Materials {
    list: Vec<Material>,
}

impl Default for Materials {
    fn default() -> Self {
        Self::new()
    }
}

impl Materials {
    pub fn new() -> Self {
        Self {
            list: vec![Material {
                color: [0, 0, 0],
                kind: Kind::Air,
                jitter: 0,
                emissive: false,
            }],
        }
    }

    pub fn add(&mut self, m: Material) -> Voxel {
        self.list.push(m);
        (self.list.len() - 1) as Voxel
    }

    #[inline]
    pub fn get(&self, v: Voxel) -> &Material {
        &self.list[v as usize]
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}
