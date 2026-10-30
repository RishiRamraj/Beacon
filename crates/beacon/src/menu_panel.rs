//! The open menu, drawn over the map panel.
//!
//! The menu is spoken, and published to the screen reader; this is the same menu for whoever
//! is watching — a sighted helper, or the player's own remaining sight — so the list being read
//! out is also on the screen, over the map it is about. Drawn by the host in the map's own
//! 5x7 font, so a plugin's lists look exactly like Beacon's.

use beacon_plugin::canvas::font;

use crate::menu::Shown;

const MARGIN: usize = 8;
const LINE: usize = 10;
const CHAR_W: usize = 6;
const BACKDROP_DIM: u32 = 3; // the map shows through, behind the text, at a quarter of its brightness
const TITLE: u32 = 0xFF_D0_60;
const TEXT: u32 = 0xE0_E8_F0;
const SELECTED_BAR: u32 = 0xFF_60_D0;
const SELECTED_TEXT: u32 = 0x10_10_18;

/// Draws `title` and `items`, with `selected` highlighted, over the `w` x `h` image in `buf`.
/// A list longer than fits scrolls to keep the selection in view.
pub fn draw(buf: &mut [u32], w: usize, h: usize, title: &str, items: &[Shown], selected: usize) {
    if buf.len() < w * h || w < 2 * MARGIN || h < 2 * MARGIN {
        return;
    }
    // Dim the map, so the list reads over it and the map still shows through.
    for px in buf.iter_mut() {
        let (r, g, b) = ((*px >> 16) & 0xFF, (*px >> 8) & 0xFF, *px & 0xFF);
        *px = ((r / (BACKDROP_DIM + 1)) << 16) | ((g / (BACKDROP_DIM + 1)) << 8) | (b / (BACKDROP_DIM + 1));
    }

    let cols = (w - 2 * MARGIN) / CHAR_W;
    text(buf, w, h, MARGIN, MARGIN, &fit(title, cols), TITLE);
    let top = MARGIN + LINE + 4;
    let rows = (h - top - MARGIN) / LINE;
    if items.is_empty() {
        text(buf, w, h, MARGIN, top, "EMPTY", TEXT);
        return;
    }
    // Scroll so the selection sits in view, a line from the edge where it can be.
    let first = selected.saturating_sub(rows.saturating_sub(2)).min(items.len().saturating_sub(rows));
    for (row, (i, item)) in items.iter().enumerate().skip(first).take(rows).enumerate() {
        let y = top + row * LINE;
        let mut label = item.label.clone();
        if item.submenu {
            label.push_str(" >");
        }
        let label = fit(&label, cols);
        if i == selected {
            fill(buf, w, MARGIN - 2, y - 2, w - 2 * MARGIN + 4, LINE, SELECTED_BAR);
            text(buf, w, h, MARGIN, y, &label, SELECTED_TEXT);
        } else {
            text(buf, w, h, MARGIN, y, &label, TEXT);
        }
    }
    // More above or below: a mark at the edge, so a long list reads as one.
    if first > 0 {
        text(buf, w, h, w - MARGIN - CHAR_W, top - LINE - 4 + 2, "-", TITLE);
    }
    if first + rows < items.len() {
        text(buf, w, h, w - MARGIN - CHAR_W, h - MARGIN - 7, ".", TITLE);
    }
}

/// Cuts a label to `cols` characters, marking a cut with ".." so it is not mistaken for whole.
fn fit(s: &str, cols: usize) -> String {
    let n = s.chars().count();
    if n <= cols {
        return s.to_string();
    }
    let keep: String = s.chars().take(cols.saturating_sub(2)).collect();
    format!("{keep}..")
}

fn fill(buf: &mut [u32], w: usize, x: usize, y: usize, rw: usize, rh: usize, color: u32) {
    let h = buf.len() / w;
    for yy in y..(y + rh).min(h) {
        for xx in x..(x + rw).min(w) {
            buf[yy * w + xx] = color;
        }
    }
}

fn text(buf: &mut [u32], w: usize, h: usize, x: usize, y: usize, s: &str, color: u32) {
    let mut cx = x;
    for ch in s.chars() {
        if let Some(glyph) = font::glyph(ch) {
            for (col, bits) in glyph.iter().enumerate() {
                for row in 0..7 {
                    if bits & (1 << row) != 0 {
                        let (px, py) = (cx + col, y + row);
                        if px < w && py < h {
                            buf[py * w + px] = color;
                        }
                    }
                }
            }
        }
        cx += CHAR_W;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(label: &str) -> Shown {
        Shown {
            label: label.to_string(),
            submenu: false,
        }
    }

    #[test]
    fn the_selected_entry_is_highlighted_and_the_map_dimmed() {
        let (w, h) = (256, 256);
        let mut buf = vec![0xFF_FF_FF; w * h];
        let items = vec![shown("STOP"), shown("CHEST - EAST - 6 TILES")];
        draw(&mut buf, w, h, "NAVIGATE", &items, 1);
        // The corner, outside the panel, is the dimmed map, not white.
        assert_eq!(buf[0], 0x3F_3F_3F);
        // The selection's bar is drawn on its row.
        let y = MARGIN + LINE + 4 + LINE; // second entry
        assert!(buf[(y + 3) * w + MARGIN - 1] == SELECTED_BAR, "the bar on the selected row");
    }

    #[test]
    fn a_long_list_scrolls_to_the_selection_and_long_labels_are_cut() {
        let (w, h) = (256, 256);
        let mut buf = vec![0; w * h];
        let items: Vec<Shown> = (0..60).map(|i| shown(&format!("ENTRY {i}"))).collect();
        // Must not panic however far down the selection is.
        draw(&mut buf, w, h, "LONG", &items, 59);
        assert_eq!(fit("ABCDEFGHIJ", 6), "ABCD..");
        assert_eq!(fit("ABC", 6), "ABC");
    }
}

#[cfg(test)]
mod preview {
    /// `MENU_PNG=/path.png cargo test -p beacon menu_panel::preview` renders a sample to look at.
    #[test]
    fn render_a_sample() {
        let Ok(path) = std::env::var("MENU_PNG") else { return };
        let (w, h) = (256usize, 256usize);
        // A stand-in map: a grid, so the dimming shows.
        let mut buf: Vec<u32> = (0..w * h)
            .map(|i| if (i % w) % 16 == 0 || (i / w) % 16 == 0 { 0x40_A0_60 } else { 0x20_50_30 })
            .collect();
        let items: Vec<super::Shown> = [
            ("Stop navigation", false),
            ("Unexplored ground - North-west - 3 tiles", false),
            ("Unexplored room - East - 1 room", false),
            ("Chest - South-east - 13 tiles", false),
            ("Locked door - North - 2 rooms", false),
            ("Exits and stairs", true),
        ]
        .iter()
        .map(|(l, s)| super::Shown { label: l.to_string(), submenu: *s })
        .collect();
        super::draw(&mut buf, w, h, "Unexplored", &items, 3);
        std::fs::write(path, crate::image::encode_png(w as u32, h as u32, &buf)).unwrap();
    }
}
