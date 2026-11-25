//! The window's look: Horizon's design tokens (horizon-nx's `horizon-theme`) and its font, Geist,
//! light or dark as the system is.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontFamily, Stroke, Theme};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

pub struct Colours {
    page: Color32,
    plate_soft: Color32,
    plate: Color32,
    rule: Color32,
    ink: Color32,
    ink_soft: Color32,
    muted: Color32,
    accent: Color32,
    on_accent: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warn: Color32,
}

const DARK: Colours = Colours {
    page: rgb(0x0d0e13),
    plate_soft: rgb(0x181a22),
    plate: rgb(0x20222c),
    rule: rgb(0x24262f),
    ink: rgb(0xecedf3),
    ink_soft: rgb(0xc5c7d1),
    muted: rgb(0x8b8f9c),
    accent: rgb(0x9a86ff),
    on_accent: rgb(0x0d0e13),
    danger: rgb(0xf07a68),
    success: rgb(0x45c27a),
    warn: rgb(0xf2c46b),
};

const LIGHT: Colours = Colours {
    page: rgb(0xfafafc),
    plate_soft: rgb(0xf3f3f8),
    plate: rgb(0xebecf4),
    rule: rgb(0xe2e3ea),
    ink: rgb(0x16181f),
    ink_soft: rgb(0x3b3e49),
    muted: rgb(0x646875),
    accent: rgb(0x5b3fd6),
    on_accent: rgb(0xffffff),
    danger: rgb(0xb4372a),
    success: rgb(0x1d7a48),
    warn: rgb(0x7a4f00),
};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Whether the window is light now.
static IS_LIGHT: AtomicBool = AtomicBool::new(false);

pub fn colours() -> &'static Colours {
    if IS_LIGHT.load(Relaxed) {
        &LIGHT
    } else {
        &DARK
    }
}

/// Takes the system's appearance over, each frame.
pub fn follow(ctx: &egui::Context) {
    IS_LIGHT.store(ctx.theme() == Theme::Light, Relaxed);
}

const SYSTEM_FONTS: &[(&str, &[&str])] = &[
    (
        "system CJK",
        &[
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msgothic.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ],
    ),
    (
        "system Korean",
        &[
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "C:\\Windows\\Fonts\\malgun.ttf",
        ],
    ),
];

pub fn install(ctx: &egui::Context) {
    // egui's own fonts stay behind Geist, for the letters and signs it lacks.
    let mut fonts = egui::FontDefinitions::default();
    let geist: [(&str, &'static [u8], FontFamily); 2] = [
        (
            "Geist",
            include_bytes!("../fonts/Geist-Regular.ttf"),
            FontFamily::Proportional,
        ),
        (
            "Geist Mono",
            include_bytes!("../fonts/GeistMono-Regular.ttf"),
            FontFamily::Monospace,
        ),
    ];
    for (name, bytes, family) in geist {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, name.into());
    }
    // Chinese, Japanese and Korean world names: the system's own fonts, last.
    for (name, places) in SYSTEM_FONTS {
        let Some(bytes) = places.iter().find_map(|p| std::fs::read(p).ok()) else {
            continue;
        };
        fonts
            .font_data
            .insert((*name).into(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push((*name).into());
        }
    }
    ctx.set_fonts(fonts);
    ctx.set_visuals_of(Theme::Dark, visuals(egui::Visuals::dark(), &DARK));
    ctx.set_visuals_of(Theme::Light, visuals(egui::Visuals::light(), &LIGHT));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        for (text, font) in style.text_styles.iter_mut() {
            font.size = match text {
                egui::TextStyle::Heading => 24.0,
                egui::TextStyle::Small => 11.0,
                egui::TextStyle::Monospace => 12.0,
                _ => 14.0,
            };
        }
    });
}

fn visuals(mut v: egui::Visuals, c: &Colours) -> egui::Visuals {
    v.panel_fill = c.page;
    v.window_fill = c.page;
    v.extreme_bg_color = c.plate_soft;
    v.faint_bg_color = c.plate_soft;
    v.weak_text_color = Some(c.muted);
    v.hyperlink_color = c.accent;
    // Progress and selected text.
    v.selection.bg_fill = c.accent;
    v.selection.stroke = Stroke::new(1.0, c.on_accent);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = c.page;
    w.noninteractive.bg_stroke = Stroke::new(1.0, c.rule);
    w.noninteractive.fg_stroke = Stroke::new(1.0, c.ink_soft);
    for (state, fill, ink) in [
        (&mut w.inactive, c.plate_soft, c.ink_soft),
        (&mut w.hovered, c.plate, c.ink),
        (&mut w.active, c.plate, c.ink),
        (&mut w.open, c.plate, c.ink),
    ] {
        state.bg_fill = c.plate;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0, ink);
        state.expansion = 0.0;
        state.corner_radius = CornerRadius::same(6);
    }
    w.inactive.bg_stroke = Stroke::new(1.0, c.rule);
    // Keyboard focus stays visible.
    w.active.bg_stroke = Stroke::new(1.0, c.accent);
    v
}

/// A page in the list at the left: the open one stands on a plate.
pub fn page_button(text: egui::RichText, open: bool) -> egui::Button<'static> {
    let fill = if open {
        colours().plate
    } else {
        Color32::TRANSPARENT
    };
    egui::Button::new(text).fill(fill).stroke(Stroke::NONE)
}

/// The page's one main action.
pub fn main_button(label: impl Into<String>) -> egui::Button<'static> {
    let c = colours();
    egui::Button::new(egui::RichText::new(label).color(c.on_accent)).fill(c.accent)
}

/// A PNG as a texture, if it is one of a sane size.
pub fn picture(ctx: &egui::Context, png: &[u8]) -> Option<egui::TextureHandle> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture("world", pixels, egui::TextureOptions::NEAREST))
}
