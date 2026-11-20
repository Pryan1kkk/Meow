// Скрывает чёрное окно консоли в release-сборке под Windows
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod boot;
mod conns;
mod install;
mod metrics;
mod perf;
mod procs;
mod services;
mod settings;
mod startup;
mod startup_tab;
mod theme;
mod tray;
mod widgets;
mod winproc;
mod winquery;

use std::time::Duration;

use eframe::egui::{self, Color32, Key, Modifiers, RichText, Sense, Vec2, vec2};
use sysinfo::System;

use conns::ConnectionsTab;
use metrics::Metrics;
use perf::PerfTab;
use procs::ProcessesTab;
use services::ServicesTab;
use settings::Settings;
use startup_tab::StartupTab;
use theme::*;
use widgets::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Processes,
    Performance,
    Services,
    Connections,
    Startup,
    Settings,
}

impl Tab {
    const NAV: [Tab; 5] = [Tab::Processes, Tab::Performance, Tab::Services, Tab::Connections, Tab::Startup];

    fn title(self) -> &'static str {
        match self {
            Tab::Processes => "Процессы",
            Tab::Performance => "Производительность",
            Tab::Services => "Службы",
            Tab::Connections => "Подключения",
            Tab::Startup => "Автозагрузка",
            Tab::Settings => "Настройки",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Tab::Processes => "▤",
            Tab::Performance => "◔",
            Tab::Services => "⚙",
            Tab::Connections => "⇄",
            Tab::Startup => "⏻",
            Tab::Settings => "☰",
        }
    }
}

// ───────────────────────────── «Новая задача» ─────────────────────────────

struct RunDialog {
    text: String,
    admin: bool,
    error: Option<String>,
    focus: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Строка в одинарных кавычках для PowerShell: внутри кавычек особый смысл имеет только сама кавычка
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn run_command(text: &str, admin: bool) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("введи имя программы, путь, папку или адрес".to_owned());
    }
    let mut cmd;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if admin {
            // запуск с повышением прав: окно запроса UAC покажет сама Windows
            cmd = std::process::Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &format!("Start-Process -Verb RunAs -FilePath {}", ps_quote(text)),
            ]);
        } else {
            // `start` ведёт себя как «Win+R»: понимает программы из PATH, папки, файлы и ссылки
            cmd = std::process::Command::new("cmd");
            cmd.raw_arg(format!("/C start \"\" {text}"));
        }
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = admin;
        cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(text);
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

// ───────────────────────────── приложение ─────────────────────────────

struct App {
    m: Metrics,
    settings: Settings,
    saved: Settings,
    tab: Tab,
    procs: ProcessesTab,
    perf: PerfTab,
    services: ServicesTab,
    conns: ConnectionsTab,
    startup: StartupTab,
    tray: Option<tray::Tray>,
    run: Option<RunDialog>,
    manual_refresh: bool,
    applied_on_top: bool,
    /// Сохранять ли настройки на диск (в тестах выключено)
    persist_enabled: bool,
    replace: install::State,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        Self::build(&cc.egui_ctx, Settings::load(), true)
    }

    fn build(ctx: &egui::Context, settings: Settings, with_tray: bool) -> Result<Self, String> {
        apply_theme(ctx);
        let m = Metrics::new()?;
        let mut procs = ProcessesTab::new(&settings);
        procs.on_refresh(&m);
        Ok(Self {
            m,
            saved: settings.clone(),
            applied_on_top: settings.always_on_top,
            procs,
            perf: PerfTab::new(),
            services: ServicesTab::new(),
            conns: ConnectionsTab::new(),
            startup: StartupTab::new(),
            tray: if with_tray { tray::create() } else { None },
            run: None,
            manual_refresh: false,
            persist_enabled: true,
            replace: install::State::new(),
            settings,
            tab: Tab::Processes,
        })
    }

    // ───────────── цикл обновления ─────────────

    fn hotkeys(&mut self, ctx: &egui::Context) {
        let (find, new_task, f5, digits) = ctx.input_mut(|i| {
            let digits: Vec<bool> = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6]
                .iter()
                .map(|k| i.consume_key(Modifiers::COMMAND, *k))
                .collect();
            (
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::NONE, Key::F5),
                digits,
            )
        });
        if find {
            self.tab = Tab::Processes;
            self.procs.focus_search = true;
        }
        if new_task {
            self.run = Some(RunDialog { text: String::new(), admin: false, error: None, focus: true });
        }
        if f5 {
            self.manual_refresh = true;
        }
        let order = [Tab::Processes, Tab::Performance, Tab::Services, Tab::Connections, Tab::Startup, Tab::Settings];
        if let Some(i) = digits.iter().position(|d| *d) {
            self.tab = order[i];
        }
    }

    fn refresh_if_due(&mut self, ctx: &egui::Context) {
        let since = self.m.since_refresh();
        let interval = self.settings.interval_ms as f64 / 1000.0;
        let due = interval > 0.0 && since >= interval;
        // слишком частое обновление даёт неточную загрузку ЦП, поэтому не чаще ~3 раз в секунду
        let forced = (self.procs.force_refresh || self.manual_refresh) && since > 0.35;
        if due || forced {
            self.m.refresh(self.procs.wants_gpu());
            self.procs.on_refresh(&self.m);
            self.procs.force_refresh = false;
            self.manual_refresh = false;
        }
        let tick = if interval > 0.0 { (interval * 1000.0).min(250.0) as u64 } else { 250 };
        ctx.request_repaint_after(Duration::from_millis(tick));
    }

    fn window_settings(&mut self, ctx: &egui::Context) {
        // крестик окна: вместо выхода прячем окно в трей
        if self.tray.is_some() && self.settings.close_to_tray && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_window();
        }
        if self.settings.always_on_top != self.applied_on_top {
            self.applied_on_top = self.settings.always_on_top;
            let level = if self.applied_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        self.procs.confirm_kill_tree = self.settings.confirm_kill_tree;
    }

    fn persist(&mut self) {
        self.procs.sync_to(&mut self.settings);
        if self.settings != self.saved {
            if self.persist_enabled {
                self.settings.save();
            }
            self.saved = self.settings.clone();
        }
    }

    // ───────────── боковая панель и статус ─────────────

    fn sidebar(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("sidebar")
            .exact_size(214.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(BG_SIDEBAR).inner_margin(egui::Margin::same(14)))
            .show(root, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("⚡ TaskMgr").size(22.0).strong().color(TEXT));
                ui.label(RichText::new("диспетчер задач на Rust").size(12.0).color(TEXT_DIM));
                ui.add_space(22.0);

                for tab in Tab::NAV {
                    self.nav_button(ui, tab);
                    ui.add_space(3.0);
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    // снизу вверх: мини-индикаторы, затем «Настройки»
                    let w = ui.available_width();
                    ui.add_space(2.0);
                    bar(ui, (self.m.mem_pct() / 100.0) as f32, PURPLE, &format!("ОЗУ {}", fmt_pct(self.m.mem_pct())), w);
                    ui.add_space(4.0);
                    bar(ui, self.m.cpu_total / 100.0, ACCENT, &format!("ЦП {}", fmt_pct(self.m.cpu_total as f64)), w);
                    ui.add_space(14.0);
                    self.nav_button(ui, Tab::Settings);
                });
            });
    }

    fn nav_button(&mut self, ui: &mut egui::Ui, tab: Tab) {
        let active = self.tab == tab;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
        let bg = if active {
            ACCENT.gamma_multiply(0.22)
        } else if resp.hovered() {
            BG_HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 10.0, bg);
        if active {
            let accent = egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 8.0), vec2(3.0, 24.0));
            ui.painter().rect_filled(accent, 2.0, ACCENT);
        }
        let color = if active { TEXT } else { TEXT_DIM };
        ui.painter().text(
            rect.left_center() + vec2(14.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{}   {}", tab.icon(), tab.title()),
            egui::FontId::proportional(14.5),
            color,
        );
        if resp.clicked() {
            self.tab = tab;
        }
    }

    fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(30.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(BG_SIDEBAR).inner_margin(egui::Margin::symmetric(16, 6)))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 22.0;
                    let m = &self.m;
                    ui.label(dim(format!("Процессов: {}", self.procs.process_count())));
                    ui.label(dim(format!("ЦП: {}", fmt_pct(m.cpu_total as f64))));
                    ui.label(dim(format!("Память: {}", fmt_pct(m.mem_pct()))));
                    ui.label(dim(format!("Диск: ↓ {}  ↑ {}", fmt_rate(m.disk_read_rate), fmt_rate(m.disk_write_rate))));
                    ui.label(dim(format!("Сеть: ↓ {}  ↑ {}", fmt_bitrate(m.net_rx_rate), fmt_bitrate(m.net_tx_rate))));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.settings.interval_ms == 0 {
                            ui.label(RichText::new("⏸ обновление приостановлено (F5 — обновить)").size(12.0).color(AMBER));
                        } else {
                            ui.label(dim(format!("Работает: {}", fmt_duration(System::uptime().unwrap_or(0)))));
                        }
                    });
                });
            });
    }

    // ───────────── страницы ─────────────

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.tab.title()).size(26.0).strong().color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let new_task = egui::Button::new(RichText::new("＋  Новая задача").color(Color32::WHITE).size(13.0))
                    .fill(ACCENT.gamma_multiply(0.85))
                    .corner_radius(8.0)
                    .min_size(vec2(0.0, 30.0));
                if ui.add(new_task).on_hover_text("Ctrl+N").clicked() {
                    self.run = Some(RunDialog { text: String::new(), admin: false, error: None, focus: true });
                }
                if self.tab == Tab::Processes && ui.button("Экспорт CSV").on_hover_text("Сохранить список процессов").clicked() {
                    self.procs.export_csv();
                }
            });
        });
        ui.add_space(12.0);
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(720.0);

            card(ui, |ui| {
                ui.label(RichText::new("Скорость обновления").strong().size(15.0).color(TEXT));
                ui.label(dim("Как часто обновляются процессы и графики. Пауза: данные не меняются, пока не нажмёшь F5."));
                ui.add_space(8.0);
                segmented(
                    ui,
                    &mut self.settings.interval_ms,
                    &[(0, "Пауза"), (500, "0,5 с"), (1000, "1 с"), (2000, "2 с"), (5000, "5 с")],
                );
            });
            ui.add_space(10.0);

            card(ui, |ui| {
                ui.label(RichText::new("Окно").strong().size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.checkbox(&mut self.settings.always_on_top, "Поверх всех окон");
                if self.tray.is_some() {
                    ui.checkbox(&mut self.settings.close_to_tray, "Крестик сворачивает в трей, а не закрывает программу");
                }
                ui.checkbox(&mut self.settings.confirm_kill_tree, "Спрашивать подтверждение перед завершением дерева процессов");
            });
            ui.add_space(10.0);

            card(ui, |ui| {
                ui.label(RichText::new("Замена диспетчера задач Windows").strong().size(15.0).color(TEXT));
                ui.add_space(4.0);
                if !cfg!(windows) {
                    ui.label(dim("Доступно только в Windows."));
                    return;
                }
                self.replace.refresh();
                ui.ctx().request_repaint_after(Duration::from_millis(1500));
                ui.label(dim(
                    "После замены Ctrl+Shift+Esc и «Диспетчер задач» в меню Win+X открывают эту программу. \
                     Перед заменой программа проверяет себя и при ошибке ничего не меняет.",
                ));
                ui.add_space(8.0);
                if self.replace.installed {
                    ui.label(RichText::new("● Замена включена").color(GREEN));
                    ui.add_space(6.0);
                    if ui.button("Вернуть стандартный диспетчер").clicked() {
                        self.replace.message = install::request_elevated("--uninstall").err();
                    }
                } else {
                    ui.label(RichText::new("● Используется стандартный диспетчер").color(TEXT_DIM));
                    ui.add_space(6.0);
                    if ui.button("Использовать эту программу вместо стандартного").clicked() {
                        self.replace.message = install::request_elevated("--install").err();
                    }
                }
                if let Some(e) = &self.replace.message {
                    ui.add_space(4.0);
                    ui.label(RichText::new(e).color(DANGER).size(13.0));
                }
            });
            ui.add_space(10.0);

            card(ui, |ui| {
                ui.label(RichText::new("О системе").strong().size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    stat(ui, "Система", &System::long_os_version().unwrap_or_else(|_| "—".into()));
                    stat(ui, "Компьютер", &System::host_name().unwrap_or_else(|_| "—".into()));
                    stat(ui, "Версия TaskMgr", env!("CARGO_PKG_VERSION"));
                });
            });
            ui.add_space(10.0);

            card(ui, |ui| {
                ui.label(RichText::new("Горячие клавиши").strong().size(15.0).color(TEXT));
                ui.add_space(6.0);
                egui::Grid::new("keys").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                    for (k, v) in [
                        ("Ctrl+F", "поиск по процессам"),
                        ("Ctrl+N", "запустить новую задачу"),
                        ("Ctrl+1 … 6", "переключение вкладок"),
                        ("F5", "обновить сейчас"),
                        ("Delete", "завершить выбранный процесс"),
                        ("↑ ↓ ← →", "выбор строки, сворачивание и раскрытие веток"),
                        ("Esc", "очистить поиск / снять выбор"),
                    ] {
                        ui.label(RichText::new(k).monospace().color(ACCENT));
                        ui.label(dim(v));
                        ui.end_row();
                    }
                });
            });
        });
    }

    fn run_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = self.run.as_mut() else { return };
        let mut go = false;
        let mut cancel = false;
        #[allow(deprecated)]
        let id = egui::Id::new("run_dialog");
        let resp = egui::Modal::new(id).show(ctx, |ui| {
            ui.set_width(430.0);
            ui.label(RichText::new("Запуск новой задачи").size(17.0).strong());
            ui.label(dim("Программа, папка, документ или адрес в интернете. Работает как «Выполнить» (Win+R)."));
            ui.add_space(10.0);
            let te = ui.add(
                egui::TextEdit::singleline(&mut d.text)
                    .desired_width(f32::INFINITY)
                    .hint_text("например: notepad или C:\\Users")
                    .margin(egui::Margin::symmetric(10, 8)),
            );
            if d.focus {
                te.request_focus();
                d.focus = false;
            }
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
            ui.add_space(6.0);
            ui.checkbox(&mut d.admin, "От имени администратора (укажи только программу, без аргументов)");
            if let Some(e) = &d.error {
                ui.add_space(4.0);
                ui.label(RichText::new(e).color(DANGER).size(12.5));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Запустить").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                    go = true;
                }
                if ui.button("Отмена").clicked() {
                    cancel = true;
                }
            });
        });
        if go {
            match run_command(&d.text, d.admin) {
                Ok(()) => self.run = None,
                Err(e) => d.error = Some(format!("Не удалось запустить: {e}")),
            }
        } else if cancel || resp.should_close() {
            self.run = None;
        }
    }
}

impl App {
    /// Один кадр интерфейса (вынесен из `eframe::App::ui`, чтобы его можно было гонять в тестах)
    fn frame(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        self.hotkeys(&ctx);
        self.refresh_if_due(&ctx);
        self.window_settings(&ctx);

        self.sidebar(root);
        self.status_bar(root);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(20)))
            .show(root, |ui| {
                self.header(ui);
                match self.tab {
                    Tab::Processes => self.procs.ui(ui, &mut self.m),
                    Tab::Performance => self.perf.ui(ui, &self.m, self.procs.process_count()),
                    Tab::Services => self.services.ui(ui),
                    Tab::Connections => {
                        if let Some(pid) = self.conns.ui(ui, &self.m) {
                            self.tab = Tab::Processes;
                            self.procs.reveal(pid);
                        }
                    }
                    Tab::Startup => self.startup.ui(ui),
                    Tab::Settings => self.settings_ui(ui),
                }
            });

        self.run_dialog(&ctx);
        self.persist();
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(root);
    }
}

/// Проверка работоспособности без окна: `TaskMgrRS.exe --selftest`.
/// Установщик запускает её до подмены системного диспетчера задач. Подробности пишутся в crash.log.
fn selftest() -> i32 {
    let mut failures = 0;
    let mut check = |name: &str, res: Result<String, String>| match res {
        Ok(info) => boot::log_line(&format!("selftest: {name}: ok {info}")),
        Err(e) => {
            failures += 1;
            boot::log_line(&format!("selftest: {name}: ОШИБКА {e}"));
        }
    };
    let guarded = |f: &dyn Fn() -> Result<String, String>| -> Result<String, String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err("паника".to_owned()))
    };

    check("настройки", guarded(&|| Ok(format!("{:?}", Settings::load().interval_ms))));
    check(
        "метрики",
        guarded(&|| {
            let mut m = Metrics::new()?;
            std::thread::sleep(Duration::from_millis(300));
            m.refresh(false);
            Ok(format!("процессов: {}", m.sys.processes().len()))
        }),
    );
    check(
        "интерфейс",
        guarded(&|| {
            let ctx = egui::Context::default();
            let mut app = App::build(&ctx, Settings::default(), false)?;
            app.persist_enabled = false;
            for (i, tab) in
                [Tab::Processes, Tab::Performance, Tab::Services, Tab::Connections, Tab::Startup, Tab::Settings]
                    .into_iter()
                    .enumerate()
            {
                app.tab = tab;
                for f in 0..2 {
                    let raw = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
                        time: Some((i * 2 + f) as f64 * 0.1),
                        ..Default::default()
                    };
                    ctx.run_ui(raw, |ui| app.frame(ui)).drop_without_applying_deltas();
                }
            }
            Ok("6 вкладок".to_owned())
        }),
    );
    check("автозагрузка", guarded(&|| Ok(format!("записей: {}", startup::list().len()))));
    check(
        "приоритеты",
        guarded(&|| Ok(format!("{:?}", winproc::get_priority(std::process::id()).map(|p| p.label())))),
    );
    boot::log_line(&format!("selftest завершён, ошибок: {failures}"));
    if failures == 0 { 0 } else { 2 }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Wgpu,
    Glow,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Backend::Wgpu => "wgpu",
            Backend::Glow => "glow",
        }
    }
}

fn run_gui(backend: Backend) -> Result<(), String> {
    let settings = Settings::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("TaskMgr") // заголовок используется для поиска окна из трея (см. tray.rs)
        .with_inner_size([1240.0, 780.0])
        .with_min_inner_size([980.0, 600.0]);
    if settings.always_on_top {
        viewport = viewport.with_always_on_top();
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: match backend {
            Backend::Wgpu => eframe::Renderer::Wgpu,
            Backend::Glow => eframe::Renderer::Glow,
        },
        ..Default::default()
    };
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            "TaskMgr",
            options,
            Box::new(|cc| match App::new(cc) {
                Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
                Err(e) => Err(e.into()),
            }),
        )
    }));
    match res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("{e}")),
        Err(_) => Err("внутренняя ошибка при запуске окна (см. журнал)".to_owned()),
    }
}

fn fatal(msg: &str) -> ! {
    boot::log_line(&format!("ОШИБКА ЗАПУСКА: {msg}"));
    let hint = boot::log_path().map(|p| format!("\n\nЖурнал: {}", p.display())).unwrap_or_default();
    boot::show_error("TaskMgr не запустился", &format!("{msg}{hint}"));
    std::process::exit(1);
}

fn main() {
    boot::install_panic_hook();
    // Аргументы чужие игнорируем: при подмене диспетчера Windows передаёт путь к taskmgr.exe
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }
    if args.iter().any(|a| a == "--install" || a == "--uninstall") {
        std::process::exit(install::run_cli(args.iter().any(|a| a == "--install")));
    }
    let forced = args.iter().find_map(|a| match a.as_str() {
        "--renderer=glow" => Some(Backend::Glow),
        "--renderer=wgpu" => Some(Backend::Wgpu),
        _ => None,
    });

    if forced.is_none() && !boot::acquire_single_instance() {
        // уже запущено: просто показываем существующее окно
        tray::show_window();
        return;
    }

    let first = forced.unwrap_or(Backend::Wgpu);
    boot::set_startup_guarded(true);
    let result = run_gui(first);
    boot::set_startup_guarded(false);
    let Err(err) = result else { return };
    boot::log_line(&format!("запуск через {} не удался: {err}", first.name()));

    if forced.is_some() || first == Backend::Glow {
        fatal(&err);
    }
    // Запасной вариант: новый процесс с OpenGL (winit не позволяет создать цикл событий повторно)
    boot::release_single_instance();
    let child = std::env::current_exe()
        .map_err(|e| e.to_string())
        .and_then(|exe| std::process::Command::new(exe).arg("--renderer=glow").status().map_err(|e| e.to_string()));
    match child {
        Ok(st) if st.success() => {}
        Ok(st) => fatal(&format!("{err}\nЗапасной запуск (OpenGL) тоже не удался: {st}")),
        Err(e) => fatal(&format!("{err}\nНе удалось запустить запасной вариант: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_quoting() {
        assert_eq!(ps_quote("notepad"), "'notepad'");
        assert_eq!(ps_quote("it's"), "'it''s'");
        // подстановка «$(...)» внутри одинарных кавычек не выполняется
        assert_eq!(ps_quote("$(calc)"), "'$(calc)'");
    }

    #[test]
    fn empty_command_is_rejected() {
        assert!(run_command("   ", false).is_err());
    }
}

/// Прогон настоящего интерфейса без окна и видеокарты: egui умеет рисовать кадры «в никуда»,
/// поэтому паники в раскладке, таблицах, модальных окнах и т. п. ловятся обычным `cargo test`.
#[cfg(test)]
mod ui_tests {
    use super::*;
    use perf::PerfSel;
    use procs::{Col, View};

    struct Harness {
        ctx: egui::Context,
        app: App,
        t: f64,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            let mut app = App::build(&ctx, Settings::default(), false).expect("build");
            app.persist_enabled = false;
            Self { ctx, app, t: 0.0 }
        }

        fn run(&mut self, frames: usize) {
            for _ in 0..frames {
                self.t += 0.05;
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0))),
                    time: Some(self.t),
                    ..Default::default()
                };
                let app = &mut self.app;
                self.ctx.run_ui(raw, |ui| app.frame(ui)).drop_without_applying_deltas();
            }
        }

        /// Новый снимок системных данных (как после тика обновления)
        fn refresh(&mut self) {
            std::thread::sleep(Duration::from_millis(260));
            self.app.m.refresh(self.app.procs.wants_gpu());
            self.app.procs.on_refresh(&self.app.m);
        }
    }

    #[test]
    fn every_tab_renders() {
        let mut h = Harness::new();
        for tab in [Tab::Processes, Tab::Performance, Tab::Services, Tab::Connections, Tab::Startup, Tab::Settings] {
            h.app.tab = tab;
            h.run(3);
            h.refresh();
            h.run(2);
        }
    }

    #[test]
    fn process_views_sorting_and_columns() {
        let mut h = Harness::new();
        h.app.tab = Tab::Processes;
        h.app.procs.cols = Col::OPTIONAL.to_vec(); // все колонки сразу
        for view in [View::List, View::Tree, View::Group] {
            h.app.procs.view = view;
            for sort in std::iter::once(Col::Name).chain(Col::OPTIONAL) {
                for desc in [true, false] {
                    h.app.procs.sort = sort;
                    h.app.procs.sort_desc = desc;
                    h.app.procs.on_refresh(&h.app.m);
                    h.run(2);
                }
            }
            // выбор своего процесса открывает панель деталей
            h.app.procs.reveal(std::process::id());
            h.run(3);
            h.refresh();
            h.run(2);
        }
    }

    #[test]
    fn search_filters_including_no_matches() {
        let mut h = Harness::new();
        h.app.tab = Tab::Processes;
        for q in ["", "a", "zzzz-no-such-process-qq", "  ", "1"] {
            h.app.procs.filter = q.to_owned();
            h.app.procs.on_refresh(&h.app.m);
            h.run(2);
        }
    }

    #[test]
    fn performance_panes_and_per_core_grid() {
        let mut h = Harness::new();
        h.app.tab = Tab::Performance;
        for sel in [PerfSel::Cpu, PerfSel::Mem, PerfSel::Disk, PerfSel::Net, PerfSel::Gpu] {
            h.app.perf.sel = sel;
            for per_core in [false, true] {
                h.app.perf.per_core = per_core;
                h.run(2);
                h.refresh();
            }
        }
    }

    #[test]
    fn run_dialog_and_modal_state() {
        let mut h = Harness::new();
        h.app.run = Some(RunDialog { text: "notepad".into(), admin: false, error: Some("тест".into()), focus: true });
        h.run(3);
        assert!(h.app.run.is_some(), "диалог не должен закрываться сам");
    }

    #[test]
    fn paused_updates_and_small_interval() {
        let mut h = Harness::new();
        for interval in [0, 500, 5000] {
            h.app.settings.interval_ms = interval;
            h.run(2);
        }
        h.app.settings.always_on_top = true;
        h.run(2);
    }
}
