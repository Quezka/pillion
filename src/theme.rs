//! The suite's look: colours token for token from suite-kit's `quezka_ui/theme.py`, its Manrope font and
//! its Material Symbols icons.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Painter, Rect, TextureHandle, TextureOptions};

use crate::symbols::SYMBOLS;

#[derive(Clone, Copy)]
pub struct Tokens {
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub sidebar: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    /// High contrast keeps stark lines around things, where Material 3 has none.
    pub outlined: bool,
    pub dark: bool,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Tokens = Tokens {
    bg: hex(0xf6f5fc),
    surface: hex(0xffffff),
    raised: hex(0xe9e8f6),
    sidebar: hex(0xeeedf8),
    border: hex(0xb9bec9),
    text: hex(0x0e1116),
    muted: hex(0x3f4654),
    faint: hex(0x5d6573),
    accent: hex(0x4b4bc8),
    accent_soft: hex(0xdddcfb),
    on_accent: hex(0xffffff),
    danger: hex(0xc4262c),
    success: hex(0x1f7a4d),
    warning: hex(0xb45309),
    outlined: false,
    dark: false,
};

pub const DARK: Tokens = Tokens {
    bg: hex(0x111119),
    surface: hex(0x1b1b25),
    raised: hex(0x28283a),
    sidebar: hex(0x16161f),
    border: hex(0x4a4e57),
    text: hex(0xf4f5f7),
    muted: hex(0xc3c8d2),
    faint: hex(0x98a0ad),
    accent: hex(0x9a9bff),
    accent_soft: hex(0x35366b),
    on_accent: hex(0x0e0f11),
    danger: hex(0xff7b80),
    success: hex(0x4fe39b),
    warning: hex(0xf5a524),
    outlined: false,
    dark: true,
};

pub const HIGH_LIGHT: Tokens = Tokens {
    bg: hex(0xffffff),
    surface: hex(0xffffff),
    raised: hex(0xf0f0f0),
    sidebar: hex(0xf0f0f0),
    border: hex(0x000000),
    text: hex(0x000000),
    muted: hex(0x1a1a1a),
    faint: hex(0x333333),
    accent: hex(0x1f1fa8),
    accent_soft: hex(0xcfcff7),
    on_accent: hex(0xffffff),
    danger: hex(0xa30000),
    success: hex(0x005a2b),
    warning: hex(0x7a4100),
    outlined: true,
    dark: false,
};

pub const HIGH_DARK: Tokens = Tokens {
    bg: hex(0x000000),
    surface: hex(0x000000),
    raised: hex(0x1a1a1a),
    sidebar: hex(0x0a0a0a),
    border: hex(0xffffff),
    text: hex(0xffffff),
    muted: hex(0xe6e6e6),
    faint: hex(0xcccccc),
    accent: hex(0xffd400),
    accent_soft: hex(0x3a3300),
    on_accent: hex(0x000000),
    danger: hex(0xff8a8a),
    success: hex(0x6dffb0),
    warning: hex(0xffd400),
    outlined: true,
    dark: true,
};

pub fn tokens(dark: bool, high_contrast: bool) -> Tokens {
    match (dark, high_contrast) {
        (true, true) => HIGH_DARK,
        (true, false) => DARK,
        (false, true) => HIGH_LIGHT,
        (false, false) => LIGHT,
    }
}

#[derive(Clone, Copy)]
pub enum Weight {
    Regular,
    Medium,
    SemiBold,
    Bold,
}

impl Weight {
    pub fn family(self) -> FontFamily {
        match self {
            Self::Regular => FontFamily::Proportional,
            Self::Medium => FontFamily::Name("medium".into()),
            Self::SemiBold => FontFamily::Name("semibold".into()),
            Self::Bold => FontFamily::Name("bold".into()),
        }
    }
}

/// Manrope, four static weights, the egui fonts behind it for any symbol Manrope lacks.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let faces: [(&str, &'static [u8], Weight); 4] = [
        ("manrope-regular", include_bytes!("../assets/Manrope-Regular.ttf"), Weight::Regular),
        ("manrope-medium", include_bytes!("../assets/Manrope-Medium.ttf"), Weight::Medium),
        ("manrope-semibold", include_bytes!("../assets/Manrope-SemiBold.ttf"), Weight::SemiBold),
        ("manrope-bold", include_bytes!("../assets/Manrope-Bold.ttf"), Weight::Bold),
    ];
    for (name, bytes, weight) in faces {
        fonts.font_data.insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
        let mut list = vec![name.to_string()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(weight.family(), list);
    }
    ctx.set_fonts(fonts);
}

type IconKey = (&'static str, bool, u32);

fn icon_cache() -> &'static Mutex<HashMap<IconKey, TextureHandle>> {
    static CACHE: OnceLock<Mutex<HashMap<IconKey, TextureHandle>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// White pixels of an icon at this size, which the painter then tints.
fn render_icon(name: &str, filled: bool, px: u32) -> Option<egui::ColorImage> {
    let (_, outlined, solid) = SYMBOLS.iter().find(|(n, _, _)| *n == name)?;
    let path = if filled { solid } else { outlined };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="#ffffff"><path d="{path}"/></svg>"##
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let scale = px as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

/// Outlined normally and filled when selected, as Material 3 has it.
pub fn icon(painter: &Painter, rect: Rect, name: &'static str, filled: bool, color: Color32) {
    let ctx = painter.ctx();
    let px = (rect.width() * ctx.pixels_per_point()).ceil().max(1.0) as u32;
    let Ok(mut cache) = icon_cache().lock() else {
        return;
    };
    let key = (name, filled, px);
    if !cache.contains_key(&key) {
        let Some(image) = render_icon(name, filled, px) else {
            return;
        };
        let texture = ctx.load_texture(format!("icon-{name}-{filled}-{px}"), image, TextureOptions::LINEAR);
        cache.insert(key, texture);
    }
    if let Some(texture) = cache.get(&key) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(texture.id(), rect, uv, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_symbol_renders() {
        for (name, _, _) in SYMBOLS {
            for filled in [false, true] {
                let image = render_icon(name, filled, 24).unwrap_or_else(|| panic!("{name} didn't render"));
                assert!(image.pixels.iter().any(|p| p.a() > 0), "{name} is blank");
            }
        }
        assert!(render_icon("no-such-icon", false, 24).is_none());
    }

    #[test]
    fn colours_are_the_kits() {
        assert_eq!(LIGHT.accent, Color32::from_rgb(0x4b, 0x4b, 0xc8));
        assert_eq!(DARK.bg, Color32::from_rgb(0x11, 0x11, 0x19));
        assert!(tokens(true, true).outlined);
        assert!(!tokens(false, false).dark);
    }
}
