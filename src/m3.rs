//! Material 3 parts in the suite's colours: type roles, tonal cards, buttons, a switch, segmented buttons, chips,
//! lists, the navigation bar, a floating action button and a snackbar. Each answers a press with a state layer.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Pos2, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2,
};

use crate::theme::{icon, Tokens, Weight};

const PRESS_SECONDS: f32 = 0.09;
const MOVE_SECONDS: f32 = 0.18;
pub const CARD_RADIUS: u8 = 20;

static TAPPED: AtomicBool = AtomicBool::new(false);

/// Whether anything was tapped since last asked, so the phone can give a small tick.
pub fn take_tap() -> bool {
    TAPPED.swap(false, Ordering::Relaxed)
}

fn tap(response: &Response) -> bool {
    let clicked = response.clicked();
    if clicked {
        TAPPED.store(true, Ordering::Relaxed);
    }
    clicked
}

/// The Material state layer: the colour of what's on top, faintly, while pressed.
fn state_layer(ui: &Ui, response: &Response, rect: Rect, round: CornerRadius, ink: Color32) {
    let id = response.id.with("press");
    let amount = ui.ctx().animate_bool_with_time(id, response.is_pointer_button_down_on(), PRESS_SECONDS);
    if amount > 0.0 {
        ui.painter_at(rect).rect_filled(rect, round, ink.gamma_multiply(0.14 * amount));
    }
}

pub fn text(value: impl Into<String>, size: f32, color: Color32, weight: Weight) -> RichText {
    RichText::new(value.into()).size(size).color(color).family(weight.family())
}

pub fn headline(ui: &mut Ui, th: &Tokens, value: &str) {
    ui.label(text(value, 20.0, th.text, Weight::Bold));
}

pub fn title(ui: &mut Ui, th: &Tokens, value: &str) {
    ui.label(text(value, 15.5, th.text, Weight::SemiBold));
}

pub fn body(ui: &mut Ui, th: &Tokens, value: &str) {
    ui.label(text(value, 14.0, th.muted, Weight::Regular));
}

pub fn small(ui: &mut Ui, th: &Tokens, value: &str) {
    ui.label(text(value, 12.0, th.faint, Weight::Regular));
}

pub fn label(ui: &mut Ui, th: &Tokens, value: &str) {
    ui.label(text(value, 13.0, th.accent, Weight::SemiBold));
    ui.add_space(6.0);
}

fn card_frame(th: &Tokens, fill: Color32) -> Frame {
    let stroke = if th.outlined { Stroke::new(1.5, th.border) } else { Stroke::NONE };
    Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(16))
}

/// A card on the lowest tonal surface.
pub fn card(ui: &mut Ui, th: &Tokens, add: impl FnOnce(&mut Ui)) {
    card_frame(th, th.surface).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(10.0);
}

/// A card in the primary container colour, for the one thing that matters most on a screen.
pub fn hero(ui: &mut Ui, th: &Tokens, add: impl FnOnce(&mut Ui)) {
    card_frame(th, th.accent_soft).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(10.0);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// One per screen at most, for the main action.
    Filled,
    Tonal,
    Outlined,
    Danger,
}

/// A full-width round button, with an optional icon.
pub fn button(ui: &mut Ui, th: &Tokens, label: &str, glyph: Option<&'static str>, kind: Kind, height: f32) -> bool {
    let (fill, ink) = match kind {
        Kind::Filled => (th.accent, th.on_accent),
        Kind::Tonal => (th.accent_soft, th.on_accent_soft),
        Kind::Outlined => (Color32::TRANSPARENT, th.accent),
        Kind::Danger => (th.danger, if th.dark { th.bg } else { th.surface }),
    };
    let size = Vec2::new(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    let round = CornerRadius::same((height / 2.0).min(120.0) as u8);
    painter.rect_filled(rect, round, fill);
    if kind == Kind::Outlined || th.outlined {
        painter.rect_stroke(rect, round, Stroke::new(1.2, th.border), StrokeKind::Inside);
    }
    state_layer(ui, &response, rect, round, ink);
    let font = FontId::new(15.0, Weight::SemiBold.family());
    let galley = painter.layout_no_wrap(label.to_string(), font, ink);
    const ICON: f32 = 20.0;
    let gap = if glyph.is_some() { ICON + 8.0 } else { 0.0 };
    let start = rect.center().x - (galley.size().x + gap) / 2.0;
    if let Some(name) = glyph {
        let square = Rect::from_center_size(Pos2::new(start + ICON / 2.0, rect.center().y), Vec2::splat(ICON));
        icon(&painter, square, name, false, ink);
    }
    painter.galley(Pos2::new(start + gap, rect.center().y - galley.size().y / 2.0), galley, ink);
    tap(&response)
}

/// A round icon-only button.
pub fn icon_button(ui: &mut Ui, th: &Tokens, glyph: &'static str, ink: Color32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::click());
    let round = CornerRadius::same(24);
    state_layer(ui, &response, rect.shrink(2.0), round, th.text);
    let square = Rect::from_center_size(rect.center(), Vec2::splat(24.0));
    icon(&ui.painter_at(rect), square, glyph, false, ink);
    tap(&response)
}

/// A Material 3 switch, with the whole row to tap: easier in gloves.
pub fn switch_row(ui: &mut Ui, th: &Tokens, label: &str, on: bool) -> bool {
    let width = ui.available_width();
    let font = FontId::new(15.5, Weight::Medium.family());
    let galley = ui.painter().layout(label.to_string(), font, th.text, width - 68.0);
    let height = (galley.size().y + 12.0).max(52.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter_at(rect);
    state_layer(ui, &response, rect, CornerRadius::same(12), th.text);
    painter.galley(Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0), galley, th.text);
    let track = Rect::from_center_size(Pos2::new(rect.right() - 28.0, rect.center().y), Vec2::new(52.0, 32.0));
    let round = CornerRadius::same(16);
    let t = ui.ctx().animate_bool_with_time(response.id.with("on"), on, MOVE_SECONDS);
    painter.rect_filled(track, round, lerp_color(th.raised, th.accent, t));
    if t < 1.0 {
        painter.rect_stroke(track, round, Stroke::new(2.0, th.border.gamma_multiply(1.0 - t)), StrokeKind::Inside);
    }
    let centre = Pos2::new(track.left() + 16.0 + 20.0 * t, track.center().y);
    painter.circle_filled(centre, 8.0 + 4.0 * t, lerp_color(th.faint, th.on_accent, t));
    if t > 0.5 {
        let square = Rect::from_center_size(centre, Vec2::splat(16.0));
        icon(&painter, square, "check", false, th.accent.gamma_multiply((t - 0.5) * 2.0));
    }
    tap(&response)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Material 3 segmented buttons: a pill split in equal parts, the chosen one tinted and ticked.
pub fn segmented(ui: &mut Ui, th: &Tokens, options: &[&str], selected: usize) -> Option<usize> {
    const HEIGHT: f32 = 48.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    let round = CornerRadius::same(24);
    let width = rect.width() / options.len() as f32;
    let mut tapped = None;
    // All the options, as two controls on one screen can start with the same word.
    let bar_id = ui.scope_id().with(("segments", options));
    let at = ui.ctx().animate_value_with_time(bar_id, selected as f32, MOVE_SECONDS);
    let pill = Rect::from_min_size(rect.min + Vec2::new(width * at, 0.0), Vec2::new(width, HEIGHT));
    painter.rect_filled(pill, round, th.accent_soft);
    painter.rect_stroke(rect, round, Stroke::new(1.0, th.border), StrokeKind::Inside);
    for (i, option) in options.iter().enumerate() {
        let cell = Rect::from_min_size(rect.min + Vec2::new(width * i as f32, 0.0), Vec2::new(width, HEIGHT));
        if i > 0 {
            painter.line_segment([cell.left_top(), cell.left_bottom()], Stroke::new(1.0, th.border));
        }
        let response = ui.interact(cell, bar_id.with(i), Sense::click());
        state_layer(ui, &response, cell, round, th.text);
        let chosen = i == selected;
        let ink = if chosen { th.on_accent_soft } else { th.muted };
        let weight = if chosen { Weight::SemiBold } else { Weight::Medium };
        let galley = painter.layout_no_wrap((*option).to_string(), FontId::new(13.5, weight.family()), ink);
        const TICK: f32 = 16.0;
        // A tick only when there's room for it next to the words.
        let ticked = chosen && galley.size().x + TICK + 12.0 < width;
        let gap = if ticked { TICK + 3.0 } else { 0.0 };
        let start = cell.center().x - (galley.size().x + gap) / 2.0;
        if ticked {
            let square = Rect::from_center_size(Pos2::new(start + TICK / 2.0, cell.center().y), Vec2::splat(TICK));
            icon(&painter, square, "check", false, ink);
        }
        painter.galley(Pos2::new(start + gap, cell.center().y - galley.size().y / 2.0), galley, ink);
        if tap(&response) {
            tapped = Some(i);
        }
    }
    tapped
}

/// A small status chip, with a dot of colour.
pub fn status_chip(ui: &mut Ui, value: &str, color: Color32) {
    let font = FontId::new(12.5, Weight::SemiBold.family());
    let galley = ui.painter().layout_no_wrap(value.to_string(), font, color);
    let size = galley.size() + Vec2::new(32.0, 12.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(10), color.gamma_multiply(0.15));
    painter.circle_filled(Pos2::new(rect.left() + 12.0, rect.center().y), 3.5, color);
    painter.galley(Pos2::new(rect.left() + 21.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// A choice chip, with a tick when it's chosen. Returns whether it was tapped.
pub fn filter_chip(ui: &mut Ui, th: &Tokens, value: &str, chosen: bool) -> bool {
    let font = FontId::new(13.0, Weight::Medium.family());
    let ink = if chosen { th.on_accent_soft } else { th.muted };
    let galley = ui.painter().layout_no_wrap(value.to_string(), font, ink);
    let extra = if chosen { 44.0 } else { 28.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + extra, 38.0), Sense::click());
    let chip = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 32.0));
    let painter = ui.painter_at(rect);
    let round = CornerRadius::same(8);
    if chosen {
        painter.rect_filled(chip, round, th.accent_soft);
    } else {
        painter.rect_stroke(chip, round, Stroke::new(1.0, th.border), StrokeKind::Inside);
    }
    state_layer(ui, &response, chip, round, ink);
    let mut x = chip.left() + 14.0;
    if chosen {
        let square = Rect::from_center_size(Pos2::new(x + 8.0, chip.center().y), Vec2::splat(16.0));
        icon(&painter, square, "check", false, ink);
        x += 20.0;
    }
    painter.galley(Pos2::new(x, chip.center().y - galley.size().y / 2.0), galley, ink);
    tap(&response)
}

/// A square tile with a big value and a label.
pub fn tile(ui: &mut Ui, th: &Tokens, glyph: &'static str, value: &str, caption: &str) {
    Frame::new()
        .fill(th.surface)
        .stroke(if th.outlined { Stroke::new(1.5, th.border) } else { Stroke::NONE })
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                icon(&ui.painter_at(rect), rect, glyph, false, th.accent);
                ui.add_space(2.0);
                ui.label(text(caption, 12.5, th.muted, Weight::Medium));
            });
            ui.add_space(2.0);
            ui.label(text(value, 21.0, th.text, Weight::Bold));
        });
}

/// Equal tiles in a row.
pub fn tile_row(ui: &mut Ui, th: &Tokens, tiles: &[(&'static str, String, &str)]) {
    ui.columns(tiles.len(), |columns| {
        for (column, (glyph, value, caption)) in columns.iter_mut().zip(tiles) {
            tile(column, th, glyph, value, caption);
        }
    });
    ui.add_space(8.0);
}

/// A quiet value with a label, for details inside a card.
pub fn stat_row(ui: &mut Ui, th: &Tokens, stats: &[(String, &str)]) {
    ui.columns(stats.len(), |columns| {
        for (column, (value, caption)) in columns.iter_mut().zip(stats) {
            column.label(text(value.as_str(), 17.0, th.text, Weight::Bold));
            column.label(text(*caption, 12.5, th.muted, Weight::Medium));
        }
    });
}

/// A determinate bar with rounded ends, its value animated.
pub fn progress(ui: &mut Ui, th: &Tokens, id: Id, fraction: f32, color: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let shown = ui.ctx().animate_value_with_time(id, fraction.clamp(0.0, 1.0), 0.5);
    let painter = ui.painter_at(rect);
    let round = CornerRadius::same((height / 2.0) as u8);
    painter.rect_filled(rect, round, th.raised);
    if shown > 0.0 {
        let width = (rect.width() * shown).max(height);
        painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(width, height)), round, color);
    }
}

/// The moving line that says something is under way.
pub fn indeterminate(ui: &mut Ui, th: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let round = CornerRadius::same(2);
    painter.rect_filled(rect, round, th.accent_soft);
    let time = ui.input(|i| i.time) as f32;
    let phase = (time * 0.7).fract();
    let (start, end) = (rect.width() * (phase * 1.5 - 0.5).max(0.0), rect.width() * (phase * 1.5).min(1.0));
    let moving = Rect::from_min_max(rect.min + Vec2::new(start, 0.0), Pos2::new(rect.left() + end, rect.bottom()));
    painter.rect_filled(moving, round, th.accent);
    ui.ctx().request_repaint_after_secs(0.04);
}

/// A tappable list row with an icon in a tinted circle, and a chevron when it leads somewhere.
pub fn list_row(ui: &mut Ui, th: &Tokens, glyph: &'static str, heading: &str, detail: &str, chevron: bool) -> bool {
    let width = ui.available_width();
    let wrap = width - 56.0 - if chevron { 28.0 } else { 0.0 };
    let title_font = FontId::new(15.5, Weight::SemiBold.family());
    let detail_font = FontId::new(13.0, Weight::Regular.family());
    let title_galley = ui.painter().layout(heading.to_string(), title_font, th.text, wrap);
    let detail_galley = ui.painter().layout(detail.to_string(), detail_font, th.muted, wrap);
    let detail_height = if detail.is_empty() { 0.0 } else { detail_galley.size().y + 2.0 };
    let height = (title_galley.size().y + detail_height + 20.0).max(60.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter_at(rect);
    state_layer(ui, &response, rect, CornerRadius::same(12), th.text);
    let circle = Pos2::new(rect.left() + 20.0, rect.center().y);
    painter.circle_filled(circle, 20.0, th.accent_soft);
    icon(&painter, Rect::from_center_size(circle, Vec2::splat(22.0)), glyph, false, th.on_accent_soft);
    let x = rect.left() + 52.0;
    let top = rect.center().y - (title_galley.size().y + detail_height) / 2.0;
    let title_height = title_galley.size().y;
    painter.galley(Pos2::new(x, top), title_galley, th.text);
    if !detail.is_empty() {
        painter.galley(Pos2::new(x, top + title_height + 2.0), detail_galley, th.muted);
    }
    if chevron {
        let square = Rect::from_center_size(Pos2::new(rect.right() - 12.0, rect.center().y), Vec2::splat(22.0));
        icon(&painter, square, "chevron-right", false, th.faint);
    }
    tap(&response)
}

/// A problem the rider can fix, with the button that fixes it.
pub fn banner(ui: &mut Ui, th: &Tokens, glyph: &'static str, heading: &str, detail: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    card(ui, th, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            icon(&ui.painter_at(rect), rect, glyph, true, th.warning);
            ui.add_space(4.0);
            ui.label(text(heading, 15.5, th.text, Weight::Bold));
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

/// One slot of the navigation bar: a pill that grows behind the icon of the current page.
pub fn nav_item(ui: &mut Ui, th: &Tokens, glyph: &'static str, name: &str, selected: bool) -> bool {
    let size = Vec2::new(ui.available_width(), 68.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    let t = ui.ctx().animate_bool_with_time(response.id.with("pill"), selected, MOVE_SECONDS);
    let centre = Pos2::new(rect.center().x, rect.top() + 22.0);
    if t > 0.0 {
        let pill = Rect::from_center_size(centre, Vec2::new(32.0 + 32.0 * t, 32.0));
        painter.rect_filled(pill, CornerRadius::same(16), th.accent_soft.gamma_multiply(t));
    }
    if response.is_pointer_button_down_on() {
        let pill = Rect::from_center_size(centre, Vec2::new(64.0, 32.0));
        painter.rect_filled(pill, CornerRadius::same(16), th.text.gamma_multiply(0.12));
    }
    let ink = if selected { th.on_accent_soft } else { th.muted };
    icon(&painter, Rect::from_center_size(centre, Vec2::splat(24.0)), glyph, selected, ink);
    let weight = if selected { Weight::Bold } else { Weight::Medium };
    let tint = if selected { th.text } else { th.muted };
    painter.text(
        Pos2::new(rect.center().x, rect.top() + 52.0),
        Align2::CENTER_CENTER,
        name,
        FontId::new(12.0, weight.family()),
        tint,
    );
    tap(&response)
}

/// A large floating action, at a place already worked out.
pub fn extended_fab(ui: &mut Ui, th: &Tokens, id: Id, right_bottom: Pos2, glyph: &'static str, name: &str) -> bool {
    let font = FontId::new(15.5, Weight::SemiBold.family());
    let painter = ui.painter().clone();
    let galley = painter.layout_no_wrap(name.to_string(), font, th.on_accent_soft);
    let size = Vec2::new(galley.size().x + 60.0, 58.0);
    let rect = Rect::from_min_max(right_bottom - size, right_bottom);
    let response = ui.interact(rect, id, Sense::click());
    let round = CornerRadius::same(18);
    let shadow = rect.translate(Vec2::new(0.0, 3.0)).expand(1.0);
    painter.rect_filled(shadow, round, Color32::from_black_alpha(46));
    painter.rect_filled(rect, round, th.accent_soft);
    if th.outlined {
        painter.rect_stroke(rect, round, Stroke::new(1.5, th.border), StrokeKind::Inside);
    }
    let amount = ui.ctx().animate_bool_with_time(id.with("press"), response.is_pointer_button_down_on(), PRESS_SECONDS);
    if amount > 0.0 {
        painter.rect_filled(rect, round, th.on_accent_soft.gamma_multiply(0.14 * amount));
    }
    let square = Rect::from_center_size(Pos2::new(rect.left() + 30.0, rect.center().y), Vec2::splat(24.0));
    icon(&painter, square, glyph, true, th.on_accent_soft);
    painter.galley(Pos2::new(rect.left() + 48.0, rect.center().y - galley.size().y / 2.0), galley, th.on_accent_soft);
    tap(&response)
}

/// A brief message at the foot of the screen, fading in and out.
pub fn snackbar(ui: &Ui, th: &Tokens, area: Rect, message: &str, opacity: f32) {
    let painter = ui.painter().clone();
    let (background, ink) = if th.dark { (th.text, th.bg) } else { (th.text, th.surface) };
    let wrap = area.width() - 32.0;
    let galley = painter.layout(message.to_string(), FontId::new(14.0, Weight::Medium.family()), ink, wrap);
    let height = galley.size().y + 26.0;
    let rect = Rect::from_min_size(Pos2::new(area.left(), area.bottom() - height), Vec2::new(area.width(), height));
    painter.rect_filled(rect, CornerRadius::same(10), background.gamma_multiply(opacity));
    painter.galley(rect.min + Vec2::new(16.0, 13.0), galley, ink.gamma_multiply(opacity));
}

