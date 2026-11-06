//! Reads the moped's QR sticker from a camera picture.

use crate::protocol::decode_pairing_code;

/// The pairing code in a picture's brightness values, one byte a pixel. Other QR codes are ignored, so that the camera
/// simply carries on looking until it sees the right one.
pub fn read_pairing_code(brightness: &[u8], width: usize, height: usize) -> Option<String> {
    if width == 0 || height == 0 || brightness.len() < width * height {
        return None;
    }
    let mut picture = rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| brightness[y * width + x]);
    picture.detect_grids().iter().find_map(|grid| {
        let (_, content) = grid.decode().ok()?;
        pairing_code_in(&content)
    })
}

/// A sticker might hold the code on its own, or inside something longer.
fn pairing_code_in(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if decode_pairing_code(trimmed).is_some() {
        return Some(trimmed.to_string());
    }
    trimmed
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|part| decode_pairing_code(part).is_some())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "ABCDEFGHIJKLMNOPQRSTUVWX";

    /// Draws a QR code the way a camera would see it: on a pale sticker in a grey picture.
    fn picture_of(content: &str, scale: usize) -> (Vec<u8>, usize, usize) {
        let code = qrcode::QrCode::new(content).unwrap();
        let side = code.width();
        let margin = 40;
        let total = side * scale + margin * 2;
        let mut pixels = vec![150u8; total * total];
        for y in margin - 16..total - margin + 16 {
            for x in margin - 16..total - margin + 16 {
                pixels[y * total + x] = 225;
            }
        }
        let colors = code.to_colors();
        for y in 0..side * scale {
            for x in 0..side * scale {
                if colors[(y / scale) * side + x / scale] == qrcode::Color::Dark {
                    pixels[(y + margin) * total + x + margin] = 35;
                }
            }
        }
        (pixels, total, total)
    }

    #[test]
    fn reads_the_sticker() {
        assert!(decode_pairing_code(CODE).is_some());
        let (pixels, w, h) = picture_of(CODE, 6);
        assert_eq!(read_pairing_code(&pixels, w, h).as_deref(), Some(CODE));
    }

    #[test]
    fn finds_the_code_inside_a_longer_content() {
        let (pixels, w, h) = picture_of(&format!("https://example.org/pair?code={CODE}"), 5);
        assert_eq!(read_pairing_code(&pixels, w, h).as_deref(), Some(CODE));
    }

    #[test]
    fn ignores_other_codes_and_empty_pictures() {
        let (pixels, w, h) = picture_of("hello there", 6);
        assert_eq!(read_pairing_code(&pixels, w, h), None);
        assert_eq!(read_pairing_code(&[128; 100], 10, 10), None);
        assert_eq!(read_pairing_code(&[], 10, 10), None);
    }
}
