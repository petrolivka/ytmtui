//! Colours. One place, so themes are a config file rather than a code change.

use ratatui::style::Color;
use ytm_config::{parse_rgb, Config};

pub struct Theme {
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub border: Color,
    pub border_focus: Color,
    pub selection_bg: Color,
    pub error: Color,
    pub ok: Color,
    pub peak: Color,
    /// Spectrum gradient, low amplitude to high.
    pub spectrum: Vec<(u8, u8, u8)>,
}

fn col(s: &str, fallback: (u8, u8, u8)) -> Color {
    let (r, g, b) = parse_rgb(s).unwrap_or(fallback);
    Color::Rgb(r, g, b)
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

impl Theme {
    pub fn from_config(c: &Config) -> Self {
        let t = &c.theme;
        let mut spectrum: Vec<(u8, u8, u8)> =
            t.spectrum.iter().filter_map(|s| parse_rgb(s)).collect();
        // A gradient needs at least two stops to interpolate between.
        if spectrum.len() < 2 {
            spectrum = vec![(0x1d, 0xb9, 0x54), (0xe8, 0xc0, 0x20), (0xff, 0x33, 0x3a)];
        }
        Self {
            fg: col(&t.fg, (0xe6, 0xe6, 0xea)),
            dim: col(&t.dim, (0x7a, 0x7a, 0x88)),
            accent: col(&t.accent, (0xff, 0x33, 0x3a)),
            border: col(&t.border, (0x3a, 0x3a, 0x46)),
            border_focus: col(&t.border_focus, (0xff, 0x33, 0x3a)),
            selection_bg: col(&t.selection_bg, (0x2a, 0x2a, 0x36)),
            error: col(&t.error, (0xff, 0x6b, 0x6b)),
            ok: col(&t.ok, (0x4a, 0xd2, 0x95)),
            peak: col(&t.peak, (0x9a, 0x9a, 0xb0)),
            spectrum,
        }
    }

    /// Amplitude (0..1) to colour, interpolating across the gradient stops.
    pub fn grad(&self, t: f32) -> Color {
        let (r, g, b) = self.grad_rgb(t);
        Color::Rgb(r, g, b)
    }

    /// The same, as components. The fire visualiser builds a palette rather
    /// than styling cells, so it needs the numbers, not a `Color`.
    pub fn grad_rgb(&self, t: f32) -> (u8, u8, u8) {
        let n = self.spectrum.len();
        if n == 0 {
            return match self.accent {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (0xff, 0x33, 0x3a),
            };
        }
        if n == 1 {
            return self.spectrum[0];
        }
        let t = t.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (t.floor() as usize).min(n - 2);
        let k = t - i as f32;
        let (a, b) = (self.spectrum[i], self.spectrum[i + 1]);
        let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
        (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
    }
}
