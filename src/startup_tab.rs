//! Вкладка «Автозагрузка»: интерфейс поверх `startup.rs`.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, RichText, Ui};
use egui_extras::{Column, TableBuilder};

use crate::startup::{self, StartupEntry};
use crate::theme::*;
use crate::widgets::*;

pub struct StartupTab {
    list: Vec<StartupEntry>,
    loaded: bool,
    filter: String,
    status: Option<(String, Instant)>,
}

impl StartupTab {
    pub fn new() -> Self {
        Self { list: Vec::new(), loaded: false, filter: String::new(), status: None }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if !self.loaded {
            self.list = startup::list();
            self.loaded = true;
        }
        let mut reload = false;
        let mut toggle: Option<(usize, bool)> = None;

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Поиск по имени или команде")
                    .desired_width(280.0)
                    .margin(egui::Margin::symmetric(10, 7)),
            );
            if ui.button("⟳  Обновить").clicked() {
                reload = true;
            }
            let enabled = self.list.iter().filter(|e| e.enabled).count();
            ui.label(dim(format!("Записей: {} · включено: {enabled}", self.list.len())));
            if let Some((msg, t)) = &self.status {
                if t.elapsed() < Duration::from_secs(8) {
                    ui.label(dim(msg.clone()));
                }
            }
        });
        ui.add_space(10.0);

        if self.list.is_empty() {
            let text = if cfg!(windows) { "В автозагрузке ничего нет" } else { "Автозагрузка доступна только в Windows" };
            ui.label(dim(text));
        } else {
            let f = self.filter.trim().to_lowercase();
            let visible: Vec<usize> = (0..self.list.len())
                .filter(|i| {
                    let e = &self.list[*i];
                    f.is_empty() || e.name.to_lowercase().contains(&f) || e.command.to_lowercase().contains(&f)
                })
                .collect();

            egui::Frame::new()
                .fill(BG_CARD)
                .corner_radius(12.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    TableBuilder::new(ui)
                        .resizable(true)
                        .cell_layout(Layout::left_to_right(Align::Center))
                        .max_scroll_height(10_000.0)
                        .column(Column::initial(210.0).at_least(80.0).clip(true))
                        .column(Column::remainder().at_least(200.0).clip(true))
                        .column(Column::initial(260.0).at_least(100.0).clip(true))
                        .column(Column::initial(130.0).at_least(100.0))
                        .header(30.0, |mut header| {
                            for label in ["Имя", "Команда", "Источник", "Состояние"] {
                                header.col(|ui| {
                                    ui.label(RichText::new(label).strong().size(12.5).color(TEXT_DIM));
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(32.0, visible.len(), |mut row| {
                                let idx = visible[row.index()];
                                let e = &self.list[idx];
                                row.col(|ui| {
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(&e.name).color(TEXT));
                                });
                                row.col(|ui| {
                                    ui.label(dim(&e.command)).on_hover_text(&e.command);
                                });
                                row.col(|ui| {
                                    ui.label(dim(e.source));
                                });
                                row.col(|ui| {
                                    let mut on = e.enabled;
                                    let label = if on { "Включено" } else { "Выключено" };
                                    if ui.checkbox(&mut on, label).changed() {
                                        toggle = Some((idx, on));
                                    }
                                });
                            });
                        });
                });
        }

        if let Some((i, on)) = toggle {
            let name = self.list[i].name.clone();
            let msg = match startup::set_enabled(&self.list[i], on) {
                Ok(()) => {
                    self.list[i].enabled = on;
                    format!("«{name}»: {}", if on { "включено" } else { "отключено" })
                }
                Err(e) => format!("Не удалось изменить «{name}»: {e}"),
            };
            self.status = Some((msg, Instant::now()));
        }
        if reload {
            self.list = startup::list();
        }
    }
}
