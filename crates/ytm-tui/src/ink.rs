//! Ink dropped into moving water.
//!
//! The same shape of idea as the fire - inject along the bottom, carry it
//! upwards - but a different physics, and it looks nothing like it. Fire
//! transports heat one pixel at a time and cools it, which makes sharp
//! tongues. This advects density along a drifting current and blurs it as it
//! goes, which makes soft plumes that spread, shear and fade.
//!
//! It also keeps the theme's whole gradient rather than a heat ramp, so the
//! dense core and the dispersing edges are different colours. Pigment, not
//! flame.

use crate::canvas::{self, Canvas};
use crate::theme::Theme;

/// How faint a plume has faded to by the end of its travel, and how far up the
/// pane that is. A typical plume reaches `REACH`; one riding an updraft goes
/// further, which is the point of having a current at all.
const FAINT: f32 = 0.02;
const REACH: f32 = 1.2;

/// How fast ink rises, in pixels per frame, before the current is added.
const RISE: f32 = 1.0;

/// Typical vertical speed the current contributes, as the root mean square of
/// the flow terms. Ink travels at the rise *plus* this, and calibrating the
/// fade against the rise alone is what floods the pane: the current is several
/// times stronger, so the plumes cross it long before they were meant to fade.
/// Derived from `FLOW` rather than written down, so retuning the current
/// cannot silently decalibrate the fade.
fn flow_lift() -> f32 {
    let sum: f32 = FLOW
        .iter()
        .map(|&(amp, kx, _, _)| {
            let v = amp * kx * std::f32::consts::TAU;
            v * v
        })
        .sum();
    (sum / 2.0).sqrt()
}

/// The current, as a stream function.
///
/// Velocity is taken as the curl of this rather than written down directly,
/// which makes the flow divergence-free: it swirls and folds without ink
/// piling up in one place or thinning out of another, and that is most of what
/// separates fluid from a blur. Three terms at frequencies that do not divide
/// into each other, so it never settles into a visible repeat.
///
/// Each entry is (strength, cycles per pixel across, cycles per pixel up,
/// radians per second). Roughly isotropic cells, because rolling a plume up
/// into a filament needs rotation, and pure sideways shear only stretches it.
const FLOW: [(f32, f32, f32, f32); 3] = [
    (14.0, 0.013, 0.021, 0.55),
    (7.0, 0.031, 0.043, -0.83),
    (3.0, 0.067, 0.089, 1.31),
];

/// Pixels between samples of the flow field. Evaluating three sines per pixel
/// is most of a millisecond a frame; every eighth pixel and a bilinear fill is
/// indistinguishable and costs a sixtieth of that.
const FLOW_GRID: usize = 8;

/// How much the bass stirs the water on top of that.
const FLOW_FROM_BASS: f32 = 1.1;

/// How quickly the injected row follows the spectrum.
const INJECT_EASE: f32 = 0.25;

/// Fraction of a fully injected column that a beat adds across the whole base.
const FLARE_GAIN: f32 = 0.45;

#[derive(Default)]
pub struct Ink {
    w: usize,
    h: usize,
    /// Ink per pixel, row 0 at the top.
    density: Vec<f32>,
    /// Destination buffer for a step; advection reads every source pixel more
    /// than once, so it cannot be done in place.
    next: Vec<f32>,
    /// The eased injection row.
    base: Vec<f32>,
    /// The current, sampled every `FLOW_GRID` pixels and interpolated between.
    flow: Vec<(f32, f32)>,
    gw: usize,
    gh: usize,
    decay: f32,
    t: f32,
    rng: u32,
}

impl Ink {
    pub fn new() -> Self {
        Self {
            rng: 0x9E37_79B9,
            decay: 0.95,
            ..Default::default()
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    fn resize(&mut self, w: usize, h: usize) {
        if self.w == w && self.h == h {
            return;
        }
        self.w = w;
        self.h = h;
        self.density = vec![0.0; w * h];
        self.next = vec![0.0; w * h];
        self.base = vec![0.0; w];
        self.gw = w / FLOW_GRID + 2;
        self.gh = h / FLOW_GRID + 2;
        self.flow = vec![(0.0, 0.0); self.gw * self.gh];
        // Solve `decay^frames = FAINT` for the survival per frame, where
        // `frames` is how long ink takes to cross `h * REACH` pixels at the
        // speed it actually travels. Without this the plumes would be as long
        // as the grid happens to be, and the same track would look different
        // under sixel and under half blocks.
        let frames = (h as f32 * REACH / (RISE + flow_lift())).max(1.0);
        self.decay = FAINT.powf(1.0 / frames);
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Advance one frame and paint it.
    pub fn step(&mut self, canvas: &mut Canvas, bands: &[f32], flare: f32, dt: f32, theme: &Theme) {
        let (w, h) = canvas.dims();
        if w == 0 || h == 0 {
            return;
        }
        self.resize(w, h);
        self.t += dt;
        let bass = bands
            .iter()
            .take(bands.len() / 6)
            .fold(0.0f32, |m, &v| m.max(v));
        self.inject(bands, flare);
        self.stir(bass);
        self.advect();
        canvas.paint(&self.density, &canvas::gradient_ramp(theme));
    }

    /// Drop ink along the bottom edge, one column per band.
    fn inject(&mut self, bands: &[f32], flare: f32) {
        let (w, h) = (self.w, self.h);
        for x in 0..w {
            let target = if bands.is_empty() {
                0.0
            } else {
                let i = (x * bands.len() / w).min(bands.len() - 1);
                bands[i]
            };
            let target = (target + flare * FLARE_GAIN).min(1.0);
            self.base[x] += (target - self.base[x]) * INJECT_EASE;
        }
        for x in 0..w {
            // A little grain, so the source edge is a row of droplets rather
            // than a painted line.
            let grain = 0.8 + (self.rand() >> 24) as f32 / 1275.0;
            self.density[(h - 1) * w + x] = (self.base[x] * grain).clamp(0.0, 1.0);
        }
    }

    /// Rebuild the current for this frame.
    fn stir(&mut self, bass: f32) {
        let strength = 1.0 + bass * FLOW_FROM_BASS;
        for gy in 0..self.gh {
            for gx in 0..self.gw {
                let (x, y) = ((gx * FLOW_GRID) as f32, (gy * FLOW_GRID) as f32);
                let (mut vx, mut vy) = (0.0, 0.0);
                for &(amp, kx, ky, speed) in &FLOW {
                    let (kx, ky) = (kx * std::f32::consts::TAU, ky * std::f32::consts::TAU);
                    let a = kx * x + self.t * speed;
                    let b = ky * y;
                    // The curl of `amp * sin(a) * cos(b)`.
                    vx += -amp * ky * a.sin() * b.sin();
                    vy += -amp * kx * a.cos() * b.cos();
                }
                self.flow[gy * self.gw + gx] = (vx * strength, vy * strength);
            }
        }
    }

    /// Carry the ink along the current.
    ///
    /// Backwards, which is what keeps it stable: rather than pushing each
    /// parcel to where it is going - which leaves gaps and stacks parcels on
    /// top of each other - every destination pixel asks where its ink came
    /// from and samples there.
    fn advect(&mut self) {
        let (w, h) = (self.w, self.h);
        for y in 0..h - 1 {
            for x in 0..w {
                let (vx, vy) = self.velocity(x, y);
                let sx = x as f32 - vx;
                let sy = y as f32 + RISE - vy;
                // A touch of blur across the flow, so filaments soften as they
                // travel instead of staying razor-edged forever.
                let v = 0.2 * self.sample(sx - 1.0, sy)
                    + 0.6 * self.sample(sx, sy)
                    + 0.2 * self.sample(sx + 1.0, sy);
                self.next[y * w + x] = v * self.decay;
            }
        }
        // The bottom row is the source and is rewritten every frame, so it is
        // carried across untouched rather than advected into itself.
        let last = (h - 1) * w;
        self.next[last..].copy_from_slice(&self.density[last..]);
        std::mem::swap(&mut self.density, &mut self.next);
    }

    /// The current at a pixel, interpolated from the coarse grid.
    fn velocity(&self, x: usize, y: usize) -> (f32, f32) {
        let (gx, gy) = (x / FLOW_GRID, y / FLOW_GRID);
        let fx = (x % FLOW_GRID) as f32 / FLOW_GRID as f32;
        let fy = (y % FLOW_GRID) as f32 / FLOW_GRID as f32;
        let at =
            |gx: usize, gy: usize| self.flow[gy.min(self.gh - 1) * self.gw + gx.min(self.gw - 1)];
        let (a, b, c, d) = (
            at(gx, gy),
            at(gx + 1, gy),
            at(gx, gy + 1),
            at(gx + 1, gy + 1),
        );
        let lerp = |p: f32, q: f32, t: f32| p + (q - p) * t;
        (
            lerp(lerp(a.0, b.0, fx), lerp(c.0, d.0, fx), fy),
            lerp(lerp(a.1, b.1, fx), lerp(c.1, d.1, fx), fy),
        )
    }

    /// Bilinear sample, clamped at the edges so ink gathers against the sides
    /// rather than vanishing into them.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let (w, h) = (self.w, self.h);
        let cx = x.clamp(0.0, (w - 1) as f32);
        let cy = y.clamp(0.0, (h - 1) as f32);
        let (i, j) = (cx.floor() as usize, cy.floor() as usize);
        let (fx, fy) = (cx - i as f32, cy - j as f32);
        let (i1, j1) = ((i + 1).min(w - 1), (j + 1).min(h - 1));
        let lerp = |p: f32, q: f32, t: f32| p + (q - p) * t;
        let top = lerp(self.density[j * w + i], self.density[j * w + i1], fx);
        let bottom = lerp(self.density[j1 * w + i], self.density[j1 * w + i1], fx);
        lerp(top, bottom, fy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bands: &[f32], frames: usize, w: usize, h: usize) -> (Ink, Canvas) {
        let mut canvas = Canvas::new();
        canvas.resize(w, h);
        let mut ink = Ink::new();
        for _ in 0..frames {
            ink.step(&mut canvas, bands, 0.0, 1.0 / 60.0, &Theme::default());
        }
        (ink, canvas)
    }

    fn top_row(ink: &Ink) -> f32 {
        (0..ink.w).fold(0.0f32, |m, x| m.max(ink.density[x]))
    }

    /// Ink has to rise and spread, but not reach the top: a plume that clips
    /// against the ceiling reads as a wall, which is the fire's failure mode
    /// too.
    #[test]
    fn plumes_rise_without_filling_the_pane() {
        let (ink, _) = run(&[1.0; 16], 400, 48, 40);
        let lit = ink.density.iter().filter(|&&d| d > 0.02).count();
        assert!(lit > ink.density.len() / 5, "barely any ink: {lit} pixels");
        assert!(
            top_row(&ink) < 0.2,
            "ink reached the top at {}",
            top_row(&ink)
        );
    }

    /// The reach must not depend on how many pixels the pane happens to have,
    /// or the same track looks different under sixel and under half blocks.
    #[test]
    fn reach_is_the_same_at_any_resolution() {
        let height_lit = |w, h| {
            let (ink, _) = run(&[1.0; 16], 500, w, h);
            let highest = (0..h)
                .find(|&y| (0..w).any(|x| ink.density[y * w + x] > 0.05))
                .unwrap_or(h);
            (h - highest) as f32 / h as f32
        };
        let small = height_lit(40, 28);
        let large = height_lit(160, 112);
        assert!(
            (small - large).abs() < 0.2,
            "reach differs with resolution: {small} vs {large}"
        );
    }

    /// The signature that separates ink from fire: a plume is *carried*. The
    /// fire walks a column up with a random nudge either way, which averages
    /// to straight; a current takes the whole plume somewhere.
    #[test]
    fn a_plume_is_carried_by_the_current() {
        // One loud band in the middle, silence either side.
        let mut bands = vec![0.0; 21];
        bands[10] = 1.0;
        let (ink, _) = run(&bands, 300, 42, 40);
        let centroid = |y: usize| {
            let row = &ink.density[y * ink.w..(y + 1) * ink.w];
            let mass: f32 = row.iter().sum();
            assert!(mass > 0.01, "no ink in row {y}");
            row.iter()
                .enumerate()
                .map(|(x, &d)| x as f32 * d)
                .sum::<f32>()
                / mass
        };
        let shift = (centroid(ink.h / 2) - centroid(ink.h - 2)).abs();
        assert!(
            shift > 2.0,
            "the plume rose straight up: centre moved {shift} pixels"
        );
    }

    /// When the music stops the water clears.
    #[test]
    fn silence_clears_the_water() {
        let (mut ink, mut canvas) = run(&[1.0; 16], 200, 48, 40);
        for _ in 0..600 {
            ink.step(&mut canvas, &[0.0; 16], 0.0, 1.0 / 60.0, &Theme::default());
        }
        let left = ink.density.iter().fold(0.0f32, |m, &d| m.max(d));
        assert!(left < 0.02, "ink left after silence: {left}");
    }
}
