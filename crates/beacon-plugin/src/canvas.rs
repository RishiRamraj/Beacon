//! A small drawing surface a plugin renders its interpretation of memory onto.
//!
//! The alttp-navi proof of concept had a "map mode" that drew Link's position and
//! surroundings, so a sighted developer could see at a glance what the tool
//! believed the game state to be. This is that, generalised: a fixed-size pixel
//! buffer with a handful of primitives, exposed to Lua as a `canvas`. A plugin
//! that defines `on_draw(canvas)` gets it called while the map view is open, and
//! whatever it draws is shown — and, through the MCP server, is legible to an
//! agent too.
//!
//! Pixels are `0x00RRGGBB`, matching the emulator framebuffer, so the host blits
//! the canvas the same way it blits a frame. Colours from Lua are 24-bit
//! `0xRRGGBB` integers.

use std::cell::RefCell;
use std::rc::Rc;

/// The canvas dimensions. Fixed for now; a plugin adapts to them via
/// `canvas.width` / `canvas.height`.
pub const WIDTH: u32 = 256;
pub const HEIGHT: u32 = 256;

/// A shared pixel buffer the Lua primitives draw into.
#[derive(Clone)]
pub struct Canvas {
    buf: Rc<RefCell<Vec<u32>>>,
}

impl Canvas {
    pub fn new() -> Self {
        Canvas {
            buf: Rc::new(RefCell::new(vec![0u32; (WIDTH * HEIGHT) as usize])),
        }
    }

    /// Copies the pixels out, for the host to blit or encode.
    pub fn copy_into(&self, out: &mut Vec<u32>) {
        let buf = self.buf.borrow();
        out.clear();
        out.extend_from_slice(&buf);
    }

    fn put(&self, x: i64, y: i64, color: u32) {
        if x < 0 || y < 0 || x >= WIDTH as i64 || y >= HEIGHT as i64 {
            return;
        }
        let idx = (y as u32 * WIDTH + x as u32) as usize;
        // Guard against a torn buffer; index is in range by the check above.
        if let Some(p) = self.buf.borrow_mut().get_mut(idx) {
            *p = color & 0x00FF_FFFF;
        }
    }

    pub fn clear(&self, color: u32) {
        let c = color & 0x00FF_FFFF;
        for p in self.buf.borrow_mut().iter_mut() {
            *p = c;
        }
    }

    pub fn pixel(&self, x: i64, y: i64, color: u32) {
        self.put(x, y, color);
    }

    /// A filled rectangle. Negative width or height draws nothing.
    pub fn rect(&self, x: i64, y: i64, w: i64, h: i64, color: u32) {
        for dy in 0..h.max(0) {
            for dx in 0..w.max(0) {
                self.put(x + dx, y + dy, color);
            }
        }
    }

    /// A line, Bresenham's algorithm.
    pub fn line(&self, x0: i64, y0: i64, x1: i64, y1: i64, color: u32) {
        let (mut x0, mut y0) = (x0, y0);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.put(x0, y0, color);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Draws text in the built-in 5x7 font. Characters without a glyph advance
    /// but draw nothing, so an unknown symbol leaves a gap rather than mojibake.
    pub fn text(&self, x: i64, y: i64, s: &str, color: u32) {
        let mut cx = x;
        for ch in s.chars() {
            if let Some(glyph) = font::glyph(ch) {
                for (col, bits) in glyph.iter().enumerate() {
                    for row in 0..7 {
                        if bits & (1 << row) != 0 {
                            self.put(cx + col as i64, y + row as i64, color);
                        }
                    }
                }
            }
            cx += 6; // 5 wide plus one space
        }
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

/// A minimal 5x7 bitmap font, column-major (each byte's bit 0 is the top row).
///
/// The classic glcd 5x7 set, restricted to the glyphs a map label needs:
/// digits, uppercase letters, space, and a little punctuation. Enough for
/// coordinates and room numbers; lowercase can be added when something wants it.
mod font {
    pub fn glyph(c: char) -> Option<[u8; 5]> {
        Some(match c.to_ascii_uppercase() {
            ' ' => [0x00, 0x00, 0x00, 0x00, 0x00],
            '0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
            '1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
            '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
            '3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
            '4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
            '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
            '6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
            '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
            '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
            '9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
            'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
            'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
            'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
            'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
            'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
            'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
            'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
            'H' => [0x7F, 0x08, 0x08, 0x08, 0x7F],
            'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
            'J' => [0x20, 0x40, 0x41, 0x3F, 0x01],
            'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
            'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
            'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
            'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
            'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
            'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
            'Q' => [0x3E, 0x41, 0x51, 0x21, 0x5E],
            'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
            'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
            'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
            'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
            'V' => [0x1F, 0x20, 0x40, 0x20, 0x1F],
            'W' => [0x7F, 0x20, 0x18, 0x20, 0x7F],
            'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
            'Y' => [0x07, 0x08, 0x70, 0x08, 0x07],
            'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
            ':' => [0x00, 0x36, 0x36, 0x00, 0x00],
            '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
            '.' => [0x00, 0x60, 0x60, 0x00, 0x00],
            ',' => [0x00, 0x50, 0x30, 0x00, 0x00],
            '/' => [0x20, 0x10, 0x08, 0x04, 0x02],
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(c: &Canvas, x: u32, y: u32) -> u32 {
        let mut out = Vec::new();
        c.copy_into(&mut out);
        out[(y * WIDTH + x) as usize]
    }

    #[test]
    fn pixel_and_clear() {
        let c = Canvas::new();
        c.clear(0x102030);
        assert_eq!(at(&c, 0, 0), 0x102030);
        c.pixel(5, 5, 0xFF0000);
        assert_eq!(at(&c, 5, 5), 0xFF0000);
        assert_eq!(at(&c, 4, 5), 0x102030);
    }

    #[test]
    fn out_of_bounds_is_ignored() {
        let c = Canvas::new();
        c.pixel(-1, 0, 0xFFFFFF);
        c.pixel(WIDTH as i64, 0, 0xFFFFFF);
        c.pixel(0, HEIGHT as i64, 0xFFFFFF);
        // No panic, and nothing set at the corner.
        assert_eq!(at(&c, 0, 0), 0);
    }

    #[test]
    fn rect_fills_its_area() {
        let c = Canvas::new();
        c.rect(2, 3, 4, 5, 0x00FF00);
        assert_eq!(at(&c, 2, 3), 0x00FF00);
        assert_eq!(at(&c, 5, 7), 0x00FF00);
        assert_eq!(at(&c, 6, 8), 0); // just outside
    }

    #[test]
    fn line_hits_both_ends() {
        let c = Canvas::new();
        c.line(0, 0, 10, 5, 0x0000FF);
        assert_eq!(at(&c, 0, 0), 0x0000FF);
        assert_eq!(at(&c, 10, 5), 0x0000FF);
    }

    #[test]
    fn text_draws_known_glyphs() {
        let c = Canvas::new();
        c.text(0, 0, "1", 0xFFFFFF);
        // '1' has a solid vertical stroke in column 2 (glyph byte 0x7F).
        let mut lit = 0;
        for row in 0..7 {
            if at(&c, 2, row) == 0xFFFFFF {
                lit += 1;
            }
        }
        assert!(lit >= 6, "expected a vertical stroke, got {lit} lit rows");
    }
}
