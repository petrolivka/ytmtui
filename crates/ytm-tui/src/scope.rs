//! The oscilloscope, drawn as pixels.
//!
//! A trace is a line, and a line is the one thing block glyphs are worst at:
//! the pane is fourteen cells tall, so a glyph scope has fourteen levels to
//! put the waveform on. The same pane is a couple of hundred pixels tall.

use crate::canvas::{self, Canvas};
use crate::theme::Theme;
use ytm_viz::FFT_SIZE;

/// Samples shown across the pane: about 20 ms at 48 kHz. Long enough that a
/// bass note spans a cycle or two and reads as a wave, short enough that
/// everything above it stays texture rather than a blur.
pub const WINDOW: usize = FFT_SIZE / 2;

/// How bright the centre line is against the trace's own colour.
const GRATICULE: f32 = 0.22;

/// How bright the halo either side of the trace is. It is the difference
/// between a line that glows and a line that looks like a scratch.
const HALO: f32 = 0.4;

/// Where to start the trace: the first rising zero crossing with room for a
/// full window after it.
///
/// The hysteresis matters. Crossings inside the noise floor are not the
/// signal's period, and locking onto one puts the trace somewhere new every
/// frame - which is exactly the shimmer a trigger exists to remove.
pub fn trigger(wave: &[f32], window: usize) -> usize {
    const HYSTERESIS: f32 = 0.02;
    let limit = wave.len().saturating_sub(window);
    let mut armed = false;
    for (i, &s) in wave.iter().enumerate().take(limit) {
        if s < -HYSTERESIS {
            armed = true;
        } else if armed && s >= 0.0 {
            return i;
        }
    }
    0
}

pub fn draw(canvas: &mut Canvas, wave: &[f32], theme: &Theme) {
    let (w, h) = canvas.dims();
    if w == 0 || h < 2 {
        return;
    }
    canvas.clear();

    let ramp = canvas::gradient_ramp(theme);
    let mid = (h - 1) as f32 / 2.0;

    // The graticule, so a silent pane still reads as an instrument showing
    // zero rather than as an instrument that is switched off.
    let grid = ramp[canvas::shade(GRATICULE)];
    for x in 0..w {
        canvas.put(x, mid as usize, grid);
    }
    if wave.is_empty() {
        return;
    }

    let window = WINDOW.min(wave.len());
    let start = trigger(wave, window);
    let row = |v: f32| (mid - v * mid).round().clamp(0.0, (h - 1) as f32) as usize;
    let mut prev: Option<usize> = None;

    for x in 0..w {
        // Each column covers a slice of the window, drawn from its extremes:
        // a signal with more cycles than there are columns then shows its
        // envelope rather than an aliased scribble.
        let a = start + x * window / w;
        let b = (start + (x + 1) * window / w).max(a + 1).min(wave.len());
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &s in &wave[a..b] {
            lo = lo.min(s);
            hi = hi.max(s);
        }
        let (top, bottom) = (row(hi), row(lo));

        // Reach back to the previous column so the trace is a line rather than
        // a row of disconnected dashes.
        let (mut y0, mut y1) = (top, bottom);
        if let Some(p) = prev {
            y0 = y0.min(p);
            y1 = y1.max(p);
        }
        prev = Some(top.midpoint(bottom));

        // Never dimmer than the graticule: at zero the trace *is* the centre
        // line, and painting it unlit would rub the line out.
        let amp = lo.abs().max(hi.abs()).max(GRATICULE);
        let colour = ramp[canvas::shade(amp)];
        for y in y0..=y1 {
            canvas.put(x, y, colour);
        }
        canvas.blend(x, y0.saturating_sub(1), colour, HALO);
        canvas.blend(x, y1 + 1, colour, HALO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine of `period` samples, `FFT_SIZE` long, rotated by `phase`.
    fn sine(period: usize, phase: usize) -> Vec<f32> {
        let raw: Vec<f32> = (0..FFT_SIZE)
            .map(|i| (std::f32::consts::TAU * i as f32 / period as f32).sin())
            .collect();
        (0..FFT_SIZE).map(|i| raw[(i + phase) % FFT_SIZE]).collect()
    }

    fn drawn(wave: &[f32], w: usize, h: usize) -> Canvas {
        let mut canvas = Canvas::new();
        canvas.resize(w, h);
        draw(&mut canvas, wave, &Theme::default());
        canvas
    }

    /// Rows carrying anything brighter than the graticule.
    fn trace_rows(canvas: &Canvas) -> Vec<u32> {
        let img = canvas.image().unwrap();
        (0..img.height())
            .filter(|&y| {
                (0..img.width()).any(|x| {
                    let p = img.get_pixel(x, y).0;
                    p.iter().map(|&c| c as u32).sum::<u32>() > 150
                })
            })
            .collect()
    }

    /// The regression the pixel scope replaces twice over: it used to draw the
    /// bands mirrored about the midline, and then to draw the waveform at the
    /// resolution of a text cell. A full-scale sine has to use the height.
    #[test]
    fn the_trace_uses_the_full_height() {
        let canvas = drawn(&sine(64, 0), 200, 64);
        let rows = trace_rows(&canvas);
        assert!(
            rows.len() > 50,
            "a full-scale sine covered only {} of 64 rows",
            rows.len()
        );
        assert!(
            rows.contains(&0) && rows.contains(&63),
            "trace clipped: {rows:?}"
        );
    }

    /// The point of a trigger: the same signal must land in the same place
    /// whatever offset the buffer happened to be captured at.
    #[test]
    fn the_trigger_holds_the_trace_still() {
        let a = drawn(&sine(64, 0), 120, 40);
        let b = drawn(&sine(64, 17), 120, 40);
        assert_eq!(
            a.image().unwrap().as_raw(),
            b.image().unwrap().as_raw(),
            "the same tone drew differently at another offset"
        );
    }

    /// Silence is a line through the middle, not a blank pane: the scope is
    /// showing zero, and it should look like it.
    #[test]
    fn silence_is_a_line_through_the_middle() {
        let canvas = drawn(&vec![0.0; FFT_SIZE], 80, 41);
        let img = canvas.image().unwrap();
        assert!(
            img.get_pixel(0, 20).0.iter().any(|&c| c > 0),
            "no graticule"
        );
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0], "something above the line");
        assert_eq!(
            img.get_pixel(0, 40).0,
            [0, 0, 0],
            "something below the line"
        );
    }

    /// An offset waveform must not be drawn centred: the trace follows the
    /// samples, so a signal that never goes negative stays in the upper half.
    #[test]
    fn the_trace_follows_the_sign_of_the_samples() {
        let wave: Vec<f32> = sine(64, 0).iter().map(|s| s.abs()).collect();
        let canvas = drawn(&wave, 120, 40);
        let rows = trace_rows(&canvas);
        assert!(
            rows.iter().all(|&y| y <= 20),
            "a positive-only signal reached row {:?}",
            rows.last()
        );
    }
}
