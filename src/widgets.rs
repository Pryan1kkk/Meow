//! Общие виджеты и форматирование.

use std::collections::VecDeque;

use eframe::egui::{self, Align, Color32, Layout, Pos2, RichText, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use crate::theme::*;

// ───────────────────────────── форматирование ─────────────────────────────

pub fn fmt_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    let b = b as f64;
    if b >= KB * KB * KB * KB {
        format!("{:.2} ТБ", b / (KB * KB * KB * KB))
    } else if b >= KB * KB * KB {
        format!("{:.1} ГБ", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} МБ", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} КБ", b / KB)
    } else {
        format!("{b:.0} Б")
    }
}

/// Скорость в байтах/с
pub fn fmt_rate(bps: f64) -> String {
    if bps < 1.0 {
        return "0 Б/с".to_owned();
    }
    format!("{}/с", fmt_bytes(bps as u64))
}

/// Скорость сети в битах/с, как в диспетчере задач Windows
pub fn fmt_bitrate(bytes_per_s: f64) -> String {
    let bits = bytes_per_s * 8.0;
    if bits >= 1e9 {
        format!("{:.2} Гбит/с", bits / 1e9)
    } else if bits >= 1e6 {
        format!("{:.1} Мбит/с", bits / 1e6)
    } else if bits >= 1e3 {
        format!("{:.0} Кбит/с", bits / 1e3)
    } else {
        format!("{bits:.0} бит/с")
    }
}

pub fn fmt_duration(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d} д {h:02}:{m:02}:{s:02}")
    } else {
        format!("{h}:{m:02}:{s:02}")
    }
}

pub fn fmt_pct(p: f64) -> String {
    if p < 0.05 {
        "0%".to_owned()
    } else if p < 10.0 {
        format!("{p:.1}%")
    } else {
        format!("{p:.0}%")
    }
}

/// Кодирование строки для подстановки в адрес запроса
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ───────────────────────────── виджеты ─────────────────────────────

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(BG_CARD)
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(14))
        .show(ui, add)
        .inner
}

/// Цвет фона ячейки таблицы по загрузке (0..1): чем выше, тем теплее — как в диспетчере задач
pub fn heat_bg(frac: f64) -> Color32 {
    let f = frac.clamp(0.0, 1.0);
    if f < 0.01 {
        return Color32::TRANSPARENT;
    }
    let (r, g, b) = if f < 0.6 { (245, 183, 49) } else { (235, 90, 70) };
    let alpha = (28.0 + f.powf(0.7) * 150.0) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// Подкрашивает фон ячейки таблицы и выводит число по правому краю
pub fn heat_cell(ui: &mut Ui, frac: f64, text: &str) {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, heat_bg(frac));
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(text).color(TEXT).size(13.0));
    });
}

/// Сегментированный переключатель; возвращает `true`, если значение изменилось
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, items: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (v, label) in items {
            let active = *value == *v;
            let btn = egui::Button::new(RichText::new(*label).size(13.0).color(if active { Color32::WHITE } else { TEXT_DIM }))
                .fill(if active { ACCENT.gamma_multiply(0.8) } else { BG_HOVER })
                .corner_radius(8.0)
                .min_size(vec2(0.0, 28.0));
            if ui.add(btn).clicked() && !active {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

/// Небольшой график-«спарклайн» без осей. `max` — верх шкалы; `None` — подбирается по данным
pub fn sparkline(
    ui: &mut Ui,
    vals: &VecDeque<f64>,
    size: Vec2,
    color: Color32,
    max: Option<f64>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    draw_spark(&ui.painter_at(rect), rect, vals, color, max, true);
    resp
}

pub fn draw_spark(
    p: &egui::Painter,
    rect: egui::Rect,
    vals: &VecDeque<f64>,
    color: Color32,
    max: Option<f64>,
    border: bool,
) {
    let top = max
        .unwrap_or_else(|| (vals.iter().copied().fold(0.0, f64::max) * 1.2).max(1.0))
        .max(1e-9);
    draw_grid(p, rect, color);
    draw_series(p, rect, vals, color, top, true);
    if border {
        p.rect_stroke(rect, 4.0, Stroke::new(1.0, color.gamma_multiply(0.35)), egui::StrokeKind::Inside);
    }
}

fn draw_grid(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    p.rect_filled(rect, 4.0, color.gamma_multiply(0.07));
    // горизонтальные линии на 25 / 50 / 75 %
    for k in 1..4 {
        let y = rect.bottom() - rect.height() * k as f32 / 4.0;
        p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, color.gamma_multiply(0.10)));
    }
}

/// Одна линия графика (с заливкой или без) на заданной шкале `0..=top`
fn draw_series(p: &egui::Painter, rect: egui::Rect, vals: &VecDeque<f64>, color: Color32, top: f64, fill: bool) {
    let n = vals.len();
    if n < 2 {
        return;
    }
    let pts: Vec<Pos2> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + rect.width() * i as f32 / (n - 1) as f32;
            let y = rect.bottom() - rect.height() * (*v / top).clamp(0.0, 1.0) as f32;
            pos2(x, y)
        })
        .collect();

    if fill {
        // заливка под графиком одним мешем (без швов между сегментами)
        let mut mesh = egui::Mesh::default();
        let (c_top, c_bot) = (color.gamma_multiply(0.30), color.gamma_multiply(0.02));
        for pt in &pts {
            mesh.colored_vertex(*pt, c_top);
            mesh.colored_vertex(pos2(pt.x, rect.bottom()), c_bot);
        }
        for i in 0..(n as u32 - 1) {
            let a = 2 * i;
            mesh.add_triangle(a, a + 1, a + 2);
            mesh.add_triangle(a + 1, a + 3, a + 2);
        }
        p.add(Shape::mesh(mesh));
    }
    p.add(Shape::line(pts, Stroke::new(1.8, color)));
}

/// Большой график с несколькими линиями на общей шкале `0..=top`.
/// `top_label` — подпись верхней границы шкалы (например «100%» или «5 МБ/с»).
pub fn chart(
    ui: &mut Ui,
    size: Vec2,
    series: &[(&VecDeque<f64>, Color32)],
    top: f64,
    top_label: &str,
    secs: usize,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    let base = series.first().map(|s| s.1).unwrap_or(ACCENT);
    draw_grid(&p, rect, base);
    // вертикальные линии раз в 10 секунд, как в диспетчере задач
    let n = series.first().map(|s| s.0.len()).unwrap_or(0);
    if n > 1 {
        let step = rect.width() * 10.0 / (n - 1) as f32;
        let mut x = rect.right() - step;
        while x > rect.left() {
            p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, base.gamma_multiply(0.07)));
            x -= step;
        }
    }
    let many = series.len() > 1;
    for (vals, color) in series {
        draw_series(&p, rect, vals, *color, top.max(1e-9), !many);
    }
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, base.gamma_multiply(0.45)), egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(11.5);
    p.text(rect.right_top() + vec2(-6.0, 4.0), egui::Align2::RIGHT_TOP, top_label, font.clone(), TEXT_DIM);
    p.text(rect.left_bottom() + vec2(6.0, -4.0), egui::Align2::LEFT_BOTTOM, format!("{secs} секунд"), font.clone(), TEXT_DIM);
    p.text(rect.right_bottom() + vec2(-6.0, -4.0), egui::Align2::RIGHT_BOTTOM, "0", font, TEXT_DIM);
    resp
}

/// Горизонтальная полоса заполнения с подписью внутри
pub fn bar(ui: &mut Ui, frac: f32, color: Color32, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 9.0, BG_HOVER);
    let mut fill = rect;
    fill.set_width((rect.width() * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { 6.0 } else { 0.0 }));
    p.rect_filled(fill, 9.0, color);
    p.text(
        rect.left_center() + vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(11.5),
        Color32::WHITE,
    );
}

/// Пара «подпись — значение» для панелей статистики
pub fn stat(ui: &mut Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(RichText::new(label).size(11.5).color(TEXT_DIM));
        ui.label(RichText::new(value).size(15.0).color(TEXT));
    });
}

/// Мелкая пометка (серый текст)
pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).size(12.0).color(TEXT_DIM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(fmt_bytes(0), "0 Б");
        assert_eq!(fmt_bytes(1536), "2 КБ");
        assert_eq!(fmt_bytes(5 * 1024 * 1024), "5.0 МБ");
        assert_eq!(fmt_bytes(3 * 1024 * 1024 * 1024), "3.0 ГБ");
    }

    #[test]
    fn rates() {
        assert_eq!(fmt_rate(0.0), "0 Б/с");
        assert_eq!(fmt_rate(2048.0), "2 КБ/с");
        assert_eq!(fmt_bitrate(125_000.0), "1.0 Мбит/с");
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(59), "0:00:59");
        assert_eq!(fmt_duration(3661), "1:01:01");
        assert_eq!(fmt_duration(90_061), "1 д 01:01:01");
    }

    #[test]
    fn percents() {
        assert_eq!(fmt_pct(0.0), "0%");
        assert_eq!(fmt_pct(3.14), "3.1%");
        assert_eq!(fmt_pct(42.6), "43%");
    }

    #[test]
    fn url_encoding() {
        assert_eq!(url_encode("svchost.exe"), "svchost.exe");
        assert_eq!(url_encode("a b&c"), "a%20b%26c");
        assert_eq!(url_encode("я"), "%D1%8F");
    }
}
