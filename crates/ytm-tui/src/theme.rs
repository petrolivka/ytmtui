//! Colours. Kept in one place so themes become a config file later (FR-U7).

use ratatui::style::Color;

pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub border: Color,
    pub border_focus: Color,
    pub selection_bg: Color,
    pub error: Color,
    pub ok: Color,
    /// Spectrum gradient, low amplitude to high.
    pub spectrum: [(u8, u8, u8); 3],
    pub peak: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Reset,
            fg: Color::Rgb(0xe6, 0xe6, 0xea),
            dim: Color::Rgb(0x7a, 0x7a, 0x88),
            accent: Color::Rgb(0xff, 0x33, 0x3a),
            border: Color::Rgb(0x3a, 0x3a, 0x46),
            border_focus: Color::Rgb(0xff, 0x33, 0x3a),
            selection_bg: Color::Rgb(0x2a, 0x2a, 0x36),
            error: Color::Rgb(0xff, 0x6b, 0x6b),
            ok: Color::Rgb(0x4a, 0xd2, 0x95),
            spectrum: [(0x1d, 0xb9, 0x54), (0xe8, 0xc0, 0x20), (0xff, 0x33, 0x3a)],
            peak: Color::Rgb(0x9a, 0x9a, 0xb0),
        }
    }
}

impl Theme {
    /// Amplitude (0..1) to colour, interpolating through the gradient stops.
    pub fn grad(&self, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let [a, b, c] = self.spectrum;
        let (from, to, k) = if t < 0.5 { (a, b, t / 0.5) } else { (b, c, (t - 0.5) / 0.5) };
        let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
        Color::Rgb(lerp(from.0, to.0), lerp(from.1, to.1), lerp(from.2, to.2))
    }
}
