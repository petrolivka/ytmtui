//! The spectrum widget - the thing that sits where album art would be.
//!
//! Renders with partial-block glyphs (8 sub-levels per cell), which is the most
//! font-safe option. Ratatui 0.30's `Marker::Octant` is available for a future
//! Canvas-based mode; see docs section 7.3.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ytm_art::Cell as ArtCell;
use ytm_viz::{SpectrumFrame, N_CHROMA};

use crate::theme::Theme;

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VizStyle {
    Bars,
    Mirrored,
    Scope,
    /// Scrolling heat-map of the last few seconds.
    Spectrogram,
    /// Scrolling pitch classes: which notes are sounding, not where the
    /// energy is.
    Chroma,
    /// Doom fire, stoked by the spectrum. Drawn as pixels, not glyphs.
    Fire,
    /// Ink dropped into moving water. Pixels too.
    Ink,
}

impl VizStyle {
    pub fn next(self) -> Self {
        match self {
            VizStyle::Bars => VizStyle::Mirrored,
            VizStyle::Mirrored => VizStyle::Scope,
            VizStyle::Scope => VizStyle::Spectrogram,
            VizStyle::Spectrogram => VizStyle::Chroma,
            VizStyle::Chroma => VizStyle::Fire,
            VizStyle::Fire => VizStyle::Ink,
            VizStyle::Ink => VizStyle::Bars,
        }
    }

    /// Styles drawn as an image rather than as glyphs.
    ///
    /// They share a pipeline: a pixel canvas, then either a graphics escape
    /// written after the frame or half blocks written into it.
    pub fn is_pixel(self) -> bool {
        matches!(self, VizStyle::Scope | VizStyle::Fire | VizStyle::Ink)
    }
    pub fn name(self) -> &'static str {
        match self {
            VizStyle::Bars => "bars",
            VizStyle::Mirrored => "mirrored",
            VizStyle::Scope => "scope",
            VizStyle::Spectrogram => "spectrogram",
            VizStyle::Chroma => "chroma",
            VizStyle::Fire => "fire",
            VizStyle::Ink => "ink",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "bars" => VizStyle::Bars,
            "scope" => VizStyle::Scope,
            "spectrogram" => VizStyle::Spectrogram,
            "chroma" => VizStyle::Chroma,
            "fire" => VizStyle::Fire,
            "ink" => VizStyle::Ink,
            _ => VizStyle::Mirrored,
        }
    }
}

pub struct Spectrum<'a> {
    pub frame: &'a SpectrumFrame,
    pub style: VizStyle,
    pub theme: &'a Theme,
    /// Newest-last history for the spectrogram; empty for other styles.
    pub history: &'a std::collections::VecDeque<Vec<f32>>,
    /// Newest-last pitch-class history for the chroma strip.
    pub chroma: &'a std::collections::VecDeque<[f32; N_CHROMA]>,
    /// The current pixel style, already rendered to half-block cells. Empty
    /// for glyph styles, and also when a graphics backend is drawing the
    /// picture as real pixels after the frame instead.
    pub pixels: &'a [Vec<ArtCell>],
    /// Columns per band. 2 leaves a blank gutter between bars, which is the
    /// difference between reading as bars and reading as a solid mass.
    pub step: u16,
}

/// The reflection is drawn shorter and dimmer than the real bars, rather than
/// as an exact mirror: a true mirror doubles the visual weight and fuses the
/// two halves into one slab across the middle.
const REFLECT_SCALE: f32 = 0.55;
const REFLECT_DIM: f32 = 0.45;

/// Pitch classes in circle-of-fifths order, bottom row first. Each is seven
/// semitones above the one before it.
const FIFTHS: [usize; N_CHROMA] = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5];
const FIFTH_NAMES: [&str; N_CHROMA] = [
    "C", "G", "D", "A", "E", "B", "F#", "C#", "G#", "D#", "A#", "F",
];

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
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Only these read the band values. The others take the waveform, the
        // pitch classes or a simulation's own state, and must not be blanked
        // just because no bands have arrived.
        if self.frame.bands.is_empty()
            && matches!(
                self.style,
                VizStyle::Bars | VizStyle::Mirrored | VizStyle::Spectrogram
            )
        {
            return;
        }
        match self.style {
            VizStyle::Bars => self.bars(area, buf, false),
            VizStyle::Mirrored => self.bars(area, buf, true),
            VizStyle::Spectrogram => self.spectrogram(area, buf),
            VizStyle::Chroma => self.chroma_strip(area, buf),
            _ => crate::cover::Cover { cells: self.pixels }.render(area, buf),
        }
    }
}

impl Spectrum<'_> {
    fn bars(&self, area: Rect, buf: &mut Buffer, mirrored: bool) {
        let h = area.height as usize;
        let step = self.step.max(1);
        let n = self.frame.bands.len().min((area.width / step) as usize);

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
                let t = if top > 1 {
                    row as f32 / (top - 1) as f32
                } else {
                    0.0
                };
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
                    let t = if bottom > 1 {
                        row as f32 / (bottom - 1) as f32
                    } else {
                        0.0
                    };
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(BLOCKS[level])
                            .set_fg(dim(self.theme.grad(t), REFLECT_DIM));
                    }
                }
            }
        }
    }

    /// Time runs left to right, frequency bottom to top, amplitude as colour.
    ///
    /// Each cell carries two frequency bins using a half block: foreground is
    /// the upper bin, background the lower, so vertical resolution doubles for
    /// free.
    fn spectrogram(&self, area: Rect, buf: &mut Buffer) {
        if self.history.is_empty() {
            return;
        }
        let h = area.height as usize;
        let w = area.width as usize;
        let bins = h * 2;
        // Show the most recent `w` columns, oldest on the left.
        let start = self.history.len().saturating_sub(w);
        for (col, frame) in self.history.iter().skip(start).enumerate() {
            if col >= w || frame.is_empty() {
                break;
            }
            let x = area.x + col as u16;
            for row in 0..h {
                // Row 0 is the top of the pane, so the highest frequencies.
                let upper = bins - 1 - row * 2;
                let lower = upper.saturating_sub(1);
                let pick = |b: usize| -> f32 {
                    let i = b * frame.len() / bins.max(1);
                    frame.get(i.min(frame.len() - 1)).copied().unwrap_or(0.0)
                };
                if let Some(cell) = buf.cell_mut((x, area.y + row as u16)) {
                    cell.set_char('\u{2580}')
                        .set_fg(self.theme.grad(pick(upper)))
                        .set_bg(self.theme.grad(pick(lower)));
                }
            }
        }
    }

    /// Pitch classes over time: which notes are sounding.
    ///
    /// Rows run up the circle of fifths rather than chromatically. The seven
    /// notes of a key are then seven *neighbouring* rows, so music in one key
    /// lights a contiguous band and a modulation slides it, where chromatic
    /// order would scatter the same notes across the pane.
    fn chroma_strip(&self, area: Rect, buf: &mut Buffer) {
        if self.chroma.is_empty() {
            return;
        }
        let h = area.height as usize;
        let w = area.width as usize;
        if h == 0 || w == 0 {
            return;
        }
        // Note names, when there is a row each to put them against.
        let gutter = if h >= N_CHROMA && w > 24 { 3 } else { 0 };
        let plot_w = w - gutter;
        if plot_w == 0 {
            return;
        }

        // Each cell carries two classes using a half block, exactly as the
        // spectrogram carries two frequency bins, so the twelve fit however
        // short the pane is.
        let slots = h * 2;
        let class_at = |slot: usize| FIFTHS[(slot * N_CHROMA / slots).min(N_CHROMA - 1)];

        let start = self.chroma.len().saturating_sub(plot_w);
        for (col, frame) in self.chroma.iter().skip(start).enumerate() {
            if col >= plot_w {
                break;
            }
            let x = area.x + (gutter + col) as u16;
            for row in 0..h {
                // Row 0 is the top of the pane, and slot 0 is the bottom.
                let upper = slots - 1 - row * 2;
                let lower = upper.saturating_sub(1);
                if let Some(cell) = buf.cell_mut((x, area.y + row as u16)) {
                    cell.set_char('\u{2580}')
                        .set_fg(self.theme.grad(frame[class_at(upper)]))
                        .set_bg(self.theme.grad(frame[class_at(lower)]));
                }
            }
        }

        if gutter == 0 {
            return;
        }
        // One label per class, on the row its upper half falls in, so the
        // names line up with the bands whatever the height.
        let mut labelled = [false; N_CHROMA];
        for row in 0..h {
            let i = ((slots - 1 - row * 2) * N_CHROMA / slots).min(N_CHROMA - 1);
            if labelled[i] {
                continue;
            }
            labelled[i] = true;
            for (k, ch) in format!("{:<3}", FIFTH_NAMES[i]).chars().enumerate() {
                if let Some(cell) = buf.cell_mut((area.x + k as u16, area.y + row as u16)) {
                    cell.set_char(ch)
                        .set_fg(self.theme.dim)
                        .set_bg(Color::Reset);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn strip(chroma: &VecDeque<[f32; N_CHROMA]>, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let theme = Theme::default();
        Spectrum {
            frame: &SpectrumFrame::default(),
            style: VizStyle::Chroma,
            theme: &theme,
            history: &VecDeque::new(),
            chroma,
            pixels: &[],
            step: 1,
        }
        .render(area, &mut buf);
        buf
    }

    /// One note held, so exactly one row of the strip should be lit.
    fn one_note(class: usize, frames: usize) -> VecDeque<[f32; N_CHROMA]> {
        let mut v = [0.0; N_CHROMA];
        v[class] = 1.0;
        (0..frames).map(|_| v).collect()
    }

    /// The note has to land on its own row, and on the row the label claims.
    #[test]
    fn a_note_lights_the_row_its_label_names() {
        // Twelve rows, so each class gets exactly one.
        let buf = strip(&one_note(9, 60), 60, 12); // 9 is A
        let lit: Vec<u16> = (0..12)
            .filter(|&y| buf[(40, y)].fg != Color::Rgb(0x1d, 0xb9, 0x54))
            .collect();
        assert_eq!(lit.len(), 1, "expected one lit row, got {lit:?}");

        // The label column on that row must read A.
        let row = lit[0];
        let name: String = (0..2).map(|x| buf[(x, row)].symbol()).collect();
        assert_eq!(name.trim(), "A", "row {row} is labelled {name:?}");
    }

    /// Circle-of-fifths order is the whole design: neighbouring rows have to
    /// be a fifth apart, so a key occupies a contiguous band.
    #[test]
    fn rows_run_up_the_circle_of_fifths() {
        for pair in FIFTHS.windows(2) {
            assert_eq!(
                (pair[0] + 7) % N_CHROMA,
                pair[1],
                "{pair:?} are not a fifth apart"
            );
        }
        assert_eq!(FIFTHS.len(), N_CHROMA);
        let mut seen = FIFTHS;
        seen.sort_unstable();
        assert!(
            seen.iter().enumerate().all(|(i, &c)| i == c),
            "the twelve classes are not all present exactly once"
        );
    }

    /// A pane too short for a row each still has to show all twelve classes,
    /// packed two to a cell, rather than dropping the ones that do not fit.
    #[test]
    fn a_short_pane_packs_two_classes_to_a_cell() {
        for class in 0..N_CHROMA {
            let buf = strip(&one_note(class, 40), 40, 6);
            let quiet = Color::Rgb(0x1d, 0xb9, 0x54);
            let shown = (0..6).any(|y| {
                let c = &buf[(20, y)];
                c.fg != quiet || c.bg != quiet
            });
            assert!(shown, "class {class} vanished in a six-row pane");
        }
    }

    #[test]
    fn every_style_has_a_name_that_round_trips() {
        let mut style = VizStyle::Bars;
        for _ in 0..7 {
            assert_eq!(VizStyle::parse(style.name()), style);
            style = style.next();
        }
        assert_eq!(style, VizStyle::Bars, "the cycle does not close");
    }
}
