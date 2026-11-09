//! The geometry and pictures behind liquid glass over the map: a rounded outline, cutting it to one map picture,
//! and the soft blur the pictures get before they show through. The map side (which pictures, where) is in `map.rs`.

use eframe::egui::{Pos2, Rect};

/// Points per quarter circle of a rounded corner.
const CORNER_STEPS: usize = 8;

/// The outline of a rounded rectangle, going round clockwise on screen (y down), starting on the left side just
/// below the top-left corner. A radius of half the height makes a pill, of half the size a circle.
pub fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    if r < 0.5 {
        return vec![rect.left_bottom(), rect.left_top(), rect.right_top(), rect.right_bottom()];
    }
    let centres = [
        (Pos2::new(rect.left() + r, rect.top() + r), 180.0f32),
        (Pos2::new(rect.right() - r, rect.top() + r), 270.0),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 90.0),
    ];
    let mut points = Vec::with_capacity(4 * (CORNER_STEPS + 1));
    for (centre, start) in centres {
        for i in 0..=CORNER_STEPS {
            let a = (start + 90.0 * i as f32 / CORNER_STEPS as f32).to_radians();
            let p = centre + eframe::egui::Vec2::new(a.cos(), a.sin()) * r;
            if points.last().is_none_or(|q: &Pos2| q.distance(p) > 0.01) {
                points.push(p);
            }
        }
    }
    points
}

fn signed_area(points: &[Pos2]) -> f32 {
    let mut sum = 0.0;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    sum / 2.0
}

/// The part of convex `subject` inside convex `clip` (Sutherland–Hodgman). Either may go round either way.
pub fn clip_convex(subject: &[Pos2], clip: &[Pos2]) -> Vec<Pos2> {
    if subject.len() < 3 || clip.len() < 3 {
        return Vec::new();
    }
    let turn = signed_area(clip).signum();
    let inside = |a: Pos2, b: Pos2, p: Pos2| ((b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)) * turn >= 0.0;
    let cross = |a: Pos2, b: Pos2, p: Pos2, q: Pos2| {
        let (r, s) = (b - a, q - p);
        let denom = r.x * s.y - r.y * s.x;
        if denom.abs() < 1e-9 {
            return p;
        }
        let t = ((p.x - a.x) * s.y - (p.y - a.y) * s.x) / denom;
        a + r * t
    };
    let mut out = subject.to_vec();
    for (i, &a) in clip.iter().enumerate() {
        let b = clip[(i + 1) % clip.len()];
        let input = std::mem::take(&mut out);
        if input.is_empty() {
            break;
        }
        for (j, &p) in input.iter().enumerate() {
            let q = input[(j + 1) % input.len()];
            match (inside(a, b, p), inside(a, b, q)) {
                (true, true) => out.push(q),
                (true, false) => out.push(cross(a, b, p, q)),
                (false, true) => {
                    out.push(cross(a, b, p, q));
                    out.push(q);
                }
                (false, false) => {}
            }
        }
    }
    out
}

/// The run of the outline along the top edge, where the light catches it.
pub fn top_rim(outline: &[Pos2], rect: Rect, radius: f32) -> Vec<Pos2> {
    let limit = rect.top() + (radius * 0.35).max(0.6);
    outline.iter().copied().filter(|p| p.y <= limit).collect()
}

/// Blurs premultiplied RGBA pixels in place: three box blurs across and down, which together look Gaussian.
pub fn blur_rgba(pixels: &mut [u8], width: usize, height: usize, radius: usize) {
    if radius == 0 || width == 0 || height == 0 || pixels.len() < width * height * 4 {
        return;
    }
    let mut scratch = vec![0u8; pixels.len()];
    for _ in 0..3 {
        box_pass(pixels, &mut scratch, width, height, radius, true);
        box_pass(&scratch, pixels, width, height, radius, false);
    }
}

fn box_pass(from: &[u8], to: &mut [u8], width: usize, height: usize, radius: usize, across: bool) {
    let (lines, length) = if across { (height, width) } else { (width, height) };
    let at = |line: usize, i: usize| if across { (line * width + i) * 4 } else { (i * width + line) * 4 };
    let span = (2 * radius + 1) as u32;
    for line in 0..lines {
        let mut sum = [0u32; 4];
        let clamp = |i: isize| i.clamp(0, length as isize - 1) as usize;
        for k in -(radius as isize)..=(radius as isize) {
            let p = at(line, clamp(k));
            for c in 0..4 {
                sum[c] += u32::from(from[p + c]);
            }
        }
        for i in 0..length {
            let p = at(line, i);
            for c in 0..4 {
                to[p + c] = (sum[c] / span) as u8;
            }
            let out = at(line, clamp(i as isize - radius as isize));
            let inn = at(line, clamp(i as isize + radius as isize + 1));
            for c in 0..4 {
                sum[c] = sum[c] + u32::from(from[inn + c]) - u32::from(from[out + c]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(points: &[Pos2]) -> f32 {
        signed_area(points).abs()
    }

    #[test]
    fn a_circle_is_round() {
        let rect = Rect::from_center_size(Pos2::new(50.0, 50.0), eframe::egui::Vec2::splat(40.0));
        let ring = rounded_outline(rect, 20.0);
        for p in &ring {
            assert!((p.distance(rect.center()) - 20.0).abs() < 0.01);
        }
        assert!((area(&ring) - std::f32::consts::PI * 400.0).abs() < 25.0);
    }

    #[test]
    fn clipping_keeps_the_overlap() {
        let a = rounded_outline(Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 50.0)), 0.0);
        let b = rounded_outline(Rect::from_min_max(Pos2::new(60.0, -10.0), Pos2::new(200.0, 30.0)), 0.0);
        let both = clip_convex(&a, &b);
        assert!((area(&both) - 40.0 * 30.0).abs() < 0.01, "{}", area(&both));
        let reversed: Vec<Pos2> = b.iter().rev().copied().collect();
        assert!((area(&clip_convex(&a, &reversed)) - 1200.0).abs() < 0.01);
        let apart = rounded_outline(Rect::from_min_max(Pos2::new(300.0, 0.0), Pos2::new(400.0, 50.0)), 0.0);
        assert!(clip_convex(&a, &apart).len() < 3);
    }

    #[test]
    fn the_rim_is_the_top() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(200.0, 56.0));
        let outline = rounded_outline(rect, 28.0);
        let rim = top_rim(&outline, rect, 28.0);
        assert!(rim.len() > 3 && rim.iter().all(|p| p.y < 10.0));
    }

    #[test]
    fn blurring_spreads_a_dot_and_keeps_its_light() {
        let (w, h) = (16, 16);
        let mut pixels = vec![0u8; w * h * 4];
        let mid = (8 * w + 8) * 4;
        pixels[mid..mid + 4].copy_from_slice(&[255, 255, 255, 255]);
        blur_rgba(&mut pixels, w, h, 2);
        assert!(pixels[mid] < 255 && pixels[mid] > 0);
        assert!(pixels[(8 * w + 10) * 4] > 0, "the light reaches two pixels away");
        let flat = vec![200u8; w * h * 4];
        let mut same = flat.clone();
        blur_rgba(&mut same, w, h, 3);
        assert_eq!(flat, same, "an even colour stays the same");
    }
}
