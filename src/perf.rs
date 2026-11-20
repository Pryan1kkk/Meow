//! Вкладка «Производительность»: слева список ресурсов с мини-графиками,
//! справа большой график и подробная статистика выбранного ресурса.

use std::collections::VecDeque;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui, vec2};
use sysinfo::System;

use crate::metrics::{HISTORY_LEN, Metrics};
use crate::theme::*;
use crate::widgets::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PerfSel {
    Cpu,
    Mem,
    Disk,
    Net,
    Gpu,
}

pub struct PerfTab {
    pub sel: PerfSel,
    pub per_core: bool,
}

const LIST_WIDTH: f32 = 250.0;

impl PerfTab {
    pub fn new() -> Self {
        Self { sel: PerfSel::Cpu, per_core: false }
    }

    pub fn ui(&mut self, ui: &mut Ui, m: &Metrics, proc_count: usize) {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(LIST_WIDTH);
                self.list(ui, m);
            });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.sel {
                    PerfSel::Cpu => self.cpu_pane(ui, m, proc_count),
                    PerfSel::Mem => self.mem_pane(ui, m),
                    PerfSel::Disk => self.disk_pane(ui, m),
                    PerfSel::Net => self.net_pane(ui, m),
                    PerfSel::Gpu => self.gpu_pane(ui, m),
                });
            });
        });
    }

    // ───────────── список ресурсов ─────────────

    fn list(&mut self, ui: &mut Ui, m: &Metrics) {
        let freq = m.sys.cpus().first().map(|c| c.frequency()).unwrap_or(0) as f64 / 1000.0;
        let items: Vec<(PerfSel, &str, String, &VecDeque<f64>, Option<f64>, Color32)> = vec![
            (PerfSel::Cpu, "ЦП", format!("{:.0}%  {:.2} ГГц", m.cpu_total, freq), m.cpu.values(), Some(100.0), ACCENT),
            (
                PerfSel::Mem,
                "Память",
                format!("{} / {}  ({:.0}%)", fmt_bytes(m.mem_used()), fmt_bytes(m.mem_total()), m.mem_pct()),
                m.mem.values(),
                Some(100.0),
                PURPLE,
            ),
            (
                PerfSel::Disk,
                "Диск",
                format!("Ч {}  З {}", fmt_rate(m.disk_read_rate), fmt_rate(m.disk_write_rate)),
                m.disk_read.values(),
                None,
                GREEN,
            ),
            (
                PerfSel::Net,
                "Сеть",
                format!("↓ {}  ↑ {}", fmt_bitrate(m.net_rx_rate), fmt_bitrate(m.net_tx_rate)),
                m.net_rx.values(),
                None,
                CYAN,
            ),
            (
                PerfSel::Gpu,
                "GPU",
                match m.gpu_usage {
                    Some(u) => format!("{u:.0}%"),
                    None => "нет данных".to_owned(),
                },
                m.gpu.values(),
                Some(100.0),
                AMBER,
            ),
        ];
        for (sel, title, sub, hist, max, color) in items {
            if self.resource_item(ui, self.sel == sel, title, &sub, hist, max, color) {
                self.sel = sel;
            }
            ui.add_space(4.0);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resource_item(
        &self,
        ui: &mut Ui,
        selected: bool,
        title: &str,
        sub: &str,
        hist: &VecDeque<f64>,
        max: Option<f64>,
        color: Color32,
    ) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(LIST_WIDTH, 66.0), Sense::click());
        let p = ui.painter_at(rect);
        let bg = if selected {
            color.gamma_multiply(0.16)
        } else if resp.hovered() {
            BG_HOVER
        } else {
            BG_CARD
        };
        p.rect_filled(rect, 10.0, bg);
        if selected {
            p.rect_filled(egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 12.0), vec2(3.0, rect.height() - 24.0)), 2.0, color);
        }
        let spark = egui::Rect::from_min_size(rect.left_top() + vec2(12.0, 11.0), vec2(72.0, 44.0));
        draw_spark(&p, spark, hist, color, max, true);
        p.text(
            rect.left_top() + vec2(98.0, 14.0),
            egui::Align2::LEFT_TOP,
            title,
            egui::FontId::proportional(14.5),
            TEXT,
        );
        // подпись может быть длинной: обрезаем по ширине карточки
        let sub_p = ui.painter_at(egui::Rect::from_min_max(rect.left_top() + vec2(98.0, 0.0), rect.right_bottom() - vec2(6.0, 0.0)));
        sub_p.text(
            rect.left_top() + vec2(98.0, 36.0),
            egui::Align2::LEFT_TOP,
            sub,
            egui::FontId::proportional(11.5),
            TEXT_DIM,
        );
        resp.clicked()
    }

    // ───────────── панели ─────────────

    fn header(ui: &mut Ui, title: &str, subtitle: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(22.0).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(subtitle).size(13.0).color(TEXT_DIM));
            });
        });
        ui.add_space(8.0);
    }

    fn chart_height(ui: &Ui) -> f32 {
        (ui.available_height() * 0.45).clamp(180.0, 320.0)
    }

    fn cpu_pane(&mut self, ui: &mut Ui, m: &Metrics, proc_count: usize) {
        let brand = m.sys.cpus().first().map(|c| c.brand().trim().to_owned()).unwrap_or_default();
        Self::header(ui, "ЦП", &brand);

        ui.horizontal(|ui| {
            ui.label(dim("Загрузка за последние 90 секунд"));
            ui.add_space(12.0);
            segmented(ui, &mut self.per_core, &[(false, "Общая"), (true, "По ядрам")]);
        });
        ui.add_space(6.0);

        let w = ui.available_width();
        if self.per_core {
            let n = m.core_hist.len().max(1);
            let cols = ((n as f64).sqrt().ceil() as usize).clamp(1, 8);
            let gap = 8.0;
            let cell_w = (w - gap * (cols as f32 - 1.0)) / cols as f32;
            let cell_h = if n > 16 { 54.0 } else { 70.0 };
            for chunk in m.core_hist.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (k, h) in chunk.1.iter().enumerate() {
                        let idx = chunk.0 * cols + k;
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let cur = m.cores.get(idx).copied().unwrap_or(0.0);
                            ui.label(dim(format!("ЦП {idx}  ·  {cur:.0}%")));
                            sparkline(ui, h.values(), vec2(cell_w, cell_h), heat(cur as f64).gamma_multiply(0.9), Some(100.0));
                        });
                    }
                });
                ui.add_space(6.0);
            }
        } else {
            chart(ui, vec2(w, Self::chart_height(ui)), &[(m.cpu.values(), ACCENT)], 100.0, "100%", HISTORY_LEN);
        }

        ui.add_space(12.0);
        let freq = m.sys.cpus().first().map(|c| c.frequency()).unwrap_or(0) as f64 / 1000.0;
        let phys = System::physical_core_count().map(|c| c.to_string()).unwrap_or_else(|_| "—".into());
        let uptime = System::uptime().unwrap_or(0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                stat(ui, "Загрузка", &format!("{:.0}%", m.cpu_total));
                stat(ui, "Скорость", &format!("{freq:.2} ГГц"));
                stat(ui, "Процессов", &proc_count.to_string());
                stat(ui, "Логических ядер", &m.cores.len().to_string());
                stat(ui, "Физических ядер", &phys);
                stat(ui, "Время работы системы", &fmt_duration(uptime));
            });
        });
    }

    fn mem_pane(&mut self, ui: &mut Ui, m: &Metrics) {
        Self::header(ui, "Память", &fmt_bytes(m.mem_total()));
        ui.label(dim("Использование памяти"));
        ui.add_space(4.0);
        let w = ui.available_width();
        chart(ui, vec2(w, Self::chart_height(ui)), &[(m.mem.values(), PURPLE)], 100.0, &fmt_bytes(m.mem_total()), HISTORY_LEN);

        ui.add_space(12.0);
        let (used, total) = (m.mem_used(), m.mem_total());
        let avail = m.sys.available_memory();
        let (swap_used, swap_total) = (m.sys.used_swap(), m.sys.total_swap());
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                stat(ui, "Используется", &fmt_bytes(used));
                stat(ui, "Доступно", &fmt_bytes(avail));
                stat(ui, "Всего", &fmt_bytes(total));
                stat(ui, "Загрузка", &format!("{:.0}%", m.mem_pct()));
                stat(ui, "Файл подкачки", &format!("{} / {}", fmt_bytes(swap_used), fmt_bytes(swap_total)));
            });
            ui.add_space(10.0);
            bar(ui, (used as f64 / total.max(1) as f64) as f32, PURPLE, &format!("Используется {}", fmt_bytes(used)), ui.available_width());
            if swap_total > 0 {
                ui.add_space(6.0);
                bar(ui, (swap_used as f64 / swap_total as f64) as f32, PINK, &format!("Подкачка {}", fmt_bytes(swap_used)), ui.available_width());
            }
        });
    }

    fn disk_pane(&mut self, ui: &mut Ui, m: &Metrics) {
        Self::header(ui, "Диск", &format!("{} томов", m.disks_info.len()));
        ui.horizontal(|ui| {
            ui.label(RichText::new("● чтение").color(GREEN).size(12.0));
            ui.label(RichText::new("● запись").color(AMBER).size(12.0));
        });
        ui.add_space(4.0);
        let top = m.disk_read.max().max(m.disk_write.max()).max(1_000_000.0) * 1.2;
        let w = ui.available_width();
        chart(
            ui,
            vec2(w, Self::chart_height(ui)),
            &[(m.disk_read.values(), GREEN), (m.disk_write.values(), AMBER)],
            top,
            &fmt_rate(top),
            HISTORY_LEN,
        );

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                stat(ui, "Скорость чтения", &fmt_rate(m.disk_read_rate));
                stat(ui, "Скорость записи", &fmt_rate(m.disk_write_rate));
            });
        });
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.label(RichText::new("Тома").strong().size(14.0).color(TEXT));
            ui.add_space(6.0);
            egui::Grid::new("disks").num_columns(4).spacing([18.0, 8.0]).show(ui, |ui| {
                for d in &m.disks_info {
                    let used = d.total.saturating_sub(d.free);
                    let frac = used as f64 / d.total.max(1) as f64;
                    ui.label(RichText::new(&d.mount).color(TEXT));
                    ui.label(dim(format!("{} · {}", d.fs, d.kind)));
                    bar(ui, frac as f32, heat(frac * 100.0), &format!("{:.0}%", frac * 100.0), 160.0);
                    ui.label(dim(format!("свободно {} из {}", fmt_bytes(d.free), fmt_bytes(d.total))));
                    ui.end_row();
                }
            });
        });
    }

    fn net_pane(&mut self, ui: &mut Ui, m: &Metrics) {
        Self::header(ui, "Сеть", &format!("{} интерфейсов", m.nets_info.len()));
        ui.horizontal(|ui| {
            ui.label(RichText::new("● приём").color(CYAN).size(12.0));
            ui.label(RichText::new("● передача").color(PINK).size(12.0));
        });
        ui.add_space(4.0);
        let top = m.net_rx.max().max(m.net_tx.max()).max(125_000.0) * 1.2;
        let w = ui.available_width();
        chart(
            ui,
            vec2(w, Self::chart_height(ui)),
            &[(m.net_rx.values(), CYAN), (m.net_tx.values(), PINK)],
            top,
            &fmt_bitrate(top),
            HISTORY_LEN,
        );

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                stat(ui, "Приём", &fmt_bitrate(m.net_rx_rate));
                stat(ui, "Передача", &fmt_bitrate(m.net_tx_rate));
                stat(ui, "Всего принято", &fmt_bytes(m.nets_info.iter().map(|n| n.rx_total).sum()));
                stat(ui, "Всего передано", &fmt_bytes(m.nets_info.iter().map(|n| n.tx_total).sum()));
            });
        });
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.label(RichText::new("Интерфейсы").strong().size(14.0).color(TEXT));
            ui.add_space(6.0);
            egui::Grid::new("nets").num_columns(5).spacing([18.0, 6.0]).show(ui, |ui| {
                for h in ["Имя", "MAC", "Приём", "Передача", "Принято / передано"] {
                    ui.label(dim(h));
                }
                ui.end_row();
                for n in &m.nets_info {
                    ui.label(RichText::new(&n.name).color(TEXT));
                    ui.label(dim(&n.mac));
                    ui.label(RichText::new(fmt_bitrate(n.rx_rate)).color(CYAN));
                    ui.label(RichText::new(fmt_bitrate(n.tx_rate)).color(PINK));
                    ui.label(dim(format!("{} / {}", fmt_bytes(n.rx_total), fmt_bytes(n.tx_total))));
                    ui.end_row();
                }
            });
        });
    }

    fn gpu_pane(&mut self, ui: &mut Ui, m: &Metrics) {
        let sub = m.gpus_info.first().map(|g| g.name.clone()).unwrap_or_default();
        Self::header(ui, "GPU", &sub);
        if m.gpus_info.is_empty() {
            card(ui, |ui| {
                ui.label(dim("Видеокарты не найдены или система не отдаёт данные о них."));
            });
            return;
        }
        ui.label(dim("Загрузка (максимум по всем видеокартам)"));
        ui.add_space(4.0);
        let w = ui.available_width();
        chart(ui, vec2(w, Self::chart_height(ui)), &[(m.gpu.values(), AMBER)], 100.0, "100%", HISTORY_LEN);
        ui.add_space(12.0);
        for g in &m.gpus_info {
            card(ui, |ui| {
                ui.label(RichText::new(&g.name).strong().size(14.5).color(TEXT));
                if !g.vendor.is_empty() {
                    ui.label(dim(&g.vendor));
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    stat(ui, "Загрузка", &g.usage.map(|u| format!("{u:.0}%")).unwrap_or_else(|| "—".into()));
                    if let (Some(u), Some(t)) = (g.mem_used, g.mem_total) {
                        stat(ui, "Видеопамять", &format!("{} / {}", fmt_bytes(u), fmt_bytes(t)));
                    }
                });
                if let (Some(u), Some(t)) = (g.mem_used, g.mem_total) {
                    ui.add_space(8.0);
                    bar(ui, (u as f64 / t.max(1) as f64) as f32, AMBER, &format!("VRAM {}", fmt_bytes(u)), ui.available_width());
                }
            });
            ui.add_space(8.0);
        }
    }
}
