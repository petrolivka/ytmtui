//! The album-art pane.
//!
//! Half blocks are drawn straight into the ratatui buffer. Sixel and the Kitty
//! protocol cannot be: they are raw escape sequences, so those cells are marked
//! skipped and the image is written after the frame, positioned by hand.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ytm_art::{Backend, Cell as ArtCell};

/// Columns a square cover needs for a given height.
///
/// Cells are taller than they are wide, so a square image needs more columns
/// than rows - but by how much depends on the font. Measuring beats the old
/// assumption of exactly 2:1, which stretched the cover horizontally in every
/// terminal whose cells are taller than that.
pub fn square_width(rows: u16) -> u16 {
    scale(rows, ytm_art::cell_px().aspect())
}

/// The inverse: rows a square cover needs for a given width.
pub fn square_height(cols: u16) -> u16 {
    scale(cols, 1.0 / ytm_art::cell_px().aspect())
}

fn scale(n: u16, by: f32) -> u16 {
    ((n as f32 * by).round() as u16).max(1)
}

pub struct Cover<'a> {
    pub cells: &'a [Vec<ArtCell>],
}

impl Widget for Cover<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (r, row) in self.cells.iter().enumerate() {
            if r as u16 >= area.height {
                break;
            }
            for (c, cell) in row.iter().enumerate() {
                if c as u16 >= area.width {
                    break;
                }
                if let Some(target) = buf.cell_mut((area.x + c as u16, area.y + r as u16)) {
                    // Upper half block: foreground is the top pixel, background
                    // the bottom one, so each cell carries two pixels.
                    target
                        .set_char('\u{2580}')
                        .set_fg(Color::Rgb(cell.upper.0, cell.upper.1, cell.upper.2))
                        .set_bg(Color::Rgb(cell.lower.0, cell.lower.1, cell.lower.2));
                }
            }
        }
    }
}

/// Blank an area for a picture written as a raw escape.
///
/// `painted` says whether a picture is already on screen there. It is not a
/// detail: a skipped cell is left out of the renderer's diff entirely, so it
/// is never written - which is what keeps a graphics escape alive across
/// frames, but also means whatever the pane held *before* is never erased. The
/// terminal keeps that text in its own grid and paints it back over the
/// picture, which is a spectrogram flickering through a fire.
///
/// So the first frame at a position clears the cells for real, forcing the
/// write even if the renderer thinks they were already blank, and only then
/// are they skipped.
///
/// See [`reclaim`] for the other half: taking the cells back afterwards.
/// Force the cells a picture occupies through the renderer's diff.
///
/// Called for every position a raw-escape picture was last drawn at, before
/// anything else in the frame. If the picture is still being drawn, `reserve`
/// marks the same cells skipped again later in the frame and nothing is
/// written; if it is not, whatever now belongs there is painted over it, which
/// is the only thing that erases a sixel.
pub fn reclaim(area: Rect, buf: &mut Buffer) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_diff_option(ratatui::buffer::CellDiffOption::AlwaysUpdate);
            }
        }
    }
}

pub fn reserve(area: Rect, buf: &mut Buffer, painted: bool) {
    use ratatui::buffer::CellDiffOption;
    let option = if painted {
        CellDiffOption::Skip
    } else {
        CellDiffOption::AlwaysUpdate
    };
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.reset();
                c.set_diff_option(option);
            }
        }
    }
}

/// Does this backend draw through raw escapes rather than buffer cells?
pub fn is_graphics(b: Backend) -> bool {
    matches!(b, Backend::Sixel | Backend::Kitty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::CellDiffOption;

    /// The flicker regression: reserving cells that were never cleared leaves
    /// the previous visualiser's glyphs in the terminal's own grid, and it
    /// paints them back over the picture.
    #[test]
    fn cells_are_cleared_before_they_are_skipped() {
        let area = Rect::new(0, 0, 3, 2);
        let mut buf = Buffer::empty(area);
        for x in 0..3 {
            buf[(x, 0)].set_char('\u{2588}').set_fg(Color::Rgb(1, 2, 3));
        }

        // Nothing on screen yet: clear for real, and force the write, because
        // the renderer may believe those cells are already blank.
        reserve(area, &mut buf, false);
        for x in 0..3 {
            assert_eq!(buf[(x, 0)].symbol(), " ");
            assert_eq!(buf[(x, 0)].fg, Color::Reset);
            assert_eq!(buf[(x, 0)].diff_option, CellDiffOption::AlwaysUpdate);
        }

        // Now a picture is there, so the renderer must leave the cells alone.
        reserve(area, &mut buf, true);
        assert_eq!(buf[(0, 0)].diff_option, CellDiffOption::Skip);
    }

    /// The two directions have to agree, or the layout that shrinks a cover to
    /// fit the width would hand back a rectangle instead of a square.
    #[test]
    fn square_width_and_height_are_inverses() {
        for rows in 1..40u16 {
            let cols = square_width(rows);
            let back = square_height(cols);
            assert!(
                back.abs_diff(rows) <= 1,
                "{rows} rows -> {cols} cols -> {back} rows"
            );
        }
    }
}
