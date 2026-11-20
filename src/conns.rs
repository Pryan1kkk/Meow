//! Вкладка «Подключения»: TCP-соединения и UDP-порты с владельцами-процессами.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use sysinfo::Pid;

use crate::metrics::Metrics;
use crate::theme::*;
use crate::widgets::*;
use crate::winquery::{self, Connection, Job, JobState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Proto {
    All,
    Tcp,
    Udp,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StateFilter {
    All,
    Established,
    Listen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Process,
    Proto,
    Local,
    Remote,
    State,
}

const AUTO_REFRESH: Duration = Duration::from_secs(5);

pub struct ConnectionsTab {
    list: Vec<Connection>,
    job: Option<Job<Result<Vec<Connection>, String>>>,
    loaded_at: Option<Instant>,
    error: Option<String>,
    filter: String,
    proto: Proto,
    state: StateFilter,
    auto: bool,
    sort: SortBy,
    desc: bool,
    selected: Option<usize>,
}

fn state_ru(s: &str) -> &str {
    match s {
        "Established" => "Установлено",
        "Listen" => "Прослушивание",
        "TimeWait" => "Ожидание закрытия",
        "CloseWait" => "Ожидание закрытия (CloseWait)",
        "SynSent" => "SYN отправлен",
        "SynReceived" => "SYN получен",
        "FinWait1" | "FinWait2" => "Закрывается",
        "LastAck" => "Последний ACK",
        "Closing" => "Закрывается",
        "Closed" => "Закрыто",
        "Bound" => "Привязано",
        other => other,
    }
}

impl ConnectionsTab {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            job: None,
            loaded_at: None,
            error: None,
            filter: String::new(),
            proto: Proto::All,
            state: StateFilter::All,
            auto: true,
            sort: SortBy::Process,
            desc: false,
            selected: None,
        }
    }

    fn reload(&mut self) {
        if self.job.is_none() {
            self.job = Some(Job::spawn(winquery::fetch_connections));
        }
    }

    fn poll(&mut self) {
        if let Some(job) = &self.job {
            match job.poll() {
                JobState::Running => {}
                JobState::Done(res) => {
                    self.job = None;
                    self.loaded_at = Some(Instant::now());
                    match res {
                        Ok(v) => {
                            self.list = v;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                JobState::Failed => {
                    self.job = None;
                    self.loaded_at = Some(Instant::now());
                    self.error = Some("не удалось прочитать подключения (внутренняя ошибка)".to_owned());
                }
            }
        }
    }

    /// Возвращает PID, который пользователь попросил показать на вкладке «Процессы»
    pub fn ui(&mut self, ui: &mut Ui, m: &Metrics) -> Option<u32> {
        if !cfg!(windows) {
            ui.label(dim("Список подключений доступен только в Windows."));
            return None;
        }
        self.poll();
        let due = match self.loaded_at {
            None => true,
            Some(t) => self.auto && t.elapsed() >= AUTO_REFRESH,
        };
        if due {
            self.reload();
        }
        ui.ctx().request_repaint_after(Duration::from_millis(if self.job.is_some() { 120 } else { 500 }));

        let mut show_pid: Option<u32> = None;
        let mut reload = false;

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Поиск по процессу, адресу, порту")
                    .desired_width(280.0)
                    .margin(egui::Margin::symmetric(10, 7)),
            );
            segmented(ui, &mut self.proto, &[(Proto::All, "Все"), (Proto::Tcp, "TCP"), (Proto::Udp, "UDP")]);
            segmented(
                ui,
                &mut self.state,
                &[(StateFilter::All, "Любые"), (StateFilter::Established, "Установленные"), (StateFilter::Listen, "Прослушивание")],
            );
            ui.checkbox(&mut self.auto, "Автообновление");
            if ui.button("⟳  Обновить").clicked() {
                reload = true;
            }
            if self.job.is_some() {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).color(DANGER).size(13.0));
        }

        let proc_name = |pid: u32| -> String {
            match pid {
                0 => "Система".to_owned(),
                p => m
                    .sys
                    .process(Pid::from_u32(p))
                    .map(|pr| pr.name().to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("PID {p}")),
            }
        };

        let f = self.filter.trim().to_lowercase();
        let mut rows: Vec<(usize, String)> = self
            .list
            .iter()
            .enumerate()
            .filter(|(_, c)| match self.proto {
                Proto::All => true,
                Proto::Tcp => c.proto == "TCP",
                Proto::Udp => c.proto == "UDP",
            })
            .filter(|(_, c)| match self.state {
                StateFilter::All => true,
                StateFilter::Established => c.state == "Established",
                StateFilter::Listen => c.state == "Listen",
            })
            .map(|(i, c)| (i, proc_name(c.pid)))
            .filter(|(i, name)| {
                if f.is_empty() {
                    return true;
                }
                let c = &self.list[*i];
                name.to_lowercase().contains(&f)
                    || c.local_addr.contains(&f)
                    || c.remote_addr.contains(&f)
                    || c.local_port.to_string().contains(&f)
                    || c.remote_port.to_string().contains(&f)
                    || c.pid.to_string() == f
            })
            .collect();

        let (sort, desc) = (self.sort, self.desc);
        rows.sort_by(|(ia, na), (ib, nb)| {
            let (a, b) = (&self.list[*ia], &self.list[*ib]);
            let ord = match sort {
                SortBy::Process => na.to_lowercase().cmp(&nb.to_lowercase()),
                SortBy::Proto => a.proto.cmp(b.proto),
                SortBy::Local => (&a.local_addr, a.local_port).cmp(&(&b.local_addr, b.local_port)),
                SortBy::Remote => (&a.remote_addr, a.remote_port).cmp(&(&b.remote_addr, b.remote_port)),
                SortBy::State => a.state.cmp(&b.state),
            };
            let ord = if desc { ord.reverse() } else { ord };
            ord.then(a.local_port.cmp(&b.local_port)).then(a.remote_port.cmp(&b.remote_port))
        });

        let mut new_sort: Option<SortBy> = None;
        let mut select: Option<usize> = None;
        let sel = self.selected;
        let count = rows.len();

        egui::Frame::new()
            .fill(BG_CARD)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                TableBuilder::new(ui)
                    .resizable(true)
                    .sense(Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .max_scroll_height(10_000.0)
                    .column(Column::remainder().at_least(180.0).clip(true))
                    .column(Column::initial(60.0).at_least(40.0))
                    .column(Column::initial(200.0).at_least(100.0).clip(true))
                    .column(Column::initial(200.0).at_least(100.0).clip(true))
                    .column(Column::initial(150.0).at_least(80.0).clip(true))
                    .header(30.0, |mut header| {
                        for (t, by) in [
                            ("Процесс", SortBy::Process),
                            ("Прот.", SortBy::Proto),
                            ("Локальный адрес", SortBy::Local),
                            ("Удалённый адрес", SortBy::Remote),
                            ("Состояние", SortBy::State),
                        ] {
                            header.col(|ui| {
                                let arrow = if sort == by { if desc { " ▼" } else { " ▲" } } else { "" };
                                let color = if sort == by { ACCENT } else { TEXT_DIM };
                                let text = RichText::new(format!("{t}{arrow}")).strong().size(12.5).color(color);
                                if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() {
                                    new_sort = Some(by);
                                }
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(26.0, rows.len(), |mut row| {
                            let (idx, name) = &rows[row.index()];
                            let c = &self.list[*idx];
                            row.set_selected(sel == Some(*idx));
                            row.col(|ui| {
                                ui.add_space(6.0);
                                ui.label(RichText::new(name).color(TEXT));
                                ui.label(dim(format!("({})", c.pid)));
                            });
                            row.col(|ui| {
                                let color = if c.proto == "TCP" { CYAN } else { PURPLE };
                                ui.label(RichText::new(c.proto).color(color).size(12.5));
                            });
                            row.col(|ui| {
                                ui.label(dim(format!("{}:{}", c.local_addr, c.local_port)));
                            });
                            row.col(|ui| {
                                if c.proto == "TCP" && !c.remote_addr.is_empty() && c.remote_port != 0 {
                                    ui.label(dim(format!("{}:{}", c.remote_addr, c.remote_port)));
                                }
                            });
                            row.col(|ui| {
                                let color = if c.state == "Established" { GREEN } else { TEXT_DIM };
                                ui.label(RichText::new(state_ru(&c.state)).color(color).size(12.5));
                            });
                            let resp = row.response();
                            if resp.clicked() || resp.secondary_clicked() {
                                select = Some(*idx);
                            }
                            resp.context_menu(|ui| {
                                select = Some(*idx);
                                if ui.add_enabled(c.pid != 0, egui::Button::new("Показать в процессах")).clicked() {
                                    show_pid = Some(c.pid);
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Копировать локальный адрес").clicked() {
                                    ui.ctx().copy_text(format!("{}:{}", c.local_addr, c.local_port));
                                    ui.close();
                                }
                                if ui.add_enabled(c.remote_port != 0, egui::Button::new("Копировать удалённый адрес")).clicked() {
                                    ui.ctx().copy_text(format!("{}:{}", c.remote_addr, c.remote_port));
                                    ui.close();
                                }
                            });
                        });
                    });
            });
        ui.add_space(4.0);
        ui.label(dim(format!("Показано: {count} из {}", self.list.len())));

        if let Some(by) = new_sort {
            if self.sort == by {
                self.desc = !self.desc;
            } else {
                self.sort = by;
                self.desc = false;
            }
        }
        if select.is_some() {
            self.selected = select;
        }
        if reload {
            self.reload();
        }
        show_pid
    }
}
