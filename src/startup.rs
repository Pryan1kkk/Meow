//! Автозагрузка: чтение списка и включение/выключение записей.
//!
//! Источники (как в настоящем диспетчере задач):
//!   * реестр `...\CurrentVersion\Run` для HKCU, HKLM и HKLM\WOW6432Node;
//!   * папки «Автозагрузка» (пользовательская и общая).
//!
//! Включение/выключение делается так же, как это делает Windows:
//! запись в `...\Explorer\StartupApproved\<Run|Run32|StartupFolder>`
//! (первый байт значения: 02 — включено, 03 — выключено).
//!
//! На других ОС модуль возвращает пустой список.
#![allow(dead_code)]

#[derive(Clone, Debug)]
pub struct StartupEntry {
    pub name: String,
    pub command: String,
    pub source: &'static str,
    pub enabled: bool,
    /// true — ветка HKCU (текущий пользователь), false — HKLM (все пользователи)
    pub user_scope: bool,
    /// имя подраздела в StartupApproved
    pub approved: &'static str,
}

pub use imp::{list, set_enabled};

#[cfg(windows)]
mod imp {
    use super::StartupEntry;
    use std::borrow::Cow;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use winreg::enums::*;
    use winreg::{RegKey, RegValue};

    const APPROVED_BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

    struct RegSource {
        user: bool,
        path: &'static str,
        approved: &'static str,
        label: &'static str,
    }

    const REG_SOURCES: [RegSource; 3] = [
        RegSource {
            user: true,
            path: r"Software\Microsoft\Windows\CurrentVersion\Run",
            approved: "Run",
            label: "Реестр · текущий пользователь",
        },
        RegSource {
            user: false,
            path: r"Software\Microsoft\Windows\CurrentVersion\Run",
            approved: "Run",
            label: "Реестр · все пользователи",
        },
        RegSource {
            user: false,
            path: r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
            approved: "Run32",
            label: "Реестр · все пользователи (32-бит)",
        },
    ];

    fn root(user: bool) -> RegKey {
        RegKey::predef(if user { HKEY_CURRENT_USER } else { HKEY_LOCAL_MACHINE })
    }

    fn is_enabled(user: bool, approved: &str, name: &str) -> bool {
        let path = format!(r"{APPROVED_BASE}\{approved}");
        let Ok(key) = root(user).open_subkey_with_flags(path, KEY_READ) else {
            return true; // записи в StartupApproved нет — значит, включено
        };
        match key.get_raw_value(name) {
            Ok(v) => v.bytes.first().map_or(true, |b| b & 1 == 0),
            Err(_) => true,
        }
    }

    fn folder_sources() -> Vec<(PathBuf, bool, &'static str)> {
        let mut v = Vec::new();
        if let Some(p) = std::env::var_os("APPDATA") {
            v.push((
                PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
                true,
                "Папка «Автозагрузка» · пользователь",
            ));
        }
        if let Some(p) = std::env::var_os("ProgramData") {
            v.push((
                PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\StartUp"),
                false,
                "Папка «Автозагрузка» · все пользователи",
            ));
        }
        v
    }

    pub fn list() -> Vec<StartupEntry> {
        let mut out = Vec::new();

        for s in REG_SOURCES.iter() {
            let Ok(key) = root(s.user).open_subkey_with_flags(s.path, KEY_READ) else {
                continue;
            };
            for item in key.enum_values() {
                let Ok((name, _)) = item else { continue };
                let command: String = key.get_value(&name).unwrap_or_default();
                let enabled = is_enabled(s.user, s.approved, &name);
                out.push(StartupEntry {
                    name,
                    command,
                    source: s.label,
                    enabled,
                    user_scope: s.user,
                    approved: s.approved,
                });
            }
        }

        for (dir, user, label) in folder_sources() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for f in rd.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if name.eq_ignore_ascii_case("desktop.ini") {
                    continue;
                }
                let enabled = is_enabled(user, "StartupFolder", &name);
                out.push(StartupEntry {
                    command: f.path().display().to_string(),
                    name,
                    source: label,
                    enabled,
                    user_scope: user,
                    approved: "StartupFolder",
                });
            }
        }

        out.sort_by_key(|e| e.name.to_lowercase());
        out
    }

    fn filetime_now() -> u64 {
        let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        (d.as_secs() + 11_644_473_600) * 10_000_000 + u64::from(d.subsec_nanos()) / 100
    }

    pub fn set_enabled(e: &StartupEntry, enabled: bool) -> Result<(), String> {
        let describe = |err: std::io::Error| {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                "нужны права администратора".to_owned()
            } else {
                err.to_string()
            }
        };

        let path = format!(r"{APPROVED_BASE}\{}", e.approved);
        let (key, _) = root(e.user_scope)
            .create_subkey_with_flags(path, KEY_READ | KEY_WRITE)
            .map_err(describe)?;

        let mut bytes = vec![0u8; 12];
        if enabled {
            bytes[0] = 0x02;
        } else {
            bytes[0] = 0x03;
            bytes[4..12].copy_from_slice(&filetime_now().to_le_bytes());
        }
        key.set_raw_value(
            &e.name,
            &RegValue {
                bytes: Cow::Owned(bytes),
                vtype: REG_BINARY,
            },
        )
        .map_err(describe)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::StartupEntry;

    pub fn list() -> Vec<StartupEntry> {
        Vec::new()
    }

    pub fn set_enabled(_e: &StartupEntry, _enabled: bool) -> Result<(), String> {
        Err("доступно только в Windows".to_owned())
    }
}
