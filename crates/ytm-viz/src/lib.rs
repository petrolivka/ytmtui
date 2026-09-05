//! Spectrum analysis. Deliberately free of I/O so it can be tuned and
//! benchmarked against local files, independently of any network plumbing.
//!
//! Tuning constants here were derived from measured band-value percentiles over
//! real tracks during the M0 spike, not chosen by eye. See `docs/M0-FINDINGS.md`.

pub mod analyser;
pub use analyser::{Analyser, SpectrumFrame, FFT_SIZE};
