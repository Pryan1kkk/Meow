//! Настройки программы: хранятся в JSON рядом с остальными данными приложений пользователя
//! (`%APPDATA%\TaskMgrRS\settings.json` в Windows). Любая ошибка чтения/записи игнорируется:
//! без файла просто используются значения по умолчанию.

use std::path::PathBuf;

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Период обновления данных, мс (0 — пауза)
    pub interval_ms: u64,
    pub always_on_top: bool,
    pub close_to_tray: bool,
    pub confirm_kill_tree: bool,
    /// "list" | "tree" | "group"
    pub view: String,
    /// "name" | "pid" | "cpu" | "mem" | "disk" | "gpu" | "time" | "user"
    pub sort: String,
    pub sort_desc: bool,
    /// Идентификаторы видимых колонок таблицы процессов
    pub columns: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            always_on_top: false,
            close_to_tray: true,
            confirm_kill_tree: true,
            view: "group".to_owned(),
            sort: "cpu".to_owned(),
            sort_desc: true,
            columns: ["pid", "cpu", "mem", "disk"].map(str::to_owned).to_vec(),
        }
    }
}

impl Settings {
    pub fn to_json(&self) -> Value {
        json!({
            "interval_ms": self.interval_ms,
            "always_on_top": self.always_on_top,
            "close_to_tray": self.close_to_tray,
            "confirm_kill_tree": self.confirm_kill_tree,
            "view": self.view,
            "sort": self.sort,
            "sort_desc": self.sort_desc,
            "columns": self.columns,
        })
    }

    /// Недостающие или некорректные поля заменяются значениями по умолчанию
    pub fn from_json(v: &Value) -> Self {
        let d = Self::default();
        let b = |k: &str, def: bool| v.get(k).and_then(Value::as_bool).unwrap_or(def);
        let s = |k: &str, def: &str| v.get(k).and_then(Value::as_str).unwrap_or(def).to_owned();
        let interval = v.get("interval_ms").and_then(Value::as_u64).unwrap_or(d.interval_ms);
        Self {
            // защита от мусора в файле: слишком частое обновление съест процессор
            interval_ms: if interval == 0 || (250..=60_000).contains(&interval) { interval } else { d.interval_ms },
            always_on_top: b("always_on_top", d.always_on_top),
            close_to_tray: b("close_to_tray", d.close_to_tray),
            confirm_kill_tree: b("confirm_kill_tree", d.confirm_kill_tree),
            view: s("view", &d.view),
            sort: s("sort", &d.sort),
            sort_desc: b("sort_desc", d.sort_desc),
            columns: match v.get("columns").and_then(Value::as_array) {
                Some(a) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
                None => d.columns,
            },
        }
    }

    /// Папка данных программы: `%APPDATA%\TaskMgrRS` (в Windows)
    pub fn data_dir() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        }?;
        Some(base.join("TaskMgrRS"))
    }

    fn path() -> Option<PathBuf> {
        Some(Self::data_dir()?.join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| Self::from_json(&v))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.to_json()) {
            let _ = std::fs::write(p, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings {
            interval_ms: 2000,
            always_on_top: true,
            close_to_tray: false,
            confirm_kill_tree: false,
            view: "tree".into(),
            sort: "mem".into(),
            sort_desc: false,
            columns: vec!["pid".into(), "cpu".into()],
        };
        assert_eq!(Settings::from_json(&s.to_json()), s);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        let v: Value = serde_json::from_str(r#"{"interval_ms":5,"view":42,"columns":"x"}"#).unwrap();
        let s = Settings::from_json(&v);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn pause_is_allowed() {
        let v: Value = serde_json::from_str(r#"{"interval_ms":0}"#).unwrap();
        assert_eq!(Settings::from_json(&v).interval_ms, 0);
    }
}
