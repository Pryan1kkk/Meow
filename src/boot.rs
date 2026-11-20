//! Защита запуска: журнал ошибок, окно с описанием сбоя и запрет второго экземпляра.
//!
//! Релизная сборка собирается как GUI-программа без консоли, поэтому без этого модуля любой сбой при
//! старте выглядел бы как «ничего не происходит». Теперь ошибка пишется в `crash.log` рядом с
//! настройками и показывается в окне.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::settings::Settings;

/// Пока включено, паника в главном потоке не показывает окно: её перехватит код запуска
/// и попробует запасной вариант (например, другой рендерер).
static STARTUP_GUARDED: AtomicBool = AtomicBool::new(false);

pub fn set_startup_guarded(on: bool) {
    STARTUP_GUARDED.store(on, Ordering::SeqCst);
}

pub fn log_path() -> Option<PathBuf> {
    Some(Settings::data_dir()?.join("crash.log"))
}

/// Дописывает строку в журнал. Любые ошибки записи игнорируются: журнал не должен ронять программу.
pub fn log_line(msg: &str) {
    let Some(path) = log_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // журнал не должен расти бесконечно
    if std::fs::metadata(&path).map(|m| m.len() > 256 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

/// Все паники попадают в журнал; из главного потока ещё и показываются в окне.
/// Паники во вспомогательных потоках (чтение служб и т. п.) программу не роняют и окна не вызывают.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("?");
        let trace = std::backtrace::Backtrace::force_capture();
        log_line(&format!("ПАНИКА в потоке «{name}»: {info}\n{trace}"));
        if name == "main" && !STARTUP_GUARDED.load(Ordering::SeqCst) {
            let hint = log_path().map(|p| format!("\n\nПодробности: {}", p.display())).unwrap_or_default();
            show_error("TaskMgr: критическая ошибка", &format!("{info}{hint}"));
        }
        default(info);
    }));
}

pub use imp::{acquire_single_instance, release_single_instance, show_error, show_info};

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn show_error(title: &str, text: &str) {
        let (t, c) = (wide(text), wide(title));
        unsafe {
            MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }

    pub fn show_info(title: &str, text: &str) {
        let (t, c) = (wide(text), wide(title));
        unsafe {
            MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONINFORMATION);
        }
    }

    /// Дескриптор именованного мьютекса: держится до конца работы процесса
    static MUTEX: AtomicUsize = AtomicUsize::new(0);

    /// `true` — мы первый экземпляр (или определить не удалось: тогда лучше запуститься)
    pub fn acquire_single_instance() -> bool {
        let name = wide("Local\\TaskMgrRS_single_instance");
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if h.is_null() {
            return true;
        }
        // GetLastError нужно читать сразу после CreateMutexW
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already {
            unsafe {
                CloseHandle(h);
            }
            return false;
        }
        MUTEX.store(h as usize, Ordering::SeqCst);
        true
    }

    /// Освобождает мьютекс (перед запуском дочернего процесса с запасным рендерером)
    pub fn release_single_instance() {
        let h = MUTEX.swap(0, Ordering::SeqCst);
        if h != 0 {
            unsafe {
                CloseHandle(h as HANDLE);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn show_error(title: &str, text: &str) {
        eprintln!("{title}: {text}");
    }
    pub fn show_info(title: &str, text: &str) {
        eprintln!("{title}: {text}");
    }
    pub fn acquire_single_instance() -> bool {
        true
    }
    pub fn release_single_instance() {}
}
