//! A tiny, dependency-free PNG encoder and base64, for the map image.
//!
//! The MCP `get_map` tool returns the plugin's map as a PNG so an agent can see
//! it. Rather than pull in an image crate for one call, this encodes a PNG by
//! hand: `0x00RRGGBB` pixels become 24-bit RGB, wrapped in a zlib stream that
//! uses only *stored* (uncompressed) deflate blocks. That is trivially correct —
//! no compressor needed — and a map is small, so size does not matter.

/// Encodes `width`x`height` `0x00RRGGBB` pixels as a PNG.
pub fn encode_png(width: u32, height: u32, pixels: &[u32]) -> Vec<u8> {
    // Raw image data: each row prefixed with a filter byte (0 = none).
    let mut raw = Vec::with_capacity((height * (1 + width * 3)) as usize);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            let p = pixels.get((y * width + x) as usize).copied().unwrap_or(0);
            raw.push((p >> 16) as u8);
            raw.push((p >> 8) as u8);
            raw.push(p as u8);
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // signature

    // IHDR: 8-bit, colour type 2 (RGB), no interlace.
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut out, b"IHDR", &ihdr);

    write_chunk(&mut out, b"IDAT", &zlib_store(&raw));
    write_chunk(&mut out, b"IEND", &[]);
    out
}

/// Wraps data in a zlib stream of stored deflate blocks.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // zlib header: deflate, default window
    let mut rest = data;
    while !rest.is_empty() {
        let n = rest.len().min(0xFFFF);
        let final_block = n == rest.len();
        out.push(if final_block { 1 } else { 0 }); // BFINAL, BTYPE=00 (stored)
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(!(n as u16)).to_le_bytes()); // NLEN
        out.extend_from_slice(&rest[..n]);
        rest = &rest[n..];
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

/// A CRC-32 (IEEE), computed with the standard polynomial on the fly.
struct Crc {
    value: u32,
}

impl Crc {
    fn new() -> Self {
        Crc { value: 0xFFFF_FFFF }
    }

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.value ^= byte as u32;
            for _ in 0..8 {
                let mask = (self.value & 1).wrapping_neg();
                self.value = (self.value >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        !self.value
    }
}

/// Standard base64, for embedding the PNG in a JSON string.
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn adler32_of_known_input() {
        // Wikipedia's worked example.
        assert_eq!(adler32(b"Wikipedia"), 0x11E60398);
    }

    #[test]
    fn png_has_signature_and_iend() {
        let png = encode_png(2, 2, &[0xFF0000, 0x00FF00, 0x0000FF, 0xFFFFFF]);
        assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }
}
