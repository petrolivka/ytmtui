//! Audio: stream resolution, ffmpeg-piped decoding, the PCM tap, and the
//! playback/queue engine.

pub mod engine;
#[cfg(target_os = "linux")]
pub mod mpris;
pub mod pcm;
pub mod resolver;

pub use engine::{spawn, Command, PlayerHandle};
pub use pcm::{Tap, CHANNELS, SAMPLE_RATE};
pub use resolver::{ResolverCache, StreamResolver, YtDlpResolver};

/// Names of the available audio output devices (FR-P9).
pub fn output_devices() -> Vec<String> {
    use rodio::cpal::traits::HostTrait;
    let host = rodio::cpal::default_host();
    host.output_devices()
        .map(|ds| ds.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

/// A device's human-readable name, for matching what the user put in the
/// config. `name()` is deprecated in favour of `description()`.
pub(crate) fn device_name(d: &rodio::cpal::Device) -> Option<String> {
    use rodio::cpal::traits::DeviceTrait;
    d.description().ok().map(|desc| desc.name().to_string())
}
