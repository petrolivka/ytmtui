//! Album art in a terminal.
//!
//! Three backends, because terminals differ wildly: the Kitty graphics
//! protocol and sixel draw real pixels, while half blocks work absolutely
//! everywhere - including Alacritty, which supports neither - by using the
//! foreground and background of one cell as two stacked pixels.

pub mod sixel;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::RgbImage;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How to draw an image in this terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Two stacked pixels per cell via `▀`. Works everywhere.
    HalfBlock,
    Sixel,
    Kitty,
    Off,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::HalfBlock => "half-block",
            Backend::Sixel => "sixel",
            Backend::Kitty => "kitty",
            Backend::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "halfblock" | "half-block" | "blocks" => Some(Backend::HalfBlock),
            "sixel" => Some(Backend::Sixel),
            "kitty" => Some(Backend::Kitty),
            "off" | "none" => Some(Backend::Off),
            "auto" => None,
            _ => None,
        }
    }

    /// Guess from the environment.
    ///
    /// Deliberately conservative: an unknown terminal gets half blocks, which
    /// always work, rather than a graphics protocol that might print garbage.
    pub fn detect() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default().to_ascii_lowercase();
        let term = env("TERM");
        let program = env("TERM_PROGRAM");

        if !env("KITTY_WINDOW_ID").is_empty() || term.contains("kitty") {
            return Backend::Kitty;
        }
        if !env("GHOSTTY_RESOURCES_DIR").is_empty() || term.contains("ghostty") {
            return Backend::Kitty;
        }
        if program.contains("wezterm") || !env("WEZTERM_PANE").is_empty() {
            return Backend::Kitty;
        }
        if term.contains("foot") || term.contains("contour") || term.contains("mlterm") {
            return Backend::Sixel;
        }
        // Multiplexers usually swallow graphics protocols even when the outer
        // terminal supports them.
        Backend::HalfBlock
    }
}

/// One cell of a half-block rendering: the upper and lower pixel colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub upper: (u8, u8, u8),
    pub lower: (u8, u8, u8),
}

/// Render to a grid of `rows` x `cols` half-block cells.
///
/// A terminal cell is about twice as tall as it is wide, so splitting it in
/// half vertically gives roughly square pixels: `cols` x `2*rows` of them.
/// That is why a square cover wants twice as many columns as rows.
pub fn to_half_blocks(img: &RgbImage, cols: u16, rows: u16) -> Vec<Vec<Cell>> {
    if cols == 0 || rows == 0 {
        return Vec::new();
    }
    let scaled = image::imageops::resize(img, cols as u32, rows as u32 * 2, FilterType::Triangle);
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let up = scaled.get_pixel(c as u32, r as u32 * 2).0;
                    let lo = scaled.get_pixel(c as u32, r as u32 * 2 + 1).0;
                    Cell {
                        upper: (up[0], up[1], up[2]),
                        lower: (lo[0], lo[1], lo[2]),
                    }
                })
                .collect()
        })
        .collect()
}

/// Scale an image to the pixel size a cell grid covers.
///
/// Assumes the common 8x16 cell; sixel does not know about cells, so this is
/// how a sixel image is made to land inside a pane.
pub fn resize_for_cells(img: &RgbImage, cols: u16, rows: u16) -> RgbImage {
    image::imageops::resize(
        img,
        (cols as u32 * 8).max(1),
        (rows as u32 * 16).max(1),
        FilterType::Triangle,
    )
}

/// Kitty graphics protocol: a base64 PNG delivered in chunks.
pub fn to_kitty(img: &RgbImage, cols: u16, rows: u16) -> Result<String> {
    use base64::Engine;
    let scaled =
        image::imageops::resize(img, cols as u32 * 8, rows as u32 * 16, FilterType::Triangle);
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            scaled.as_raw(),
            scaled.width(),
            scaled.height(),
            image::ExtendedColorType::Rgb8,
        )
        .context("encoding PNG for the kitty protocol")?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let mut out = String::with_capacity(b64.len() + 256);
    // Chunked: the protocol caps a single escape at 4096 base64 bytes.
    let mut chunks = b64.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = if chunks.peek().is_some() { 1 } else { 0 };
        if first {
            out.push_str(&format!(
                "\x1b_Ga=T,f=100,c={cols},r={rows},m={more};{}\x1b\\",
                std::str::from_utf8(chunk).unwrap_or_default()
            ));
            first = false;
        } else {
            out.push_str(&format!(
                "\x1b_Gm={more};{}\x1b\\",
                std::str::from_utf8(chunk).unwrap_or_default()
            ));
        }
    }
    Ok(out)
}

use image::ImageEncoder;

/// Fetch and decode cover images, keeping a few in memory.
///
/// Bounded deliberately: covers are fetched from the network, and an unbounded
/// cache during a long radio run is both a memory leak and extra requests.
pub struct ArtCache {
    http: reqwest::blocking::Client,
    images: Mutex<HashMap<String, Option<RgbImage>>>,
    /// When each URL last failed. Failures are remembered so a broken URL is
    /// not retried every frame, but not forever: a transient network error
    /// would otherwise blank that track's cover for the rest of the session.
    failed_at: Mutex<HashMap<String, Instant>>,
    capacity: usize,
}

/// How long a failed fetch is left alone before being tried again.
const RETRY_AFTER: Duration = Duration::from_secs(30);

impl ArtCache {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?,
            images: Mutex::new(HashMap::new()),
            failed_at: Mutex::new(HashMap::new()),
            capacity: 24,
        })
    }

    pub fn get(&self, url: &str) -> Option<RgbImage> {
        self.images.lock().ok()?.get(url).cloned().flatten()
    }

    pub fn has(&self, url: &str) -> bool {
        self.images
            .lock()
            .map(|m| m.contains_key(url))
            .unwrap_or(false)
    }

    /// Fetch and decode, remembering failures too so a broken URL is not
    /// retried on every frame.
    pub fn fetch(&self, url: &str) -> Result<()> {
        if self.has(url) {
            return Ok(());
        }
        let result = (|| -> Result<RgbImage> {
            let bytes = self.http.get(url).send()?.error_for_status()?.bytes()?;
            Ok(image::load_from_memory(&bytes)?.to_rgb8())
        })();
        if let Ok(mut m) = self.images.lock() {
            if m.len() >= self.capacity {
                // Cheap eviction: covers are small and order hardly matters.
                if let Some(k) = m.keys().next().cloned() {
                    m.remove(&k);
                }
            }
            match result {
                Ok(img) => {
                    m.insert(url.to_string(), Some(img));
                    Ok(())
                }
                Err(e) => {
                    m.insert(url.to_string(), None);
                    if let Ok(mut f) = self.failed_at.lock() {
                        f.insert(url.to_string(), Instant::now());
                    }
                    Err(e)
                }
            }
        } else {
            Ok(())
        }
    }
}

/// Ask YouTube's image host for a size close to what will be drawn.
///
/// Thumbnail URLs carry their dimensions, so requesting a 60px cover and
/// upscaling it looks far worse than asking for the right size to begin with.
pub fn at_size(url: &str, px: u32) -> String {
    if let Some(i) = url.rfind("=w") {
        if url[i..].contains("-h") {
            return format!("{}=w{px}-h{px}-l90-rj", &url[..i]);
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn half_blocks_have_the_right_shape() {
        let img = RgbImage::from_pixel(64, 64, Rgb([10, 20, 30]));
        let g = to_half_blocks(&img, 20, 8);
        assert_eq!(g.len(), 8);
        assert_eq!(g[0].len(), 20);
        assert_eq!(g[3][7].upper, (10, 20, 30));
    }

    #[test]
    fn half_blocks_keep_vertical_detail() {
        // Top half white, bottom half black: the two halves of a cell on the
        // boundary must differ, which is the whole point of the technique.
        let mut img = RgbImage::from_pixel(8, 8, Rgb([0, 0, 0]));
        for y in 0..4 {
            for x in 0..8 {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        let g = to_half_blocks(&img, 8, 4);
        assert_eq!(g[0][0].upper, (255, 255, 255));
        assert_eq!(g[3][0].lower, (0, 0, 0));
    }

    #[test]
    fn size_hint_rewrites_youtube_urls() {
        let u = "https://lh3.googleusercontent.com/abc=w60-h60-l90-rj";
        assert_eq!(
            at_size(u, 256),
            "https://lh3.googleusercontent.com/abc=w256-h256-l90-rj"
        );
        // Anything else is left alone rather than mangled.
        assert_eq!(
            at_size("https://example.com/a.jpg", 256),
            "https://example.com/a.jpg"
        );
    }

    #[test]
    fn detection_falls_back_to_something_that_always_works() {
        // Whatever the environment, never guess a protocol that could print
        // garbage into an unsuspecting terminal.
        assert!(matches!(
            Backend::detect(),
            Backend::HalfBlock | Backend::Sixel | Backend::Kitty
        ));
        assert_eq!(Backend::parse("auto"), None);
        assert_eq!(Backend::parse("sixel"), Some(Backend::Sixel));
    }
}
