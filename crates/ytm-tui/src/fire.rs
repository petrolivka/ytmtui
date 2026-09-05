//! Doom fire, fed by the spectrum.
//!
//! The 1993 algorithm, unchanged: a row of embers along the bottom, and every
//! pixel above copies the one below it, cooled a little and nudged sideways at
//! random. What makes it a visualiser is where the embers come from - each
//! column is stoked by the band beneath it, so the flames follow the music and
//! an onset throws a flare across the whole base.

use crate::canvas::{self, Canvas};
use crate::theme::Theme;

/// Full heat. Wider than a byte so the per-row cooling stays a whole number
/// even on a tall grid, where a byte would round it to zero and the fire would
/// never die down.
const MAX_HEAT: u16 = 1023;

/// How far up a fully stoked column reaches, as a fraction of the height.
/// Below 1.0 the flames would clip against the top of the pane; this leaves
/// them room to lick at it on the loudest passages only.
const REACH: f32 = 0.72;

/// How quickly the base row follows the band under it. Driving it straight
/// from the spectrum makes the fire strobe at frame rate.
const BASE_EASE: f32 = 0.35;

/// Heat the base row holds when a band is at full amplitude, and the least it
/// holds while anything at all is playing, so quiet passages keep embers
/// instead of going black.
const BASE_GAIN: f32 = 1.0;
const BASE_FLOOR: f32 = 0.06;

#[derive(Default)]
pub struct Fire {
    w: usize,
    h: usize,
    /// Heat per pixel, row 0 at the top.
    heat: Vec<u16>,
    /// The eased base row, kept in its own buffer because it is a target the
    /// bottom row moves towards rather than a value written straight in.
    base: Vec<f32>,
    /// Average cooling per row, derived from the height so the flames reach
    /// the same fraction of the pane whatever resolution it is drawn at.
    cool: u16,
    /// Normalised heat, handed to the canvas. Kept between frames so painting
    /// does not allocate.
    shades: Vec<f32>,
    rng: u32,
}

impl Fire {
    pub fn new() -> Self {
        Self {
            // Any non-zero seed will do; xorshift only needs one.
            rng: 0x2545_F491,
            cool: 1,
            ..Default::default()
        }
    }

    /// Drop the simulation, for when the style changes away from fire.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    fn resize(&mut self, w: usize, h: usize) {
        if self.w == w && self.h == h {
            return;
        }
        self.w = w;
        self.h = h;
        self.heat = vec![0; w * h];
        self.shades = vec![0.0; w * h];
        self.base = vec![0.0; w];
        // At full heat a pixel survives `MAX_HEAT / cool` rows; solve that for
        // the fraction of the height the flames should reach.
        self.cool = ((MAX_HEAT as f32 / (h as f32 * REACH)) as u16).max(1);
    }

    /// Xorshift. A real generator is overkill for jitter nobody can inspect,
    /// and this one costs three shifts.
    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Advance one frame and paint it. `flare` is the decaying beat accent.
    pub fn step(&mut self, canvas: &mut Canvas, bands: &[f32], flare: f32, theme: &Theme) {
        let (w, h) = canvas.dims();
        if w == 0 || h == 0 {
            return;
        }
        self.resize(w, h);
        self.stoke(bands, flare);
        self.spread();
        for (s, &heat) in self.shades.iter_mut().zip(self.heat.iter()) {
            *s = heat as f32 / MAX_HEAT as f32;
        }
        canvas.paint(&self.shades, &canvas::heat_ramp(theme));
    }

    /// Set the bottom row from the spectrum.
    fn stoke(&mut self, bands: &[f32], flare: f32) {
        let (w, h) = (self.w, self.h);
        for x in 0..w {
            // Map the column onto a band. Bands are usually far fewer than
            // pixels, so this widens each one into a plume.
            let target = if bands.is_empty() {
                0.0
            } else {
                let i = (x * bands.len() / w).min(bands.len() - 1);
                bands[i]
            };
            // The flare lifts the whole base, so an onset reads as the fire
            // surging rather than as one column spiking.
            let target = (target * BASE_GAIN + flare * 0.5).min(1.0);
            let target = if target > 0.01 {
                target.max(BASE_FLOOR)
            } else {
                0.0
            };
            self.base[x] += (target - self.base[x]) * BASE_EASE;
        }
        for x in 0..w {
            // Per-pixel flicker, or the base row reads as a painted bar chart.
            let jitter = 0.75 + (self.rand() >> 24) as f32 / 1020.0;
            let v = (self.base[x] * jitter * MAX_HEAT as f32).clamp(0.0, MAX_HEAT as f32);
            self.heat[(h - 1) * w + x] = v as u16;
        }
    }

    /// Propagate heat upwards, cooled and nudged sideways.
    fn spread(&mut self) {
        let (w, h) = (self.w, self.h);
        // Cooling is random around the average, which is what turns a smooth
        // gradient into flame tips.
        let spread = self.cool.saturating_mul(2);
        for y in (1..h).rev() {
            for x in 0..w {
                let r = self.rand();
                let v = self.heat[y * w + x];
                let decay = (r % (spread as u32 + 1)) as u16;
                // -1, 0 or +1 column, which is what makes the flames lean.
                let dst = match (r >> 8) % 3 {
                    0 => x.saturating_sub(1),
                    1 => x,
                    _ => (x + 1).min(w - 1),
                };
                self.heat[(y - 1) * w + dst] = v.saturating_sub(decay);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bands: &[f32], frames: usize) -> (Fire, Canvas) {
        let mut canvas = Canvas::new();
        canvas.resize(32, 24);
        let mut f = Fire::new();
        for _ in 0..frames {
            f.step(&mut canvas, bands, 0.0, &Theme::default());
        }
        (f, canvas)
    }

    /// The row a column's heat has died out by, counted from the bottom.
    fn reach(f: &Fire, x: usize) -> usize {
        (0..f.h)
            .find(|&y| f.heat[y * f.w + x] > 0)
            .map(|y| f.h - y)
            .unwrap_or(0)
    }

    /// The point of the thing: loud bands make tall flames, quiet ones do not.
    #[test]
    fn flames_follow_the_bands() {
        // Left half loud, right half silent.
        let bands: Vec<f32> = (0..8).map(|i| if i < 4 { 1.0 } else { 0.0 }).collect();
        let (f, _) = run(&bands, 120);
        let loud = reach(&f, 4);
        let quiet = reach(&f, 28);
        assert!(loud > f.h / 3, "loud column only reached {loud} of {}", f.h);
        assert!(
            quiet * 3 < loud,
            "silent column reached {quiet}, loud one {loud}"
        );
    }

    /// Flames must not clip against the top of the pane at full drive, or the
    /// fire reads as a solid wall with a straight edge.
    #[test]
    fn full_drive_leaves_headroom() {
        let (f, _) = run(&[1.0; 8], 200);
        let top_row_lit = (0..f.w).filter(|&x| f.heat[x] > 0).count();
        assert!(
            top_row_lit * 4 < f.w,
            "{top_row_lit} of {} top-row pixels lit",
            f.w
        );
    }

    /// When the music stops the fire has to go out, not freeze mid-flame.
    #[test]
    fn silence_puts_the_fire_out() {
        let (mut f, mut canvas) = run(&[1.0; 8], 120);
        for _ in 0..300 {
            f.step(&mut canvas, &[0.0; 8], 0.0, &Theme::default());
        }
        assert!(
            f.heat.iter().all(|&h| h == 0),
            "embers left after silence: {}",
            f.heat.iter().filter(|&&h| h > 0).count()
        );
    }

    /// Resizing must not leave the old grid behind, and cooling has to be
    /// recomputed or the flames change length with the pane.
    #[test]
    fn resizing_rebuilds_the_grid() {
        let (mut f, mut canvas) = run(&[1.0; 8], 60);
        let tall_cool = f.cool;
        canvas.resize(64, 96);
        f.step(&mut canvas, &[1.0; 8], 0.0, &Theme::default());
        assert_eq!(f.heat.len(), 64 * 96);
        assert!(f.cool < tall_cool, "cooling did not scale with the height");
    }

    #[test]
    fn paints_an_image_of_the_right_size() {
        let (_, canvas) = run(&[1.0; 8], 60);
        let img = canvas.image().expect("painted");
        assert_eq!((img.width(), img.height()), (32, 24));
    }
}
