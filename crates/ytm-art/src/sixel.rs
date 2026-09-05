//! Minimal sixel encoder.
//!
//! Album art is large flat colour, not fine detail, so a fixed palette (a
//! 6x6x6 cube plus greys) is plenty and avoids carrying a quantiser.

use image::RgbImage;
use std::fmt::Write;

/// Palette index for an RGB triple: 216-colour cube plus 24 greys.
fn palette_index(r: u8, g: u8, b: u8) -> u8 {
    // Near-grey pixels quantise badly on the cube, so give them their own ramp.
    let max = r.max(g).max(b) as i32;
    let min = r.min(g).min(b) as i32;
    if max - min < 16 {
        let level = ((r as u32 + g as u32 + b as u32) / 3 * 23 / 255) as u8;
        return 216 + level;
    }
    let q = |v: u8| (v as u32 * 5 / 255) as u8;
    36 * q(r) + 6 * q(g) + q(b)
}

fn palette_rgb(i: u8) -> (u8, u8, u8) {
    if i >= 216 {
        let level = (i - 216) as u32 * 255 / 23;
        return (level as u8, level as u8, level as u8);
    }
    let to = |v: u32| (v * 255 / 5) as u8;
    let i = i as u32;
    (to(i / 36 % 6), to(i / 6 % 6), to(i % 6))
}

/// Encode an image as a sixel escape sequence.
pub fn encode(img: &RgbImage) -> String {
    let (w, h) = img.dimensions();
    let mut out = String::with_capacity((w * h) as usize / 4 + 1024);
    // DCS, then raster attributes: pixel aspect 1:1, then the size.
    let _ = write!(out, "\x1bPq\"1;1;{w};{h}");

    // Only emit the palette entries actually used.
    let mut indexed = vec![0u8; (w * h) as usize];
    let mut used = [false; 256];
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y).0;
            let i = palette_index(p[0], p[1], p[2]);
            indexed[(y * w + x) as usize] = i;
            used[i as usize] = true;
        }
    }
    for (i, u) in used.iter().enumerate() {
        if *u {
            let (r, g, b) = palette_rgb(i as u8);
            // Sixel colour components are percentages, not 0-255.
            let pc = |v: u8| (v as u32 * 100 / 255) as u32;
            let _ = write!(out, "#{};2;{};{};{}", i, pc(r), pc(g), pc(b));
        }
    }

    // Sixels cover six pixel rows at a time.
    for band in 0..h.div_ceil(6) {
        let y0 = band * 6;
        let mut first_colour_in_band = true;
        for (ci, u) in used.iter().enumerate() {
            if !*u {
                continue;
            }
            // Build this colour's row of sixels, then run-length encode it.
            let mut row: Vec<u8> = Vec::with_capacity(w as usize);
            let mut any = false;
            for x in 0..w {
                let mut bits = 0u8;
                for dy in 0..6 {
                    let y = y0 + dy;
                    if y < h && indexed[(y * w + x) as usize] as usize == ci {
                        bits |= 1 << dy;
                    }
                }
                any |= bits != 0;
                row.push(bits);
            }
            if !any {
                continue;
            }
            if !first_colour_in_band {
                out.push('$'); // back to the start of the band
            }
            first_colour_in_band = false;
            let _ = write!(out, "#{ci}");
            let mut i = 0usize;
            while i < row.len() {
                let v = row[i];
                let mut run = 1usize;
                while i + run < row.len() && row[i + run] == v {
                    run += 1;
                }
                let ch = (63 + v) as char;
                if run > 3 {
                    let _ = write!(out, "!{run}{ch}");
                } else {
                    for _ in 0..run {
                        out.push(ch);
                    }
                }
                i += run;
            }
        }
        out.push('-'); // next band
    }
    out.push_str("\x1b\\"); // ST
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn solid(w: u32, h: u32, c: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb(c))
    }

    #[test]
    fn produces_a_well_formed_sequence() {
        let s = encode(&solid(12, 12, [255, 0, 0]));
        assert!(s.starts_with("\x1bPq"), "missing DCS introducer");
        assert!(s.ends_with("\x1b\\"), "missing string terminator");
        assert!(s.contains("\"1;1;12;12"), "missing raster attributes");
        assert!(s.contains("#180;2;"), "expected the pure-red palette entry");
        // Two bands of six rows.
        assert_eq!(s.matches('-').count(), 2);
    }

    #[test]
    fn greys_use_the_grey_ramp_not_the_cube() {
        // Mid grey on a 6-level cube would land on 153 or 102; the ramp is closer.
        let i = palette_index(128, 128, 128);
        assert!(i >= 216, "grey should use the grey ramp, got index {i}");
        let (r, g, b) = palette_rgb(i);
        assert!(r == g && g == b);
        assert!((r as i32 - 128).abs() < 12, "grey {r} is far from 128");
    }

    #[test]
    fn run_length_encodes_flat_areas() {
        // A wide solid image must not emit one character per column.
        let s = encode(&solid(200, 6, [0, 0, 255]));
        assert!(s.contains('!'), "expected run-length compression");
        assert!(s.len() < 400, "flat image encoded to {} bytes", s.len());
    }
}
