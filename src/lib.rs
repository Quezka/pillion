//! Pillion, a riding companion for an electric moped. See MECHANICS.md for the facts it's built on.

//! Layers (Clean Architecture): `domain` (rules) ← `application` (the Engine) ← `infra` (web services, adapters) and
//! `ui` (screens). `android.rs` and `main.rs` wire them together. `tests/architecture.rs` keeps the arrows pointing in.
pub mod application;
pub mod domain;
pub mod infra;
pub mod ui;
mod wiring;

// The old flat paths, so code written against them keeps working.
pub use application::{demo, engine, locale, nav};
pub use domain::protocol;
pub use infra::{qr, update};
pub use ui::{m3, model3d, theme};
pub(crate) use ui::symbols;

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
        match command {
            Command::AskText { key, title, .. } => prompts.push((key, title)),
            // There's no camera on the preview, so the code gets typed.
            Command::ScanQr { title, .. } => prompts.push(("pair".to_string(), title)),
            Command::FetchTile { z, x, y } => preview_tiles::fetch(z, x, y),
            _ => {}
        }
    }
    for (key, path) in preview_tiles::arrived() {
        match path {
            Some(path) => engine.on_tile(key, &path),
            None => engine.on_tile_failed(key),
        }
    }
    prompts
}

/// The desktop preview's map pictures: fetched with `curl` on a worker thread into a cache folder, so the Route tab
/// can be checked on a computer. The phone does this in Java.
#[cfg(not(target_os = "android"))]
mod preview_tiles {
    use std::sync::Mutex;

    type Arrival = ((u8, u32, u32), Option<String>);
    static ARRIVED: Mutex<Vec<Arrival>> = Mutex::new(Vec::new());

    pub fn fetch(z: u8, x: u32, y: u32) {
        std::thread::spawn(move || {
            let folder = std::env::temp_dir().join("pillion-preview-tiles");
            let _ = std::fs::create_dir_all(&folder);
            let file = folder.join(format!("{z}-{x}-{y}.png"));
            let ok = file.exists()
                || std::process::Command::new("curl")
                    .args(["-sfL", "-A", "Pillion-preview (github.com/Quezka/pillion)", "-o"])
                    .arg(&file)
                    .arg(format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png"))
                    .status()
                    .is_ok_and(|s| s.success());
            let path = ok.then(|| file.to_string_lossy().into_owned());
            if let Ok(mut list) = ARRIVED.lock() {
                list.push(((z, x, y), path));
            }
        });
    }

    pub fn arrived() -> Vec<Arrival> {
        ARRIVED.lock().map(|mut list| std::mem::take(&mut *list)).unwrap_or_default()
    }
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
