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
/// Terminal cells are about twice as tall as they are wide, so a square image
/// wants twice as many columns as rows.
pub fn square_width(rows: u16) -> u16 {
    rows.saturating_mul(2)
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

/// Blank an area and mark it skipped, so the terminal backend leaves those
/// cells alone and a graphics escape written there survives the frame.
pub fn reserve(area: Rect, buf: &mut Buffer) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_char(' ');
                c.set_diff_option(ratatui::buffer::CellDiffOption::Skip);
            }
        }
    }
}

/// Does this backend draw through raw escapes rather than buffer cells?
pub fn is_graphics(b: Backend) -> bool {
    matches!(b, Backend::Sixel | Backend::Kitty)
}
