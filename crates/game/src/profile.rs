//! Where each frame's time goes: GPU timestamps around the render passes, and laps of
//! the CPU work between them. Shown in the debug panel and logged after `--frames`.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use crate::gfx::Gpu;

/// The stretches of a frame on the GPU, in the order they run.
pub const GPU_SPANS: [&str; 9] = ["shadow 1", "shadow 2", "shadow 3", "sky", "far", "terrain", "water", "bloom", "tonemap"];
const STAMPS: u32 = GPU_SPANS.len() as u32 + 1;
const STAMP_BYTES: u64 = 8;
/// Results take a few frames to come back, so several can be on their way at once.
const SLOTS: usize = 4;

struct Slot {
    buffer: wgpu::Buffer,
    ready: Arc<AtomicBool>,
    busy: bool,
}

/// Times spans of GPU work with timestamp queries, where the GPU has them.
pub struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// The slot this frame's results are being copied into.
    writing: Option<usize>,
    tick_ms: f32,
    /// Smoothed milliseconds per span.
    pub ms: [f32; GPU_SPANS.len()],
}

impl GpuTimer {
    pub fn new(gpu: &Gpu) -> Option<Self> {
        let needed = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        if !gpu.device.features().contains(needed) {
            return None;
        }
        let set = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame timer"),
            ty: wgpu::QueryType::Timestamp,
            count: STAMPS,
        });
        let size = STAMPS as u64 * STAMP_BYTES;
        let buffer = |label, usage| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let resolve = buffer("timer resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC);
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: buffer("timer readback", wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ),
                ready: Arc::new(AtomicBool::new(false)),
                busy: false,
            })
            .collect();
        Some(Self {
            set,
            resolve,
            slots,
            writing: None,
            tick_ms: gpu.queue.get_timestamp_period() / 1e6,
            ms: [0.0; GPU_SPANS.len()],
        })
    }

    /// Marks the end of span `index - 1`, between passes.
    pub fn mark(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        encoder.write_timestamp(&self.set, index);
    }

    /// Marks the end of span `index - 1`, inside a pass.
    pub fn mark_in_pass(&self, pass: &mut wgpu::RenderPass, index: u32) {
        pass.write_timestamp(&self.set, index);
    }

    /// Copies this frame's timestamps out, if a slot is free.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.writing = self.slots.iter().position(|s| !s.busy);
        if let Some(i) = self.writing {
            encoder.resolve_query_set(&self.set, 0..STAMPS, &self.resolve, 0);
            encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.slots[i].buffer, 0, STAMPS as u64 * STAMP_BYTES);
        }
    }

    /// Call after submitting. Requests this frame's results and collects any that are in.
    pub fn after_submit(&mut self, gpu: &Gpu) {
        if let Some(i) = self.writing.take() {
            let slot = &mut self.slots[i];
            slot.busy = true;
            let ready = slot.ready.clone();
            slot.buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| ready.store(r.is_ok(), Ordering::Release));
        }
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        let mut newest = None;
        for slot in self.slots.iter_mut().filter(|s| s.busy && s.ready.load(Ordering::Acquire)) {
            if let Ok(data) = slot.buffer.slice(..).get_mapped_range() {
                let stamps: &[u64] = bytemuck::cast_slice(&data);
                newest = Some(std::array::from_fn::<f32, { GPU_SPANS.len() }, _>(|i| stamps[i + 1].wrapping_sub(stamps[i]) as f32));
            }
            slot.buffer.unmap();
            slot.ready.store(false, Ordering::Release);
            slot.busy = false;
        }
        if let Some(ticks) = newest {
            for (ms, t) in self.ms.iter_mut().zip(ticks) {
                *ms += (t * self.tick_ms - *ms) * 0.05;
            }
        }
    }

    /// One line for humans.
    pub fn summary(&self) -> String {
        let parts: Vec<String> = GPU_SPANS.iter().zip(self.ms).map(|(name, ms)| format!("{name} {ms:.2}")).collect();
        format!("GPU {:.1} ms: {}", self.ms.iter().sum::<f32>(), parts.join(", "))
    }
}

/// The stretches of a frame on the CPU.
pub const CPU_SPANS: [&str; 7] = ["sim", "stream", "life", "acquire", "render", "ui", "submit"];

/// Times stretches of CPU work one after another.
pub struct CpuTimer {
    last: Instant,
    /// Smoothed milliseconds per span.
    pub ms: [f32; CPU_SPANS.len()],
    /// The longest each span has taken since the last reset.
    worst: [f32; CPU_SPANS.len()],
}

impl CpuTimer {
    pub fn new() -> Self {
        Self {
            last: Instant::now(),
            ms: [0.0; CPU_SPANS.len()],
            worst: [0.0; CPU_SPANS.len()],
        }
    }

    pub fn reset_worst(&mut self) {
        self.worst = [0.0; CPU_SPANS.len()];
    }

    pub fn start(&mut self) {
        self.last = Instant::now();
    }

    /// Ends span `index`.
    pub fn lap(&mut self, index: usize) {
        let now = Instant::now();
        let ms = (now - self.last).as_secs_f32() * 1000.0;
        self.ms[index] += (ms - self.ms[index]) * 0.05;
        self.worst[index] = self.worst[index].max(ms);
        self.last = now;
    }

    pub fn summary(&self) -> String {
        let parts: Vec<String> = CPU_SPANS.iter().zip(self.ms).map(|(name, ms)| format!("{name} {ms:.2}")).collect();
        format!("CPU: {}", parts.join(", "))
    }

    pub fn worst_summary(&self) -> String {
        let parts: Vec<String> = CPU_SPANS.iter().zip(self.worst).map(|(name, ms)| format!("{name} {ms:.1}")).collect();
        format!("CPU worst: {}", parts.join(", "))
    }
}

/// Milliseconds at a given quantile (0 to 1) of a sorted list.
pub fn quantile(sorted: &[f32], q: f32) -> f32 {
    sorted.get(((sorted.len().saturating_sub(1)) as f32 * q) as usize).copied().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_read_from_the_sorted_list() {
        let list: Vec<f32> = (0..101).map(|i| i as f32).collect();
        assert_eq!(quantile(&list, 0.5), 50.0);
        assert_eq!(quantile(&list, 0.99), 99.0);
        assert_eq!(quantile(&list, 1.0), 100.0);
        assert_eq!(quantile(&[], 0.5), 0.0);
    }
}
