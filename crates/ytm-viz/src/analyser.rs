//! Spectrum analyser: PCM in, render-ready bands out.
//!
//! Deliberately free of I/O so it can be tuned and benchmarked against local
//! files long before any network plumbing exists.

use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub const FFT_SIZE: usize = 2048;
const F_LO: f32 = 30.0;
const F_HI: f32 = 16_000.0;
/// Display window below the rolling ceiling. Measured on real tracks: a 60 dB
/// window bunches the median band at ~0.59 and reads as a solid wall, so the
/// window is deliberately narrow.
const DB_FLOOR: f32 = -45.0;
/// Expands the quiet end of the range. Tuned against measured percentiles.
const GAMMA: f32 = 1.4;
/// Gentle high-frequency lift, in dB per octave, compensating music's natural
/// roll-off so the top of the spectrum stays legible.
const TILT_DB_PER_OCT: f32 = 1.0;
/// Hard floor for the automatic-gain ceiling, in raw magnitude units.
///
/// Without it, silence drives the ceiling toward zero and the normalisation
/// amplifies the noise floor: with playback paused the bars visibly *climb*
/// instead of resting at zero.
const AGC_FLOOR: f32 = 1.0;

/// One frame of render-ready spectrum data, normalised to 0.0..=1.0.
#[derive(Clone, Debug, Default)]
pub struct SpectrumFrame {
    pub bands: Vec<f32>,
    pub peaks: Vec<f32>,
    pub rms: f32,
    pub seq: u64,
}

pub struct Analyser {
    fft: Arc<dyn RealToComplex<f32>>,
    /// Rolling mono history, always FFT_SIZE long.
    history: Vec<f32>,
    window: Vec<f32>,
    scratch_in: Vec<f32>,
    scratch_out: Vec<realfft::num_complex::Complex<f32>>,
    band_ranges: Vec<(usize, usize)>,
    /// Per-band tilt gain, precomputed in dB.
    tilt_db: Vec<f32>,
    bands: Vec<f32>,
    peaks: Vec<f32>,
    peak_age: Vec<f32>,
    /// Slow rolling ceiling so quiet tracks still fill the display.
    agc: f32,
    seq: u64,
    sample_rate: f32,
    /// Whether any audio arrived since the last `analyse`. Without this, a
    /// paused player re-analyses the same stale buffer forever while the gain
    /// ceiling decays toward it - and the bars visibly climb.
    fed_since_analyse: bool,
}

impl Analyser {
    pub fn new(n_bands: usize, sample_rate: u32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let scratch_out = fft.make_output_vec();

        // Hann window
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / (FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
            })
            .collect();

        let sr = sample_rate as f32;
        let bin_hz = sr / FFT_SIZE as f32;
        let nyquist_bin = FFT_SIZE / 2;

        // Logarithmic (perceptual) band edges. Linear bins would cram all the
        // music into the leftmost 10% of the display.
        let mut band_ranges = Vec::with_capacity(n_bands);
        let mut tilt_db = Vec::with_capacity(n_bands);
        for i in 0..n_bands {
            let t0 = i as f32 / n_bands as f32;
            let t1 = (i + 1) as f32 / n_bands as f32;
            let f0 = F_LO * (F_HI / F_LO).powf(t0);
            let f1 = F_LO * (F_HI / F_LO).powf(t1);
            let mut b0 = (f0 / bin_hz).floor() as usize;
            let mut b1 = (f1 / bin_hz).ceil() as usize;
            b0 = b0.max(1).min(nyquist_bin - 1);
            b1 = b1.max(b0 + 1).min(nyquist_bin);
            band_ranges.push((b0, b1));

            let f_centre = (f0 * f1).sqrt();
            tilt_db.push(TILT_DB_PER_OCT * (f_centre / F_LO).log2());
        }

        Self {
            fft,
            history: vec![0.0; FFT_SIZE],
            window,
            scratch_in: vec![0.0; FFT_SIZE],
            scratch_out,
            band_ranges,
            tilt_db,
            bands: vec![0.0; n_bands],
            peaks: vec![0.0; n_bands],
            peak_age: vec![0.0; n_bands],
            agc: 1e-4,
            seq: 0,
            sample_rate: sr,
            fed_since_analyse: false,
        }
    }

    pub fn n_bands(&self) -> usize {
        self.bands.len()
    }

    /// Feed interleaved stereo samples; they are downmixed to mono internally.
    pub fn feed_interleaved(&mut self, samples: &[f32], channels: usize) {
        if channels == 0 {
            return;
        }
        self.fed_since_analyse = true;
        let frames = samples.len() / channels;
        if frames == 0 {
            return;
        }
        // Only the most recent FFT_SIZE frames can matter.
        let start_frame = frames.saturating_sub(FFT_SIZE);
        let new = frames - start_frame;
        if new >= FFT_SIZE {
            self.history.clear();
        } else {
            self.history.drain(0..new);
        }
        for f in start_frame..frames {
            let mut acc = 0.0;
            for c in 0..channels {
                acc += samples[f * channels + c];
            }
            self.history.push(acc / channels as f32);
        }
        // Guard against drift.
        if self.history.len() > FFT_SIZE {
            let excess = self.history.len() - FFT_SIZE;
            self.history.drain(0..excess);
        }
        while self.history.len() < FFT_SIZE {
            self.history.insert(0, 0.0);
        }
    }

    /// Feed `frames` of silence. Call this when no audio is arriving (paused,
    /// stopped, buffering) so the display decays to zero rather than drifting.
    pub fn feed_silence(&mut self, frames: usize) {
        if frames == 0 {
            return;
        }
        self.fed_since_analyse = true;
        let n = frames.min(FFT_SIZE);
        self.history.drain(0..n);
        self.history.extend(std::iter::repeat_n(0.0, n));
    }

    /// Run one analysis pass. `dt` is seconds since the previous frame and
    /// drives the decay/peak-fall envelopes, so the feel is frame-rate
    /// independent.
    pub fn analyse(&mut self, dt: f32) -> SpectrumFrame {
        // Nothing arrived since the last pass: the source is paused, stopped or
        // starved. Shift silence in so the display falls to zero, instead of
        // re-measuring a frozen buffer against a decaying ceiling.
        if !self.fed_since_analyse {
            let frames = ((dt * self.sample_rate) as usize).clamp(1, FFT_SIZE);
            self.history.drain(0..frames);
            self.history.extend(std::iter::repeat_n(0.0, frames));
        }
        self.fed_since_analyse = false;

        for i in 0..FFT_SIZE {
            self.scratch_in[i] = self.history[i] * self.window[i];
        }
        let _ = self.fft.process(&mut self.scratch_in, &mut self.scratch_out);

        // RMS for the level meter / silence detection.
        let rms = (self.history.iter().map(|s| s * s).sum::<f32>() / FFT_SIZE as f32).sqrt();

        // Track a slow ceiling for automatic gain.
        let frame_max = self
            .scratch_out
            .iter()
            .map(|c| c.norm())
            .fold(0.0f32, f32::max);
        if frame_max > self.agc {
            self.agc = frame_max; // instant attack
        } else {
            self.agc += (frame_max - self.agc) * (1.0 - (-dt / 2.0).exp()); // 2s release
        }
        let ceiling = self.agc.max(AGC_FLOOR);

        // Release envelope: ~8 dB per 100 ms, expressed in normalised units.
        let release = (dt / 0.100) * (8.0 / -DB_FLOOR);

        let n = self.bands.len();
        let mut raw = vec![0.0f32; n];
        for (i, &(b0, b1)) in self.band_ranges.iter().enumerate() {
            // Peak within the band reads better than the mean for music.
            let mut m = 0.0f32;
            for b in b0..b1 {
                m = m.max(self.scratch_out[b].norm());
            }
            let db = 20.0 * (m / ceiling).max(1e-9).log10() + self.tilt_db[i];
            let lin = ((db - DB_FLOOR) / -DB_FLOOR).clamp(0.0, 1.0);
            raw[i] = lin.powf(GAMMA);
        }

        // Light inter-band blur removes comb artefacts from sparse low bins.
        for i in 0..n {
            let l = raw[i.saturating_sub(1)];
            let c = raw[i];
            let r = raw[(i + 1).min(n - 1)];
            let smoothed = 0.25 * l + 0.5 * c + 0.25 * r;

            // Instant attack, gradual release.
            if smoothed >= self.bands[i] {
                self.bands[i] = smoothed;
            } else {
                self.bands[i] = (self.bands[i] - release).max(smoothed);
            }

            // Peak hold: 700 ms, then fall at ~12 dB/s.
            if self.bands[i] >= self.peaks[i] {
                self.peaks[i] = self.bands[i];
                self.peak_age[i] = 0.0;
            } else {
                self.peak_age[i] += dt;
                if self.peak_age[i] > 0.700 {
                    let fall = dt * (12.0 / -DB_FLOOR);
                    self.peaks[i] = (self.peaks[i] - fall).max(self.bands[i]);
                }
            }
        }

        self.seq += 1;
        SpectrumFrame {
            bands: self.bands.clone(),
            peaks: self.peaks.clone(),
            rms,
            seq: self.seq,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loud_noise(n: usize) -> Vec<f32> {
        // Deterministic broadband signal, stereo interleaved.
        let mut x = 0x9E3779B9u32;
        (0..n * 2)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32) - 0.5
            })
            .collect()
    }

    fn warmed() -> Analyser {
        let mut a = Analyser::new(32, 48_000);
        for _ in 0..40 {
            a.feed_interleaved(&loud_noise(1024), 2);
            a.analyse(1.0 / 60.0);
        }
        a
    }

    fn peak(f: &SpectrumFrame) -> f32 {
        f.bands.iter().cloned().fold(0.0f32, f32::max)
    }

    #[test]
    fn responds_to_audio() {
        let mut a = warmed();
        a.feed_interleaved(&loud_noise(1024), 2);
        let p = peak(&a.analyse(1.0 / 60.0));
        assert!(p > 0.3, "expected activity, got peak {p}");
    }

    /// The paused-playback regression. When playback pauses the tap goes quiet,
    /// so `analyse` is called repeatedly with no new samples. The bars must fall
    /// to zero; before the fix they climbed, because the gain ceiling decayed
    /// toward the frozen buffer and inflated every band against it.
    #[test]
    fn bands_never_climb_while_no_audio_arrives() {
        let mut a = warmed();
        let mut prev = peak(&a.analyse(1.0 / 60.0));
        let start = prev;
        for i in 0..600 {
            // Deliberately feed nothing at all - exactly what a paused player does.
            let p = peak(&a.analyse(1.0 / 60.0));
            assert!(
                p <= prev + 1e-4,
                "bands climbed while paused at step {i}: {prev} -> {p} (started at {start})"
            );
            prev = p;
        }
        assert!(prev < 0.01, "expected decay to silence, got {prev}");
    }
}
