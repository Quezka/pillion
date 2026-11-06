//! Draws the 3D moped to a picture, to compare it with photos of the real one.
//! `cargo run --release --example model_shot -- out.png yaw pitch seat zoom body_hex [background_hex]`

use pillion::model3d::{render, Scene};

fn colour(hex: &str) -> [u8; 3] {
    let value = u32::from_str_radix(hex, 16).unwrap_or(0xFFFFFF);
    [(value >> 16) as u8, (value >> 8) as u8, value as u8]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |i: usize, default: f32| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default);
    let scene = Scene {
        yaw: number(1, 0.0),
        pitch: number(2, 0.0),
        seat: number(3, 0.0),
        zoom: number(4, 1.0),
        body: colour(args.get(5).map_or("16BEC3", String::as_str)),
        accent: [90, 90, 210],
        danger: [210, 50, 50],
        charge: [Some(64), Some(100)],
        lights_on: true,
        warning: false,
    };
    let back = colour(args.get(6).map_or("F4F4FA", String::as_str));
    let (w, h) = (1000u32, 720u32);
    let rgba = render(&scene, w as usize, h as usize, 2);
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for p in rgba.chunks(4) {
        for k in 0..3 {
            rgb.push((u32::from(p[k]) + u32::from(back[k]) * (255 - u32::from(p[3])) / 255).min(255) as u8);
        }
    }
    image::save_buffer(&args[0], &rgb, w, h, image::ColorType::Rgb8).expect("the picture should save");
}
