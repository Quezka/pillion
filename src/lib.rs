//! Pillion, a riding companion for an electric moped. See MECHANICS.md for the facts it's built on.

pub mod demo;
pub mod engine;
pub mod locale;
pub mod nav;
pub mod protocol;
mod symbols;
pub mod theme;
pub mod ui;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
pub use android::android_main;

use engine::{Command, Engine};

/// Commands only the desktop preview answers itself: it can only ask for typed text.
#[cfg(not(target_os = "android"))]
pub fn desktop_prompts(engine: &mut Engine) -> Vec<(String, String)> {
    let mut prompts = Vec::new();
    while let Some(command) = engine.poll_command() {
        if let Command::AskText { key, title, .. } = command {
            prompts.push((key, title));
        }
    }
    prompts
}

/// On a phone the Java side carries out commands, so the screen has nothing to do here.
#[cfg(target_os = "android")]
pub fn desktop_prompts(_engine: &mut Engine) -> Vec<(String, String)> {
    let _: Option<Command> = None;
    Vec::new()
}

#[cfg(not(target_os = "android"))]
pub fn save_png(path: &str, image: &eframe::egui::ColorImage) {
    let [width, height] = image.size;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = image::save_buffer(path, &bytes, width as u32, height as u32, image::ColorType::Rgba8) {
        eprintln!("Could not save the picture to {path}: {error}");
    }
}

#[cfg(target_os = "android")]
pub fn save_png(_path: &str, _image: &eframe::egui::ColorImage) {}
