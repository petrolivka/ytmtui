//! Album art in a terminal.
//!
//! Three backends, because terminals differ wildly: the Kitty graphics
//! protocol and sixel draw real pixels, while half blocks work absolutely
//! everywhere - including Alacritty, which supports neither - by using the
//! foreground and background of one cell as two stacked pixels.

pub mod sixel;

use anyhow::{Context, Result};
use image::imageops::FilterType;
pub use image::RgbImage;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
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

/// The pixel size of one terminal cell.
///
/// Every drawing decision here needs it: "square" means an equal number of
/// pixels each way, but an image is placed in *cells*. Assuming the classic
/// 8x16 - exactly twice as tall as wide - is what made covers come out
/// stretched under any font whose cells are not 2:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPx {
    pub w: u16,
    pub h: u16,
}

/// What to assume when the terminal will not say. The historical VGA cell, and
/// what this code used to hard-code everywhere.
pub const DEFAULT_CELL: CellPx = CellPx { w: 8, h: 16 };

impl CellPx {
    /// Columns per row for a square image: `cols = rows * aspect`.
    pub fn aspect(self) -> f32 {
        self.h as f32 / self.w as f32
    }
}

/// A configured cell aspect, as raw `f32` bits; zero means "measure".
///
/// A process-wide setting rather than a parameter because every drawing path
/// needs it and none of them otherwise carries the config. It is written once
/// at startup and again on a config reload.
static ASPECT_OVERRIDE: AtomicU32 = AtomicU32::new(0);

/// Override the measured cell shape. A value outside the plausible range - 0
/// included - restores measurement.
pub fn set_cell_aspect(aspect: f32) {
    ASPECT_OVERRIDE.store(aspect.to_bits(), Ordering::Relaxed);
}

fn aspect_override() -> Option<f32> {
    let a = f32::from_bits(ASPECT_OVERRIDE.load(Ordering::Relaxed));
    (a.is_finite() && (1.2..=4.0).contains(&a)).then_some(a)
}

/// Measure the terminal's cell size from the kernel's window size.
///
/// `ws_xpixel`/`ws_ypixel` are zero under tmux and in a few terminals, and
/// some report values that cannot be right; in either case there is nothing
/// better to do than fall back to 8x16. Cheap enough (one ioctl) to call per
/// frame, which keeps it correct across a font-size change.
pub fn cell_px() -> CellPx {
    let measured = measure_cell();
    match aspect_override() {
        // Keep the measured width: only the shape was disputed, and the width
        // is what a sixel image is scaled against.
        Some(a) => CellPx {
            w: measured.w,
            h: ((measured.w as f32 * a).round() as u16).max(2),
        },
        None => measured,
    }
}

fn measure_cell() -> CellPx {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let fd = std::io::stdout().as_raw_fd();
        if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
            && ws.ws_xpixel > 0
            && ws.ws_ypixel > 0
        {
            let cell = CellPx {
                w: ws.ws_xpixel / ws.ws_col,
                h: ws.ws_ypixel / ws.ws_row,
            };
            if plausible(cell) {
                return cell;
            }
        }
    }
    DEFAULT_CELL
}

/// Reject nonsense rather than trust it: a terminal reporting a cell 1px wide,
/// or one wider than it is tall, is reporting something other than a cell, and
/// believing it would distort every image far worse than the 8x16 guess.
fn plausible(c: CellPx) -> bool {
    c.w >= 2 && c.h >= 2 && (1.2..=4.0).contains(&c.aspect())
}

/// One cell of a half-block rendering: the upper and lower pixel colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub upper: (u8, u8, u8),
    pub lower: (u8, u8, u8),
}

/// Render to a grid of `rows` x `cols` half-block cells.
///
/// Splitting a cell vertically gives a pixel grid of `cols` x `2*rows`, whose
/// pixels are only square when the cell is exactly 2:1. The image is simply
/// stretched into that grid; keeping it undistorted on screen is the caller's
/// job, by choosing `cols` and `rows` from [`cell_px`].
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
/// Sixel does not know about cells, so this is how a sixel image is made to
/// land inside a pane - which means it has to use the terminal's real cell
/// size, not a guess.
pub fn resize_for_cells(img: &RgbImage, cols: u16, rows: u16) -> RgbImage {
    let cell = cell_px();
    image::imageops::resize(
        img,
        (cols as u32 * cell.w as u32).max(1),
        (rows as u32 * cell.h as u32).max(1),
        FilterType::Triangle,
    )
}

/// Nearest-neighbour scale. Used for the fire visualiser, where the simulation
/// runs at a deliberately low resolution and chunky pixels are the look.
pub fn scale_nearest(img: &RgbImage, w: u32, h: u32) -> RgbImage {
    image::imageops::resize(img, w.max(1), h.max(1), FilterType::Nearest)
}

/// The image id used for the cover.
///
/// Fixed rather than terminal-assigned so the placement can be deleted again:
/// unlike sixel, a Kitty placement is an object that survives the text being
/// redrawn over it, and stays until it is explicitly removed.
pub const COVER_IMAGE_ID: u32 = 7714;
/// The visualiser's own id, so deleting one image never disturbs the other.
pub const VIZ_IMAGE_ID: u32 = 7715;

/// Remove an image and its placements.
///
/// Needed whenever a picture stops being shown - fullscreen, a dialogue, art
/// turned off - because redrawing the screen does not erase it.
pub fn kitty_delete(id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")
}

/// Placement id for the visualiser.
///
/// Naming the placement is what makes an animation possible: transmitting
/// under the same image *and* placement id replaces what is on screen in one
/// step. Deleting first and re-transmitting leaves a gap every frame in which
/// the cells underneath show through.
const VIZ_PLACEMENT_ID: u32 = 1;

/// Kitty graphics protocol, raw RGB rather than PNG.
///
/// For the fire visualiser, which sends a new image every frame: PNG
/// compression costs milliseconds per frame and buys nothing, because the
/// image is deliberately small and Kitty scales it up to `cols` x `rows`.
pub fn to_kitty_rgb(img: &RgbImage, cols: u16, rows: u16) -> String {
    use base64::Engine;
    let (w, h) = img.dimensions();
    let b64 = base64::engine::general_purpose::STANDARD.encode(img.as_raw());
    let mut out = String::with_capacity(b64.len() + 256);
    let mut chunks = b64.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = if chunks.peek().is_some() { 1 } else { 0 };
        let payload = std::str::from_utf8(chunk).unwrap_or_default();
        if first {
            out.push_str(&format!(
                "\x1b_Ga=T,f=24,s={w},v={h},i={VIZ_IMAGE_ID},p={VIZ_PLACEMENT_ID},\
                 c={cols},r={rows},m={more},q=2;{payload}\x1b\\"
            ));
            first = false;
        } else {
            out.push_str(&format!("\x1b_Gm={more},q=2;{payload}\x1b\\"));
        }
    }
    out
}

/// Kitty graphics protocol: a base64 PNG delivered in chunks.
pub fn to_kitty(img: &RgbImage, cols: u16, rows: u16) -> Result<String> {
    use base64::Engine;
    let scaled = resize_for_cells(img, cols, rows);
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
            // q=2 suppresses the terminal's acknowledgement. Without it the
            // reply arrives on stdin and the key parser has to cope with it.
            out.push_str(&format!(
                "\x1b_Ga=T,f=100,i={COVER_IMAGE_ID},c={cols},r={rows},m={more},q=2;{}\x1b\\",
                std::str::from_utf8(chunk).unwrap_or_default()
            ));
            first = false;
        } else {
            out.push_str(&format!(
                "\x1b_Gm={more},q=2;{}\x1b\\",
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

    /// Should a fetch be skipped for this URL right now? True when it is
    /// already loaded, or failed recently enough not to retry yet.
    pub fn has(&self, url: &str) -> bool {
        let loaded = self
            .images
            .lock()
            .map(|m| matches!(m.get(url), Some(Some(_))))
            .unwrap_or(false);
        if loaded {
            return true;
        }
        self.failed_at
            .lock()
            .map(|f| f.get(url).is_some_and(|t| t.elapsed() < RETRY_AFTER))
            .unwrap_or(false)
    }

    /// Record a failure without a network round trip, for tests.
    #[cfg(test)]
    fn mark_failed(&self, url: &str, at: Instant) {
        self.images.lock().unwrap().insert(url.to_string(), None);
        self.failed_at.lock().unwrap().insert(url.to_string(), at);
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

    /// A transient network error must not blank a track's cover for the rest
    /// of the session, which is what caching the failure forever did.
    #[test]
    fn failed_fetches_are_retried_after_a_while() {
        let c = ArtCache::new().unwrap();
        let url = "https://example.invalid/cover.jpg";
        assert!(!c.has(url), "an unknown URL should not be skipped");

        c.mark_failed(url, Instant::now());
        assert!(c.has(url), "a fresh failure should be left alone");
        assert!(c.get(url).is_none());

        c.mark_failed(url, Instant::now() - RETRY_AFTER - Duration::from_secs(1));
        assert!(!c.has(url), "an old failure should be retried");
    }

    /// A Kitty placement survives the screen being redrawn, so there has to be
    /// a way to remove it, and the transmit has to name an id the delete can
    /// refer to.
    #[test]
    fn kitty_images_can_be_deleted_again() {
        let img = RgbImage::from_pixel(8, 8, Rgb([1, 2, 3]));
        let seq = to_kitty(&img, 4, 2).unwrap();
        assert!(
            seq.contains(&format!("i={COVER_IMAGE_ID}")),
            "transmit carries no image id"
        );
        assert!(seq.contains("q=2"), "responses are not suppressed");

        let del = kitty_delete(COVER_IMAGE_ID);
        assert!(del.contains("a=d"), "not a delete command");
        assert!(
            del.contains(&format!("i={COVER_IMAGE_ID}")),
            "delete targets no id"
        );
        assert!(del.starts_with("\x1b_G") && del.ends_with("\x1b\\"));
    }

    /// A bad cell measurement distorts every image, so anything implausible
    /// has to fall back rather than be believed.
    #[test]
    fn implausible_cell_sizes_are_rejected() {
        assert!(plausible(DEFAULT_CELL));
        assert!(plausible(CellPx { w: 9, h: 21 }));
        assert!(!plausible(CellPx { w: 1, h: 16 }), "1px-wide cell");
        assert!(!plausible(CellPx { w: 16, h: 8 }), "wider than tall");
        assert!(!plausible(CellPx { w: 4, h: 40 }), "ten times taller");
        assert!((DEFAULT_CELL.aspect() - 2.0).abs() < 1e-6);
        // Whatever the environment, the measurement itself must be usable.
        assert!(plausible(cell_px()));
    }

    /// The escape hatch for terminals that will not report a cell size. It has
    /// to actually take effect, and nonsense has to fall back to measuring.
    #[test]
    fn a_configured_aspect_overrides_the_measurement() {
        let measured = cell_px();
        set_cell_aspect(2.5);
        let forced = cell_px();
        assert_eq!(forced.w, measured.w, "the width should not move");
        assert!(
            (forced.aspect() - 2.5).abs() < 0.1,
            "got {}",
            forced.aspect()
        );

        set_cell_aspect(0.0);
        assert_eq!(cell_px(), measured, "zero should restore measurement");
        set_cell_aspect(f32::NAN);
        assert_eq!(cell_px(), measured, "nonsense should restore measurement");
        set_cell_aspect(0.0);
    }

    /// The visualiser sends a frame every tick, so the transmit has to replace
    /// what is on screen by itself. Without a placement id the only way to
    /// replace it is to delete first, and the gap that leaves is visible.
    #[test]
    fn the_animated_transmit_replaces_its_own_placement() {
        let img = RgbImage::from_pixel(4, 6, Rgb([9, 9, 9]));
        let seq = to_kitty_rgb(&img, 2, 1);
        assert!(seq.starts_with("\x1b_G") && seq.ends_with("\x1b\\"));
        assert!(seq.contains(&format!("i={VIZ_IMAGE_ID}")), "no image id");
        assert!(
            seq.contains(&format!("p={VIZ_PLACEMENT_ID}")),
            "no placement id, so each frame would need a delete first"
        );
        assert!(seq.contains("f=24"), "not raw RGB");
        assert!(seq.contains("s=4,v=6"), "wrong source size");
        assert!(seq.contains("c=2,r=1"), "not placed in the pane");
        // A stray newline or space in the control block is a protocol error.
        let controls = &seq[..seq.find(';').expect("no payload separator")];
        assert!(
            !controls.contains(' ') && !controls.contains('\n'),
            "whitespace in the control block: {controls:?}"
        );
        // The visualiser and the cover must never delete each other.
        assert_ne!(VIZ_IMAGE_ID, COVER_IMAGE_ID);
    }

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
