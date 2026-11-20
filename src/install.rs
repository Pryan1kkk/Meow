//! Установка и снятие замены системного диспетчера задач прямо из программы (без скриптов).
//!
//! Замена — это значение `Debugger` в ключе Image File Execution Options для `taskmgr.exe`.
//! Запись в HKLM требует прав администратора, поэтому кнопка в настройках запускает копию программы
//! с `--install` / `--uninstall` через запрос UAC. Используются только `reg.exe` и PowerShell из системы.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::boot;

const KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe";

fn system_cmd(program: &str) -> Command {
    let mut c = Command::new(program);
    crate::winquery::hide_console(&mut c);
    c
}

/// Установлена ли замена (читать ключ может любой пользователь)
pub fn is_installed() -> bool {
    cfg!(windows)
        && system_cmd("reg")
            .args(["query", KEY, "/v", "Debugger"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn install_dir() -> PathBuf {
    let base = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    base.join("TaskMgrRS")
}

/// Перезапускает программу с правами администратора (покажется окно UAC)
pub fn request_elevated(arg: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let script = format!(
        "Start-Process -FilePath '{}' -ArgumentList '{arg}' -Verb RunAs",
        exe.display().to_string().replace('\'', "''")
    );
    let out = system_cmd("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .output()
        .map_err(|e| format!("не удалось запустить PowerShell: {e}"))?;
    if out.status.success() { Ok(()) } else { Err("запрос прав администратора отклонён".to_owned()) }
}

fn selftest_ok(exe: &std::path::Path) -> Result<(), String> {
    let mut child = Command::new(exe).arg("--selftest").spawn().map_err(|e| format!("не удалось запустить: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => return Ok(()),
            Ok(Some(st)) => return Err(format!("самопроверка вернула {st}")),
            Ok(None) if start.elapsed() > Duration::from_secs(60) => {
                let _ = child.kill();
                return Err("самопроверка не завершилась за 60 секунд".to_owned());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn install() -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir();
    let dst = dir.join("TaskMgrRS.exe");
    // На время проверки снимаем старую замену: если что-то сломается, останется стандартный диспетчер
    let _ = system_cmd("reg").args(["delete", KEY, "/f"]).output();
    std::fs::create_dir_all(&dir).map_err(|e| format!("не удалось создать {}: {e}", dir.display()))?;
    if me != dst {
        std::fs::copy(&me, &dst).map_err(|e| format!("не удалось скопировать в {}: {e}", dir.display()))?;
    }
    selftest_ok(&dst).map_err(|e| format!("{e}. Замена не установлена, стандартный диспетчер на месте."))?;
    let out = system_cmd("reg")
        .args(["add", KEY, "/v", "Debugger", "/t", "REG_SZ", "/d", &format!("\"{}\"", dst.display()), "/f"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err("не удалось записать ключ реестра (нужны права администратора)".to_owned()) }
}

fn uninstall() -> Result<(), String> {
    if !is_installed() {
        return Ok(());
    }
    let out = system_cmd("reg").args(["delete", KEY, "/f"]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err("не удалось удалить ключ реестра (нужны права администратора)".to_owned()) }
}

/// Точка входа для `--install` / `--uninstall`; возвращает код выхода
pub fn run_cli(install_mode: bool) -> i32 {
    let res = if install_mode { install() } else { uninstall() };
    boot::log_line(&format!("{}: {res:?}", if install_mode { "install" } else { "uninstall" }));
    match res {
        Ok(()) if install_mode => {
            boot::show_info("TaskMgr", "Готово. Теперь Ctrl+Shift+Esc открывает TaskMgr.\n\nВернуть стандартный диспетчер можно в Настройках программы.");
            0
        }
        Ok(()) => {
            boot::show_info("TaskMgr", "Стандартный диспетчер задач Windows возвращён.");
            0
        }
        Err(e) => {
            boot::show_error("TaskMgr: не получилось", &e);
            1
        }
    }
}

/// Состояние для настроек: кэширует проверку реестра
pub struct State {
    pub installed: bool,
    checked: Instant,
    pub message: Option<String>,
}

impl State {
    pub fn new() -> Self {
        Self { installed: is_installed(), checked: Instant::now(), message: None }
    }

    /// Обновляет статус не чаще раза в 1,5 с (после UAC-окна статус меняется без перезапуска)
    pub fn refresh(&mut self) {
        if self.checked.elapsed() > Duration::from_millis(1500) {
            self.installed = is_installed();
            self.checked = Instant::now();
        }
    }
}
