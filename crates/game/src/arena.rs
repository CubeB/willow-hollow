//! One GPU buffer holding every chunk mesh. Binding a single buffer lets a whole list
//! of chunks go to the GPU as one multi-draw instead of one draw call each.

use std::collections::BTreeMap;

use crate::gfx::Gpu;

const INITIAL_BYTES: u64 = 64 << 20;

/// Where one mesh lives in the arena, in the arena's units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshSpan {
    start: u32,
    pub quads: u32,
}

impl MeshSpan {
    pub fn first_vertex(&self) -> u32 {
        self.start * 4
    }

    /// The first unit, for arenas that hold something other than quads.
    pub fn first(&self) -> u32 {
        self.start
    }

    /// The `quads` of this span that start `offset` into it: one of several meshes
    /// stored together. Each part may be removed on its own, and once all of them are,
    /// the whole span is free again.
    pub fn part(&self, offset: u32, quads: u32) -> MeshSpan {
        debug_assert!(offset + quads <= self.quads);
        MeshSpan { start: self.start + offset, quads }
    }
}

/// First-fit allocator over a range of quads. Neighbouring free blocks are merged.
struct FreeList {
    /// Free blocks, start to length.
    blocks: BTreeMap<u32, u32>,
    capacity: u32,
}

impl FreeList {
    fn new(capacity: u32) -> Self {
        Self {
            blocks: BTreeMap::from([(0, capacity)]),
            capacity,
        }
    }

    fn alloc(&mut self, quads: u32) -> Option<u32> {
        let (&start, &len) = self.blocks.iter().find(|(_, len)| **len >= quads)?;
        self.blocks.remove(&start);
        if len > quads {
            self.blocks.insert(start + quads, len - quads);
        }
        Some(start)
    }

    fn free(&mut self, mut start: u32, mut len: u32) {
        if let Some((&before, &before_len)) = self.blocks.range(..start).next_back()
            && before + before_len == start
        {
            self.blocks.remove(&before);
            start = before;
            len += before_len;
        }
        if let Some(after_len) = self.blocks.remove(&(start + len)) {
            len += after_len;
        }
        self.blocks.insert(start, len);
    }

    fn grow(&mut self, capacity: u32) {
        let old = self.capacity;
        self.capacity = capacity;
        self.free(old, capacity - old);
    }
}

pub struct VertexArena {
    buffer: wgpu::Buffer,
    free: FreeList,
    max_units: u32,
    /// Bytes in one allocation unit: a quad of chunk vertices, say.
    unit: u64,
    label: &'static str,
}

fn create(gpu: &Gpu, label: &str, bytes: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// Like [`create`], but reports running out of GPU memory instead of treating it as fatal.
fn try_create(gpu: &Gpu, label: &str, bytes: u64) -> Option<wgpu::Buffer> {
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let buffer = create(gpu, label, bytes);
    pollster::block_on(scope.pop()).is_none().then_some(buffer)
}

impl VertexArena {
    /// `expected_bytes` is how much is likely to be needed. Reserving it now avoids
    /// stopping to copy everything into a larger buffer while the world loads.
    /// `unit` is the size in bytes of what is allocated.
    pub fn new(gpu: &Gpu, label: &'static str, expected_bytes: u64, unit: u64) -> Self {
        // Vertex numbers are u32, so four billion vertices is the ceiling regardless.
        let max_units = (gpu.device.limits().max_buffer_size / unit).min(u32::MAX as u64 / 4) as u32;
        let smallest = ((INITIAL_BYTES / unit) as u32).min(max_units);
        let hoped = ((expected_bytes.max(INITIAL_BYTES) / unit).min(max_units as u64)) as u32;
        // If the GPU can't spare that much at once, grow into it as usual.
        let (buffer, units) = match try_create(gpu, label, hoped as u64 * unit) {
            Some(buffer) => (buffer, hoped),
            None => (create(gpu, label, smallest as u64 * unit), smallest),
        };
        Self {
            buffer,
            free: FreeList::new(units),
            max_units,
            unit,
            label,
        }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn capacity_bytes(&self) -> u64 {
        self.free.capacity as u64 * self.unit
    }

    /// Stores some items. None if the arena is full and the GPU cannot hold a larger one.
    pub fn add<T: bytemuck::Pod>(&mut self, gpu: &Gpu, items: &[T]) -> Option<MeshSpan> {
        let quads = (size_of_val(items) as u64 / self.unit) as u32;
        let start = loop {
            if let Some(start) = self.free.alloc(quads) {
                break start;
            }
            if !self.grow(gpu) {
                return None;
            }
        };
        gpu.queue
            .write_buffer(&self.buffer, start as u64 * self.unit, bytemuck::cast_slice(items));
        Some(MeshSpan { start, quads })
    }

    pub fn remove(&mut self, span: MeshSpan) {
        self.free.free(span.start, span.quads);
    }

    fn grow(&mut self, gpu: &Gpu) -> bool {
        // By half again rather than double: the old and new buffers have to fit together.
        let capacity = self.free.capacity.saturating_add(self.free.capacity / 2).min(self.max_units);
        if capacity == self.free.capacity {
            return false;
        }
        // The old and new buffers exist together during the copy, so the GPU has to hold both.
        let Some(buffer) = try_create(gpu, self.label, capacity as u64 * self.unit) else {
            log::warn!("no GPU memory for a larger {}, staying at {} MiB", self.label, self.capacity_bytes() >> 20);
            self.max_units = self.free.capacity;
            return false;
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &buffer, 0, self.capacity_bytes());
        // Submitting also flushes earlier writes into the old buffer, ahead of the copy.
        gpu.queue.submit([encoder.finish()]);
        self.buffer = buffer;
        self.free.grow(capacity);
        log::info!("{} grew to {} MiB", self.label, self.capacity_bytes() >> 20);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_do_not_overlap() {
        let mut list = FreeList::new(100);
        assert_eq!(list.alloc(30), Some(0));
        assert_eq!(list.alloc(30), Some(30));
        assert_eq!(list.alloc(41), None);
        assert_eq!(list.alloc(40), Some(60));
        assert_eq!(list.alloc(1), None);
    }

    #[test]
    fn freed_neighbours_merge_into_one_block() {
        let mut list = FreeList::new(90);
        let (a, b, c) = (list.alloc(30).unwrap(), list.alloc(30).unwrap(), list.alloc(30).unwrap());
        list.free(a, 30);
        list.free(c, 30);
        assert_eq!(list.alloc(60), None);
        list.free(b, 30);
        assert_eq!(list.blocks.len(), 1);
        assert_eq!(list.alloc(90), Some(0));
    }

    #[test]
    fn the_parts_of_a_span_freed_one_by_one_free_it_all() {
        let mut list = FreeList::new(100);
        let span = MeshSpan { start: list.alloc(60).unwrap(), quads: 60 };
        let parts = [span.part(0, 10), span.part(10, 35), span.part(45, 15)];
        for p in [parts[1], parts[2], parts[0]] {
            list.free(p.start, p.quads);
        }
        assert_eq!(list.blocks.len(), 1);
        assert_eq!(list.alloc(100), Some(0));
    }

    #[test]
    fn growing_joins_the_free_tail() {
        let mut list = FreeList::new(10);
        assert_eq!(list.alloc(5), Some(0));
        list.grow(20);
        assert_eq!(list.alloc(15), Some(5));
    }
}
