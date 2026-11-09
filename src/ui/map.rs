//! The Route tab: the street map fills the screen, and everything over it is liquid glass that shows the map
//! beneath, blurred: the bars, the search bar or the next turn, the map's buttons and a sheet with places. The map
//! can be dragged and zoomed, and shows the area the battery reaches, saved places and the speed limit.

use std::collections::HashMap;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, LayerId, Pos2, Rect, ScrollArea, Sense, Shape, Stroke,
    TextureOptions, Ui, UiBuilder, Vec2,
};

use super::glass;
use super::m3::{self, text, Kind};
use super::theme::{icon, Tokens, Weight};
use super::{draw_turn, format_distance, CompanionApp, MAX_RECENTS_SHOWN};
use crate::domain::places::{BookmarkKind, Place};
use crate::engine::{Engine, TileKey};
use crate::locale::{t, tf};
use crate::nav::{self, LatLon};

const TILE_POINTS: f32 = 256.0;
const CLOSE_ZOOM: f32 = 17.0;
/// While riding fast the map zooms out to this, to see further ahead.
const FAR_ZOOM: f32 = 16.0;
/// Above this speed the map zooms out a little, to see further ahead.
const FAR_ZOOM_KMH: f64 = 28.0;
const ZOOM_RANGE: (f32, f32) = (4.0, 18.5);
const TILES_DECODED_PER_FRAME: usize = 3;
const TILES_KEPT: usize = 120;
const HEADING_EASE: f32 = 0.12;
/// How much a turn of the mouse wheel zooms, on the desktop preview.
const WHEEL_ZOOM: f32 = 0.004;
/// Where the map starts when there's no GPS yet: Bologna, near the owner.
const NO_GPS_START: LatLon = (44.4949, 11.3426);
/// How many zoom levels lower the map pictures under glass are. Each level halves the detail; the picture is then
/// blurred too, so what shows through is the map's colours and shapes, not its words.
const GLASS_BLUR_LEVELS: u8 = 2;
/// How far the blur reaches, in pixels of those lower pictures (each is about four points on screen).
const GLASS_BLUR_PX: usize = 5;
/// Blurring a picture takes a moment, so only a few each frame.
const GLASS_BLURS_PER_FRAME: usize = 2;

/// Corner radii, nested the way Apple nests them: what sits inside something round is rounder by exactly the gap
/// less, so the curves run side by side.
pub(super) const SHEET_RADIUS: f32 = 30.0;
pub(super) const GAP: f32 = 10.0;
const SEARCH_HEIGHT: f32 = 56.0;
const BUTTON_RADIUS: f32 = 24.0;
const SHEET_PEEK: f32 = 74.0;
const SHEET_PAD: f32 = 12.0;
const SHEET_EASE_S: f32 = 0.22;

/// How a pane of glass sits on the screen, which sets its outline and how see-through it is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Pane {
    /// Floats over the map: a shadow, a light rim along the top and a fine outline.
    Floating,
    /// A small round button: more see-through.
    Button,
    /// A bar along the bottom of the screen: a fine line along its top.
    BottomBar,
    /// A bar along the top of the screen: a fine line along its bottom.
    TopBar,
}

/// The street map: its pictures, and how it's looking.
#[derive(Default)]
pub(super) struct MapView {
    tiles: HashMap<TileKey, egui::TextureHandle>,
    /// Where each map picture is on disk, so a blurred copy can be made for glass.
    paths: HashMap<TileKey, String>,
    /// The blurred copies glass shows.
    blurred: HashMap<TileKey, egui::TextureHandle>,
    blurred_this_frame: usize,
    dark: bool,
    whole_route: bool,
    heading_deg: f32,
    /// Where the rider dragged the map to, and its zoom. `None` follows the rider.
    free: Option<(LatLon, f32)>,
    /// A spot picked with a long press, waiting for "Go here" or "Save".
    marked: Option<LatLon>,
    /// How the map looked this frame (when the Route tab is showing).
    view: Option<View>,
    /// The part of the screen no glass covered last frame: the map centres itself there.
    clear: Option<Rect>,
    sheet_open: bool,
    results_seen: usize,
    was_navigating: bool,
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
#[derive(Clone, Copy)]
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
    /// Where the map looks this frame, inside `clear` (the part no glass covers).
    fn look(&mut self, ctx: &egui::Context, engine: &Engine, clear: Rect) -> View {
        let nav = engine.nav.as_ref();
        let rider = engine.gps;
        if let Some((centre, zoom)) = self.map.free {
            View { zoom, centre, anchor: clear.center(), turn: 0.0 }
        } else if let (Some(nav), true) = (nav, self.map.whole_route) {
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
                if span.x * TILE_POINTS < clear.width() * 0.85 && span.y * TILE_POINTS < clear.height() * 0.85 {
                    break;
                }
                z -= 1;
            }
            View { zoom: f32::from(z), centre: ((lat0 + lat1) / 2.0, (lon0 + lon1) / 2.0), anchor: clear.center(), turn: 0.0 }
        } else if let Some(nav) = nav {
            let zoom = if engine.speed_kmh.is_some_and(|s| s > FAR_ZOOM_KMH) { FAR_ZOOM } else { CLOSE_ZOOM };
            let at = rider.unwrap_or(nav.route.points[nav.index_along()]);
            let wanted = engine.heading_deg.unwrap_or(0.0) as f32;
            let gap = (wanted - self.map.heading_deg + 540.0).rem_euclid(360.0) - 180.0;
            if gap.abs() > 0.5 {
                self.map.heading_deg += gap * HEADING_EASE;
                ctx.request_repaint();
            }
            View { zoom, centre: at, anchor: clear.center() + Vec2::new(0.0, clear.height() * 0.24), turn: self.map.heading_deg.to_radians() }
        } else {
            // Not navigating: the rider in the middle, zoomed to show how far the battery reaches.
            let at = rider.unwrap_or(NO_GPS_START);
            let zoom = engine.range_km().map_or(14.0, |km| zoom_for_radius(at, km * 1000.0, clear.width()));
            View { zoom, centre: at, anchor: clear.center(), turn: 0.0 }
        }
    }

    /// Paints the map over the whole screen, under everything else. Called before the bars and the Route tab, which
    /// then lay their glass over it.
    pub(super) fn paint_map(&mut self, ctx: &egui::Context, engine: &mut Engine, th: &Tokens, screen: Rect) {
        if self.map.dark != th.dark {
            self.map.dark = th.dark;
            self.forget_map_pictures(engine);
        }
        self.map.blurred_this_frame = 0;
        for _ in 0..TILES_DECODED_PER_FRAME {
            let Some((key, path)) = engine.tiles_ready.pop() else {
                break;
            };
            if let Some(image) = load_tile(&path, th.dark) {
                if self.map.tiles.len() >= TILES_KEPT {
                    self.forget_map_pictures(engine);
                }
                self.map.tiles.insert(key, ctx.load_texture("map", image, TextureOptions::LINEAR));
                self.map.paths.insert(key, path);
            }
        }
        if !engine.tiles_ready.is_empty() {
            ctx.request_repaint();
        }
        engine.want_reach();

        let clear = self.map.clear.filter(|c| c.width() > 50.0 && c.height() > 50.0).unwrap_or(screen);
        let view = self.look(ctx, engine, clear);
        self.map.view = Some(view);
        let painter = ctx.layer_painter(LayerId::background()).with_clip_rect(screen);
        painter.rect_filled(screen, 0.0, th.raised);
        let nav = engine.nav.clone();
        let rider = engine.gps;

        // The map pictures that fall inside.
        let z = view.level();
        let scale = view.scale();
        let centre_tile = map_position(view.centre, z);
        let reach = (screen.size().length() / scale).ceil() as i32 + 1;
        let count = 1i64 << z;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (tx, ty) = (centre_tile.x.floor() as i64 + i64::from(dx), centre_tile.y.floor() as i64 + i64::from(dy));
                if tx < 0 || ty < 0 || tx >= count || ty >= count {
                    continue;
                }
                let quad = tile_quad(&view, (z, tx as u32, ty as u32));
                if !Rect::from_points(&quad).intersects(screen) {
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
                    painter.add(Shape::mesh(mesh));
                }
            }
        }

        // How far the battery reaches: the area on roads when the map service gave it, a circle otherwise.
        let reach_ink = th.success;
        if !engine.reach.is_empty() {
            for ring in &engine.reach {
                let line: Vec<Pos2> = ring.iter().map(|p| view.to_screen(*p)).collect();
                painter.add(Shape::closed_line(line.clone(), Stroke::new(12.0, reach_ink.gamma_multiply(0.18))));
                painter.add(Shape::closed_line(line, Stroke::new(3.0, reach_ink)));
            }
        } else if let Some(ring) = engine.range_ring() {
            let line: Vec<Pos2> = ring.iter().map(|p| view.to_screen(*p)).collect();
            painter.add(Shape::convex_polygon(line.clone(), reach_ink.gamma_multiply(0.08), Stroke::NONE));
            painter.add(Shape::closed_line(line, Stroke::new(3.0, reach_ink)));
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
            if !screen.expand(-6.0).contains(spot) {
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
            painter.add(Shape::convex_polygon(vec![arrow[0], arrow[1], arrow[2]], th.accent, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![arrow[0], arrow[2], arrow[3]], th.accent, Stroke::NONE));
            painter.add(Shape::closed_line(arrow, Stroke::new(2.0, th.surface)));
        }
    }

    fn forget_map_pictures(&mut self, engine: &mut Engine) {
        self.map.tiles.clear();
        self.map.paths.clear();
        self.map.blurred.clear();
        engine.forget_tiles();
    }

    /// Moving the map: drag, pinch, the mouse wheel, double tap; press and hold to pick a spot.
    fn map_input(&mut self, ui: &Ui, response: &egui::Response, view: View, engine: &mut Engine) {
        let ctx = ui.ctx();
        let pinch = if response.hovered() { ui.input(|i| i.zoom_delta()) } else { 1.0 };
        let wheel = if response.hovered() { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        let mut zoom_by = pinch.log2() + wheel * WHEEL_ZOOM;
        if response.double_clicked() {
            zoom_by += 1.0;
        }
        if response.dragged() || zoom_by.abs() > 1e-4 {
            let (mut centre, mut zoom) = self.map.free.unwrap_or((view.to_place(view.anchor), view.zoom));
            if response.dragged() {
                let free = View { zoom, centre, anchor: view.anchor, turn: 0.0 };
                centre = free.to_place(view.anchor - response.drag_delta());
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
    }

    /// A blurred map picture for glass, or the nearest blurred one further out that covers the same place. Asks for
    /// what's missing; it shows up a frame or two later.
    fn glass_picture(&mut self, ctx: &egui::Context, key: TileKey, engine: &mut Engine) -> Option<(egui::TextureId, TileKey)> {
        if !self.map.blurred.contains_key(&key) {
            match self.map.paths.get(&key).cloned() {
                Some(path) if self.map.blurred_this_frame < GLASS_BLURS_PER_FRAME => {
                    self.map.blurred_this_frame += 1;
                    if let Some(mut image) = load_tile(&path, self.map.dark) {
                        let [w, h] = image.size;
                        let mut pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                        glass::blur_rgba(&mut pixels, w, h, GLASS_BLUR_PX);
                        image = egui::ColorImage::from_rgba_premultiplied([w, h], &pixels);
                        self.map.blurred.insert(key, ctx.load_texture("glass", image, TextureOptions::LINEAR));
                    }
                }
                Some(_) => ctx.request_repaint(),
                None => engine.want_tile(key),
            }
        }
        let mut k = key;
        loop {
            if let Some(texture) = self.map.blurred.get(&k) {
                return Some((texture.id(), k));
            }
            if k.0 <= ZOOM_RANGE.0 as u8 {
                return None;
            }
            k = (k.0 - 1, k.1 / 2, k.2 / 2);
        }
    }

    /// A pane of liquid glass over the map: what's beneath shows through, blurred, under a light tint, with light
    /// catching its top edge. Solid when glass is off (high contrast, or the rider turned it off).
    pub(super) fn glass(&mut self, ctx: &egui::Context, rect: Rect, radius: f32, pane: Pane, engine: &mut Engine, th: &Tokens) -> Shape {
        let mut shapes = Vec::new();
        let round = CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8);
        let view = self.map.view.filter(|_| th.glass);
        if pane == Pane::Floating || pane == Pane::Button {
            let shadow = egui::epaint::Shadow { offset: [0, 3], blur: 16, spread: 0, color: Color32::from_black_alpha(if th.dark { 90 } else { 46 }) };
            shapes.push(Shape::from(shadow.as_shape(rect, round)));
        }
        let Some(view) = view else {
            let fill = if matches!(pane, Pane::BottomBar | Pane::TopBar) { th.sidebar } else { th.surface };
            shapes.push(Shape::rect_filled(rect, round, fill));
            if th.outlined {
                shapes.push(Shape::rect_stroke(rect, round, Stroke::new(1.5, th.border), egui::StrokeKind::Inside));
            }
            return Shape::Vec(shapes);
        };
        let outline = glass::rounded_outline(rect, radius);
        let low = view.level().saturating_sub(GLASS_BLUR_LEVELS).max(ZOOM_RANGE.0 as u8);
        let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].map(|p| map_position(view.to_place(p), low));
        let count = 1u32 << low;
        let min_x = corners.iter().map(|c| c.x).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
        let max_x = (corners.iter().map(|c| c.x).fold(f32::MIN, f32::max).floor().max(0.0) as u32).min(count - 1);
        let min_y = corners.iter().map(|c| c.y).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
        let max_y = (corners.iter().map(|c| c.y).fold(f32::MIN, f32::max).floor().max(0.0) as u32).min(count - 1);
        let mut covered = false;
        for ty in min_y..=max_y {
            for tx in min_x..=max_x {
                let key = (low, tx, ty);
                let Some((texture, from)) = self.glass_picture(ctx, key, engine) else {
                    continue;
                };
                let piece = glass::clip_convex(&outline, &tile_quad(&view, key));
                if piece.len() < 3 {
                    continue;
                }
                covered = true;
                let corner = Vec2::new(from.1 as f32, from.2 as f32);
                let mut mesh = egui::Mesh::with_texture(texture);
                for p in &piece {
                    let uv = map_position(view.to_place(*p), from.0) - corner;
                    mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv: Pos2::new(uv.x, uv.y), color: Color32::WHITE });
                }
                for i in 1..piece.len() as u32 - 1 {
                    mesh.add_triangle(0, i, i + 1);
                }
                shapes.push(Shape::mesh(mesh));
            }
        }
        // The tint: just enough for words on it to stay easy to read, and more while the picture is still coming.
        let base = match pane {
            Pane::Button => 0.22,
            Pane::BottomBar | Pane::TopBar => 0.28,
            Pane::Floating => 0.32,
        } + if th.dark { 0.10 } else { 0.0 };
        let tint = th.surface.gamma_multiply(if covered { base } else { 0.88 });
        shapes.push(Shape::rect_filled(rect, round, tint));
        // The light catching the top edge, and a hairline round it.
        let hairline = if th.dark { Color32::from_white_alpha(38) } else { Color32::from_black_alpha(26) };
        let shine = Color32::from_white_alpha(if th.dark { 80 } else { 210 });
        match pane {
            Pane::Floating | Pane::Button => {
                let inner = rect.shrink(1.0);
                let rim = glass::top_rim(&glass::rounded_outline(inner, (radius - 1.0).max(0.0)), inner, radius);
                if rim.len() > 1 {
                    shapes.push(Shape::line(rim, Stroke::new(1.5, shine)));
                }
                shapes.push(Shape::rect_stroke(rect, round, Stroke::new(1.0, hairline), egui::StrokeKind::Inside));
            }
            Pane::BottomBar => {
                shapes.push(Shape::line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, hairline)));
                shapes.push(Shape::line_segment(
                    [rect.left_top() + Vec2::new(0.0, 1.0), rect.right_top() + Vec2::new(0.0, 1.0)],
                    Stroke::new(1.0, shine.gamma_multiply(0.6)),
                ));
            }
            Pane::TopBar => {
                shapes.push(Shape::line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, hairline)));
            }
        }
        Shape::Vec(shapes)
    }

    /// Glass at `top`, as tall as what `add` puts in it (laid out with `pad` all round). Touches on the glass don't
    /// reach the map. Returns the pane.
    fn glass_block(
        &mut self,
        ui: &mut Ui,
        engine: &mut Engine,
        th: &Tokens,
        id: &str,
        frame: (Pos2, f32, f32),
        pad: f32,
        add: impl FnOnce(&mut Self, &mut Ui, &mut Engine),
    ) -> (Rect, egui::Response) {
        let (top_left, width, radius) = frame;
        let key = ui.scope_id().with(("glass", id));
        // Last frame's size blocks the map now; this frame's is worked out from the content below.
        let before: Rect = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(Rect::from_min_size(top_left, Vec2::new(width, 48.0)));
        let response = ui.interact(before, key.with("hit"), Sense::click());
        let slot = ui.painter().add(Shape::Noop);
        let inner = Rect::from_min_size(top_left + Vec2::splat(pad), Vec2::new(width - 2.0 * pad, f32::INFINITY));
        let mut child = ui.new_child(UiBuilder::new().max_rect(inner).id_salt(key));
        child.set_width(inner.width());
        add(self, &mut child, engine);
        let used = child.min_rect();
        let rect = Rect::from_min_size(top_left, Vec2::new(width, used.height() + 2.0 * pad));
        let shape = self.glass(ui.ctx(), rect, radius, Pane::Floating, engine, th);
        ui.painter().set(slot, shape);
        ui.ctx().data_mut(|d| d.insert_temp(key, rect));
        (rect, response)
    }

    /// The Route tab: the map behind, glass over it. Returns the room left for brief messages, above the sheet.
    pub(super) fn route_screen(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) -> Rect {
        let area = ui.max_rect();
        let map_hit = ui.interact(area, ui.scope_id().with("map"), Sense::click_and_drag());
        if let Some(view) = self.map.view {
            self.map_input(ui, &map_hit, view, engine);
        }
        // The sheet opens by itself for search results and a picked spot, and steps aside when a route starts.
        let results = engine.search_results.len();
        if results != self.map.results_seen {
            self.map.results_seen = results;
            self.map.sheet_open = results > 0;
        }
        let navigating = engine.nav.is_some();
        if navigating && !self.map.was_navigating {
            self.map.sheet_open = false;
        }
        self.map.was_navigating = navigating;
        if self.map.marked.is_some() {
            self.map.sheet_open = true;
        }

        // At the top: the next turn while navigating, otherwise the search bar.
        let mut y = area.top() + 4.0;
        if let Some(nav) = engine.nav.clone() {
            let arrival = engine.arrival_time();
            let (rect, _) = self.glass_block(ui, engine, th, "turn", (Pos2::new(area.left(), y), area.width(), SHEET_RADIUS), 14.0, |_, ui, _| {
                turn_card(ui, th, &nav, arrival);
            });
            y = rect.bottom() + GAP;
        } else {
            let rect = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(area.width(), SEARCH_HEIGHT));
            let hit = ui.interact(rect, ui.scope_id().with("search"), Sense::click());
            let shape = self.glass(ui.ctx(), rect, SEARCH_HEIGHT / 2.0, Pane::Floating, engine, th);
            ui.painter().add(shape);
            let painter = ui.painter_at(rect);
            if hit.is_pointer_button_down_on() {
                painter.rect_filled(rect, CornerRadius::same((SEARCH_HEIGHT / 2.0) as u8), th.text.gamma_multiply(0.08));
            }
            icon(&painter, Rect::from_center_size(Pos2::new(rect.left() + 30.0, rect.center().y), Vec2::splat(26.0)), "search", false, th.accent);
            let galley = one_line(ui, t("Where to?"), 17.0, Weight::SemiBold, th.text, rect.width() - 76.0);
            painter.galley(Pos2::new(rect.left() + 58.0, rect.center().y - galley.size().y / 2.0), galley, th.text);
            if hit.clicked() {
                engine.tick_haptic();
                engine.ask_search();
            }
            y = rect.bottom() + GAP;
        }
        if !engine.nav_status.is_empty() {
            let status = engine.nav_status.clone();
            let (rect, _) = self.glass_block(ui, engine, th, "status", (Pos2::new(area.left(), y), area.width(), 22.0), 12.0, |_, ui, _| {
                ui.horizontal_wrapped(|ui| ui.label(text(status.as_str(), 14.5, th.warning, Weight::SemiBold)));
            });
            y = rect.bottom() + GAP;
        }

        // Down the left: the speed limit. Down the right: follow me, whole route, zoom in and out.
        if let Some(limit) = engine.limits.limit() {
            speed_sign(ui.painter(), Pos2::new(area.left() + 30.0, y + 30.0), 26.0, limit);
        }
        let rider = engine.gps;
        let mut buttons: Vec<(&'static str, &'static str)> = Vec::new();
        if self.map.free.is_some() || rider.is_none() {
            buttons.push(("target", "follow"));
        }
        if navigating {
            buttons.push((if self.map.whole_route { "near" } else { "route" }, "whole"));
        }
        buttons.push(("plus", "in"));
        buttons.push(("minus", "out"));
        for (i, (glyph, what)) in buttons.into_iter().enumerate() {
            let centre = Pos2::new(area.right() - BUTTON_RADIUS - 2.0, y + BUTTON_RADIUS + i as f32 * (2.0 * BUTTON_RADIUS + GAP));
            let rect = Rect::from_center_size(centre, Vec2::splat(2.0 * BUTTON_RADIUS));
            let hit = ui.interact(rect, ui.scope_id().with(("map-button", what)), Sense::click());
            let shape = self.glass(ui.ctx(), rect, BUTTON_RADIUS, Pane::Button, engine, th);
            ui.painter().add(shape);
            if hit.is_pointer_button_down_on() {
                ui.painter().circle_filled(centre, BUTTON_RADIUS, th.text.gamma_multiply(0.10));
            }
            icon(ui.painter(), Rect::from_center_size(centre, Vec2::splat(24.0)), glyph, false, th.accent);
            if hit.clicked() {
                engine.tick_haptic();
                match what {
                    "follow" => self.map.free = None,
                    "whole" => {
                        self.map.whole_route = !self.map.whole_route;
                        self.map.free = None;
                    }
                    _ => {
                        if let Some(view) = self.map.view {
                            let step = if what == "in" { 1.0 } else { -1.0 };
                            let (centre, zoom) = self.map.free.unwrap_or((view.to_place(view.anchor), view.zoom));
                            self.map.free = Some((centre, (zoom + step).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1)));
                        }
                    }
                }
            }
        }

        let sheet_top = self.sheet(ui, engine, th, area, y);
        let credit = one_line(ui, "© OpenStreetMap", 10.5, Weight::Medium, th.muted, area.width());
        let back = Rect::from_min_size(
            Pos2::new(area.right() - credit.size().x - 14.0, sheet_top - credit.size().y - 12.0 - GAP / 2.0),
            credit.size() + Vec2::new(14.0, 8.0),
        );
        let shape = self.glass(ui.ctx(), back, back.height() / 2.0, Pane::Button, engine, th);
        ui.painter().add(shape);
        ui.painter().galley(back.min + Vec2::new(7.0, 4.0), credit, th.muted);

        self.map.clear = Some(Rect::from_min_max(Pos2::new(area.left(), y), Pos2::new(area.right() - 2.0 * BUTTON_RADIUS, back.top())));
        Rect::from_min_max(area.min, Pos2::new(area.right(), sheet_top))
    }

    /// The sheet along the bottom: a handle and a heading when folded, places and directions when open. Returns its top.
    fn sheet(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens, area: Rect, top_limit: f32) -> f32 {
        let open_height = ((area.bottom() - top_limit) * 0.62).max(SHEET_PEEK);
        let target = if self.map.sheet_open { open_height } else { SHEET_PEEK };
        let height = ui.ctx().animate_value_with_time(ui.scope_id().with("sheet-height"), target, SHEET_EASE_S);
        let rect = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - height - 4.0), Pos2::new(area.right(), area.bottom() - 4.0));
        let _block = ui.interact(rect, ui.scope_id().with("sheet"), Sense::click());
        let shape = self.glass(ui.ctx(), rect, SHEET_RADIUS, Pane::Floating, engine, th);
        ui.painter().add(shape);

        // The handle and the heading: a tap opens or folds the sheet.
        let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), SHEET_PEEK));
        let head_hit = ui.interact(head, ui.scope_id().with("sheet-head"), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 9.0), Vec2::new(38.0, 5.0)),
            CornerRadius::same(3),
            th.muted.gamma_multiply(0.55),
        );
        let heading = if navigating_heading(engine) {
            t("Coming up")
        } else if !engine.search_results.is_empty() {
            t("Places found")
        } else if self.map.marked.is_some() {
            t("The spot you picked")
        } else {
            t("Saved places")
        };
        let mut stop_room = 0.0;
        if engine.nav.is_some() {
            // Stopping stays one tap away, even with the sheet folded.
            let label = m3::fit(ui, t("Stop"), 15.0, 12.0, Weight::SemiBold, th.danger, rect.width() * 0.35);
            let size = Vec2::new(label.size().x + 28.0, 44.0);
            let pill = Rect::from_min_size(Pos2::new(rect.right() - SHEET_PAD - size.x, head.top() + 18.0), size);
            let hit = ui.interact(pill, ui.scope_id().with("stop"), Sense::click());
            painter.rect_filled(pill, CornerRadius::same(22), th.danger.gamma_multiply(if hit.is_pointer_button_down_on() { 0.28 } else { 0.16 }));
            painter.galley(pill.center() - label.size() / 2.0, label, th.danger);
            if hit.clicked() {
                engine.tick_haptic();
                engine.stop_navigation();
            }
            stop_room = size.x + SHEET_PAD;
        }
        let title = one_line(ui, heading, 18.0, Weight::Bold, th.text, rect.width() - 2.0 * SHEET_PAD - stop_room - 36.0);
        painter.galley(Pos2::new(rect.left() + SHEET_PAD + 8.0, head.top() + 40.0 - title.size().y / 2.0), title, th.text);
        let mid = Pos2::new(rect.right() - SHEET_PAD - stop_room - 16.0, head.top() + 40.0);
        let tip = if self.map.sheet_open { 4.0 } else { -4.0 };
        painter.line(vec![mid + Vec2::new(-7.0, -tip), mid + Vec2::new(0.0, tip), mid + Vec2::new(7.0, -tip)], Stroke::new(2.5, th.muted));
        if head_hit.clicked() {
            self.map.sheet_open = !self.map.sheet_open;
            engine.tick_haptic();
        }

        // What's inside, when there's room for it.
        let body = Rect::from_min_max(Pos2::new(rect.left() + SHEET_PAD, head.bottom()), rect.max - Vec2::splat(SHEET_PAD));
        if body.height() > 24.0 {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body).id_salt("sheet-body"));
            child.set_clip_rect(body);
            ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
                self.sheet_contents(ui, engine, th);
            });
        }
        rect.top()
    }

    fn sheet_contents(&mut self, ui: &mut Ui, engine: &mut Engine, th: &Tokens) {
        self.typed_prompt(ui, engine, th);
        Self::message_prompt(ui, engine, th);
        self.problems(ui, engine, th);
        let mut chosen: Option<Place> = None;
        let mut save: Option<Place> = None;
        if let Some(at) = self.map.marked {
            m3::section(ui, th, |ui| {
                if let Some(from) = engine.gps {
                    m3::body(ui, th, &tf("{} from you", &[&format_distance(nav::distance_m(from, at))]));
                    ui.add_space(6.0);
                }
                if m3::button(ui, th, t("Go here"), Some("route"), Kind::Filled, 48.0) {
                    chosen = Some(Place::new(t("The spot on the map"), at));
                    self.map.marked = None;
                }
                ui.add_space(6.0);
                if m3::button(ui, th, t("Save"), Some("star"), Kind::Tonal, 48.0) {
                    engine.ask_bookmark_name(at);
                    self.map.marked = None;
                }
            });
        }
        if !engine.search_results.is_empty() {
            m3::section(ui, th, |ui| {
                if !navigating_heading(engine) {
                    m3::label(ui, th, t("Places found"));
                }
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
                if m3::button(ui, th, t("Clear the results"), Some("x"), Kind::Text, 44.0) {
                    engine.search_results.clear();
                }
            });
        }
        if let Some(nav) = &engine.nav {
            let upcoming: Vec<_> = nav.route.maneuvers.iter().skip(nav.next).take(5).cloned().collect();
            m3::section(ui, th, |ui| {
                for m in &upcoming {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                        draw_turn(&ui.painter_at(rect), rect, Some(m), false, th.accent);
                        ui.add_space(8.0);
                        ui.add(egui::Label::new(text(m.text.as_str(), 15.0, th.text, Weight::Regular)).wrap());
                    });
                    ui.add_space(6.0);
                }
            });
        }
        if engine.search_results.is_empty() {
            Self::saved_places(ui, engine, th, &mut chosen);
            if engine.nav.is_none() && !engine.settings.recents.is_empty() {
                m3::section(ui, th, |ui| {
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
        ui.add(egui::Label::new(text(
            t("Planning a route sends your position and destination to free OpenStreetMap services. Routes and places: © OpenStreetMap contributors"),
            12.0,
            th.muted,
            Weight::Regular,
        )).wrap());
        ui.add_space(8.0);
    }

    /// Home and Work first (or a way to set them), then the other saved places.
    fn saved_places(ui: &mut Ui, engine: &mut Engine, th: &Tokens, chosen: &mut Option<Place>) {
        let mut remove = None;
        let mut set_here: Option<BookmarkKind> = None;
        m3::section(ui, th, |ui| {
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

}

/// Whether the sheet is about the route right now.
fn navigating_heading(engine: &Engine) -> bool {
    engine.nav.is_some() && engine.search_results.is_empty()
}

/// Where a map picture's corners are on screen.
fn tile_quad(view: &View, key: TileKey) -> [Pos2; 4] {
    let (z, tx, ty) = key;
    let corner = |ox: f32, oy: f32| view.to_screen(map_place(Vec2::new(tx as f32 + ox, ty as f32 + oy), z));
    [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)]
}

/// One line of words, cut short with "…" if it doesn't fit `width`.
fn one_line(ui: &Ui, s: &str, size: f32, weight: Weight, ink: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat::simple(FontId::new(size, weight.family()), ink));
    job.wrap = egui::text::TextWrapping { max_width: width.max(20.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

/// The next turn, on glass: the turn in a solid circle, how far, what to do, and how much is left.
fn turn_card(ui: &mut Ui, th: &Tokens, nav: &nav::Navigation, arrival: Option<String>) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.circle_filled(rect.center(), 32.0, th.accent);
        let square = Rect::from_center_size(rect.center(), Vec2::splat(42.0));
        if nav.off_route {
            icon(&painter, square, "alert", true, th.on_accent);
        } else {
            draw_turn(&painter, square, nav.current(), true, th.on_accent);
        }
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.label(text(format_distance(nav.distance_to_next_m()), 32.0, th.text, Weight::Bold));
            let what = if nav.off_route { t("Off route, finding a new way").to_string() } else { nav.current().map(|m| m.text.clone()).unwrap_or_default() };
            if !what.is_empty() {
                ui.add(egui::Label::new(text(what, 16.0, th.text, Weight::SemiBold)).wrap());
            }
            ui.add_space(2.0);
            let minutes = format!("{:.0}", (nav.remaining_s() / 60.0).ceil());
            let left = format_distance(nav.remaining_m());
            let line = match &arrival {
                Some(time) => tf("{} to go, {} min, arriving {}", &[&left, &minutes, time]),
                None => tf("{} to go, {} min", &[&left, &minutes]),
            };
            ui.add(egui::Label::new(text(line, 13.5, th.muted, Weight::Medium)).wrap());
        });
    });
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
        painter.rect_filled(rect, CornerRadius::same(18), th.text.gamma_multiply(0.08));
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
