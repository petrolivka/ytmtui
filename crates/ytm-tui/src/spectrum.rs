//! The spectrum widget - the thing that sits where album art would be.
//!
//! Renders with partial-block glyphs (8 sub-levels per cell), which is the most
//! font-safe option. Ratatui 0.30's `Marker::Octant` is available for a future
//! Canvas-based mode; see docs section 7.3.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ytm_viz::SpectrumFrame;

use crate::theme::Theme;

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VizStyle {
    Bars,
    Mirrored,
    Scope,
}

impl VizStyle {
    pub fn next(self) -> Self {
        match self {
            VizStyle::Bars => VizStyle::Mirrored,
            VizStyle::Mirrored => VizStyle::Scope,
            VizStyle::Scope => VizStyle::Bars,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            VizStyle::Bars => "bars",
            VizStyle::Mirrored => "mirrored",
            VizStyle::Scope => "scope",
        }
    }
}

pub struct Spectrum<'a> {
    pub frame: &'a SpectrumFrame,
    pub style: VizStyle,
    pub theme: &'a Theme,
    /// Columns per band. 2 leaves a blank gutter between bars, which is the
    /// difference between reading as bars and reading as a solid mass.
    pub step: u16,
}

/// The reflection is drawn shorter and dimmer than the real bars, rather than
/// as an exact mirror: a true mirror doubles the visual weight and fuses the
/// two halves into one slab across the middle.
const REFLECT_SCALE: f32 = 0.55;
const REFLECT_DIM: f32 = 0.45;

fn dim(c: Color, k: f32) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * k) as u8,
            (g as f32 * k) as u8,
            (b as f32 * k) as u8,
        ),
        other => other,
    }
}

impl Widget for Spectrum<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.frame.bands.is_empty() {
            return;
        }
        match self.style {
            VizStyle::Bars => self.bars(area, buf, false),
            VizStyle::Mirrored => self.bars(area, buf, true),
            VizStyle::Scope => self.scope(area, buf),
        }
    }
}

impl Spectrum<'_> {
    fn bars(&self, area: Rect, buf: &mut Buffer, mirrored: bool) {
        let h = area.height as usize;
        let step = self.step.max(1);
        let n = self
            .frame
            .bands
            .len()
            .min((area.width / step) as usize);

        // Reserve a baseline row so the two halves never touch, and give the
        // real bars two thirds of the height - an even split makes the
        // reflection compete with the signal instead of supporting it.
        let (top, bottom) = if mirrored && h >= 4 {
            let t = ((h - 1) * 2).div_ceil(3);
            (t, h - 1 - t)
        } else {
            (h, 0)
        };
        if top == 0 {
            return;
        }

        for i in 0..n {
            let x = area.x + i as u16 * step;
            let v = self.frame.bands[i];
            let p = self.frame.peaks[i];

            let sub = (v * (top * 8) as f32).round() as usize;
            for row in 0..top {
                let y = area.y + (top - 1 - row) as u16;
                let level = sub.saturating_sub(row * 8).min(8);
                if level == 0 {
                    continue; // leave empty cells untouched so caps can show
                }
                let t = if top > 1 { row as f32 / (top - 1) as f32 } else { 0.0 };
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(BLOCKS[level]).set_fg(self.theme.grad(t));
                }
            }

            // Peak cap: only above the bar, and only when it is clear of it, so
            // it never speckles the inside of a filled column.
            if p > 0.02 && p > v + 0.04 {
                let prow = ((p * top as f32).round() as usize).min(top - 1);
                let y = area.y + (top - 1 - prow) as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    if cell.symbol() == " " {
                        cell.set_char('▁').set_fg(self.theme.peak);
                    }
                }
            }

            // Reflection: shorter, dimmer, no caps.
            if bottom > 0 {
                let rv = v * REFLECT_SCALE;
                let sub = (rv * (bottom * 8) as f32).round() as usize;
                for row in 0..bottom {
                    let y = area.y + (top + 1 + row) as u16;
                    let level = sub.saturating_sub(row * 8).min(8);
                    if level == 0 {
                        continue;
                    }
                    let t = if bottom > 1 { row as f32 / (bottom - 1) as f32 } else { 0.0 };
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(BLOCKS[level])
                            .set_fg(dim(self.theme.grad(t), REFLECT_DIM));
                    }
                }
            }
        }
    }

    fn scope(&self, area: Rect, buf: &mut Buffer) {
        let h = area.height as usize;
        if h < 2 {
            return;
        }
        let mid = h / 2;
        let step = self.step.max(1);
        let n = self.frame.bands.len().min((area.width / step) as usize);
        for i in 0..n {
            let x = area.x + i as u16 * step;
            let amp = (self.frame.bands[i] * mid as f32) as usize;
            for y in [mid.saturating_sub(amp), (mid + amp).min(h - 1)] {
                if let Some(cell) = buf.cell_mut((x, area.y + y as u16)) {
                    let t = amp as f32 / mid.max(1) as f32;
                    cell.set_char('─').set_fg(self.theme.grad(t));
                }
            }
        }
    }
}
