//! The phone screens, in the suite's Material 3 look. All of them read from and act on the shared `Engine`.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align, Align2, CentralPanel, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, Painter, Panel, Pos2,
    Rect, ScrollArea, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2,
};

use crate::engine::{Appearance, BluetoothState, Engine, Link, MopedColor, NotifLevel, TileKey, UpdateState, VoicePack};
use crate::locale::{self, t, tf, Lang};
use crate::m3::{self, text, Kind};
use crate::model3d::{self, Scene};
use crate::nav::{self, LatLon, Maneuver, Navigation};
use crate::protocol as proto;
use crate::theme::{self, icon, Tokens, Weight};

const LOW_PERCENT: u8 = 20;
const REPAINT_SECONDS: f32 = 0.25;
const SPEECH_RATE_STEP: f32 = 0.1;
const RING_START_DEG: f32 = 135.0;
const RING_SWEEP_DEG: f32 = 270.0;
const SCREENSHOT_AFTER_FRAMES: u32 = 120;
const MAX_RECENTS_SHOWN: usize = 5;
const FADE_SECONDS: f64 = 0.16;
const SNACK_SECONDS: f64 = 4.0;
const FAB_ROOM: f32 = 84.0;

const MODEL_HEIGHT: f32 = 290.0;
const TURN_PER_POINT: f32 = 0.011;
const CAMERA_EASE: f32 = 0.16;
const SETTLED: f32 = 0.0015;
const ZOOM_LIMITS: (f32, f32) = (0.75, 2.6);
/// Longest side of the picture while it moves. Small enough to turn smoothly on a phone.
const MOVING_MAX_PX: f32 = 560.0;
const STILL_MAX_PX: f32 = 900.0;
const WHITE_BODY: [u8; 3] = [236, 236, 232];
const BLACK_BODY: [u8; 3] = [38, 38, 44];
const TEAL_BODY: [u8; 3] = [22, 190, 195];

const MAP_HEIGHT: f32 = 320.0;
const TILE_POINTS: f32 = 256.0;
const CLOSE_ZOOM: u8 = 17;
const FAR_ZOOM: u8 = 16;
/// Above this speed the map looks further ahead.
const FAR_ZOOM_KMH: f64 = 28.0;
const TILES_DECODED_PER_FRAME: usize = 3;
const TILES_KEPT: usize = 60;
const HEADING_EASE: f32 = 0.12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Ride,
    Route,
    Voice,
    Moped,
    Alerts,
    Settings,
}

/// The part of the moped being looked at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Batteries,
    Motor,
    Dash,
}

impl Focus {
    /// Where the camera goes to see this part best: (yaw, pitch).
    fn view(self) -> (f32, f32) {
        match self {
            Self::Batteries => (0.62, 0.52),
            Self::Motor => (-0.42, 0.14),
            Self::Dash => (2.45, 0.42),
        }
    }
}

struct ModelView {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    seat: f32,
    target_yaw: f32,
    target_pitch: f32,
    focus: Focus,
    texture: Option<TextureHandle>,
    drawn: Option<(Scene, usize, usize, usize)>,
}

impl ModelView {
    fn new() -> Self {
        let focus = Focus::Batteries;
        let (yaw, pitch) = focus.view();
        Self {
            yaw,
            pitch,
            zoom: 1.0,
            seat: 1.0,
            target_yaw: yaw,
            target_pitch: pitch,
            focus,
            texture: None,
            drawn: None,
        }
    }

    fn select(&mut self, focus: Focus) {
        self.focus = focus;
        let (yaw, pitch) = focus.view();
        // The short way round from wherever it's been turned to.
        let turns = ((self.yaw - yaw) / std::f32::consts::TAU).round();
        self.target_yaw = yaw + turns * std::f32::consts::TAU;
        self.target_pitch = pitch;
    }

    fn seat_target(&self) -> f32 {
        if self.focus == Focus::Batteries {
            1.0
        } else {
            0.0
        }
    }
}

pub struct CompanionApp {
    engine: Arc<Mutex<Engine>>,
    screen: Screen,
    last_tab: Screen,
    shot: Option<ScreenshotJob>,
    typed: String,
    typed_key: Option<(String, String)>,
    confirm_forget: bool,
    fonts_ready: bool,
    changed_screen_at: f64,
    snack: Option<(String, f64)>,
    model: ModelView,
    map: MapView,
}

/// The street map: pictures of it, and how it's looking at the route.
#[derive(Default)]
struct MapView {
    tiles: HashMap<TileKey, TextureHandle>,
    dark: bool,
    whole_route: bool,
    heading_deg: f32,
}

/// A position in map pictures, at a zoom level. North is up.
fn map_position(at: LatLon, z: u8) -> Vec2 {
    let n = f64::from(1u32 << z);
    let x = (at.1 + 180.0) / 360.0 * n;
    let lat = at.0.to_radians();
    let y = (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n;
    Vec2::new(x as f32, y as f32)
}

/// Loads a map picture. The dark look turns it into a night map, keeping its roads readable.
fn load_tile(path: &str, dark: bool) -> Option<egui::ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let picture = resvg::tiny_skia::Pixmap::decode_png(&bytes).ok()?;
    let (w, h) = (picture.width() as usize, picture.height() as usize);
    let mut pixels: Vec<u8> = picture.take();
    if dark {
        for p in pixels.chunks_mut(4) {
            let grey = (u16::from(p[0]) * 3 + u16::from(p[1]) * 6 + u16::from(p[2])) / 10;
            let night = 255 - grey as u8;
            p[0] = (f32::from(night) * 0.62 + f32::from(p[0]) * 0.06) as u8;
            p[1] = (f32::from(night) * 0.64 + f32::from(p[1]) * 0.06) as u8;
            p[2] = (f32::from(night) * 0.74 + f32::from(p[2]) * 0.06) as u8;
        }
    }
    Some(egui::ColorImage::from_rgba_premultiplied([w, h], &pixels))
}

struct ScreenshotJob {
    path: String,
    frames: u32,
    asked: bool,
}

fn draw_ring(painter: &Painter, th: &Tokens, center: Pos2, radius: f32, fraction: f32, color: Color32) {
    let arc = |fraction: f32| -> Vec<Pos2> {
        let steps = 64;
        (0..=steps)
            .map(|i| {
                let deg = RING_START_DEG + RING_SWEEP_DEG * fraction * i as f32 / steps as f32;
                let a = deg * PI / 180.0;
                center + Vec2::new(a.cos(), a.sin()) * radius
            })
            .collect()
    };
    painter.line(arc(1.0), Stroke::new(12.0, th.raised));
    if fraction > 0.0 {
        painter.line(arc(fraction.clamp(0.0, 1.0)), Stroke::new(12.0, color));
        // Rounded ends, as Material 3 draws its progress indicators.
        let round_at = |f: f32| {
            let a = (RING_START_DEG + RING_SWEEP_DEG * f) * PI / 180.0;
            center + Vec2::new(a.cos(), a.sin()) * radius
        };
        painter.circle_filled(round_at(0.0), 6.0, color);
        painter.circle_filled(round_at(fraction.clamp(0.0, 1.0)), 6.0, color);
    }
}

/// A step's sign: its arrow, or for a roundabout a ring with the exit to take.
fn draw_turn(painter: &Painter, rect: Rect, maneuver: Option<&Maneuver>, filled: bool, color: Color32) {
    let Some(m) = maneuver.filter(|m| m.is_roundabout()) else {
        icon(painter, rect, maneuver.map_or("turn-straight", Maneuver::symbol), filled, color);
        return;
    };
    let side = rect.width();
    let centre = rect.center() - Vec2::new(0.0, side * 0.08);
    let radius = side * 0.3;
    painter.circle_stroke(centre, radius, Stroke::new(side * 0.1, color));
    painter.line_segment([centre + Vec2::new(0.0, radius), rect.center_bottom()], Stroke::new(side * 0.1, color));
    if m.exit > 0 {
        painter.text(centre, Align2::CENTER_CENTER, m.exit.to_string(), FontId::new(side * 0.34, Weight::Bold.family()), color);
    }
}

fn format_distance(m: f64) -> String {
    if m >= 1000.0 {
        tf("{} km", &[&format!("{:.1}", m / 1000.0)])
    } else {
        tf("{} m", &[&format!("{:.0}", (m / 10.0).round() * 10.0)])
    }
}

fn link_status(engine: &Engine, th: &Tokens) -> (&'static str, Color32) {
    if engine.demo_active {
        return (t("Demo ride"), th.warning);
    }
    match engine.link {
        Link::Online if engine.data_is_stale() => (t("No data"), th.warning),
        Link::Online => (t("Connected"), th.success),
        Link::Scanning => (t("Searching"), th.muted),
        Link::Connecting | Link::LoggingIn => (t("Connecting"), th.muted),
        Link::Refused => (t("Code refused"), th.danger),
        Link::Idle => (t("Not connected"), th.muted),
        Link::NeedsPairing => (t("Not paired"), th.muted),
    }
}

fn is_connecting(engine: &Engine) -> bool {
    !engine.demo_active && matches!(engine.link, Link::Scanning | Link::Connecting | Link::LoggingIn)
}

/// Places come with long addresses. The first part is the name, the rest says where.
fn split_place(name: &str) -> (&str, &str) {
    match name.split_once(", ") {
        Some((first, rest)) => (first, rest),
        None => (name, ""),
    }
}

/// How far a place is from the rider, then where it is.
fn place_detail(from: Option<LatLon>, at: LatLon, detail: &str) -> String {
    match from {
        Some(from) => format!("{}, {detail}", format_distance(nav::distance_m(from, at))),
        None => detail.to_string(),
    }
}

fn charge_color(th: &Tokens, percent: u8, low: bool) -> Color32 {
    if low || percent <= LOW_PERCENT {
        th.danger
    } else {
        th.accent
    }
}

fn bytes(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

impl CompanionApp {
    pub fn new(engine: Arc<Mutex<Engine>>, screenshot: Option<String>) -> Self {
        Self {
            engine,
            screen: Screen::Ride,
            last_tab: Screen::Ride,
            shot: screenshot.map(|path| ScreenshotJob { path, frames: 0, asked: false }),
            typed: String::new(),
            typed_key: None,
            confirm_forget: false,
            fonts_ready: false,
            changed_screen_at: -1.0,
            snack: None,
            model: ModelView::new(),
            map: MapView::default(),
        }
    }

    /// Opens on a tab by its English name. For looking at a screen from the command line.
    pub fn open_tab(&mut self, name: &str) {
        let screen = match name {
            "route" => Screen::Route,
            "voice" => Screen::Voice,
            "moped" => Screen::Moped,
            "alerts" => Screen::Alerts,
            "settings" => Screen::Settings,
            _ => Screen::Ride,
        };
        self.screen = screen;
        if screen != Screen::Settings {
            self.last_tab = screen;
        }
    }

    fn go_to(&mut self, ui: &Ui, screen: Screen) {
        if self.screen != screen {
            self.changed_screen_at = ui.input(|i| i.time);
            self.screen = screen;
            if screen != Screen::Settings {
                self.last_tab = screen;
            }
            self.confirm_forget = false;
        }
    }

    fn top_bar(&mut self, ui: &mut Ui, engine: &Engine, th: &Tokens) {
        let heading = match self.screen {
            Screen::Ride => t("Ride"),
            Screen::Route => t("Route"),
            Screen::Voice => t("Voice"),
            Screen::Moped => t("Moped"),
            Screen::Alerts => t("Alerts"),
            Screen::Settings => t("Settings"),
        };
        ui.horizontal(|ui| {
            ui.set_min_height(52.0);
            if self.screen == Screen::Settings && m3::icon_button(ui, th, "arrow-back", th.text) {
                self.go_to(ui, self.last_tab);
            }
            ui.label(text(heading, 28.0, th.text, Weight::Bold));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.screen != Screen::Settings && m3::icon_button(ui, th, "settings", th.muted) {
                    self.go_to(ui, Screen::Settings);
                }
                let (name, color) = link_status(engine, th);
                m3::status_chip(ui, name, color);
            });
        });
        if is_connecting(engine) {
            m3::indeterminate(ui, th);
        }
    }

    /// Things in the way, shown where they matter and not on every screen.
    fn problems(&self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let on = |screens: &[Screen]| screens.contains(&self.screen);
        if on(&[Screen::Ride, Screen::Moped]) {
            match engine.bluetooth {
                BluetoothState::On => {}
                BluetoothState::Off => {
                    if engine.has_pairing()
                        && m3::banner(
                            ui,
                            th,
                            "bluetooth",
                            t("Bluetooth is off"),
                            t("Turn it on so Pillion can reach the moped."),
                            Some(t("Turn on Bluetooth")),
                        )
                    {
                        engine.enable_bluetooth();
                    }
                }
                BluetoothState::Denied => {
                    if m3::banner(
                        ui,
                        th,
                        "bluetooth",
                        t("Bluetooth isn't allowed"),
                        t("Pillion needs the Nearby devices permission to reach the moped."),
                        Some(t("Open app settings")),
                    ) {
                        engine.open_app_settings();
                    }
                }
            }
        }
        if on(&[Screen::Ride, Screen::Route])
            && engine.nav.is_some()
            && !engine.demo_active
            && !engine.location_on
            && m3::banner(
                ui,
                th,
                "target",
                t("Location is off"),
                t("Directions need it to follow you along the route."),
                Some(t("Turn on location")),
            )
        {
            engine.open_location_settings();
        }
        if on(&[Screen::Voice]) && engine.voice_missing && engine.voice_pack != VoicePack::Ready {
            m3::banner(
                ui,
                th,
                "alert",
                t("This phone can't speak yet"),
                t("Install Speech Services by Google from the Play Store, or download the natural voice below."),
                None,
            );
        }
    }

    fn get_ready(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        m3::hero(ui, th, |ui| {
            ui.label(text(t("Get ready to ride"), 22.0, th.on_accent_soft, Weight::Bold));
            ui.add_space(4.0);
            ui.label(text(
                t("Pillion rides along and tells you what's happening, so your eyes stay on the road."),
                14.0,
                th.on_accent_soft,
                Weight::Regular,
            ));
        });
        m3::card(ui, th, |ui| {
            let refused = engine.link == Link::Refused;
            if m3::list_row(
                ui,
                th,
                "moped",
                if refused { t("The moped refused this code") } else { t("Pair your moped") },
                t("Use the camera to read the QR sticker on your moped."),
                true,
            ) {
                engine.scan_pairing();
            }
            if !engine.notification_access
                && m3::list_row(
                    ui,
                    th,
                    "bell",
                    t("Allow notifications"),
                    t("So messages can be read to you and light up the dash."),
                    true,
                )
            {
                engine.open_notification_access();
            }
            if m3::list_row(ui, th, "hearing", t("Test your earpiece"), t("Pillion says a sentence so you know you'll hear it."), true) {
                engine.test_voice();
            }
            if m3::list_row(ui, th, "play", t("Try a demo ride"), t("See and hear everything first, with invented figures."), true) {
                engine.start_demo();
            }
        });
    }

    fn ride_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        // Not while riding: an update closes the app.
        let riding = engine.live.as_ref().is_some_and(|l| l.switched_on);
        if engine.update_state == UpdateState::Available && !riding && !engine.demo_active {
            let version = engine.update.as_ref().map_or(String::new(), |r| r.version.clone());
            let detail = tf("Version {} is ready to install.", &[&version]);
            if m3::banner(ui, th, "refresh", t("Update available"), &detail, Some(t("See what's new"))) {
                self.go_to(ui, Screen::Settings);
            }
        }
        if engine.live.is_none() && !engine.has_pairing() && !engine.demo_active {
            Self::get_ready(ui, engine, th);
        } else if engine.live.is_none() && !engine.demo_active {
            Self::connection_card(ui, engine, th, &mut self.confirm_forget);
        }
        // While navigating, the next turn is what a glance is for, so it comes first.
        if let Some(nav) = &engine.nav {
            Self::next_turn(ui, th, nav, engine.arrival_time());
        }
        if engine.live.is_some() {
            Self::battery_card(ui, engine, th);
        }
        if engine.nav.is_none() && (engine.has_pairing() || engine.demo_active) {
            m3::card(ui, th, |ui| {
                if m3::list_row(ui, th, "route", t("Plan a route"), t("Spoken directions, on roads a moped can use."), true) {
                    self.go_to(ui, Screen::Route);
                }
            });
        }
        if engine.live.is_some() {
            let speed = engine.speed_kmh.map_or("--".to_string(), |s| format!("{s:.0}"));
            let power = engine.power_w().map_or("--".to_string(), |p| tf("{} kW", &[&format!("{:.1}", p / 1000.0)]));
            m3::tile_row(
                ui,
                th,
                &[("moped", speed, t("km/h")), ("route", format_distance(engine.trip.km * 1000.0), t("this ride"))],
            );
            m3::tile_row(
                ui,
                th,
                &[
                    ("battery", tf("{} kWh", &[&format!("{:.2}", engine.trip.kwh)]), t("energy")),
                    ("bolt", power, t("power")),
                ],
            );
            Self::dash_card(ui, engine, th);
        }
        if let Some(said) = engine.spoken.front() {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                icon(&ui.painter_at(rect), rect, "voice", false, th.accent);
                ui.label(text(said.as_str(), 13.5, th.muted, Weight::Regular));
            });
        }
        ui.add_space(FAB_ROOM);
    }

    fn battery_card(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        let percent = engine.percent();
        m3::card(ui, th, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(136.0), Sense::hover());
                let painter = ui.painter_at(rect);
                let center = rect.center();
                let color = match percent {
                    Some(p) if p <= LOW_PERCENT => th.danger,
                    _ => th.accent,
                };
                let shown = ui.ctx().animate_value_with_time(
                    Id::NULL.with("ride-ring"),
                    f32::from(percent.unwrap_or(0)) / 100.0,
                    0.7,
                );
                draw_ring(&painter, th, center, 56.0, shown, color);
                let value = percent.map_or("--".to_string(), |p| format!("{p}%"));
                painter.text(center, Align2::CENTER_CENTER, value, FontId::new(34.0, Weight::Bold.family()), th.text);
                painter.text(
                    center + Vec2::new(0.0, 45.0),
                    Align2::CENTER_CENTER,
                    t("battery"),
                    FontId::new(12.5, Weight::Medium.family()),
                    th.muted,
                );
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    let range = engine.range_km().map_or("--".to_string(), |r| tf("{} km", &[&format!("{r:.0}")]));
                    ui.label(text(range, 34.0, th.text, Weight::Bold));
                    let withheld = percent.is_some() && engine.range_km().is_none();
                    let caption = if withheld { t("too low to estimate") } else { t("range left") };
                    ui.label(text(caption, 13.0, th.muted, Weight::Medium));
                    ui.add_space(8.0);
                    let eff = engine.measured_efficiency();
                    let value = eff.map_or("--".to_string(), |e| tf("{} km/kWh", &[&format!("{e:.0}")]));
                    ui.label(text(value, 18.0, th.text, Weight::Bold));
                    let caption = if eff.is_some() { t("this ride's consumption") } else { t("consumption: measuring") };
                    ui.label(text(caption, 13.0, th.muted, Weight::Medium));
                });
            });
            let packs = engine.live.as_ref().map(|l| l.packs).filter(|p| p[0].fitted && p[1].fitted);
            if let Some(packs) = packs {
                ui.add_space(10.0);
                ui.columns(2, |c| {
                    for (i, (column, pack)) in c.iter_mut().zip(packs.iter()).enumerate() {
                        column.label(text(tf("Battery {}: {}%", &[&(i + 1), &pack.percent]), 13.5, th.text, Weight::SemiBold));
                        column.add_space(4.0);
                        m3::progress(
                            column,
                            th,
                            Id::NULL.with(("ride-pack", i)),
                            f32::from(pack.percent) / 100.0,
                            charge_color(th, pack.percent, pack.low),
                            8.0,
                        );
                    }
                });
            }
        });
    }

    fn next_turn(ui: &mut Ui, th: &Tokens, nav: &Navigation, arrival: Option<String>) {
        m3::hero(ui, th, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(76.0), Sense::hover());
                let painter = ui.painter_at(rect);
                painter.circle_filled(rect.center(), 38.0, th.accent);
                let square = Rect::from_center_size(rect.center(), Vec2::splat(50.0));
                if nav.off_route {
                    icon(&painter, square, "alert", true, th.on_accent);
                } else {
                    draw_turn(&painter, square, nav.current(), true, th.on_accent);
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(text(format_distance(nav.distance_to_next_m()), 36.0, th.on_accent_soft, Weight::Bold));
                    if nav.off_route {
                        ui.label(text(t("Off route, finding a new way"), 15.0, th.on_accent_soft, Weight::Medium));
                    } else if let Some(m) = nav.current() {
                        ui.label(text(m.text.as_str(), 15.0, th.on_accent_soft, Weight::Medium));
                    }
                    ui.add_space(2.0);
                    let minutes = format!("{:.0}", (nav.remaining_s() / 60.0).ceil());
                    let left = format_distance(nav.remaining_m());
                    let line = match &arrival {
                        Some(time) => tf("{} to go, {} min, arriving {}", &[&left, &minutes, time]),
                        None => tf("{} to go, {} min", &[&left, &minutes]),
                    };
                    ui.label(text(line, 13.0, th.on_accent_soft, Weight::Regular));
                });
            });
        });
    }

    fn dash_lights(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
            let lit = engine.flags.ringing();
            let painter = ui.painter_at(rect);
            painter.circle_filled(rect.center(), 16.0, if lit { th.accent } else { th.raised });
            let square = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
            icon(&painter, square, "call", lit, if lit { th.on_accent } else { th.faint });
            ui.add_space(6.0);
            for slot in 0..proto::APP_SLOTS {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                let lit = engine.flags.app_on(slot);
                ui.painter_at(rect).circle_filled(rect.center(), 12.0, if lit { th.accent } else { th.raised });
            }
        });
    }

    fn dash_card(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Lights on the moped's dash"));
            Self::dash_lights(ui, engine, th);
            if let Some(n) = engine.notifications.front() {
                ui.add_space(8.0);
                m3::title(ui, th, &format!("{}: {}", n.label, n.title));
                m3::body(ui, th, n.text.as_str());
            }
        });
    }

    fn route_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
        let painter = ui.painter_at(rect);
        let round = CornerRadius::same(28);
        painter.rect_filled(rect, round, th.raised);
        if th.outlined {
            painter.rect_stroke(rect, round, Stroke::new(1.5, th.border), egui::StrokeKind::Inside);
        }
        let square = Rect::from_center_size(Pos2::new(rect.left() + 28.0, rect.center().y), Vec2::splat(24.0));
        icon(&painter, square, "search", false, th.muted);
        painter.text(
            Pos2::new(rect.left() + 54.0, rect.center().y),
            Align2::LEFT_CENTER,
            t("Where to?"),
            FontId::new(16.0, Weight::Medium.family()),
            th.muted,
        );
        if response.clicked() {
            engine.ask_search();
        }
        ui.add_space(12.0);
        if !engine.nav_status.is_empty() {
            ui.label(text(engine.nav_status.as_str(), 14.5, th.warning, Weight::Medium));
            ui.add_space(8.0);
        }
        let mut chosen = None;
        if !engine.search_results.is_empty() {
            m3::card(ui, th, |ui| {
                for place in &engine.search_results {
                    let (name, detail) = split_place(&place.name);
                    let detail = place_detail(engine.gps, place.at, detail);
                    if m3::list_row(ui, th, "route", name, &detail, true) {
                        chosen = Some(place.clone());
                    }
                }
            });
        }
        if engine.nav.is_some() {
            if let Some(nav) = &engine.nav {
                Self::next_turn(ui, th, nav, engine.arrival_time());
            }
            self.map_card(ui, engine, th);
            if let Some(nav) = &engine.nav {
                m3::card(ui, th, |ui| {
                    m3::label(ui, th, t("Coming up"));
                    for m in nav.route.maneuvers.iter().skip(nav.next).take(5) {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                            draw_turn(&ui.painter_at(rect), rect, Some(m), false, th.accent);
                            ui.add_space(6.0);
                            ui.label(text(m.text.as_str(), 14.5, th.text, Weight::Regular));
                        });
                        ui.add_space(4.0);
                    }
                });
            }
            if m3::button(ui, th, t("Stop navigation"), Some("x"), Kind::Outlined, 48.0) {
                engine.stop_navigation();
            }
            ui.add_space(12.0);
        } else if engine.search_results.is_empty() {
            if engine.settings.recents.is_empty() {
                m3::card(ui, th, |ui| {
                    m3::headline(ui, th, t("Nowhere yet"));
                    ui.add_space(2.0);
                    m3::body(ui, th, t("Search for an address or a place. Places you've been will wait here for a quick start."));
                });
            } else {
                m3::card(ui, th, |ui| {
                    m3::label(ui, th, t("Recent places"));
                    for place in engine.settings.recents.iter().take(MAX_RECENTS_SHOWN) {
                        let (name, detail) = split_place(&place.name);
                        let detail = place_detail(engine.gps, place.at, detail);
                        if m3::list_row(ui, th, "history", name, &detail, true) {
                            chosen = Some(place.clone());
                        }
                    }
                });
            }
        }
        if let Some(place) = chosen {
            engine.navigate_to(&place);
        }
        m3::small(ui, th, t("Planning a route sends your position and destination to free OpenStreetMap services. Routes and places: © OpenStreetMap contributors"));
    }

    /// The street map with the route on it. It turns with the rider, or shows the whole route, north up.
    fn map_card(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let Some(nav) = engine.nav.clone() else {
            return;
        };
        if self.map.dark != th.dark {
            self.map.dark = th.dark;
            self.map.tiles.clear();
            engine.forget_tiles();
        }
        let ctx = ui.ctx().clone();
        for _ in 0..TILES_DECODED_PER_FRAME {
            let Some((key, path)) = engine.tiles_ready.pop() else {
                break;
            };
            if let Some(image) = load_tile(&path, th.dark) {
                if self.map.tiles.len() >= TILES_KEPT {
                    self.map.tiles.clear();
                    engine.forget_tiles();
                }
                self.map.tiles.insert(key, ctx.load_texture("map", image, TextureOptions::LINEAR));
            }
        }
        if !engine.tiles_ready.is_empty() {
            ctx.request_repaint();
        }

        let frame = Frame::new().fill(th.raised).corner_radius(CornerRadius::same(m3::CARD_RADIUS));
        frame.show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), MAP_HEIGHT), Sense::hover());
            let painter = ui.painter_at(rect);
            let points = &nav.route.points;

            // Where to look: the whole route, or just ahead of the rider.
            let (z, centre_at, anchor, turn) = if self.map.whole_route {
                let (mut lat0, mut lat1, mut lon0, mut lon1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
                for p in points {
                    lat0 = lat0.min(p.0);
                    lat1 = lat1.max(p.0);
                    lon0 = lon0.min(p.1);
                    lon1 = lon1.max(p.1);
                }
                let mut z = CLOSE_ZOOM;
                while z > 8 {
                    let span = map_position((lat0, lon1), z) - map_position((lat1, lon0), z);
                    if span.x * TILE_POINTS < rect.width() * 0.85 && span.y * TILE_POINTS < rect.height() * 0.85 {
                        break;
                    }
                    z -= 1;
                }
                (z, ((lat0 + lat1) / 2.0, (lon0 + lon1) / 2.0), rect.center(), 0.0f32)
            } else {
                let z = if engine.speed_kmh.is_some_and(|s| s > FAR_ZOOM_KMH) { FAR_ZOOM } else { CLOSE_ZOOM };
                let at = engine.gps.unwrap_or(points[nav.index_along()]);
                let wanted = engine.heading_deg.unwrap_or(0.0) as f32;
                let gap = (wanted - self.map.heading_deg + 540.0).rem_euclid(360.0) - 180.0;
                if gap.abs() > 0.5 {
                    self.map.heading_deg += gap * HEADING_EASE;
                    ctx.request_repaint();
                }
                (z, at, rect.center() + Vec2::new(0.0, rect.height() * 0.24), self.map.heading_deg.to_radians())
            };
            let (sin, cos) = (-turn).sin_cos();
            let centre = map_position(centre_at, z);
            let to_screen = |p: Vec2| {
                let d = (p - centre) * TILE_POINTS;
                anchor + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
            };

            // The map pictures that fall inside the card.
            let reach = (rect.size().length() / TILE_POINTS).ceil() as i32 + 1;
            let count = 1i64 << z;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (tx, ty) = (centre.x.floor() as i64 + i64::from(dx), centre.y.floor() as i64 + i64::from(dy));
                    if tx < 0 || ty < 0 || tx >= count || ty >= count {
                        continue;
                    }
                    let corner = |ox: f32, oy: f32| to_screen(Vec2::new(tx as f32 + ox, ty as f32 + oy));
                    let quad = [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)];
                    if !Rect::from_points(&quad).intersects(rect) {
                        continue;
                    }
                    let key = (z, tx as u32, ty as u32);
                    engine.want_tile(key);
                    if let Some(texture) = self.map.tiles.get(&key) {
                        let mut mesh = egui::Mesh::with_texture(texture.id());
                        let uvs = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
                        for (pos, uv) in quad.into_iter().zip(uvs) {
                            mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
                        }
                        mesh.add_triangle(0, 1, 2);
                        mesh.add_triangle(0, 2, 3);
                        painter.add(egui::Shape::mesh(mesh));
                    }
                }
            }

            // The route: what's been ridden fades, and what's ahead stands out.
            let along = nav.index_along();
            let line = |part: &[LatLon]| -> Vec<Pos2> { part.iter().map(|p| to_screen(map_position(*p, z))).collect() };
            painter.line(line(&points[..=along]), Stroke::new(5.0, th.faint));
            painter.line(line(&points[along..]), Stroke::new(9.0, th.surface));
            painter.line(line(&points[along..]), Stroke::new(6.0, th.accent));
            if let Some(m) = nav.current() {
                let spot = points[m.begin.min(points.len() - 1)];
                painter.circle(to_screen(map_position(spot, z)), 6.0, th.surface, Stroke::new(3.0, th.accent));
            }
            if let Some(last) = points.last() {
                painter.circle(to_screen(map_position(*last, z)), 8.0, th.warning, Stroke::new(3.0, th.surface));
            }

            // The rider, as an arrow pointing the way they're going.
            if let Some(at) = engine.gps {
                let spot = to_screen(map_position(at, z));
                let way = if self.map.whole_route { engine.heading_deg.unwrap_or(0.0) as f32 } else { 0.0 };
                let (s, c) = way.to_radians().sin_cos();
                let tip = |x: f32, y: f32| spot + Vec2::new(x * c - y * s, x * s + y * c);
                let arrow = vec![tip(0.0, -15.0), tip(11.0, 11.0), tip(0.0, 5.0), tip(-11.0, 11.0)];
                painter.circle_filled(spot, 21.0, th.accent.gamma_multiply(0.22));
                painter.add(egui::Shape::convex_polygon(vec![arrow[0], arrow[1], arrow[2]], th.accent, Stroke::NONE));
                painter.add(egui::Shape::convex_polygon(vec![arrow[0], arrow[2], arrow[3]], th.accent, Stroke::NONE));
                painter.add(egui::Shape::closed_line(arrow, Stroke::new(2.0, th.surface)));
            }

            // The switch between following the rider and seeing the whole route.
            let button = Rect::from_center_size(rect.right_top() + Vec2::new(-34.0, 34.0), Vec2::splat(48.0));
            let response = ui.interact(button, ui.scope_id().with("map-view"), Sense::click());
            painter.circle_filled(button.center(), 22.0, th.surface);
            let glyph = if self.map.whole_route { "target" } else { "route" };
            icon(&painter, Rect::from_center_size(button.center(), Vec2::splat(24.0)), glyph, false, th.accent);
            if response.clicked() {
                self.map.whole_route = !self.map.whole_route;
            }

            let credit = rect.right_bottom() + Vec2::new(-8.0, -6.0);
            let font = FontId::new(10.0, Weight::Medium.family());
            let galley = painter.layout_no_wrap("© OpenStreetMap".to_string(), font, th.muted);
            let back = Rect::from_min_size(credit - galley.size() - Vec2::splat(3.0), galley.size() + Vec2::splat(6.0));
            painter.rect_filled(back, CornerRadius::same(4), th.surface.gamma_multiply(0.85));
            painter.galley(back.min + Vec2::splat(3.0), galley, th.muted);
        });
        ui.add_space(10.0);
    }

    fn voice_screen(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 156.0), Sense::click());
        let painter = ui.painter_at(rect);
        let round = CornerRadius::same(28);
        painter.rect_filled(rect, round, th.accent);
        if response.is_pointer_button_down_on() {
            painter.rect_filled(rect, round, th.on_accent.gamma_multiply(0.14));
        }
        let square = Rect::from_center_size(rect.center() - Vec2::new(0.0, 40.0), Vec2::splat(38.0));
        icon(&painter, square, "voice", true, th.on_accent);
        painter.text(
            rect.center() + Vec2::new(0.0, 8.0),
            Align2::CENTER_CENTER,
            t("Tell me how things are"),
            FontId::new(21.0, Weight::Bold.family()),
            th.on_accent,
        );
        painter.text(
            rect.center() + Vec2::new(0.0, 40.0),
            Align2::CENTER_CENTER,
            t("Tap here, or press your earpiece's button"),
            FontId::new(13.5, Weight::Medium.family()),
            th.on_accent,
        );
        if response.clicked() {
            engine.speak_status();
        }
        ui.add_space(10.0);
        Self::natural_voice_card(ui, engine, th);
        m3::card(ui, th, |ui| {
            let on = engine.settings.voice_mode;
            if m3::switch_row(ui, th, t("Speak to me"), on) {
                engine.set_voice_mode(!on);
            }
            ui.add_space(4.0);
            if m3::button(ui, th, t("Test your earpiece"), Some("hearing"), Kind::Tonal, 46.0) {
                engine.test_voice();
            }
            ui.add_space(12.0);
            let rate = format!("{:.0}%", engine.settings.speech_rate * 100.0);
            ui.horizontal(|ui| {
                ui.label(text(t("Speech speed"), 15.5, th.text, Weight::Medium));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(text(rate, 15.5, th.accent, Weight::Bold));
                });
            });
            ui.add_space(6.0);
            ui.columns(3, |c| {
                if m3::button(&mut c[0], th, t("Slower"), None, Kind::Outlined, 44.0) {
                    engine.change_speech_rate(-SPEECH_RATE_STEP);
                }
                if m3::button(&mut c[1], th, t("Repeat"), Some("refresh"), Kind::Outlined, 44.0) {
                    engine.repeat_last();
                }
                if m3::button(&mut c[2], th, t("Faster"), None, Kind::Outlined, 44.0) {
                    engine.change_speech_rate(SPEECH_RATE_STEP);
                }
            });
        });
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("What to tell me about"));
            let s = &mut engine.settings;
            let mut changed = false;
            let choices = [
                (t("Battery and range"), &mut s.say_battery),
                (t("Consumption"), &mut s.say_consumption),
                (t("Directions"), &mut s.say_navigation),
                (t("Calls"), &mut s.say_calls),
            ];
            for (name, value) in choices {
                if m3::switch_row(ui, th, name, *value) {
                    *value = !*value;
                    changed = true;
                }
            }
            ui.add_space(4.0);
            m3::small(ui, th, t("Warnings are always spoken. During a call, only directions and warnings are."));
            if changed {
                engine.save_settings();
            }
        });
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Last said"));
            if engine.spoken.is_empty() {
                m3::body(ui, th, t("Nothing yet."));
            }
            for said in engine.spoken.iter().take(5) {
                ui.label(text(said.as_str(), 14.5, th.text, Weight::Regular));
                ui.add_space(4.0);
            }
        });
    }

    fn natural_voice_card(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Natural voice"));
            match engine.voice_pack {
                VoicePack::Ready => {
                    let on = engine.settings.natural_voice;
                    if m3::switch_row(ui, th, t("Use the natural voice"), on) {
                        engine.set_natural_voice(!on);
                    }
                    ui.add_space(4.0);
                    if m3::button(ui, th, t("Remove the voice"), Some("x"), Kind::Outlined, 44.0) {
                        engine.remove_voice();
                    }
                }
                VoicePack::Downloading(percent) => {
                    m3::body(ui, th, &tf("Downloading the voice: {}%", &[&percent]));
                    ui.add_space(6.0);
                    m3::progress(ui, th, Id::NULL.with("voice-download"), f32::from(percent) / 100.0, th.accent, 8.0);
                }
                VoicePack::None | VoicePack::Failed => {
                    let failed = engine.voice_pack == VoicePack::Failed;
                    if failed {
                        ui.label(text(t("The download failed. Check the internet connection."), 14.0, th.warning, Weight::Medium));
                    } else {
                        m3::body(ui, th, t("A much clearer voice that runs on the phone, with no internet. It's a single download of about 70 MB, best on Wi-Fi."));
                    }
                    ui.add_space(8.0);
                    let name = if failed { t("Try again") } else { t("Download the voice") };
                    if m3::button(ui, th, name, Some("voice"), Kind::Filled, 46.0) {
                        engine.download_voice();
                    }
                }
            }
        });
    }

    fn scene(&self, engine: &Engine, th: &Tokens) -> Scene {
        let live = engine.live.as_ref();
        let charge = |i: usize| live.filter(|l| l.packs[i].fitted).map(|l| l.packs[i].percent);
        Scene {
            yaw: self.model.yaw,
            pitch: self.model.pitch,
            zoom: self.model.zoom,
            seat: self.model.seat,
            body: match engine.settings.moped_color {
                MopedColor::Teal => TEAL_BODY,
                MopedColor::White => WHITE_BODY,
                MopedColor::Black => BLACK_BODY,
            },
            accent: bytes(th.accent),
            danger: bytes(th.danger),
            charge: [charge(0), charge(1)],
            lights_on: live.is_some_and(|l| l.switched_on),
            warning: live.is_some_and(|l| l.warning != 0),
        }
    }

    /// The moped itself: drag to turn, pinch to zoom, double tap to reset, tap a dot for that part.
    fn model_card(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let frame = Frame::new()
            .fill(th.surface)
            .stroke(if th.outlined { Stroke::new(1.5, th.border) } else { Stroke::NONE })
            .corner_radius(CornerRadius::same(m3::CARD_RADIUS))
            .inner_margin(Margin::same(0));
        frame.show(ui, |ui| {
            let size = Vec2::new(ui.available_width(), MODEL_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let ctx = ui.ctx().clone();

            if response.dragged() {
                let delta = response.drag_delta();
                self.model.target_yaw += delta.x * TURN_PER_POINT;
                self.model.target_pitch = (self.model.target_pitch + delta.y * TURN_PER_POINT).clamp(-0.05, 1.25);
            }
            if response.hovered() || response.dragged() {
                let pinch = ctx.input(|i| i.zoom_delta());
                if (pinch - 1.0).abs() > 0.001 {
                    self.model.zoom = (self.model.zoom * pinch).clamp(ZOOM_LIMITS.0, ZOOM_LIMITS.1);
                }
            }
            if response.double_clicked() {
                self.model.zoom = 1.0;
                self.model.select(self.model.focus);
            }

            let ease = |value: &mut f32, target: f32| {
                let gap = target - *value;
                if gap.abs() < SETTLED {
                    *value = target;
                    false
                } else {
                    *value += gap * CAMERA_EASE;
                    true
                }
            };
            let dragging = response.dragged();
            let target_seat = self.model.seat_target();
            let (target_yaw, target_pitch) = (self.model.target_yaw, self.model.target_pitch);
            let moving_yaw = ease(&mut self.model.yaw, target_yaw);
            let moving_pitch = ease(&mut self.model.pitch, target_pitch);
            let moving_seat = ease(&mut self.model.seat, target_seat);
            let moving = dragging || moving_yaw || moving_pitch || moving_seat;
            if moving {
                ctx.request_repaint();
            }

            // A lighter picture while it moves, and a crisp one once it rests.
            let ppp = ctx.pixels_per_point();
            let longest = rect.width().max(rect.height()) * ppp;
            let scale = (if moving { MOVING_MAX_PX } else { STILL_MAX_PX } / longest).min(1.0);
            let width = ((rect.width() * ppp * scale) as usize).max(16);
            let height = ((rect.height() * ppp * scale) as usize).max(16);
            let smooth = if moving { 1 } else { 2 };
            let scene = self.scene(engine, th);
            let wanted = (scene, width, height, smooth);
            if self.model.drawn != Some(wanted) {
                let pixels = model3d::render(&scene, width, height, smooth);
                let image = egui::ColorImage::from_rgba_premultiplied([width, height], &pixels);
                match &mut self.model.texture {
                    Some(texture) => texture.set(image, TextureOptions::LINEAR),
                    None => self.model.texture = Some(ctx.load_texture("moped", image, TextureOptions::LINEAR)),
                }
                self.model.drawn = Some(wanted);
            }
            let painter = ui.painter_at(rect);
            if let Some(texture) = &self.model.texture {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture.id(), rect, uv, Color32::WHITE);
            }

            let aspect = rect.width() / rect.height();
            let spots = [
                (Focus::Batteries, model3d::ANCHOR_BATTERIES),
                (Focus::Motor, model3d::ANCHOR_MOTOR),
                (Focus::Dash, model3d::ANCHOR_DASH),
            ];
            for (focus, anchor) in spots {
                let Some((x, y)) = model3d::project_point(&scene, anchor, aspect) else {
                    continue;
                };
                let centre = rect.min + Vec2::new(x * rect.width(), y * rect.height());
                let chosen = self.model.focus == focus;
                let area = Rect::from_center_size(centre, Vec2::splat(44.0));
                let spot = ui.interact(area, ui.scope_id().with(("spot", anchor[0].to_bits())), Sense::click());
                painter.circle_filled(centre, if chosen { 13.0 } else { 11.0 }, th.accent.gamma_multiply(0.28));
                painter.circle_filled(centre, if chosen { 7.0 } else { 5.5 }, th.accent);
                painter.circle_stroke(centre, if chosen { 7.0 } else { 5.5 }, Stroke::new(1.5, th.surface));
                if spot.clicked() {
                    self.model.select(focus);
                }
            }

            let hint = rect.left_bottom() + Vec2::new(14.0, -14.0);
            painter.text(hint, Align2::LEFT_BOTTOM, t("Drag to turn it. Pinch to zoom."), FontId::new(12.0, Weight::Medium.family()), th.faint);
        });
        ui.add_space(10.0);
    }

    fn moped_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        if engine.link != Link::Online || engine.demo_active {
            Self::connection_card(ui, engine, th, &mut self.confirm_forget);
        }
        self.model_card(ui, engine, th);
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 38.0), Layout::left_to_right(Align::Center), |ui| {
            let color = engine.settings.moped_color;
            if m3::filter_chip(ui, th, t("Teal"), color == MopedColor::Teal) {
                engine.set_moped_color(MopedColor::Teal);
            }
            if m3::filter_chip(ui, th, t("White"), color == MopedColor::White) {
                engine.set_moped_color(MopedColor::White);
            }
            if m3::filter_chip(ui, th, t("Black"), color == MopedColor::Black) {
                engine.set_moped_color(MopedColor::Black);
            }
        });
        ui.add_space(8.0);
        let focuses = [Focus::Batteries, Focus::Motor, Focus::Dash];
        let selected = focuses.iter().position(|f| *f == self.model.focus).unwrap_or(0);
        if let Some(i) = m3::segmented(ui, th, &[t("Batteries"), t("Motor"), t("Dash")], selected) {
            self.model.select(focuses[i]);
        }
        ui.add_space(10.0);
        match self.model.focus {
            Focus::Batteries => Self::batteries_panel(ui, engine, th),
            Focus::Motor => Self::motor_panel(ui, engine, th),
            Focus::Dash => {
                Self::dash_panel(ui, engine, th);
                Self::about_moped_card(ui, engine, th);
            }
        }
        if !engine.demo_active && engine.link == Link::Online {
            Self::connection_card(ui, engine, th, &mut self.confirm_forget);
        }
        if engine.demo_active && m3::button(ui, th, t("End the demo ride"), Some("x"), Kind::Outlined, 48.0) {
            engine.stop_demo();
        }
    }

    fn batteries_panel(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        let Some(live) = &engine.live else {
            m3::card(ui, th, |ui| m3::body(ui, th, t("Connect to the moped to see this.")));
            return;
        };
        if !live.packs.iter().any(|p| p.fitted) {
            m3::card(ui, th, |ui| m3::body(ui, th, t("No battery fitted")));
        }
        for (i, pack) in live.packs.iter().enumerate().filter(|(_, p)| p.fitted) {
            m3::card(ui, th, |ui| {
                ui.horizontal(|ui| {
                    ui.label(text(tf("Battery {}", &[&(i + 1)]), 16.0, th.text, Weight::Bold));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(text(format!("{}%", pack.percent), 22.0, charge_color(th, pack.percent, pack.low), Weight::Bold));
                    });
                });
                ui.add_space(6.0);
                m3::progress(
                    ui,
                    th,
                    Id::NULL.with(("moped-pack", i)),
                    f32::from(pack.percent) / 100.0,
                    charge_color(th, pack.percent, pack.low),
                    10.0,
                );
                ui.add_space(12.0);
                m3::stat_row(
                    ui,
                    th,
                    &[
                        (format!("{:.1} Ah", pack.remaining_ah), t("stored")),
                        (format!("{}%", pack.health), t("health")),
                        (tf("{} h", &[&format!("{:.1}", Engine::charge_hours(pack.percent, pack.health))]), t("to charge")),
                    ],
                );
                ui.add_space(8.0);
                m3::stat_row(
                    ui,
                    th,
                    &[
                        (format!("{:.1} V", pack.voltage), t("voltage")),
                        (format!("{:.1} A", (pack.current_ma as f32 / 1000.0).abs()), t("current")),
                        (format!("{:.0} °C", pack.temp_max), t("temperature")),
                    ],
                );
                if let (Some(high), Some(low)) = (pack.cell_max_v, pack.cell_min_v) {
                    ui.add_space(8.0);
                    m3::stat_row(
                        ui,
                        th,
                        &[
                            (format!("{high:.2} V"), t("highest cell")),
                            (format!("{low:.2} V"), t("lowest cell")),
                            (format!("{} A", pack.limit_discharge_a), t("current limit")),
                        ],
                    );
                }
            });
        }
        for record in &engine.battery_records {
            m3::card(ui, th, |ui| {
                m3::label(ui, th, t("Battery history"));
                ui.label(text(tf("Charged {} times", &[&record.charges]), 16.0, th.text, Weight::Bold));
                ui.add_space(8.0);
                m3::stat_row(
                    ui,
                    th,
                    &[
                        (record.normal_charges.to_string(), t("normal")),
                        (record.slow_charges.to_string(), t("slow")),
                        (record.fast_charges.to_string(), t("fast")),
                    ],
                );
                if !record.started.is_empty() {
                    ui.add_space(6.0);
                    m3::small(ui, th, &tf("In use since {}", &[&record.started]));
                }
            });
        }
        if let Some(full) = engine.full_range_km() {
            ui.label(text(tf("About {} km on a full charge", &[&format!("{full:.0}")]), 13.5, th.muted, Weight::Medium));
            ui.add_space(10.0);
        }
    }

    fn motor_panel(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        m3::card(ui, th, |ui| {
            let Some(live) = &engine.live else {
                m3::body(ui, th, t("Connect to the moped to see this."));
                return;
            };
            m3::stat_row(
                ui,
                th,
                &[
                    (format!("{:.0} °C", live.motor_temp), t("motor")),
                    (format!("{:.0} °C", live.controller_temp), t("controller")),
                    (live.motor_rpm.to_string(), t("revs")),
                ],
            );
            ui.add_space(10.0);
            m3::stat_row(
                ui,
                th,
                &[(format_distance(f64::from(live.odometer_m)), t("odometer")), (format!("{}%", live.throttle), t("throttle"))],
            );
        });
    }

    fn dash_panel(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Lights on the moped's dash"));
            Self::dash_lights(ui, engine, th);
            let Some(live) = &engine.live else {
                ui.add_space(8.0);
                m3::body(ui, th, t("Connect to the moped to see this."));
                return;
            };
            ui.add_space(12.0);
            m3::stat_row(ui, th, &[(format!("{:.1} V", live.aux_voltage), t("12 V battery"))]);
            ui.add_space(10.0);
            let errors: Vec<String> = live.active_faults().iter().map(|c| format!("{c:04X}")).collect();
            if live.warning != 0 || !errors.is_empty() {
                let line = if errors.is_empty() {
                    t("Warning on").to_string()
                } else {
                    tf("Warning on: {}", &[&errors.join(", ")])
                };
                ui.label(text(line, 15.5, th.danger, Weight::Bold));
            } else {
                ui.label(text(t("No warnings"), 15.0, th.success, Weight::SemiBold));
            }
            ui.add_space(4.0);
            m3::small(ui, th, &tf("Model number {}", &[&live.model_number]));
        });
    }

    fn about_moped_card(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        m3::card(ui, th, |ui| {
            if let Some(vehicle) = &engine.vehicle {
                m3::label(ui, th, t("About this moped"));
                ui.label(text(vehicle.vin.as_str(), 16.0, th.text, Weight::Bold));
                m3::small(ui, th, t("Frame number"));
                ui.add_space(8.0);
                m3::stat_row(
                    ui,
                    th,
                    &[(vehicle.software.to_string(), t("software")), (vehicle.times_on.to_string(), t("times switched on"))],
                );
                ui.add_space(10.0);
            }
            let on = engine.recording;
            if m3::switch_row(ui, th, t("Record what the moped sends"), on) {
                engine.set_recording(!on);
            }
            m3::small(ui, th, t("This helps work out more of its data. The recording includes the frame number, so share it only with someone you trust."));
            let count = engine.recorded_count();
            if count > 0 {
                ui.add_space(8.0);
                if m3::button(ui, th, &tf("Share the recording ({} messages)", &[&count]), None, Kind::Tonal, 44.0) {
                    engine.share_recording();
                }
            }
        });
    }

    fn connection_card(ui: &mut Ui, engine: &mut Engine, th: &Tokens, confirm_forget: &mut bool) {
        if engine.demo_active {
            m3::card(ui, th, |ui| {
                ui.label(text(t("This is a demo ride"), 16.0, th.warning, Weight::Bold));
                m3::body(ui, th, t("These figures are invented to show the app. They aren't your moped's."));
            });
            return;
        }
        m3::card(ui, th, |ui| match engine.link {
            Link::NeedsPairing | Link::Refused => {
                let heading = if engine.link == Link::Refused { t("The moped refused this code") } else { t("Pair your moped") };
                m3::headline(ui, th, heading);
                ui.add_space(4.0);
                m3::body(ui, th, t("Use the camera to read the QR sticker on your moped."));
                ui.add_space(12.0);
                if m3::button(ui, th, t("Scan the QR code"), Some("target"), Kind::Filled, 48.0) {
                    engine.scan_pairing();
                }
                ui.add_space(6.0);
                if m3::button(ui, th, t("Type the code instead"), None, Kind::Outlined, 46.0) {
                    engine.ask_pairing();
                }
                ui.add_space(6.0);
                if m3::button(ui, th, t("Try a demo ride"), Some("play"), Kind::Outlined, 46.0) {
                    engine.start_demo();
                }
            }
            Link::Idle => {
                m3::headline(ui, th, t("Moped paired"));
                ui.add_space(12.0);
                if m3::button(ui, th, t("Connect"), Some("bluetooth"), Kind::Filled, 48.0) {
                    engine.connect();
                }
                ui.add_space(6.0);
                let name = if *confirm_forget { t("Tap again to forget this moped") } else { t("Forget this moped") };
                let kind = if *confirm_forget { Kind::Danger } else { Kind::Outlined };
                if m3::button(ui, th, name, None, kind, 46.0) {
                    if *confirm_forget {
                        engine.forget_moped();
                    }
                    *confirm_forget = !*confirm_forget;
                }
            }
            Link::Scanning | Link::Connecting | Link::LoggingIn => {
                let line = match engine.link {
                    Link::Scanning => t("Looking for the moped. Turn it on and stay close."),
                    Link::Connecting => t("Connecting to the moped..."),
                    _ => t("Signing in to the moped..."),
                };
                m3::title(ui, th, line);
                let devices = engine.found_devices.clone();
                for (name, address) in devices {
                    ui.add_space(6.0);
                    if m3::button(ui, th, &tf("Use {}", &[&name]), Some("moped"), Kind::Tonal, 46.0) {
                        engine.choose_device(&address);
                    }
                }
                ui.add_space(8.0);
                if m3::button(ui, th, t("Cancel"), None, Kind::Outlined, 46.0) {
                    engine.disconnect();
                }
            }
            Link::Online => {
                ui.label(text(t("Connected to the moped"), 15.5, th.success, Weight::SemiBold));
                ui.add_space(8.0);
                if m3::button(ui, th, t("Disconnect"), None, Kind::Outlined, 46.0) {
                    engine.disconnect();
                }
            }
        });
    }

    fn alerts_screen(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        if !engine.notification_access
            && !engine.demo_active
            && m3::banner(
                ui,
                th,
                "bell",
                t("Let Pillion see notifications"),
                t("Android has to allow this. Message text is only read aloud on your phone and is never sent anywhere. If Android calls it a restricted setting, open Pillion's App info, tap the three dots, then Allow restricted settings."),
                Some(t("Open the Android setting")),
            )
        {
            engine.open_notification_access();
        }
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Read notifications aloud"));
            let levels = [NotifLevel::Off, NotifLevel::AppAndSender, NotifLevel::Full];
            let selected = levels.iter().position(|l| *l == engine.settings.notif_level).unwrap_or(2);
            if let Some(i) = m3::segmented(ui, th, &[t("Off"), t("Who"), t("Full text")], selected) {
                engine.settings.notif_level = levels[i];
                engine.save_settings();
            }
            ui.add_space(8.0);
            m3::small(ui, th, t("Several messages in a row from one app are told as a single count."));
        });
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Lights on the moped's dash"));
            let calls = engine.settings.dash_calls;
            if m3::switch_row(ui, th, t("Show calls on the dash"), calls) {
                engine.settings.dash_calls = !calls;
                engine.save_settings();
            }
            let apps = engine.apps.clone();
            for app in apps {
                let muted = engine.settings.muted_apps.contains(&app.package);
                let slot = engine.settings.dash_slots.iter().position(|p| *p == app.package);
                let light = slot.map_or(t("no light").to_string(), |s| tf("light {}", &[&(s + 1)]));
                if m3::switch_row(ui, th, &format!("{} ({light})", app.label), !muted) {
                    engine.toggle_app_muted(&app.package);
                }
            }
            ui.add_space(4.0);
            if engine.apps.is_empty() {
                m3::small(ui, th, t("Apps appear here after their first notification."));
            }
            m3::small(ui, th, t("The dash can only light up, it can't show message text. Each app below gets a light, up to six."));
        });
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Recent"));
            if engine.notifications.is_empty() {
                m3::body(ui, th, t("Nothing yet."));
            }
            for n in engine.notifications.iter().take(8) {
                m3::title(ui, th, &format!("{}: {}", n.label, n.title));
                m3::body(ui, th, n.text.as_str());
                ui.add_space(6.0);
            }
        });
    }

    fn updates_card(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Updates"));
            let state = engine.update_state;
            let install_button = |ui: &mut Ui, engine: &mut Engine| {
                if m3::button(ui, th, t("Download and install"), Some("refresh"), Kind::Filled, 46.0) {
                    engine.install_update();
                }
            };
            match state {
                UpdateState::Idle => {}
                UpdateState::Checking => m3::body(ui, th, t("Checking...")),
                UpdateState::UpToDate => {
                    ui.label(text(t("Pillion is up to date."), 14.0, th.success, Weight::SemiBold));
                }
                UpdateState::CheckFailed => {
                    ui.label(text(t("Couldn't check for updates."), 14.0, th.warning, Weight::Medium));
                }
                UpdateState::Available | UpdateState::InstallFailed => {
                    if state == UpdateState::InstallFailed {
                        ui.label(text(t("The update didn't install."), 14.0, th.warning, Weight::Medium));
                    }
                    if let Some(release) = engine.update.clone() {
                        m3::title(ui, th, &tf("Version {} is ready to install.", &[&release.version]));
                        if !release.notes.is_empty() {
                            ui.add_space(4.0);
                            m3::body(ui, th, &release.notes);
                        }
                        ui.add_space(8.0);
                        install_button(ui, engine);
                        ui.add_space(4.0);
                        m3::small(ui, th, t("Android asks you to confirm, then Pillion closes to update."));
                    }
                }
                UpdateState::Downloading(percent) => {
                    m3::body(ui, th, &tf("Downloading the update: {}%", &[&percent]));
                    ui.add_space(6.0);
                    m3::progress(ui, th, Id::NULL.with("update-download"), f32::from(percent) / 100.0, th.accent, 8.0);
                }
                UpdateState::NeedsPermission => {
                    m3::body(ui, th, t("Android needs your permission for Pillion to install updates. Allow it, come back, and tap below."));
                    ui.add_space(8.0);
                    install_button(ui, engine);
                }
                UpdateState::Installing => m3::body(ui, th, t("Android asks you to confirm, then Pillion closes to update.")),
            }
            let busy = matches!(state, UpdateState::Checking | UpdateState::Downloading(_) | UpdateState::Installing);
            if !busy {
                ui.add_space(8.0);
                if m3::button(ui, th, t("Check for updates"), None, Kind::Outlined, 44.0) {
                    engine.check_for_update_now();
                }
            }
            ui.add_space(4.0);
            let auto = engine.settings.update_auto;
            if m3::switch_row(ui, th, t("Check once a day"), auto) {
                engine.set_update_auto(!auto);
            }
            m3::small(ui, th, t("This only asks GitHub which version is the newest."));
        });
    }

    fn settings_screen(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Appearance"));
            let modes = [Appearance::System, Appearance::Light, Appearance::Dark];
            let selected = modes.iter().position(|m| *m == engine.settings.appearance).unwrap_or(0);
            if let Some(i) = m3::segmented(ui, th, &[t("System"), t("Light"), t("Dark")], selected) {
                engine.settings.appearance = modes[i];
                engine.save_settings();
            }
            ui.add_space(6.0);
            let high = engine.settings.high_contrast;
            if m3::switch_row(ui, th, t("High contrast"), high) {
                engine.settings.high_contrast = !high;
                engine.save_settings();
            }
        });
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Language"));
            let choices = [None, Some(Lang::En), Some(Lang::It), Some(Lang::Ru)];
            let selected = choices.iter().position(|c| *c == engine.settings.language).unwrap_or(0);
            // Each language is written in itself, so it can be found from any of them.
            if let Some(i) = m3::segmented(ui, th, &[t("System"), "English", "Italiano", "Русский"], selected) {
                engine.set_language(choices[i]);
            }
            ui.add_space(8.0);
            m3::small(ui, th, t("Spoken directions follow this too."));
        });
        Self::updates_card(ui, engine, th);
        m3::card(ui, th, |ui| {
            m3::headline(ui, th, "Pillion");
            ui.label(text(tf("Version {}", &[&env!("CARGO_PKG_VERSION")]), 13.5, th.muted, Weight::Medium));
            ui.add_space(6.0);
            m3::body(ui, th, t("The seat behind the rider. A companion for your electric moped, part of the Quezka suite."));
            ui.add_space(6.0);
            m3::small(ui, th, t("Manrope font (SIL Open Font License), Material Symbols (Apache 2.0), routes and places © OpenStreetMap contributors."));
            ui.add_space(4.0);
            m3::small(ui, th, t("Natural voices: Piper, run by sherpa-onnx (Apache 2.0) with espeak-ng (GPL 3). Voices: Alba (CC BY 4.0), Paola, Irina."));
            ui.add_space(4.0);
            m3::small(ui, th, t("Copyright © 2026 Quezka. Free software under the GNU GPL version 3, with no warranty. Licence, notices and source code: github.com/Quezka/pillion"));
        });
    }

    fn nav_bar(&mut self, ui: &mut Ui, th: &Tokens) {
        let current = if self.screen == Screen::Settings { self.last_tab } else { self.screen };
        ui.add_space(4.0);
        let mut chosen = None;
        ui.columns(5, |c| {
            for (column, (screen, name, glyph)) in c.iter_mut().zip([
                (Screen::Ride, t("Ride"), "moped"),
                (Screen::Route, t("Route"), "route"),
                (Screen::Voice, t("Voice"), "voice"),
                (Screen::Moped, t("Moped"), "battery"),
                (Screen::Alerts, t("Alerts"), "bell"),
            ]) {
                if m3::nav_item(column, th, glyph, name, current == screen) {
                    chosen = Some(screen);
                }
            }
        });
        if let Some(screen) = chosen {
            self.go_to(ui, screen);
        }
    }

    /// Desktop preview only. On a phone Android's own dialog asks instead, because the on-screen
    /// keyboard doesn't reach this kind of window.
    fn typed_prompt(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let Some((key, heading)) = self.typed_key.clone() else {
            return;
        };
        m3::card(ui, th, |ui| {
            m3::title(ui, th, &heading);
            ui.add_space(6.0);
            ui.text_edit_singleline(&mut self.typed);
            ui.add_space(6.0);
            ui.columns(2, |c| {
                if m3::button(&mut c[0], th, t("OK"), None, Kind::Filled, 40.0) {
                    engine.on_text(&key, &self.typed);
                    self.typed.clear();
                    self.typed_key = None;
                }
                if m3::button(&mut c[1], th, t("Cancel"), None, Kind::Outlined, 40.0) {
                    self.typed.clear();
                    self.typed_key = None;
                }
            });
        });
    }

    pub fn ask_typed(&mut self, key: String, heading: String) {
        self.typed_key = Some((key, heading));
    }

    fn message_prompt(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let Some((heading, detail)) = engine.text_prompt.clone() else {
            return;
        };
        if m3::banner(ui, th, "alert", &heading, &detail, Some(t("OK"))) {
            engine.text_prompt = None;
        }
    }

    fn take_screenshot_if_due(&mut self, ctx: &egui::Context) {
        let Some(job) = self.shot.as_mut() else {
            return;
        };
        job.frames += 1;
        ctx.request_repaint();
        if job.frames >= SCREENSHOT_AFTER_FRAMES && !job.asked {
            job.asked = true;
            let path = job.path.clone();
            let closer = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                crate::save_png(&path, &image);
                closer.send_viewport_cmd(egui::ViewportCommand::Close);
                closer.request_repaint();
            })));
        }
    }

    /// The main action of the Ride screen, within reach of a thumb, and any brief message above the tabs.
    fn overlays(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens, area: Rect) {
        let now = ui.input(|i| i.time);
        if let Some(message) = engine.take_snack() {
            self.snack = Some((message, now));
        }
        let mut lift = 0.0;
        if let Some((message, since)) = &self.snack {
            let age = now - since;
            if age > SNACK_SECONDS {
                self.snack = None;
            } else {
                let fade = (age / 0.2).min((SNACK_SECONDS - age) / 0.3).clamp(0.0, 1.0) as f32;
                let room = area.shrink2(Vec2::new(14.0, 10.0));
                m3::snackbar(ui, th, room, message, fade);
                lift = 62.0;
                ui.ctx().request_repaint_after_secs(0.05);
            }
        }
        if self.screen == Screen::Ride && (engine.has_pairing() || engine.demo_active) {
            let corner = area.right_bottom() - Vec2::new(16.0, 14.0 + lift);
            if m3::extended_fab(ui, th, Id::NULL.with("tell-me"), corner, "voice", t("Tell me")) {
                engine.speak_status();
            }
        }
    }
}

fn is_dark(ctx: &egui::Context, engine: &Engine) -> bool {
    engine.settings_say_dark().unwrap_or_else(|| ctx.input(|i| i.raw.system_theme).is_none_or(|theme| theme == egui::Theme::Dark))
}

impl eframe::App for CompanionApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            self.fonts_ready = true;
            theme::install_fonts(&ctx);
        }
        self.take_screenshot_if_due(&ctx);
        ctx.request_repaint_after_secs(REPAINT_SECONDS);

        let engine_handle = Arc::clone(&self.engine);
        let Ok(mut engine) = engine_handle.lock() else {
            return;
        };
        locale::set(engine.language());
        for prompt in crate::desktop_prompts(&mut engine) {
            self.ask_typed(prompt.0, prompt.1);
        }
        let th = theme::tokens(is_dark(&ctx, &engine), engine.settings.high_contrast);
        ctx.set_visuals(if th.dark { egui::Visuals::dark() } else { egui::Visuals::light() });

        let scale = ctx.pixels_per_point();
        let (bar_top, bar_bottom) = engine.system_bars_px;
        Panel::bottom("tabs")
            .frame(Frame::new().fill(th.sidebar))
            .show_separator_line(false)
            .show(ui, |ui| {
                self.nav_bar(ui, &th);
                ui.add_space(bar_bottom / scale);
            });
        Panel::top("title")
            .frame(Frame::new().fill(th.bg).inner_margin(Margin::symmetric(14, 4)))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(bar_top / scale);
                self.top_bar(ui, &engine, &th);
            });
        CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin::symmetric(14, 4)))
            .show(ui, |ui| {
                let area = ui.max_rect().expand2(Vec2::new(14.0, 4.0));
                let age = ui.input(|i| i.time) - self.changed_screen_at;
                let fading = self.changed_screen_at >= 0.0 && age < FADE_SECONDS;
                if fading {
                    ui.ctx().request_repaint();
                }
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if fading {
                        ui.set_opacity((age / FADE_SECONDS) as f32);
                    }
                    self.typed_prompt(ui, &mut engine, &th);
                    Self::message_prompt(ui, &mut engine, &th);
                    self.problems(ui, &mut engine, &th);
                    match self.screen {
                        Screen::Ride => self.ride_screen(ui, &mut engine, &th),
                        Screen::Route => self.route_screen(ui, &mut engine, &th),
                        Screen::Voice => Self::voice_screen(ui, &mut engine, &th),
                        Screen::Moped => self.moped_screen(ui, &mut engine, &th),
                        Screen::Alerts => Self::alerts_screen(ui, &mut engine, &th),
                        Screen::Settings => Self::settings_screen(ui, &mut engine, &th),
                    }
                });
                self.overlays(ui, &mut engine, &th, area);
            });
        if m3::take_tap() {
            engine.tick_haptic();
        }
    }
}
