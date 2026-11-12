//! Sound output: plays footsteps through the default output device.

mod synth;

use std::sync::mpsc::{Sender, channel};

use cpal::{
    FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use voxel::Voxel;
use worldgen::materials as mat;

pub use synth::Ground;
use synth::Mixer;

enum Message {
    Volume(f32),
    Step { ground: Ground, loud: f32 },
}

/// A running output stream. Dropping it stops the sound.
pub struct Audio {
    messages: Sender<Message>,
    _stream: Stream,
}

impl Audio {
    /// Starts playing, or None if this machine has nothing to play through.
    pub fn start(volume: f32) -> Option<Self> {
        let device = cpal::default_host().default_output_device()?;
        let supported = device
            .default_output_config()
            .map_err(|e| log::warn!("no audio output format: {e}"))
            .ok()?;
        let format = supported.sample_format();
        let config = supported.config();
        let stream = match format {
            SampleFormat::F32 => open::<f32>(&device, config, volume),
            SampleFormat::I16 => open::<i16>(&device, config, volume),
            SampleFormat::U16 => open::<u16>(&device, config, volume),
            other => {
                log::warn!("unsupported audio sample format {other:?}");
                return None;
            }
        };
        let (stream, messages) = stream.map_err(|e| log::warn!("could not open audio output: {e}")).ok()?;
        stream.play().map_err(|e| log::warn!("could not start audio: {e}")).ok()?;
        Some(Self {
            messages,
            _stream: stream,
        })
    }

    pub fn set_volume(&self, volume: f32) {
        let _ = self.messages.send(Message::Volume(volume));
    }

    /// Plays a footstep; `loud` is 0 to 1.
    pub fn step(&self, ground: Ground, loud: f32) {
        let _ = self.messages.send(Message::Step { ground, loud });
    }
}

fn open<T>(device: &cpal::Device, config: StreamConfig, volume: f32) -> Result<(Stream, Sender<Message>), cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut mixer = Mixer::new(config.sample_rate as f32, 0x51ce, volume);
    let (tx, rx) = channel();
    let mut stereo = [0.0f32; 2048];
    let stream = device.build_output_stream::<T, _, _>(
        config,
        move |out: &mut [T], _| {
            while let Ok(message) = rx.try_recv() {
                match message {
                    Message::Volume(volume) => mixer.set_volume(volume),
                    Message::Step { ground, loud } => mixer.step(ground, loud),
                }
            }
            for block in out.chunks_mut(channels * stereo.len() / 2) {
                let frames = block.len() / channels;
                let made = &mut stereo[..frames * 2];
                mixer.render(made);
                for (frame, sound) in block.chunks_exact_mut(channels).zip(made.as_chunks::<2>().0) {
                    if channels == 1 {
                        frame[0] = T::from_sample((sound[0] + sound[1]) * 0.5);
                        continue;
                    }
                    frame[0] = T::from_sample(sound[0]);
                    frame[1] = T::from_sample(sound[1]);
                    for extra in &mut frame[2..] {
                        *extra = T::from_sample(0.0);
                    }
                }
            }
        },
        |e| log::warn!("audio stream error: {e}"),
        None,
    )?;
    Ok((stream, tx))
}

/// What a footstep on this voxel sounds like.
pub fn ground_of(v: Voxel) -> Ground {
    match v {
        mat::STONE | mat::ROCK_DARK | mat::ROCK_PALE | mat::ROCK_BROWN | mat::SANDSTONE_RED | mat::SANDSTONE | mat::SANDSTONE_DARK | mat::MOSSY_STONE | mat::SLATE | mat::ROOF_TILE => Ground::Stone,
        mat::SAND | mat::DESERT_SAND | mat::DUNE_SAND | mat::BLACK_SAND | mat::PATH_SAND => Ground::Sand,
        mat::GRAVEL | mat::SCREE | mat::PATH_GRAVEL => Ground::Gravel,
        mat::DIRT | mat::MUD | mat::PATH_DIRT | mat::PATH_DARK | mat::PATH_LITTER | mat::PATH_NEEDLES => Ground::Earth,
        mat::SNOW | mat::PATH_SNOW => Ground::Snow,
        mat::PLANK | mat::PLANK_PALE | mat::PLANK_DARK | mat::PLANK_RED | mat::WOOD_DEAD
        | mat::BARK_OAK | mat::BARK_BIRCH | mat::BARK_PINE | mat::BARK_REDWOOD | mat::BARK_JUNGLE
        | mat::BARK_PALM | mat::BARK_WILLOW | mat::BARK_ACACIA | mat::BARK_BAOBAB
        | mat::BARK_HOLLY | mat::BARK_YEW | mat::BARK_SCOTS => Ground::Wood,
        _ => Ground::Grass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_sound_like_what_they_are_made_of() {
        assert_eq!(ground_of(mat::GRASS), Ground::Grass);
        assert_eq!(ground_of(mat::PLANK_DARK), Ground::Wood);
        assert_eq!(ground_of(mat::STONE), Ground::Stone);
        assert_eq!(ground_of(mat::PATH_GRAVEL), Ground::Gravel);
        assert_eq!(ground_of(mat::SNOW), Ground::Snow);
    }
}
