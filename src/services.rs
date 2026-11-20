//! Вкладка «Службы»: список служб Windows, запуск / остановка / перезапуск.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui, vec2};
use egui_extras::{Column, TableBuilder};

use crate::theme::*;
use crate::widgets::*;
use crate::winquery::{self, Job, JobState, Service, ServiceAction};

pub struct ServicesTab {
    list: Vec<Service>,
    load: Option<Job<Result<Vec<Service>, String>>>,
    act: Option<Job<(String, Result<(), String>)>>,
    loaded: bool,
    error: Option<String>,
    filter: String,
    only_running: bool,
    selected: Option<String>,
    status: Option<(String, Instant)>,
}

fn state_ru(s: &str) -> &str {
    match s {
        "Running" => "Работает",
        "Stopped" => "Остановлена",
        "Start Pending" => "Запускается",
        "Stop Pending" => "Останавливается",
        "Paused" => "Приостановлена",
        "Continue Pending" => "Возобновляется",
        "Pause Pending" => "Приостанавливается",
        other => other,
    }
}

fn mode_ru(s: &str) -> &str {
    match s {
        "Auto" => "Автоматически",
        "Manual" => "Вручную",
        "Disabled" => "Отключена",
        "Boot" | "System" => "Системная",
        other => other,
    }
}

impl ServicesTab {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            load: None,
            act: None,
            loaded: false,
            error: None,
            filter: String::new(),
            only_running: false,
            selected: None,
            status: None,
        }
    }

    fn reload(&mut self) {
        if self.load.is_none() {
            self.load = Some(Job::spawn(winquery::fetch_services));
        }
    }

    fn run(&mut self, name: String, action: ServiceAction) {
        if self.act.is_some() {
            return;
        }
        let verb = match action {
            ServiceAction::Start => "запущена",
            ServiceAction::Stop => "остановлена",
            ServiceAction::Restart => "перезапущена",
        };
        self.status = Some((format!("Выполняю: «{name}»…"), Instant::now()));
        self.act = Some(Job::spawn(move || {
            let res = winquery::service_action(&name, action);
            (format!("Служба «{name}» {verb}"), res)
        }));
    }

    fn poll(&mut self) {
        if let Some(job) = &self.load {
            match job.poll() {
                JobState::Running => {}
                JobState::Done(res) => {
                    self.load = None;
                    match res {
                        Ok(v) => {
                            self.list = v;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                JobState::Failed => {
                    self.load = None;
                    self.error = Some("не удалось прочитать список служб (внутренняя ошибка)".to_owned());
                }
            }
        }
        if let Some(job) = &self.act {
            let finished = match job.poll() {
                JobState::Running => None,
                JobState::Done((ok_msg, res)) => Some(match res {
                    Ok(()) => ok_msg,
                    Err(e) => format!("Не удалось: {e}"),
                }),
                JobState::Failed => Some("Не удалось: внутренняя ошибка".to_owned()),
            };
            if let Some(msg) = finished {
                self.act = None;
                self.status = Some((msg, Instant::now()));
                self.reload();
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if !cfg!(windows) {
            ui.label(dim("Службы Windows доступны только в Windows."));
            return;
        }
        self.poll();
        if !self.loaded {
            self.loaded = true;
            self.reload();
        }
        let busy = self.load.is_some() || self.act.is_some();
        if busy {
            ui.ctx().request_repaint_after(Duration::from_millis(120));
        }

        let selected = self.selected.as_ref().and_then(|n| self.list.iter().find(|s| &s.name == n)).cloned();
        let mut do_action: Option<(String, ServiceAction)> = None;
        let mut reload = false;

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Поиск по имени или описанию")
                    .desired_width(280.0)
                    .margin(egui::Margin::symmetric(10, 7)),
            );
            ui.checkbox(&mut self.only_running, "Только работающие");
            if ui.button("⟳  Обновить").clicked() {
                reload = true;
            }
            if busy {
                ui.spinner();
            }
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            let run = selected.as_ref().map(|s| s.is_running());
            let mk = |text: &str, color: Color32, enabled: bool| {
                (egui::Button::new(RichText::new(text).color(Color32::WHITE).size(13.0))
                    .fill(if enabled { color } else { BG_CARD })
                    .corner_radius(8.0)
                    .min_size(vec2(0.0, 30.0)), enabled)
            };
            let has = selected.is_some() && self.act.is_none();
            let (b, en) = mk("▶  Запустить", GREEN.gamma_multiply(0.7), has && run == Some(false));
            if ui.add_enabled(en, b).clicked() {
                do_action = selected.as_ref().map(|s| (s.name.clone(), ServiceAction::Start));
            }
            let (b, en) = mk("■  Остановить", DANGER, has && run == Some(true));
            if ui.add_enabled(en, b).clicked() {
                do_action = selected.as_ref().map(|s| (s.name.clone(), ServiceAction::Stop));
            }
            let (b, en) = mk("⟳  Перезапустить", BG_HOVER, has);
            if ui.add_enabled(en, b).clicked() {
                do_action = selected.as_ref().map(|s| (s.name.clone(), ServiceAction::Restart));
            }
            if let Some((msg, t)) = &self.status {
                if t.elapsed() < Duration::from_secs(10) || self.act.is_some() {
                    ui.add_space(8.0);
                    ui.label(dim(msg.clone()));
                }
            }
        });
        ui.add_space(10.0);

        if let Some(e) = &self.error {
            ui.label(RichText::new(e).color(DANGER).size(13.0));
        }

        let f = self.filter.trim().to_lowercase();
        let visible: Vec<&Service> = self
            .list
            .iter()
            .filter(|s| !self.only_running || s.is_running())
            .filter(|s| {
                f.is_empty()
                    || s.name.to_lowercase().contains(&f)
                    || s.display.to_lowercase().contains(&f)
                    || s.description.to_lowercase().contains(&f)
            })
            .collect();

        let mut select: Option<String> = None;
        let sel_name = self.selected.clone();
        let desc_height = if selected.is_some() { 64.0 } else { 0.0 };

        egui::Frame::new()
            .fill(BG_CARD)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                // высота списка: всё доступное место минус блок с описанием снизу
                let list_height = (ui.available_height() - desc_height - 24.0).max(120.0);
                TableBuilder::new(ui)
                    .resizable(true)
                    .sense(Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .max_scroll_height(list_height)
                    .column(Column::remainder().at_least(200.0).clip(true))
                    .column(Column::initial(170.0).at_least(80.0).clip(true))
                    .column(Column::initial(70.0).at_least(50.0))
                    .column(Column::initial(130.0).at_least(80.0))
                    .column(Column::initial(130.0).at_least(80.0))
                    .header(30.0, |mut header| {
                        for t in ["Название", "Имя службы", "PID", "Состояние", "Тип запуска"] {
                            header.col(|ui| {
                                ui.label(RichText::new(t).strong().size(12.5).color(TEXT_DIM));
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(28.0, visible.len(), |mut row| {
                            let s = visible[row.index()];
                            row.set_selected(sel_name.as_deref() == Some(s.name.as_str()));
                            row.col(|ui| {
                                ui.add_space(6.0);
                                ui.label(RichText::new(&s.display).color(TEXT));
                            });
                            row.col(|ui| {
                                ui.label(dim(&s.name));
                            });
                            row.col(|ui| {
                                if s.pid != 0 {
                                    ui.label(RichText::new(s.pid.to_string()).monospace().color(TEXT_DIM));
                                }
                            });
                            row.col(|ui| {
                                let color = if s.is_running() { GREEN } else { TEXT_DIM };
                                ui.label(RichText::new(format!("● {}", state_ru(&s.state))).color(color).size(12.5));
                            });
                            row.col(|ui| {
                                ui.label(dim(mode_ru(&s.start_mode)));
                            });
                            let resp = row.response();
                            if resp.clicked() || resp.secondary_clicked() {
                                select = Some(s.name.clone());
                            }
                            resp.context_menu(|ui| {
                                select = Some(s.name.clone());
                                if ui.add_enabled(!s.is_running(), egui::Button::new("Запустить")).clicked() {
                                    do_action = Some((s.name.clone(), ServiceAction::Start));
                                    ui.close();
                                }
                                if ui.add_enabled(s.is_running(), egui::Button::new("Остановить")).clicked() {
                                    do_action = Some((s.name.clone(), ServiceAction::Stop));
                                    ui.close();
                                }
                                if ui.button("Перезапустить").clicked() {
                                    do_action = Some((s.name.clone(), ServiceAction::Restart));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Копировать имя службы").clicked() {
                                    ui.ctx().copy_text(s.name.clone());
                                    ui.close();
                                }
                            });
                        });
                    });
            });

        if let Some(s) = &selected {
            ui.add_space(8.0);
            card(ui, |ui| {
                ui.set_height(desc_height - 28.0);
                ui.label(RichText::new(&s.display).strong().color(TEXT));
                if s.description.is_empty() {
                    ui.label(dim("Описание отсутствует"));
                } else {
                    ui.add(egui::Label::new(dim(&s.description)).truncate());
                }
            });
        }

        if let Some(n) = select {
            self.selected = Some(n);
        }
        if let Some((name, a)) = do_action {
            self.run(name, a);
        }
        if reload {
            self.reload();
        }
    }
}
