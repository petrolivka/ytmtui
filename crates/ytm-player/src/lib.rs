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
