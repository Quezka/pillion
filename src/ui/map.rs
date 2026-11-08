//! The Route tab: search, saved places, recent places, and the street map, which you can drag and zoom, with the
//! area the battery reaches, saved places and the speed limit drawn on it.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, TextureOptions, Ui, Vec2,
};

use super::m3::{self, text, Kind};
use super::theme::{icon, Tokens, Weight};
use super::{draw_turn, format_distance, CompanionApp, MAX_RECENTS_SHOWN};
use crate::domain::places::{BookmarkKind, Place};
use crate::engine::Engine;
use crate::locale::{t, tf};
use crate::nav::{self, LatLon};

const MAP_HEIGHT_FREE: f32 = 360.0;
const MAP_HEIGHT_NAV: f32 = 420.0;
const TILE_POINTS: f32 = 256.0;
const CLOSE_ZOOM: f32 = 17.0;
const FAR_ZOOM: f32 = 16.0;
/// Above this speed the map zooms out a little, to see further ahead.
const FAR_ZOOM_KMH: f64 = 28.0;
const ZOOM_RANGE: (f32, f32) = (4.0, 18.5);
const TILES_DECODED_PER_FRAME: usize = 3;
const TILES_KEPT: usize = 80;
const HEADING_EASE: f32 = 0.12;
/// How much a turn of the mouse wheel zooms, on the desktop preview.
const WHEEL_ZOOM: f32 = 0.004;
/// Where the map starts when there's no GPS yet: Bologna, near the owner.
const NO_GPS_START: LatLon = (44.4949, 11.3426);
/// How many zoom levels lower the map picture under a glass button is: each level halves the detail.
const GLASS_BLUR_LEVELS: u8 = 3;

/// The street map: its pictures, and how it's looking.
#[derive(Default)]
pub(super) struct MapView {
    tiles: std::collections::HashMap<crate::engine::TileKey, egui::TextureHandle>,
    dark: bool,
    whole_route: bool,
    heading_deg: f32,
    /// Where the rider dragged the map to, and its zoom. `None` follows the rider.
    free: Option<(LatLon, f32)>,
    /// A spot picked with a long press, waiting for "Go here" or "Save".
    marked: Option<LatLon>,
}

/// A position in map pictures at a (whole) zoom level. North is up.
fn map_position(at: LatLon, z: u8) -> Vec2 {
    let n = f64::from(1u32 << z);
    let x = (at.1 + 180.0) / 360.0 * n;
    let lat = at.0.to_radians();
    let y = (1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n;
    Vec2::new(x as f32, y as f32)
}

/// The other way round.
fn map_place(p: Vec2, z: u8) -> LatLon {
    let n = f64::from(1u32 << z);
    let lon = f64::from(p.x) / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * f64::from(p.y) / n)).sinh().atan().to_degrees();
    (lat, lon)
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

/// How the map is looking: zoom (with a fraction), the point at the anchor, where on screen that is, and how far it's
/// turned (radians; 0 is north up).
struct View {
    zoom: f32,
    centre: LatLon,
    anchor: Pos2,
    turn: f32,
}

impl View {
    fn level(&self) -> u8 {
        self.zoom.floor().clamp(ZOOM_RANGE.0, 18.0) as u8
    }

    fn scale(&self) -> f32 {
        TILE_POINTS * 2f32.powf(self.zoom - f32::from(self.level()))
    }

    fn to_screen(&self, at: LatLon) -> Pos2 {
        let z = self.level();
        let d = (map_position(at, z) - map_position(self.centre, z)) * self.scale();
        let (sin, cos) = (-self.turn).sin_cos();
        self.anchor + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
    }

    fn to_place(&self, pos: Pos2) -> LatLon {
        let z = self.level();
        let d = pos - self.anchor;
        let (sin, cos) = self.turn.sin_cos();
        let unturned = Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos) / self.scale();
        map_place(map_position(self.centre, z) + unturned, z)
    }
}

/// A zoom that shows `radius_m` around a point across most of `width` points.
fn zoom_for_radius(at: LatLon, radius_m: f64, width: f32) -> f32 {
    let metres_per_point_z0 = 156_543.03 * at.0.to_radians().cos() / f64::from(TILE_POINTS / 256.0);
    let wanted = radius_m * 2.0 / f64::from(width * 0.85);
    ((metres_per_point_z0 / wanted).log2() as f32).clamp(ZOOM_RANGE.0, CLOSE_ZOOM)
}

impl CompanionApp {
    pub(super) fn route_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        Self::search_bar(ui, engine, th);
        ui.add_space(12.0);
        if !engine.nav_status.is_empty() {
            ui.label(text(engine.nav_status.as_str(), 14.5, th.warning, Weight::Medium));
            ui.add_space(8.0);
        }
        let mut chosen: Option<Place> = None;
        let mut save: Option<Place> = None;
        if !engine.search_results.is_empty() {
            m3::card(ui, th, |ui| {
                m3::label(ui, th, t("Places found"));
                for place in &engine.search_results {
                    let saved = engine.settings.bookmarks.at(place.at).is_some();
                    let detail = place_detail(engine.gps, place.at, &place.detail);
                    let (go, star) = place_row(ui, th, "pin", &place.name, &detail, Some(saved));
                    if go {
                        chosen = Some(place.clone());
                    }
                    if star && !saved {
                        save = Some(place.clone());
                    }
                }
                if m3::button(ui, th, t("Clear the results"), Some("x"), Kind::Text, 40.0) {
                    engine.search_results.clear();
                }
            });
        }
        if let Some(nav) = &engine.nav {
            Self::next_turn(ui, th, nav, engine.arrival_time());
        }
        let height = if engine.nav.is_some() { MAP_HEIGHT_NAV } else { MAP_HEIGHT_FREE };
        self.map_view(ui, engine, th, height);
        if let Some(at) = self.map.marked {
            m3::card(ui, th, |ui| {
                m3::label(ui, th, t("The spot you picked"));
                if let Some(from) = engine.gps {
                    m3::body(ui, th, &tf("{} from you", &[&format_distance(nav::distance_m(from, at))]));
                }
                ui.add_space(6.0);
                ui.columns(2, |cols| {
                    if m3::button(&mut cols[0], th, t("Go here"), Some("route"), Kind::Filled, 48.0) {
                        chosen = Some(Place::new(t("The spot on the map"), at));
                        self.map.marked = None;
                    }
                    if m3::button(&mut cols[1], th, t("Save"), Some("star"), Kind::Tonal, 48.0) {
                        engine.ask_bookmark_name(at);
                        self.map.marked = None;
                    }
                });
            });
        }
        if let Some(nav) = &engine.nav {
            let upcoming: Vec<_> = nav.route.maneuvers.iter().skip(nav.next).take(5).cloned().collect();
            m3::card(ui, th, |ui| {
                m3::label(ui, th, t("Coming up"));
                for m in &upcoming {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                        draw_turn(&ui.painter_at(rect), rect, Some(m), false, th.accent);
                        ui.add_space(8.0);
                        ui.label(text(m.text.as_str(), 15.0, th.text, Weight::Regular));
                    });
                    ui.add_space(6.0);
                }
            });
            if m3::button(ui, th, t("Stop navigation"), Some("x"), Kind::Outlined, 52.0) {
                engine.stop_navigation();
            }
            ui.add_space(12.0);
        }
        if engine.search_results.is_empty() {
            Self::saved_places(ui, engine, th, &mut chosen);
            if engine.nav.is_none() && !engine.settings.recents.is_empty() {
                m3::card(ui, th, |ui| {
                    m3::label(ui, th, t("Recent places"));
                    for place in engine.settings.recents.iter().take(MAX_RECENTS_SHOWN) {
                        let saved = engine.settings.bookmarks.at(place.at).is_some();
                        let (name, detail) = split_place(&place.name);
                        let detail = place_detail(engine.gps, place.at, detail);
                        let (go, star) = place_row(ui, th, "history", name, &detail, Some(saved));
                        if go {
                            chosen = Some(place.clone());
                        }
                        if star && !saved {
                            save = Some(place.clone());
                        }
                    }
                });
            }
        }
        if let Some(place) = save {
            engine.save_bookmark(BookmarkKind::Other, &place);
        }
        if let Some(place) = chosen {
            self.map.free = None;
            engine.navigate_to(&place);
        }
        m3::small(ui, th, t("Planning a route sends your position and destination to free OpenStreetMap services. Routes and places: © OpenStreetMap contributors"));
    }

    fn search_bar(ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 58.0), Sense::click());
        let painter = ui.painter_at(rect);
        m3::glass_panel(&painter, rect, 29, th);
        if response.is_pointer_button_down_on() {
            painter.rect_filled(rect, CornerRadius::same(29), th.text.gamma_multiply(0.06));
        }
        let square = Rect::from_center_size(Pos2::new(rect.left() + 30.0, rect.center().y), Vec2::splat(26.0));
        icon(&painter, square, "search", false, th.accent);
        painter.text(
            Pos2::new(rect.left() + 58.0, rect.center().y),
            Align2::LEFT_CENTER,
            t("Where to?"),
            FontId::new(17.0, Weight::SemiBold.family()),
            th.muted,
        );
        if response.clicked() {
            engine.tick_haptic();
            engine.ask_search();
        }
    }

    /// Home and Work first (or a way to set them), then the other saved places.
    fn saved_places(ui: &mut Ui, engine: &mut Engine, th: &Tokens, chosen: &mut Option<Place>) {
        let mut remove = None;
        let mut set_here: Option<BookmarkKind> = None;
        m3::card(ui, th, |ui| {
            m3::label(ui, th, t("Saved places"));
            for kind in [BookmarkKind::Home, BookmarkKind::Work] {
                let glyph = if kind == BookmarkKind::Home { "home" } else { "work" };
                let label = if kind == BookmarkKind::Home { t("Home") } else { t("Work") };
                match engine.settings.bookmarks.items.iter().position(|b| b.kind == kind) {
                    Some(i) => {
                        let place = engine.settings.bookmarks.items[i].place.clone();
                        let detail = place_detail(engine.gps, place.at, &place.name);
                        let (go, star) = place_row(ui, th, glyph, label, &detail, Some(true));
                        if go {
                            *chosen = Some(Place::new(label, place.at));
                        }
                        if star {
                            remove = Some(i);
                        }
                    }
                    None => {
                        let hint = if engine.gps.is_some() { t("Tap to save where you are now") } else { t("Waiting for GPS...") };
                        let (go, _) = place_row(ui, th, glyph, &tf("Set {}", &[&label]), hint, None);
                        if go && engine.gps.is_some() {
                            set_here = Some(kind);
                        }
                    }
                }
            }
            let others: Vec<(usize, Place)> = engine
                .settings
                .bookmarks
                .items
                .iter()
                .enumerate()
                .filter(|(_, b)| b.kind == BookmarkKind::Other)
                .map(|(i, b)| (i, b.place.clone()))
                .collect();
            for (i, place) in others {
                let detail = place_detail(engine.gps, place.at, &place.detail);
                let (go, star) = place_row(ui, th, "star", &place.name, &detail, Some(true));
                if go {
                    *chosen = Some(place);
                }
                if star {
                    remove = Some(i);
                }
            }
            m3::small(ui, th, t("Star a place to save it, or press and hold on the map. Tap a filled star to forget it."));
        });
        if let Some(i) = remove {
            engine.remove_bookmark(i);
        }
        if let (Some(kind), Some(at)) = (set_here, engine.gps) {
            let name = if kind == BookmarkKind::Home { t("Home") } else { t("Work") };
            engine.save_bookmark(kind, &Place::new(name, at));
        }
    }

    /// The street map. It follows the rider (turning with them while navigating) until it's dragged; then the
    /// target button brings it back.
    pub(super) fn map_view(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens, height: f32) {
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
        engine.want_reach();

        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let round = CornerRadius::same(m3::CARD_RADIUS);
        painter.rect_filled(rect, round, th.raised);
        let nav = engine.nav.clone();
        let rider = engine.gps;

        // Where to look.
        let view = if let Some((centre, zoom)) = self.map.free {
            View { zoom, centre, anchor: rect.center(), turn: 0.0 }
        } else if let (Some(nav), true) = (&nav, self.map.whole_route) {
            let (mut lat0, mut lat1, mut lon0, mut lon1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for p in &nav.route.points {
                lat0 = lat0.min(p.0);
                lat1 = lat1.max(p.0);
                lon0 = lon0.min(p.1);
                lon1 = lon1.max(p.1);
            }
            let mut z = CLOSE_ZOOM as u8;
            while z > 8 {
                let span = map_position((lat0, lon1), z) - map_position((lat1, lon0), z);
                if span.x * TILE_POINTS < rect.width() * 0.85 && span.y * TILE_POINTS < rect.height() * 0.85 {
                    break;
                }
                z -= 1;
            }
            View { zoom: f32::from(z), centre: ((lat0 + lat1) / 2.0, (lon0 + lon1) / 2.0), anchor: rect.center(), turn: 0.0 }
        } else if let Some(nav) = &nav {
            let zoom = if engine.speed_kmh.is_some_and(|s| s > FAR_ZOOM_KMH) { FAR_ZOOM } else { CLOSE_ZOOM };
            let at = rider.unwrap_or(nav.route.points[nav.index_along()]);
            let wanted = engine.heading_deg.unwrap_or(0.0) as f32;
            let gap = (wanted - self.map.heading_deg + 540.0).rem_euclid(360.0) - 180.0;
            if gap.abs() > 0.5 {
                self.map.heading_deg += gap * HEADING_EASE;
                ctx.request_repaint();
            }
            View { zoom, centre: at, anchor: rect.center() + Vec2::new(0.0, rect.height() * 0.24), turn: self.map.heading_deg.to_radians() }
        } else {
            // Not navigating: the rider in the middle, zoomed to show how far the battery reaches.
            let at = rider.unwrap_or(NO_GPS_START);
            let zoom = engine.range_km().map_or(14.0, |km| zoom_for_radius(at, km * 1000.0, rect.width()));
            View { zoom, centre: at, anchor: rect.center(), turn: 0.0 }
        };

        // Moving the map: drag, pinch, the mouse wheel, double tap.
        let pinch = if response.hovered() { ui.input(|i| i.zoom_delta()) } else { 1.0 };
        let wheel = if response.hovered() { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        let mut zoom_by = pinch.log2() + wheel * WHEEL_ZOOM;
        if response.double_clicked() {
            zoom_by += 1.0;
        }
        if response.dragged() || zoom_by.abs() > 1e-4 {
            let (mut centre, mut zoom) = self.map.free.unwrap_or((view.to_place(rect.center()), view.zoom));
            if response.dragged() {
                let free = View { zoom, centre, anchor: rect.center(), turn: 0.0 };
                centre = free.to_place(rect.center() - response.drag_delta());
            }
            zoom = (zoom + zoom_by).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
            self.map.free = Some((centre, zoom));
            self.map.marked = None;
            ctx.request_repaint();
        }
        if response.long_touched() || response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.map.marked = Some(view.to_place(pos));
                engine.tick_haptic();
            }
        } else if response.clicked() {
            self.map.marked = None;
        }

        // The map pictures that fall inside.
        let z = view.level();
        let scale = view.scale();
        let centre_tile = map_position(view.centre, z);
        let reach = (rect.size().length() / scale).ceil() as i32 + 1;
        let count = 1i64 << z;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (tx, ty) = (centre_tile.x.floor() as i64 + i64::from(dx), centre_tile.y.floor() as i64 + i64::from(dy));
                if tx < 0 || ty < 0 || tx >= count || ty >= count {
                    continue;
                }
                let corner = |ox: f32, oy: f32| view.to_screen(map_place(Vec2::new(tx as f32 + ox, ty as f32 + oy), z));
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

        // How far the battery reaches: the area on roads when the map service gave it, a circle otherwise.
        let reach_ink = th.success;
        if !engine.reach.is_empty() {
            for ring in &engine.reach {
                let line: Vec<Pos2> = ring.iter().map(|p| view.to_screen(*p)).collect();
                painter.add(egui::Shape::closed_line(line.clone(), Stroke::new(12.0, reach_ink.gamma_multiply(0.18))));
                painter.add(egui::Shape::closed_line(line, Stroke::new(3.0, reach_ink)));
            }
        } else if let Some(ring) = engine.range_ring() {
            let line: Vec<Pos2> = ring.iter().map(|p| view.to_screen(*p)).collect();
            painter.add(egui::Shape::convex_polygon(line.clone(), reach_ink.gamma_multiply(0.08), Stroke::NONE));
            painter.add(egui::Shape::closed_line(line, Stroke::new(3.0, reach_ink)));
        }

        // The route: what's been ridden fades, and what's ahead stands out.
        if let Some(nav) = &nav {
            let points = &nav.route.points;
            let along = nav.index_along();
            let line = |part: &[LatLon]| -> Vec<Pos2> { part.iter().map(|p| view.to_screen(*p)).collect() };
            painter.line(line(&points[..=along]), Stroke::new(5.0, th.faint));
            painter.line(line(&points[along..]), Stroke::new(10.0, th.surface));
            painter.line(line(&points[along..]), Stroke::new(6.5, th.accent));
            if let Some(m) = nav.current() {
                let spot = points[m.begin.min(points.len() - 1)];
                painter.circle(view.to_screen(spot), 6.0, th.surface, Stroke::new(3.0, th.accent));
            }
            if let Some(last) = points.last() {
                painter.circle(view.to_screen(*last), 9.0, th.warning, Stroke::new(3.0, th.surface));
            }
        }

        // Saved places.
        for bookmark in &engine.settings.bookmarks.items {
            let spot = view.to_screen(bookmark.place.at);
            if !rect.expand(-6.0).contains(spot) {
                continue;
            }
            let glyph = match bookmark.kind {
                BookmarkKind::Home => "home",
                BookmarkKind::Work => "work",
                BookmarkKind::Other => "star",
            };
            painter.circle(spot, 14.0, th.surface, Stroke::new(2.0, th.warning));
            icon(&painter, Rect::from_center_size(spot, Vec2::splat(18.0)), glyph, true, th.warning);
        }

        // A picked spot.
        if let Some(at) = self.map.marked {
            let spot = view.to_screen(at);
            icon(&painter, Rect::from_center_size(spot - Vec2::new(0.0, 16.0), Vec2::splat(40.0)), "pin", true, th.danger);
        }

        // The rider, as an arrow pointing the way they're going.
        if let Some(at) = rider {
            let spot = view.to_screen(at);
            let way = if view.turn == 0.0 { engine.heading_deg.unwrap_or(0.0) as f32 } else { 0.0 };
            let (s, c) = way.to_radians().sin_cos();
            let tip = |x: f32, y: f32| spot + Vec2::new(x * c - y * s, x * s + y * c);
            let arrow = vec![tip(0.0, -15.0), tip(11.0, 11.0), tip(0.0, 5.0), tip(-11.0, 11.0)];
            painter.circle_filled(spot, 22.0, th.accent.gamma_multiply(0.22));
            painter.add(egui::Shape::convex_polygon(vec![arrow[0], arrow[1], arrow[2]], th.accent, Stroke::NONE));
            painter.add(egui::Shape::convex_polygon(vec![arrow[0], arrow[2], arrow[3]], th.accent, Stroke::NONE));
            painter.add(egui::Shape::closed_line(arrow, Stroke::new(2.0, th.surface)));
        }

        // The speed limit, as a road sign, in the top left corner.
        if let Some(limit) = engine.limits.limit() {
            speed_sign(&painter, rect.left_top() + Vec2::new(36.0, 36.0), 26.0, limit);
        }

        // Buttons down the right side: follow me, whole route, zoom in and out.
        let mut buttons: Vec<(&'static str, &'static str)> = Vec::new();
        if self.map.free.is_some() || rider.is_none() {
            buttons.push(("target", "follow"));
        }
        if nav.is_some() {
            buttons.push((if self.map.whole_route { "near" } else { "route" }, "whole"));
        }
        buttons.push(("plus", "in"));
        buttons.push(("minus", "out"));
        for (i, (glyph, what)) in buttons.into_iter().enumerate() {
            let centre = rect.right_top() + Vec2::new(-34.0, 34.0 + i as f32 * 56.0);
            let area = Rect::from_center_size(centre, Vec2::splat(48.0));
            let hit = ui.interact(area, ui.scope_id().with(("map-button", what)), Sense::click());
            self.glass_button(&painter, centre, 24.0, &view, engine, th);
            icon(&painter, Rect::from_center_size(centre, Vec2::splat(24.0)), glyph, false, th.accent);
            if hit.clicked() {
                engine.tick_haptic();
                match what {
                    "follow" => self.map.free = None,
                    "whole" => {
                        self.map.whole_route = !self.map.whole_route;
                        self.map.free = None;
                    }
                    _ => {
                        let step = if what == "in" { 1.0 } else { -1.0 };
                        let (centre, zoom) = self.map.free.unwrap_or((view.to_place(rect.center()), view.zoom));
                        self.map.free = Some((centre, (zoom + step).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1)));
                    }
                }
            }
        }

        let credit = rect.right_bottom() + Vec2::new(-10.0, -8.0);
        let font = FontId::new(10.5, Weight::Medium.family());
        let galley = painter.layout_no_wrap("© OpenStreetMap".to_string(), font, th.muted);
        let back = Rect::from_min_size(credit - galley.size() - Vec2::splat(3.0), galley.size() + Vec2::splat(6.0));
        painter.rect_filled(back, CornerRadius::same(5), th.surface.gamma_multiply(0.85));
        painter.galley(back.min + Vec2::splat(3.0), galley, th.muted);
        painter.rect_stroke(rect, round, Stroke::new(1.0, th.glass_edge), egui::StrokeKind::Inside);
        ui.add_space(12.0);
    }
}

impl CompanionApp {
    /// A round glass button over the map. What shows through is the real map under it, blurred: the same place
    /// from a map picture three zoom levels lower, stretched up, so its detail melts into colour. Then a light tint,
    /// a bright rim along the top and a soft shadow.
    fn glass_button(&self, painter: &egui::Painter, centre: Pos2, radius: f32, view: &View, engine: &mut Engine, th: &Tokens) {
        if !th.glass {
            painter.circle_filled(centre, radius, th.surface);
            if th.outlined {
                painter.circle_stroke(centre, radius - 0.75, Stroke::new(1.5, th.border));
            }
            return;
        }
        painter.circle_filled(centre + Vec2::new(0.0, 2.0), radius + 1.0, Color32::from_black_alpha(46));
        let low = view.level().saturating_sub(GLASS_BLUR_LEVELS).max(ZOOM_RANGE.0 as u8);
        let under = map_position(view.to_place(centre), low);
        let key = (low, under.x.floor().max(0.0) as u32, under.y.floor().max(0.0) as u32);
        engine.want_tile(key);
        if let Some(texture) = self.map.tiles.get(&key) {
            let corner = Vec2::new(key.1 as f32, key.2 as f32);
            let uv = |q: Pos2| {
                let m = map_position(view.to_place(q), low) - corner;
                Pos2::new(m.x.clamp(0.0, 1.0), m.y.clamp(0.0, 1.0))
            };
            let mut mesh = egui::Mesh::with_texture(texture.id());
            mesh.vertices.push(egui::epaint::Vertex { pos: centre, uv: uv(centre), color: Color32::WHITE });
            const SIDES: u32 = 40;
            for i in 0..=SIDES {
                let a = i as f32 / SIDES as f32 * std::f32::consts::TAU;
                let q = centre + Vec2::new(a.cos(), a.sin()) * radius;
                mesh.vertices.push(egui::epaint::Vertex { pos: q, uv: uv(q), color: Color32::WHITE });
                if i > 0 {
                    mesh.add_triangle(0, i, i + 1);
                }
            }
            painter.add(egui::Shape::mesh(mesh));
            painter.circle_filled(centre, radius, th.surface.gamma_multiply(if th.dark { 0.42 } else { 0.48 }));
        } else {
            painter.circle_filled(centre, radius, th.surface.gamma_multiply(0.9));
        }
        // The light along the top edge, fading towards the sides.
        let rim: Vec<Pos2> = (0..=24)
            .map(|i| {
                let a = std::f32::consts::PI * (1.15 + 0.7 * i as f32 / 24.0);
                centre + Vec2::new(a.cos(), a.sin()) * (radius - 1.5)
            })
            .collect();
        painter.add(egui::Shape::line(rim, Stroke::new(1.5, Color32::from_white_alpha(if th.dark { 90 } else { 220 }))));
        painter.circle_stroke(centre, radius - 0.5, Stroke::new(1.0, if th.dark { Color32::from_white_alpha(40) } else { Color32::from_black_alpha(30) }));
    }
}

/// A round speed-limit sign: red ring, white disc, black number, as on the road.
pub(super) fn speed_sign(painter: &egui::Painter, centre: Pos2, radius: f32, limit: u32) {
    painter.circle_filled(centre, radius, Color32::from_rgb(0xd3, 0x2f, 0x2f));
    painter.circle_filled(centre, radius * 0.76, Color32::WHITE);
    let size = if limit >= 100 { radius * 0.72 } else { radius * 0.9 };
    painter.text(centre, Align2::CENTER_CENTER, limit.to_string(), FontId::new(size, Weight::Bold.family()), Color32::BLACK);
}

/// Places come with long addresses. The first part is the name, the rest says where.
pub(super) fn split_place(name: &str) -> (&str, &str) {
    match name.split_once(", ") {
        Some((first, rest)) => (first, rest),
        None => (name, ""),
    }
}

/// How far a place is from the rider, then where it is.
pub(super) fn place_detail(from: Option<LatLon>, at: LatLon, detail: &str) -> String {
    match (from, detail.is_empty()) {
        (Some(from), true) => format_distance(nav::distance_m(from, at)),
        (Some(from), false) => format!("{} · {detail}", format_distance(nav::distance_m(from, at))),
        (None, _) => detail.to_string(),
    }
}

/// A place in a list: tap the row to go there; the star on the right saves it (`Some(false)`) or forgets it
/// (`Some(true)`), `None` has no star. Returns (go, star).
fn place_row(ui: &mut Ui, th: &Tokens, glyph: &'static str, heading: &str, detail: &str, saved: Option<bool>) -> (bool, bool) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 64.0), Sense::click());
    let painter = ui.painter_at(rect);
    if response.is_pointer_button_down_on() {
        painter.rect_filled(rect, CornerRadius::same(14), th.text.gamma_multiply(0.06));
    }
    let bubble = Pos2::new(rect.left() + 24.0, rect.center().y);
    painter.circle_filled(bubble, 21.0, th.accent_soft);
    icon(&painter, Rect::from_center_size(bubble, Vec2::splat(22.0)), glyph, false, th.accent);
    let text_left = rect.left() + 56.0;
    let star_room = if saved.is_some() { 52.0 } else { 8.0 };
    let wrap = (rect.right() - star_room - text_left).max(40.0);
    let fit = |s: &str, size: f32, weight: Weight, ink: Color32| {
        let mut job = egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat::simple(FontId::new(size, weight.family()), ink));
        job.wrap = egui::text::TextWrapping { max_width: wrap, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        ui.painter().layout_job(job)
    };
    let head = fit(heading, 16.0, Weight::SemiBold, th.text);
    let sub = fit(detail, 13.5, Weight::Regular, th.muted);
    let top = if detail.is_empty() { rect.center().y - head.size().y / 2.0 } else { rect.top() + 12.0 };
    painter.galley(Pos2::new(text_left, top), head, th.text);
    if !detail.is_empty() {
        painter.galley(Pos2::new(text_left, rect.top() + 36.0), sub, th.muted);
    }
    let mut star = false;
    if let Some(on) = saved {
        let area = Rect::from_center_size(Pos2::new(rect.right() - 26.0, rect.center().y), Vec2::splat(48.0));
        let hit = ui.interact(area, ui.scope_id().with(("star", heading, detail)), Sense::click());
        icon(&painter, Rect::from_center_size(area.center(), Vec2::splat(26.0)), "star", on, if on { th.warning } else { th.muted });
        star = hit.clicked();
    }
    (response.clicked() && !star, star)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_positions_round_trip() {
        for at in [(44.4949, 11.3426), (-33.86, 151.21), (60.0, -20.5)] {
            for z in [5u8, 12, 17] {
                let back = map_place(map_position(at, z), z);
                assert!((back.0 - at.0).abs() < 1e-4 && (back.1 - at.1).abs() < 1e-4, "{at:?} {back:?} at {z}");
            }
        }
    }

    #[test]
    fn a_view_turns_both_ways() {
        let view = View { zoom: 15.5, centre: (44.49, 11.34), anchor: Pos2::new(200.0, 300.0), turn: 0.7 };
        let p = view.to_screen((44.492, 11.345));
        let back = view.to_place(p);
        assert!((back.0 - 44.492).abs() < 1e-5 && (back.1 - 11.345).abs() < 1e-5);
    }

    #[test]
    fn the_range_fits_the_map() {
        let z30 = zoom_for_radius((44.49, 11.34), 30_000.0, 380.0);
        let z5 = zoom_for_radius((44.49, 11.34), 5_000.0, 380.0);
        assert!(z5 > z30 && (8.0..12.0).contains(&z30), "{z30} {z5}");
    }
}
