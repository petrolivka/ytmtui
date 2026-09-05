//! The pixel visualisers, behind one door.
//!
//! Styles here are drawn as an image and delivered as real pixels where the
//! terminal has sixel or the Kitty protocol, and as half blocks where it has
//! neither. Everything above this - the app, the layout, the escape writing -
//! only needs to know which styles those are and where the current frame is.

use crate::canvas::Canvas;
use crate::fire::Fire;
use crate::ink::Ink;
use crate::scope;
use crate::spectrum::VizStyle;
use crate::theme::Theme;
use ytm_art::RgbImage;
use ytm_viz::SpectrumFrame;

#[derive(Default)]
pub struct Pixels {
    canvas: Canvas,
    fire: Fire,
    ink: Ink,
    /// Which style the simulations currently hold state for. Switching styles
    /// has to reset them, or a fire lit before the switch smoulders on inside
    /// the ink.
    live: Option<VizStyle>,
}

impl Pixels {
    pub fn new() -> Self {
        Self {
            fire: Fire::new(),
            ink: Ink::new(),
            ..Default::default()
        }
    }

    /// The frame to draw, or `None` when nothing has been rendered yet.
    pub fn image(&self) -> Option<&RgbImage> {
        self.canvas.image()
    }

    /// Let go of every buffer, for when a glyph style is showing.
    pub fn clear(&mut self) {
        if self.live.is_none() {
            return;
        }
        self.live = None;
        self.canvas.release();
        self.fire.clear();
        self.ink.clear();
    }

    /// Render one frame at `w` x `h` pixels.
    pub fn step(
        &mut self,
        style: VizStyle,
        size: (usize, usize),
        frame: &SpectrumFrame,
        flare: f32,
        dt: f32,
        theme: &Theme,
    ) {
        if self.live != Some(style) {
            self.clear();
            self.live = Some(style);
        }
        // A resize invalidates a simulation's grid as surely as a style change
        // does, and both of them start from an empty canvas.
        if self.canvas.resize(size.0, size.1) {
            self.fire.clear();
            self.ink.clear();
        }
        match style {
            VizStyle::Scope => scope::draw(&mut self.canvas, &frame.wave, theme),
            VizStyle::Fire => self.fire.step(&mut self.canvas, &frame.bands, flare, theme),
            VizStyle::Ink => self
                .ink
                .step(&mut self.canvas, &frame.bands, flare, dt, theme),
            // Not a pixel style; nothing to draw.
            _ => self.clear(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> SpectrumFrame {
        SpectrumFrame {
            bands: vec![0.8; 24],
            peaks: vec![0.8; 24],
            wave: (0..2048)
                .map(|i| (i as f32 * 0.1).sin())
                .collect::<Vec<_>>(),
            ..Default::default()
        }
    }

    fn run(style: VizStyle, frames: usize) -> Pixels {
        let mut p = Pixels::new();
        for _ in 0..frames {
            p.step(
                style,
                (40, 24),
                &frame(),
                0.0,
                1.0 / 60.0,
                &Theme::default(),
            );
        }
        p
    }

    #[test]
    fn every_pixel_style_paints_something() {
        for style in [VizStyle::Scope, VizStyle::Fire, VizStyle::Ink] {
            assert!(style.is_pixel(), "{} is not marked as pixels", style.name());
            let p = run(style, 120);
            let img = p.image().unwrap_or_else(|| panic!("{}", style.name()));
            assert_eq!((img.width(), img.height()), (40, 24));
            let lit = img.pixels().filter(|p| p.0.iter().any(|&c| c > 8)).count();
            assert!(lit > 20, "{} painted {lit} lit pixels", style.name());
        }
    }

    /// Switching styles must not leave the previous simulation running
    /// underneath: fire heat carried into the ink would show as plumes that
    /// nothing injected.
    #[test]
    fn switching_style_starts_from_nothing() {
        let mut p = run(VizStyle::Fire, 200);
        // One ink frame from cold cannot have reached any height yet.
        p.step(
            VizStyle::Ink,
            (40, 24),
            &frame(),
            0.0,
            1.0 / 60.0,
            &Theme::default(),
        );
        let img = p.image().unwrap();
        let top_half_lit = (0..12)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y).0.iter().any(|&c| c > 8))
            .count();
        assert_eq!(top_half_lit, 0, "the fire survived the switch to ink");
    }

    /// A glyph style asks for nothing, so the buffers must go.
    #[test]
    fn a_glyph_style_releases_the_buffers() {
        let mut p = run(VizStyle::Fire, 60);
        assert!(p.image().is_some());
        p.step(
            VizStyle::Bars,
            (40, 24),
            &frame(),
            0.0,
            1.0 / 60.0,
            &Theme::default(),
        );
        assert!(p.image().is_none(), "buffers kept for a glyph style");
    }
}
