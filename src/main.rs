//! The desktop preview of the phone app. It has no Bluetooth or GPS, so it always plays the demo ride.

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result {
    use eframe::egui;
    use pillion::engine::{Appearance, Engine};
    use pillion::ui::CompanionApp;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const PHONE_SIZE: [f32; 2] = [412.0, 892.0];
    const TICK: Duration = Duration::from_millis(50);

    let mut args = std::env::args().skip(1);
    let mut screenshot = None;
    let mut tab = None;
    let mut appearance = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => screenshot = args.next(),
            "--tab" => tab = args.next(),
            "--appearance" => appearance = args.next(),
            _ => {}
        }
    }

    // A Wayland window that's covered by others is never asked to draw, which would stall the
    // screenshot. X11 draws regardless, so use it for screenshots when it's there.
    if screenshot.is_some() && std::env::var_os("DISPLAY").is_some() {
        std::env::remove_var("WAYLAND_DISPLAY");
    }

    let engine = Arc::new(Mutex::new(Engine::new()));
    if let Ok(mut e) = engine.lock() {
        e.start_demo();
        match appearance.as_deref() {
            Some("light") => e.settings.appearance = Appearance::Light,
            Some("dark") => e.settings.appearance = Appearance::Dark,
            _ => {}
        }
    }
    let ticking = Arc::clone(&engine);
    std::thread::spawn(move || {
        let start = Instant::now();
        loop {
            if let Ok(mut e) = ticking.lock() {
                e.tick(start.elapsed().as_secs_f64());
            }
            std::thread::sleep(TICK);
        }
    });

    // A window behind others gets no refresh signal, and waiting for one would stall the screenshot.
    let wait_for_refresh = screenshot.is_none();
    let options = eframe::NativeOptions {
        glow_options: eframe::egui_glow::GlowConfiguration { vsync: wait_for_refresh, ..Default::default() },
        viewport: egui::ViewportBuilder::default()
            .with_title("Pillion (preview)")
            .with_inner_size(PHONE_SIZE)
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "Pillion",
        options,
        Box::new(move |_cc| {
            let mut app = CompanionApp::new(engine, screenshot);
            if let Some(tab) = &tab {
                app.open_tab(tab);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_os = "android")]
fn main() {}
