//! The phone screens, in the suite's Material 3 look. All of them read from and act on the shared `Engine`.

use std::f32::consts::PI;
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, Align, Align2, CentralPanel, Color32, CornerRadius, FontId, Frame, Layout, Margin, Painter, Panel, Pos2, Rect,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, Vec2,
};

use crate::engine::{Appearance, BluetoothState, Engine, Link, NotifLevel};
use crate::locale::{self, t, tf, Lang};
use crate::nav::{Maneuver, Navigation};
use crate::protocol as proto;
use crate::theme::{self, icon, Tokens, Weight};

const LOW_PERCENT: u8 = 20;
const REPAINT_SECONDS: f32 = 0.25;
const SPEECH_RATE_STEP: f32 = 0.1;
const RING_START_DEG: f32 = 135.0;
const RING_SWEEP_DEG: f32 = 270.0;
const SCREENSHOT_AFTER_FRAMES: u32 = 120;
const MAX_RECENTS_SHOWN: usize = 5;
const CARD_RADIUS: u8 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Ride,
    Route,
    Voice,
    Moped,
    Alerts,
    Settings,
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
}

struct ScreenshotJob {
    path: String,
    frames: u32,
    asked: bool,
}

fn text(value: impl Into<String>, size: f32, color: Color32, weight: Weight) -> RichText {
    RichText::new(value.into()).size(size).color(color).family(weight.family())
}

fn card_frame(th: &Tokens, fill: Color32) -> Frame {
    let stroke = if th.outlined { Stroke::new(1.5, th.border) } else { Stroke::NONE };
    Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(16))
}

fn card(ui: &mut Ui, th: &Tokens, add: impl FnOnce(&mut Ui)) {
    card_frame(th, th.surface).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(10.0);
}

/// The one filled colour of a screen, for what matters most on it.
fn tonal_card(ui: &mut Ui, th: &Tokens, add: impl FnOnce(&mut Ui)) {
    card_frame(th, th.accent_soft).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(10.0);
}

fn caption(ui: &mut Ui, th: &Tokens, value: &str) {
    ui.label(text(value, 13.0, th.faint, Weight::SemiBold));
    ui.add_space(6.0);
}

#[derive(Clone, Copy)]
enum Kind {
    Primary,
    Tonal,
    Danger,
}

/// A round Material button, with an optional icon. Full width of whatever holds it.
fn button(ui: &mut Ui, th: &Tokens, label: &str, glyph: Option<&'static str>, kind: Kind, height: f32) -> bool {
    let (fill, ink) = match kind {
        Kind::Primary => (th.accent, th.on_accent),
        Kind::Tonal => (th.raised, th.text),
        Kind::Danger => (th.danger, if th.dark { th.bg } else { th.surface }),
    };
    let size = Vec2::new(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    let shade = if response.is_pointer_button_down_on() { fill.gamma_multiply(0.8) } else { fill };
    let round = CornerRadius::same((height / 2.0).min(120.0) as u8);
    painter.rect_filled(rect, round, shade);
    if th.outlined {
        painter.rect_stroke(rect, round, Stroke::new(1.5, th.border), StrokeKind::Inside);
    }
    let font = FontId::new(15.5, Weight::SemiBold.family());
    let galley = painter.layout_no_wrap(label.to_string(), font, ink);
    let icon_size = 20.0;
    let gap = if glyph.is_some() { icon_size + 8.0 } else { 0.0 };
    let start = rect.center().x - (galley.size().x + gap) / 2.0;
    if let Some(name) = glyph {
        let square = Rect::from_center_size(Pos2::new(start + icon_size / 2.0, rect.center().y), Vec2::splat(icon_size));
        icon(&painter, square, name, false, ink);
    }
    painter.galley(Pos2::new(start + gap, rect.center().y - galley.size().y / 2.0), galley, ink);
    response.clicked()
}

/// A Material 3 switch, with the whole row to tap: easier in gloves.
fn switch_row(ui: &mut Ui, th: &Tokens, label: &str, on: bool) -> bool {
    let wrap = ui.available_width() - 64.0;
    let font = FontId::new(15.5, Weight::Medium.family());
    let galley = ui.painter().layout(label.to_string(), font, th.text, wrap);
    let height = (galley.size().y + 12.0).max(48.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let painter = ui.painter_at(rect);
    painter.galley(Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0), galley, th.text);
    let track = Rect::from_center_size(Pos2::new(rect.right() - 26.0, rect.center().y), Vec2::new(52.0, 32.0));
    let round = CornerRadius::same(16);
    if on {
        painter.rect_filled(track, round, th.accent);
    } else {
        painter.rect_filled(track, round, th.raised);
        painter.rect_stroke(track, round, Stroke::new(2.0, th.border), StrokeKind::Inside);
    }
    let (x, radius, knob) = if on {
        (track.right() - 16.0, 12.0, th.on_accent)
    } else {
        (track.left() + 16.0, 8.0, th.faint)
    };
    painter.circle_filled(Pos2::new(x, track.center().y), radius, knob);
    response.clicked()
}

/// The small choice control the suite uses instead of tabs. Returns the option tapped.
fn segmented(ui: &mut Ui, th: &Tokens, options: &[&str], selected: usize) -> Option<usize> {
    let size = Vec2::new(ui.available_width(), 44.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, th.border), StrokeKind::Inside);
    let width = rect.width() / options.len() as f32;
    let mut tapped = None;
    for (i, option) in options.iter().enumerate() {
        let cell = Rect::from_min_size(rect.min + Vec2::new(width * i as f32, 0.0), Vec2::new(width, rect.height()));
        let response = ui.interact(cell, ui.scope_id().with(("segment", i, *option)), Sense::click());
        if i == selected {
            painter.rect_filled(cell.shrink(3.0), CornerRadius::same(10), th.accent_soft);
        }
        let weight = if i == selected { Weight::SemiBold } else { Weight::Medium };
        let ink = if i == selected { th.text } else { th.muted };
        painter.text(cell.center(), Align2::CENTER_CENTER, *option, FontId::new(14.0, weight.family()), ink);
        if response.clicked() {
            tapped = Some(i);
        }
    }
    tapped
}

fn stat(ui: &mut Ui, th: &Tokens, value: &str, label: &str) {
    ui.vertical(|ui| {
        ui.label(text(value, 17.5, th.text, Weight::Bold));
        ui.label(text(label, 12.5, th.muted, Weight::Medium));
    });
}

/// A stat row with equal columns, whatever the phone's width.
fn stat_row(ui: &mut Ui, th: &Tokens, stats: &[(String, &str)]) {
    ui.columns(stats.len(), |columns| {
        for (column, (value, label)) in columns.iter_mut().zip(stats) {
            stat(column, th, value, label);
        }
    });
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
    painter.line(arc(1.0), Stroke::new(11.0, th.raised));
    if fraction > 0.0 {
        painter.line(arc(fraction.clamp(0.0, 1.0)), Stroke::new(11.0, color));
    }
}

fn turn_symbol(maneuver: Option<&Maneuver>) -> &'static str {
    let Some(m) = maneuver else {
        return "turn-straight";
    };
    if m.is_arrival() {
        return "flag";
    }
    match m.kind {
        9 | 19 | 23 => "turn-slight-right",
        10 | 20 => "turn-right",
        11 => "turn-sharp-right",
        12 => "u-turn-right",
        13 => "u-turn-left",
        14 => "turn-sharp-left",
        15 | 21 => "turn-left",
        16 | 22 | 24 => "turn-slight-left",
        _ => "turn-straight",
    }
}

fn format_distance(m: f64) -> String {
    if m >= 1000.0 {
        tf("{} km", &[&format!("{:.1}", m / 1000.0)])
    } else {
        tf("{} m", &[&format!("{:.0}", (m / 10.0).round() * 10.0)])
    }
}

fn link_pill(engine: &Engine, th: &Tokens) -> (&'static str, Color32) {
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

/// Places come with long addresses. The first part is the name, the rest says where.
fn split_place(name: &str) -> (&str, &str) {
    match name.split_once(", ") {
        Some((first, rest)) => (first, rest),
        None => (name, ""),
    }
}

/// A tappable two-line list row, with an icon.
fn list_row(ui: &mut Ui, th: &Tokens, glyph: &'static str, title: &str, detail: &str) -> bool {
    let width = ui.available_width();
    let wrap = width - 44.0 - 28.0;
    let font_title = FontId::new(15.5, Weight::SemiBold.family());
    let font_detail = FontId::new(13.0, Weight::Regular.family());
    let title_galley = ui.painter().layout(title.to_string(), font_title, th.text, wrap);
    let detail_galley = ui.painter().layout(detail.to_string(), font_detail, th.muted, wrap);
    let detail_height = if detail.is_empty() { 0.0 } else { detail_galley.size().y + 2.0 };
    let height = (title_galley.size().y + detail_height + 18.0).max(56.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter_at(rect);
    if response.is_pointer_button_down_on() {
        painter.rect_filled(rect, CornerRadius::same(12), th.raised);
    }
    let square = Rect::from_center_size(Pos2::new(rect.left() + 20.0, rect.center().y), Vec2::splat(24.0));
    icon(&painter, square, glyph, false, th.muted);
    let x = rect.left() + 44.0;
    let top = rect.center().y - (title_galley.size().y + detail_height) / 2.0;
    let title_height = title_galley.size().y;
    painter.galley(Pos2::new(x, top), title_galley, th.text);
    if !detail.is_empty() {
        painter.galley(Pos2::new(x, top + title_height + 2.0), detail_galley, th.muted);
    }
    let chevron = Rect::from_center_size(Pos2::new(rect.right() - 12.0, rect.center().y), Vec2::splat(22.0));
    icon(&painter, chevron, "chevron-right", false, th.faint);
    response.clicked()
}

/// A problem the rider can fix right now, with the button that fixes it.
fn banner(ui: &mut Ui, th: &Tokens, glyph: &'static str, title: &str, detail: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    card(ui, th, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            icon(&ui.painter_at(rect), rect, glyph, true, th.warning);
            ui.add_space(4.0);
            ui.label(text(title, 15.5, th.text, Weight::Bold));
        });
        ui.add_space(4.0);
        ui.label(text(detail, 13.5, th.muted, Weight::Regular));
        if let Some(action) = action {
            ui.add_space(10.0);
            clicked = button(ui, th, action, None, Kind::Tonal, 44.0);
        }
    });
    clicked
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
        }
    }

    fn top_bar(&mut self, ui: &mut Ui, engine: &Engine, th: &Tokens) {
        let title = match self.screen {
            Screen::Ride => t("Ride"),
            Screen::Route => t("Route"),
            Screen::Voice => t("Voice"),
            Screen::Moped => t("Moped"),
            Screen::Alerts => t("Alerts"),
            Screen::Settings => t("Settings"),
        };
        ui.horizontal(|ui| {
            if self.screen == Screen::Settings {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
                let square = Rect::from_center_size(rect.center(), Vec2::splat(24.0));
                icon(&ui.painter_at(rect), square, "arrow-back", false, th.text);
                if response.clicked() {
                    self.screen = self.last_tab;
                }
            }
            ui.label(text(title, 26.0, th.text, Weight::Bold));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.screen != Screen::Settings {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
                    let square = Rect::from_center_size(rect.center(), Vec2::splat(24.0));
                    icon(&ui.painter_at(rect), square, "settings", false, th.muted);
                    if response.clicked() {
                        self.screen = Screen::Settings;
                    }
                }
                let (label, color) = link_pill(engine, th);
                let font = FontId::new(12.5, Weight::SemiBold.family());
                let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
                let size = galley.size() + Vec2::new(20.0, 10.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, CornerRadius::same(12), color.gamma_multiply(0.16));
                painter.galley(rect.center() - galley.size() / 2.0, galley, color);
            });
        });
    }

    fn problems(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        match engine.bluetooth {
            BluetoothState::On => {}
            BluetoothState::Off => {
                if engine.has_pairing()
                    && banner(
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
                if banner(
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
        if engine.nav.is_some()
            && !engine.demo_active
            && !engine.location_on
            && banner(
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
        if engine.voice_missing {
            banner(
                ui,
                th,
                "alert",
                t("This phone can't speak yet"),
                t("It has no working speech engine. Install Speech Services by Google from the Play Store, then reopen Pillion."),
                None,
            );
        }
    }

    fn get_ready(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        tonal_card(ui, th, |ui| {
            ui.label(text(t("Get ready to ride"), 19.0, th.text, Weight::Bold));
            ui.add_space(2.0);
            ui.label(text(t("Pillion rides along and tells you what's happening, so your eyes stay on the road."), 14.0, th.muted, Weight::Regular));
        });
        card(ui, th, |ui| {
            let pairing = engine.link != Link::Refused;
            if list_row(
                ui,
                th,
                "moped",
                if pairing { t("Pair your moped") } else { t("The moped refused this code") },
                t("Type the 24 characters from its QR sticker. A QR reader app shows them."),
            ) {
                engine.ask_pairing();
            }
            if !engine.notification_access
                && list_row(ui, th, "bell", t("Allow notifications"), t("So messages can be read to you and light up the dash."))
            {
                engine.open_notification_access();
            }
            if list_row(ui, th, "hearing", t("Test your earpiece"), t("Pillion says a sentence so you know you'll hear it.")) {
                engine.test_voice();
            }
            if list_row(ui, th, "play", t("Try a demo ride"), t("See and hear everything first, with invented figures.")) {
                engine.start_demo();
            }
        });
    }

    fn ride_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        if engine.live.is_none() && !engine.has_pairing() && !engine.demo_active {
            Self::get_ready(ui, engine, th);
        } else if engine.live.is_none() && !engine.demo_active {
            Self::connection_card(ui, engine, th, &mut self.confirm_forget);
        }
        // While navigating, the next turn is what a glance is for, so it comes first.
        if let Some(nav) = &engine.nav {
            Self::next_turn(ui, th, nav);
        }
        if engine.live.is_some() {
            Self::battery_card(ui, engine, th);
        }
        if engine.nav.is_none() && (engine.has_pairing() || engine.demo_active) {
            card(ui, th, |ui| {
                if list_row(ui, th, "route", t("Plan a route"), t("Spoken directions, on roads a moped can use.")) {
                    self.screen = Screen::Route;
                    self.last_tab = Screen::Route;
                }
            });
        }
        if engine.live.is_some() {
            card(ui, th, |ui| {
                let speed = engine.speed_kmh.map_or("--".to_string(), |s| format!("{s:.0}"));
                let power = engine.power_w().map_or("--".to_string(), |p| tf("{} kW", &[&format!("{:.1}", p / 1000.0)]));
                stat_row(
                    ui,
                    th,
                    &[
                        (speed, t("km/h")),
                        (format_distance(engine.trip.km * 1000.0), t("this ride")),
                        (tf("{} kWh", &[&format!("{:.2}", engine.trip.kwh)]), t("energy")),
                        (power, t("power")),
                    ],
                );
            });
            Self::dash_card(ui, engine, th);
        }
        if let Some(said) = engine.spoken.front() {
            card(ui, th, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                    icon(&ui.painter_at(rect), rect, "voice", false, th.accent);
                    ui.add_space(4.0);
                    ui.label(text(said.as_str(), 14.0, th.muted, Weight::Regular));
                });
            });
        }
    }

    fn battery_card(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        let percent = engine.percent();
        card(ui, th, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(138.0), Sense::hover());
                let painter = ui.painter_at(rect);
                let center = rect.center();
                let color = match percent {
                    Some(p) if p <= LOW_PERCENT => th.danger,
                    _ => th.accent,
                };
                draw_ring(&painter, th, center, 58.0, f32::from(percent.unwrap_or(0)) / 100.0, color);
                let value = percent.map_or("--".to_string(), |p| format!("{p}%"));
                painter.text(center, Align2::CENTER_CENTER, value, FontId::new(36.0, Weight::Bold.family()), th.text);
                painter.text(
                    center + Vec2::new(0.0, 45.0),
                    Align2::CENTER_CENTER,
                    t("battery"),
                    FontId::new(12.5, Weight::Medium.family()),
                    th.muted,
                );
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    let range = engine.range_km().map_or("--".to_string(), |r| tf("{} km", &[&format!("{r:.0}")]));
                    ui.label(text(range, 32.0, th.text, Weight::Bold));
                    let withheld = percent.is_some() && engine.range_km().is_none();
                    let label = if withheld { t("too low to estimate") } else { t("range left") };
                    ui.label(text(label, 13.0, th.muted, Weight::Medium));
                    ui.add_space(8.0);
                    let eff = engine.measured_efficiency();
                    let value = eff.map_or("--".to_string(), |e| tf("{} km/kWh", &[&format!("{e:.0}")]));
                    ui.label(text(value, 19.0, th.text, Weight::Bold));
                    let label = if eff.is_some() { t("this ride's consumption") } else { t("consumption: measuring") };
                    ui.label(text(label, 13.0, th.muted, Weight::Medium));
                });
            });
        });
    }

    fn next_turn(ui: &mut Ui, th: &Tokens, nav: &Navigation) {
        tonal_card(ui, th, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(76.0), Sense::hover());
                let color = if nav.off_route { th.warning } else { th.accent };
                let symbol = if nav.off_route { "alert" } else { turn_symbol(nav.current()) };
                icon(&ui.painter_at(rect), rect, symbol, true, color);
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(text(format_distance(nav.distance_to_next_m()), 34.0, th.text, Weight::Bold));
                    if nav.off_route {
                        ui.label(text(t("Off route, finding a new way"), 15.0, th.text, Weight::Medium));
                    } else if let Some(m) = nav.current() {
                        ui.label(text(m.text.as_str(), 15.0, th.text, Weight::Medium));
                    }
                    ui.add_space(2.0);
                    let minutes = format!("{:.0}", (nav.remaining_s() / 60.0).ceil());
                    let line = tf("{} to go, {} min", &[&format_distance(nav.remaining_m()), &minutes]);
                    ui.label(text(line, 13.0, th.muted, Weight::Medium));
                });
            });
        });
    }

    fn dash_card(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        card(ui, th, |ui| {
            caption(ui, th, t("Lights on the moped's dash"));
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                let lit = engine.flags.ringing();
                let painter = ui.painter_at(rect);
                painter.circle_filled(rect.center(), 15.0, if lit { th.accent } else { th.raised });
                let square = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
                icon(&painter, square, "call", lit, if lit { th.on_accent } else { th.faint });
                ui.add_space(6.0);
                for slot in 0..proto::APP_SLOTS {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                    let lit = engine.flags.app_on(slot);
                    ui.painter_at(rect).circle_filled(rect.center(), 11.0, if lit { th.accent } else { th.raised });
                }
            });
            if let Some(n) = engine.notifications.front() {
                ui.add_space(8.0);
                ui.label(text(format!("{}: {}", n.label, n.title), 14.5, th.text, Weight::SemiBold));
                ui.label(text(n.text.as_str(), 14.0, th.muted, Weight::Regular));
            }
        });
    }

    fn route_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(27), th.surface);
        if th.outlined {
            painter.rect_stroke(rect, CornerRadius::same(27), Stroke::new(1.5, th.border), StrokeKind::Inside);
        }
        let square = Rect::from_center_size(Pos2::new(rect.left() + 28.0, rect.center().y), Vec2::splat(24.0));
        icon(&painter, square, "search", false, th.muted);
        painter.text(
            Pos2::new(rect.left() + 52.0, rect.center().y),
            Align2::LEFT_CENTER,
            t("Where to?"),
            FontId::new(16.0, Weight::Medium.family()),
            th.muted,
        );
        if response.clicked() {
            engine.ask_search();
        }
        ui.add_space(10.0);
        if !engine.nav_status.is_empty() {
            ui.label(text(engine.nav_status.as_str(), 14.5, th.warning, Weight::Medium));
            ui.add_space(8.0);
        }
        let mut chosen = None;
        if !engine.search_results.is_empty() {
            card(ui, th, |ui| {
                for place in &engine.search_results {
                    let (name, detail) = split_place(&place.name);
                    if list_row(ui, th, "route", name, detail) {
                        chosen = Some(place.clone());
                    }
                }
            });
        }
        if engine.nav.is_some() {
            if let Some(nav) = &engine.nav {
                Self::next_turn(ui, th, nav);
            }
            Self::route_map(ui, engine, th);
            if let Some(nav) = &engine.nav {
                card(ui, th, |ui| {
                    caption(ui, th, t("Coming up"));
                    for m in nav.route.maneuvers.iter().skip(nav.next).take(5) {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                            icon(&ui.painter_at(rect), rect, turn_symbol(Some(m)), false, th.muted);
                            ui.add_space(4.0);
                            ui.label(text(m.text.as_str(), 14.5, th.text, Weight::Regular));
                        });
                        ui.add_space(3.0);
                    }
                });
            }
            if button(ui, th, t("Stop navigation"), Some("x"), Kind::Danger, 48.0) {
                engine.stop_navigation();
            }
            ui.add_space(10.0);
        } else if engine.search_results.is_empty() {
            if engine.settings.recents.is_empty() {
                card(ui, th, |ui| {
                    ui.label(text(t("Nowhere yet"), 16.0, th.text, Weight::Bold));
                    ui.add_space(2.0);
                    ui.label(text(t("Search for an address or a place. Places you've been will wait here for a quick start."), 14.0, th.muted, Weight::Regular));
                });
            } else {
                card(ui, th, |ui| {
                    caption(ui, th, t("Recent places"));
                    for place in engine.settings.recents.iter().take(MAX_RECENTS_SHOWN) {
                        let (name, detail) = split_place(&place.name);
                        if list_row(ui, th, "history", name, detail) {
                            chosen = Some(place.clone());
                        }
                    }
                });
            }
        }
        if let Some(place) = chosen {
            engine.navigate_to(&place);
        }
        ui.label(text(
            t("Planning a route sends your position and destination to free OpenStreetMap services. Routes and places: © OpenStreetMap contributors"),
            11.5,
            th.faint,
            Weight::Regular,
        ));
    }

    /// The route as a line with a position dot. There's no street map behind it yet.
    fn route_map(ui: &mut Ui, engine: &Engine, th: &Tokens) {
        let Some(nav) = &engine.nav else {
            return;
        };
        card(ui, th, |ui| {
            let size = Vec2::new(ui.available_width(), 180.0);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let painter = ui.painter_at(rect);
            let area = rect.shrink(12.0);
            let points = &nav.route.points;
            let (mut lat0, mut lat1, mut lon0, mut lon1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for p in points {
                lat0 = lat0.min(p.0);
                lat1 = lat1.max(p.0);
                lon0 = lon0.min(p.1);
                lon1 = lon1.max(p.1);
            }
            let aspect = ((lat0 + lat1) / 2.0).to_radians().cos();
            let dx = ((lon1 - lon0) * aspect).max(1e-6);
            let dy = (lat1 - lat0).max(1e-6);
            let scale = (f64::from(area.width()) / dx).min(f64::from(area.height()) / dy);
            let to_screen = |p: (f64, f64)| {
                area.center()
                    + Vec2::new(
                        ((p.1 - (lon0 + lon1) / 2.0) * aspect * scale) as f32,
                        (((lat0 + lat1) / 2.0 - p.0) * scale) as f32,
                    )
            };
            let pos = nav.route.along_m.partition_point(|a| *a < nav.along_m).min(points.len() - 1);
            let travelled: Vec<Pos2> = points[..=pos].iter().map(|p| to_screen(*p)).collect();
            let ahead: Vec<Pos2> = points[pos..].iter().map(|p| to_screen(*p)).collect();
            painter.line(travelled, Stroke::new(4.0, th.border));
            painter.line(ahead, Stroke::new(5.0, th.accent));
            if let Some(last) = points.last() {
                painter.circle_filled(to_screen(*last), 6.0, th.warning);
            }
            if let Some(at) = engine.gps {
                painter.circle(to_screen(at), 7.0, th.accent, Stroke::new(3.0, th.surface));
            }
        });
    }

    fn voice_screen(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 148.0), Sense::click());
        let painter = ui.painter_at(rect);
        let fill = if response.is_pointer_button_down_on() { th.accent.gamma_multiply(0.8) } else { th.accent };
        painter.rect_filled(rect, CornerRadius::same(CARD_RADIUS), fill);
        let square = Rect::from_center_size(rect.center() - Vec2::new(0.0, 36.0), Vec2::splat(34.0));
        icon(&painter, square, "voice", true, th.on_accent);
        painter.text(
            rect.center() + Vec2::new(0.0, 8.0),
            Align2::CENTER_CENTER,
            t("Tell me how things are"),
            FontId::new(21.0, Weight::Bold.family()),
            th.on_accent,
        );
        painter.text(
            rect.center() + Vec2::new(0.0, 38.0),
            Align2::CENTER_CENTER,
            t("Tap here, or press your earpiece's button"),
            FontId::new(13.5, Weight::Medium.family()),
            th.on_accent,
        );
        if response.clicked() {
            engine.speak_status();
        }
        ui.add_space(10.0);
        ui.columns(3, |c| {
            if button(&mut c[0], th, t("Repeat"), Some("refresh"), Kind::Tonal, 48.0) {
                engine.repeat_last();
            }
            if button(&mut c[1], th, t("Slower"), None, Kind::Tonal, 48.0) {
                engine.change_speech_rate(-SPEECH_RATE_STEP);
            }
            if button(&mut c[2], th, t("Faster"), None, Kind::Tonal, 48.0) {
                engine.change_speech_rate(SPEECH_RATE_STEP);
            }
        });
        ui.add_space(10.0);
        card(ui, th, |ui| {
            let on = engine.settings.voice_mode;
            if switch_row(ui, th, t("Speak to me"), on) {
                engine.set_voice_mode(!on);
            }
            ui.add_space(4.0);
            if list_row(ui, th, "hearing", t("Test your earpiece"), t("Pillion says a sentence so you know you'll hear it.")) {
                engine.test_voice();
            }
        });
        card(ui, th, |ui| {
            caption(ui, th, t("What to tell me about"));
            let s = &mut engine.settings;
            let mut changed = false;
            let choices = [
                (t("Battery and range"), &mut s.say_battery),
                (t("Consumption"), &mut s.say_consumption),
                (t("Directions"), &mut s.say_navigation),
                (t("Calls"), &mut s.say_calls),
            ];
            for (name, value) in choices {
                if switch_row(ui, th, name, *value) {
                    *value = !*value;
                    changed = true;
                }
            }
            ui.add_space(4.0);
            ui.label(text(t("Warnings are always spoken. During a call, only directions and warnings are."), 13.0, th.muted, Weight::Regular));
            if changed {
                engine.save_settings();
            }
        });
        card(ui, th, |ui| {
            caption(ui, th, t("Last said"));
            if engine.spoken.is_empty() {
                ui.label(text(t("Nothing yet."), 14.0, th.muted, Weight::Regular));
            }
            for said in engine.spoken.iter().take(5) {
                ui.label(text(said.as_str(), 14.5, th.text, Weight::Regular));
                ui.add_space(3.0);
            }
        });
    }

    fn moped_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        Self::connection_card(ui, engine, th, &mut self.confirm_forget);
        let Some(live) = engine.live.clone() else {
            return;
        };
        for (i, pack) in live.packs.iter().enumerate().filter(|(_, p)| p.fitted) {
            card(ui, th, |ui| {
                ui.label(text(tf("Battery {}: {}%", &[&(i + 1), &pack.percent]), 18.0, th.text, Weight::Bold));
                ui.add_space(8.0);
                stat_row(
                    ui,
                    th,
                    &[
                        (format!("{:.1} V", pack.voltage), t("voltage")),
                        (format!("{:.1} A", (pack.current_ma as f32 / 1000.0).abs()), t("current")),
                        (format!("{:.0} °C", pack.temp_max), t("temperature")),
                    ],
                );
                ui.add_space(8.0);
                stat_row(
                    ui,
                    th,
                    &[
                        (format!("{:.1} Ah", pack.remaining_ah), t("stored")),
                        (format!("{}%", pack.health), t("health")),
                        (
                            tf("{} h", &[&format!("{:.1}", Engine::charge_hours(pack.percent, pack.health))]),
                            t("to charge"),
                        ),
                    ],
                );
            });
        }
        card(ui, th, |ui| {
            stat_row(
                ui,
                th,
                &[
                    (format_distance(f64::from(live.odometer_m)), t("odometer")),
                    (format!("{:.1} V", live.aux_voltage), t("12 V battery")),
                    (format!("{:.0} °C", live.motor_temp), t("motor")),
                ],
            );
            ui.add_space(10.0);
            if let Some(full) = engine.full_range_km() {
                ui.label(text(tf("About {} km on a full charge", &[&format!("{full:.0}")]), 13.5, th.muted, Weight::Medium));
                ui.add_space(4.0);
            }
            ui.label(text(tf("Model number {}", &[&live.model_number]), 12.5, th.faint, Weight::Regular));
            ui.add_space(8.0);
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
        });
        if engine.demo_active && button(ui, th, t("End the demo ride"), Some("x"), Kind::Tonal, 48.0) {
            engine.stop_demo();
        }
    }

    fn connection_card(ui: &mut Ui, engine: &mut Engine, th: &Tokens, confirm_forget: &mut bool) {
        if engine.demo_active {
            card(ui, th, |ui| {
                ui.label(text(t("This is a demo ride"), 17.0, th.warning, Weight::Bold));
                ui.label(text(t("These figures are invented to show the app. They aren't your moped's."), 14.0, th.muted, Weight::Regular));
            });
            return;
        }
        card(ui, th, |ui| match engine.link {
            Link::NeedsPairing | Link::Refused => {
                let title = if engine.link == Link::Refused { t("The moped refused this code") } else { t("Pair your moped") };
                ui.label(text(title, 18.0, th.text, Weight::Bold));
                ui.add_space(4.0);
                ui.label(text(t("Type the 24 characters from its QR sticker. A QR reader app shows them."), 14.0, th.muted, Weight::Regular));
                ui.add_space(10.0);
                if button(ui, th, t("Enter pairing code"), None, Kind::Primary, 48.0) {
                    engine.ask_pairing();
                }
                ui.add_space(6.0);
                if button(ui, th, t("Try a demo ride"), Some("play"), Kind::Tonal, 44.0) {
                    engine.start_demo();
                }
            }
            Link::Idle => {
                ui.label(text(t("Moped paired"), 18.0, th.text, Weight::Bold));
                ui.add_space(10.0);
                if button(ui, th, t("Connect"), Some("bluetooth"), Kind::Primary, 48.0) {
                    engine.connect();
                }
                ui.add_space(6.0);
                let label = if *confirm_forget { t("Tap again to forget this moped") } else { t("Forget this moped") };
                let kind = if *confirm_forget { Kind::Danger } else { Kind::Tonal };
                if button(ui, th, label, None, kind, 44.0) {
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
                ui.label(text(line, 15.5, th.text, Weight::SemiBold));
                let devices = engine.found_devices.clone();
                for (name, address) in devices {
                    ui.add_space(6.0);
                    if button(ui, th, &tf("Use {}", &[&name]), Some("moped"), Kind::Tonal, 44.0) {
                        engine.choose_device(&address);
                    }
                }
                ui.add_space(8.0);
                if button(ui, th, t("Cancel"), None, Kind::Tonal, 44.0) {
                    engine.disconnect();
                }
            }
            Link::Online => {
                ui.label(text(t("Connected to the moped"), 15.5, th.success, Weight::SemiBold));
                ui.add_space(8.0);
                if button(ui, th, t("Disconnect"), None, Kind::Tonal, 44.0) {
                    engine.disconnect();
                }
            }
        });
    }

    fn alerts_screen(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        if !engine.notification_access
            && !engine.demo_active
            && banner(
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
        card(ui, th, |ui| {
            caption(ui, th, t("Read notifications aloud"));
            let levels = [NotifLevel::Off, NotifLevel::AppAndSender, NotifLevel::Full];
            let selected = levels.iter().position(|l| *l == engine.settings.notif_level).unwrap_or(2);
            if let Some(i) = segmented(ui, th, &[t("Off"), t("Who"), t("Full text")], selected) {
                engine.settings.notif_level = levels[i];
                engine.save_settings();
            }
            ui.add_space(6.0);
            ui.label(text(t("Several messages in a row from one app are told as a single count."), 13.0, th.muted, Weight::Regular));
        });
        card(ui, th, |ui| {
            let calls = engine.settings.dash_calls;
            if switch_row(ui, th, t("Show calls on the dash"), calls) {
                engine.settings.dash_calls = !calls;
                engine.save_settings();
            }
            ui.add_space(6.0);
            caption(ui, th, t("Apps"));
            ui.label(text(t("The dash can only light up, it can't show message text. Each app below gets a light, up to six."), 13.0, th.muted, Weight::Regular));
            ui.add_space(6.0);
            if engine.apps.is_empty() {
                ui.label(text(t("Apps appear here after their first notification."), 14.0, th.muted, Weight::Regular));
            }
            let apps = engine.apps.clone();
            for app in apps {
                let muted = engine.settings.muted_apps.contains(&app.package);
                let slot = engine.settings.dash_slots.iter().position(|p| *p == app.package);
                let light = slot.map_or(t("no light").to_string(), |s| tf("light {}", &[&(s + 1)]));
                if switch_row(ui, th, &format!("{} ({light})", app.label), !muted) {
                    engine.toggle_app_muted(&app.package);
                }
            }
        });
        card(ui, th, |ui| {
            caption(ui, th, t("Recent"));
            if engine.notifications.is_empty() {
                ui.label(text(t("Nothing yet."), 14.0, th.muted, Weight::Regular));
            }
            for n in engine.notifications.iter().take(8) {
                ui.label(text(format!("{}: {}", n.label, n.title), 14.5, th.text, Weight::SemiBold));
                ui.label(text(n.text.as_str(), 14.0, th.muted, Weight::Regular));
                ui.add_space(4.0);
            }
        });
    }

    fn settings_screen(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        card(ui, th, |ui| {
            caption(ui, th, t("Appearance"));
            let modes = [Appearance::System, Appearance::Light, Appearance::Dark];
            let selected = modes.iter().position(|m| *m == engine.settings.appearance).unwrap_or(0);
            if let Some(i) = segmented(ui, th, &[t("System"), t("Light"), t("Dark")], selected) {
                engine.settings.appearance = modes[i];
                engine.save_settings();
            }
            ui.add_space(8.0);
            let high = engine.settings.high_contrast;
            if switch_row(ui, th, t("High contrast"), high) {
                engine.settings.high_contrast = !high;
                engine.save_settings();
            }
        });
        card(ui, th, |ui| {
            caption(ui, th, t("Language"));
            let choices = [None, Some(Lang::En), Some(Lang::It), Some(Lang::Ru)];
            let selected = choices.iter().position(|c| *c == engine.settings.language).unwrap_or(0);
            // Each language is written in itself, so it can be found from any of them.
            if let Some(i) = segmented(ui, th, &[t("System"), "English", "Italiano", "Русский"], selected) {
                engine.set_language(choices[i]);
            }
            ui.add_space(6.0);
            ui.label(text(t("Spoken directions follow this too."), 13.0, th.muted, Weight::Regular));
        });
        card(ui, th, |ui| {
            ui.label(text("Pillion", 20.0, th.text, Weight::Bold));
            ui.label(text(tf("Version {}", &[&env!("CARGO_PKG_VERSION")]), 13.5, th.muted, Weight::Medium));
            ui.add_space(6.0);
            ui.label(text(t("The seat behind the rider. A companion for your electric moped, part of the Quezka suite."), 14.0, th.muted, Weight::Regular));
            ui.add_space(6.0);
            ui.label(text(t("Manrope font (SIL Open Font License), Material Symbols (Apache 2.0), routes and places © OpenStreetMap contributors."), 12.0, th.faint, Weight::Regular));
        });
    }

    fn nav_bar(&mut self, ui: &mut Ui, th: &Tokens) {
        ui.add_space(6.0);
        ui.columns(5, |c| {
            for (column, (screen, label, glyph)) in c.iter_mut().zip([
                (Screen::Ride, t("Ride"), "moped"),
                (Screen::Route, t("Route"), "route"),
                (Screen::Voice, t("Voice"), "voice"),
                (Screen::Moped, t("Moped"), "battery"),
                (Screen::Alerts, t("Alerts"), "bell"),
            ]) {
                let selected = self.screen == screen;
                let size = Vec2::new(column.available_width(), 62.0);
                let (rect, response) = column.allocate_exact_size(size, Sense::click());
                let painter = column.painter_at(rect);
                let pill = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 18.0), Vec2::new(56.0, 32.0));
                if selected {
                    painter.rect_filled(pill, CornerRadius::same(16), th.accent_soft);
                }
                let square = Rect::from_center_size(pill.center(), Vec2::splat(24.0));
                icon(&painter, square, glyph, selected, if selected { th.text } else { th.muted });
                let weight = if selected { Weight::Bold } else { Weight::Medium };
                painter.text(
                    Pos2::new(rect.center().x, rect.top() + 48.0),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::new(12.0, weight.family()),
                    if selected { th.text } else { th.muted },
                );
                if response.clicked() {
                    self.screen = screen;
                    self.last_tab = screen;
                    self.confirm_forget = false;
                }
            }
        });
    }

    /// Desktop preview only. On a phone Android's own dialog asks instead, because the on-screen
    /// keyboard doesn't reach this kind of window.
    fn typed_prompt(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let Some((key, title)) = self.typed_key.clone() else {
            return;
        };
        card(ui, th, |ui| {
            ui.label(text(title, 16.0, th.text, Weight::Bold));
            ui.add_space(6.0);
            ui.text_edit_singleline(&mut self.typed);
            ui.add_space(6.0);
            ui.columns(2, |c| {
                if button(&mut c[0], th, t("OK"), None, Kind::Primary, 40.0) {
                    engine.on_text(&key, &self.typed);
                    self.typed.clear();
                    self.typed_key = None;
                }
                if button(&mut c[1], th, t("Cancel"), None, Kind::Tonal, 40.0) {
                    self.typed.clear();
                    self.typed_key = None;
                }
            });
        });
    }

    pub fn ask_typed(&mut self, key: String, title: String) {
        self.typed_key = Some((key, title));
    }

    fn message_prompt(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let Some((title, detail)) = engine.text_prompt.clone() else {
            return;
        };
        if banner(ui, th, "alert", &title, &detail, Some(t("OK"))) {
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
}

fn is_dark(ctx: &egui::Context, engine: &Engine) -> bool {
    match engine.settings.appearance {
        Appearance::Light => false,
        Appearance::Dark => true,
        Appearance::System => engine.system_dark.unwrap_or_else(|| {
            ctx.input(|i| i.raw.system_theme).is_none_or(|theme| theme == egui::Theme::Dark)
        }),
    }
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
            .frame(Frame::new().fill(th.bg).inner_margin(Margin::symmetric(14, 6)))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(bar_top / scale);
                self.top_bar(ui, &engine, &th);
            });
        CentralPanel::default()
            .frame(Frame::new().fill(th.bg).inner_margin(Margin::symmetric(14, 4)))
            .show(ui, |ui| {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    self.typed_prompt(ui, &mut engine, &th);
                    Self::message_prompt(ui, &mut engine, &th);
                    Self::problems(ui, &mut engine, &th);
                    match self.screen {
                        Screen::Ride => self.ride_screen(ui, &mut engine, &th),
                        Screen::Route => self.route_screen(ui, &mut engine, &th),
                        Screen::Voice => Self::voice_screen(ui, &mut engine, &th),
                        Screen::Moped => self.moped_screen(ui, &mut engine, &th),
                        Screen::Alerts => Self::alerts_screen(ui, &mut engine, &th),
                        Screen::Settings => Self::settings_screen(ui, &mut engine, &th),
                    }
                });
            });
    }
}
