use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

pub const BG: Color32 = Color32::from_rgb(15, 17, 22);
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(20, 23, 30);
pub const BG_CARD: Color32 = Color32::from_rgb(24, 28, 37);
pub const BG_HOVER: Color32 = Color32::from_rgb(36, 41, 54);
pub const TEXT: Color32 = Color32::from_rgb(230, 234, 242);
pub const TEXT_DIM: Color32 = Color32::from_rgb(140, 149, 168);
pub const ACCENT: Color32 = Color32::from_rgb(79, 140, 255);
pub const PURPLE: Color32 = Color32::from_rgb(168, 120, 255);
pub const DANGER: Color32 = Color32::from_rgb(220, 70, 80);
pub const GREEN: Color32 = Color32::from_rgb(70, 190, 130);
pub const AMBER: Color32 = Color32::from_rgb(245, 183, 49);
pub const CYAN: Color32 = Color32::from_rgb(64, 200, 220);
pub const PINK: Color32 = Color32::from_rgb(240, 110, 170);

/// Цвет «температуры» нагрузки: зелёный → жёлтый → красный. `v` примерно 0..100
pub fn heat(v: f64) -> Color32 {
    let t = (v / 100.0).clamp(0.0, 1.0) as f32;
    let (a, b, k) = if t < 0.5 {
        (Color32::from_rgb(70, 190, 130), Color32::from_rgb(240, 200, 70), t * 2.0)
    } else {
        (Color32::from_rgb(240, 200, 70), Color32::from_rgb(235, 80, 80), (t - 0.5) * 2.0)
    };
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Файл шрифта годится, только если это одиночный TrueType/OpenType (коллекции .ttc и мусор egui может не принять)
fn valid_font(bytes: &[u8]) -> bool {
    bytes.len() > 1024 && matches!(&bytes[..4], [0, 1, 0, 0] | b"true" | b"OTTO")
}

pub fn apply_theme(ctx: &egui::Context) {
    // На Windows добавляем Segoe UI как основной шрифт — у него есть кириллица
    // и он выглядит родным. Если файла нет (другая ОС) — остаётся шрифт по умолчанию.
    let mut fonts = FontDefinitions::default();
    for path in [r"C:\Windows\Fonts\segoeui.ttf", r"C:\Windows\Fonts\segoeuib.ttf"] {
        if let Some(bytes) = std::fs::read(path).ok().filter(|b| valid_font(b)) {
            let name = path.rsplit('\\').next().unwrap_or("segoe").to_owned();
            fonts.font_data.insert(name.clone(), std::sync::Arc::new(FontData::from_owned(bytes)));
            fonts.families.entry(FontFamily::Proportional).or_default().insert(0, name);
            break;
        }
    }
    // Запасной шрифт со значками (стрелки, геометрические фигуры, ⏻ и т. п.)
    if let Some(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf").ok().filter(|b| valid_font(b)) {
        fonts
            .font_data
            .insert("seguisym".to_owned(), std::sync::Arc::new(FontData::from_owned(bytes)));
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("seguisym".to_owned());
        }
    }
    ctx.set_fonts(fonts);

    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG_CARD;
    v.extreme_bg_color = BG_CARD;
    v.faint_bg_color = BG_CARD;
    v.override_text_color = None;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let r = CornerRadius::same(8);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(38, 44, 58));
    v.widgets.inactive.bg_fill = BG_HOVER;
    v.widgets.inactive.weak_bg_fill = BG_HOVER;
    v.widgets.hovered.bg_fill = Color32::from_rgb(46, 53, 70);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(46, 53, 70);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
}
