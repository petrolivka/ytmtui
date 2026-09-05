//! A pixel buffer for the visualisers that draw pictures rather than glyphs.
//!
//! Everything here works in pixels and knows nothing about cells. What those
//! pixels become is decided later: real ones through sixel or the Kitty
//! protocol, or half blocks where the terminal has neither.

use crate::theme::Theme;
use ytm_art::RgbImage;

/// Colour steps in a ramp.
///
/// Far fewer than the values being coloured, on purpose: neighbouring pixels
/// then share an entry, which is what lets the sixel encoder compress a frame
/// into long runs instead of a byte per pixel.
pub const SHADES: usize = 64;

pub type Ramp = [[u8; 3]; SHADES];

/// Index into a ramp for a 0..=1 value.
pub fn shade(v: f32) -> usize {
    ((v.clamp(0.0, 1.0) * (SHADES - 1) as f32) as usize).min(SHADES - 1)
}

fn scale(c: (u8, u8, u8), k: f32) -> [u8; 3] {
    [
        (c.0 as f32 * k) as u8,
        (c.1 as f32 * k) as u8,
        (c.2 as f32 * k) as u8,
    ]
}

/// The theme gradient, faded to black at the low end.
///
/// For anything whose value really is an amount of something - ink in water,
/// the height of a trace - where the gradient's own quiet-to-loud direction is
/// the right one and only the unlit end needs inventing.
pub fn gradient_ramp(theme: &Theme) -> Ramp {
    let mut ramp = [[0u8; 3]; SHADES];
    for (i, slot) in ramp.iter_mut().enumerate() {
        let t = i as f32 / (SHADES - 1) as f32;
        *slot = scale(theme.grad_rgb(t), t.sqrt());
    }
    ramp
}

/// Black, through the gradient's hottest colour, to white.
///
/// For heat, which is not amplitude: using the gradient as it stands puts its
/// *cold* end - green, by default - at the tips of the flames. Built around
/// one colour instead, so any theme survives it and a green theme burns green.
pub fn heat_ramp(theme: &Theme) -> Ramp {
    /// Where the theme's colour sits on the ramp. Most of the range is spent
    /// approaching it, so only the cores burn out to white.
    const HOT_AT: f32 = 0.62;

    let hot = theme.grad_rgb(1.0);
    let mut ramp = [[0u8; 3]; SHADES];
    for (i, slot) in ramp.iter_mut().enumerate() {
        let t = i as f32 / (SHADES - 1) as f32;
        let mix = |c: u8, towards: f32, k: f32| (c as f32 + (towards - c as f32) * k) as u8;
        *slot = if t < HOT_AT {
            scale(hot, t / HOT_AT)
        } else {
            // Steep, so white is the core rather than the whole upper half.
            let k = ((t - HOT_AT) / (1.0 - HOT_AT)).powf(1.8);
            [
                mix(hot.0, 255.0, k),
                mix(hot.1, 255.0, k),
                mix(hot.2, 255.0, k),
            ]
        };
    }
    ramp
}

#[derive(Default)]
pub struct Canvas {
    w: usize,
    h: usize,
    img: Option<RgbImage>,
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    pub fn image(&self) -> Option<&RgbImage> {
        self.img.as_ref()
    }

    /// Size the buffer, reporting whether it changed - which is a simulation's
    /// cue that whatever state it holds is the wrong shape now.
    pub fn resize(&mut self, w: usize, h: usize) -> bool {
        if self.w == w && self.h == h && self.img.is_some() {
            return false;
        }
        self.w = w;
        self.h = h;
        self.img = (w > 0 && h > 0).then(|| RgbImage::new(w as u32, h as u32));
        true
    }

    /// Release the buffer. Called when no picture is being drawn, so a
    /// pane-sized image is not held for a visualiser nobody is looking at.
    pub fn release(&mut self) {
        self.w = 0;
        self.h = 0;
        self.img = None;
    }

    pub fn clear(&mut self) {
        if let Some(img) = &mut self.img {
            img.fill(0);
        }
    }

    pub fn put(&mut self, x: usize, y: usize, c: [u8; 3]) {
        if x >= self.w || y >= self.h {
            return;
        }
        let Some(img) = &mut self.img else { return };
        let buf: &mut [u8] = img;
        let i = (y * self.w + x) * 3;
        buf[i..i + 3].copy_from_slice(&c);
    }

    /// Draw over what is there, weighted. Used for the soft edges that make a
    /// one-pixel line read as a glowing trace rather than a scratch.
    pub fn blend(&mut self, x: usize, y: usize, c: [u8; 3], alpha: f32) {
        if x >= self.w || y >= self.h {
            return;
        }
        let Some(img) = &mut self.img else { return };
        let buf: &mut [u8] = img;
        let i = (y * self.w + x) * 3;
        for k in 0..3 {
            let under = buf[i + k] as f32;
            let over = c[k] as f32;
            buf[i + k] = under.max(under + (over - under) * alpha) as u8;
        }
    }

    /// Paint the whole canvas from a value-per-pixel grid through a ramp.
    ///
    /// The simulations all end this way, and doing it here keeps the bounds
    /// check and the buffer reuse in one place.
    pub fn paint(&mut self, values: &[f32], ramp: &Ramp) {
        let Some(img) = &mut self.img else { return };
        let buf: &mut [u8] = img;
        for (i, &v) in values.iter().take(self.w * self.h).enumerate() {
            buf[i * 3..i * 3 + 3].copy_from_slice(&ramp[shade(v)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ramp_starts_unlit() {
        for ramp in [
            gradient_ramp(&Theme::default()),
            heat_ramp(&Theme::default()),
        ] {
            assert_eq!(ramp[0], [0, 0, 0], "the cold end has to be black");
            assert!(
                ramp[SHADES - 1].iter().any(|&c| c > 100),
                "the hot end has to be lit"
            );
        }
    }

    /// The heat ramp exists because the gradient runs the wrong way for it:
    /// its hottest colour must be the theme's, not the theme's quiet end.
    #[test]
    fn the_heat_ramp_ends_where_the_gradient_does() {
        let theme = Theme::default();
        let hot = theme.grad_rgb(1.0);
        let ramp = heat_ramp(&theme);
        let mid = ramp[(SHADES as f32 * 0.62) as usize];
        for k in 0..3 {
            let want = [hot.0, hot.1, hot.2][k] as i32;
            assert!(
                (mid[k] as i32 - want).abs() < 12,
                "ramp reaches {mid:?}, not {hot:?}"
            );
        }
    }

    #[test]
    fn drawing_outside_the_canvas_is_ignored() {
        let mut c = Canvas::new();
        assert!(c.resize(4, 3));
        assert!(!c.resize(4, 3), "an unchanged size should not rebuild");
        c.put(99, 0, [255, 0, 0]);
        c.put(0, 99, [255, 0, 0]);
        c.put(1, 1, [255, 0, 0]);
        let img = c.image().unwrap();
        assert_eq!(img.get_pixel(1, 1).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0]);
    }
}
