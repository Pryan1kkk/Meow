//! Службы Windows и сетевые подключения.
//!
//! Данные берутся из PowerShell (`Get-CimInstance Win32_Service`, `Get-NetTCPConnection`,
//! `Get-NetUDPEndpoint`) в формате JSON: так не нужны дополнительные зависимости и локализация
//! Windows не ломает разбор. Запросы идут в фоновом потоке (`Job`), чтобы интерфейс не зависал.
//! Разбор JSON вынесен в отдельные функции и покрыт тестами (они работают и без Windows).
#![allow(dead_code)]

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::Value;

// ───────────────────────────── фоновые задачи ─────────────────────────────

/// Состояние фоновой задачи
pub enum JobState<T> {
    Running,
    Done(T),
    /// Поток завершился, ничего не вернув (например, из-за паники)
    Failed,
}

/// Результат работы в отдельном потоке, который можно опрашивать из UI без блокировки
pub struct Job<T> {
    rx: Receiver<T>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(f: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        Self { rx }
    }

    pub fn poll(&self) -> JobState<T> {
        match self.rx.try_recv() {
            Ok(v) => JobState::Done(v),
            Err(TryRecvError::Empty) => JobState::Running,
            Err(TryRecvError::Disconnected) => JobState::Failed,
        }
    }
}

// ───────────────────────────── службы ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    pub name: String,
    pub display: String,
    pub state: String,      // Running / Stopped / ...
    pub start_mode: String, // Auto / Manual / Disabled
    pub pid: u32,
    pub description: String,
}

impl Service {
    pub fn is_running(&self) -> bool {
        self.state.eq_ignore_ascii_case("running")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

pub fn parse_services(json: &str) -> Result<Vec<Service>, String> {
    let items = json_items(json)?;
    let mut out: Vec<Service> = items
        .iter()
        .filter_map(|v| {
            let name = v.get("Name")?.as_str()?.to_owned();
            Some(Service {
                display: str_field(v, "DisplayName").unwrap_or_else(|| name.clone()),
                name,
                state: str_field(v, "State").unwrap_or_default(),
                start_mode: str_field(v, "StartMode").unwrap_or_default(),
                pid: v.get("ProcessId").and_then(Value::as_u64).unwrap_or(0) as u32,
                description: str_field(v, "Description").unwrap_or_default(),
            })
        })
        .collect();
    out.sort_by_key(|s| s.display.to_lowercase());
    Ok(out)
}

pub fn fetch_services() -> Result<Vec<Service>, String> {
    let script = "Get-CimInstance Win32_Service | \
                  Select-Object Name,DisplayName,State,StartMode,ProcessId,Description | \
                  ConvertTo-Json -Compress";
    parse_services(&run_powershell(script)?)
}

/// Имя службы подставляется в аргументы `net`, поэтому отсекаем то, что можно принять за ключ
fn valid_service_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with(['/', '-']) && !name.contains(['"', '\r', '\n', '\0'])
}

pub fn service_action(name: &str, action: ServiceAction) -> Result<(), String> {
    if !valid_service_name(name) {
        return Err("недопустимое имя службы".to_owned());
    }
    match action {
        ServiceAction::Start => net("start", name),
        ServiceAction::Stop => net("stop", name),
        ServiceAction::Restart => {
            // Если служба уже остановлена, stop вернёт ошибку — это нормально
            let _ = net("stop", name);
            net("start", name)
        }
    }
}

fn net(verb: &str, name: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("net");
    cmd.arg(verb).arg(name);
    hide_console(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else if out.status.code() == Some(5) {
        Err("отказано в доступе (запусти от имени администратора)".to_owned())
    } else {
        Err(format!("net {verb} вернул код {}", out.status.code().unwrap_or(-1)))
    }
}

// ───────────────────────────── подключения ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub proto: &'static str, // "TCP" / "UDP"
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    pub state: String,
    pub pid: u32,
}

pub fn parse_connections(json: &str) -> Result<Vec<Connection>, String> {
    let root: Value = serde_json::from_str(strip_bom(json).trim()).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (key, proto) in [("tcp", "TCP"), ("udp", "UDP")] {
        for v in as_items(root.get(key)) {
            let port = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as u16;
            out.push(Connection {
                proto,
                local_addr: str_field(&v, "LocalAddress").unwrap_or_default(),
                local_port: port("LocalPort"),
                remote_addr: str_field(&v, "RemoteAddress").unwrap_or_default(),
                remote_port: port("RemotePort"),
                state: if proto == "UDP" {
                    String::new()
                } else {
                    str_field(&v, "State").unwrap_or_default()
                },
                pid: v.get("OwningProcess").and_then(Value::as_u64).unwrap_or(0) as u32,
            });
        }
    }
    Ok(out)
}

pub fn fetch_connections() -> Result<Vec<Connection>, String> {
    let script = "@{ \
        tcp = @(Get-NetTCPConnection | Select-Object LocalAddress,LocalPort,RemoteAddress,RemotePort,\
                @{n='State';e={[string]$_.State}},OwningProcess); \
        udp = @(Get-NetUDPEndpoint | Select-Object LocalAddress,LocalPort,OwningProcess) \
    } | ConvertTo-Json -Compress -Depth 4";
    parse_connections(&run_powershell(script)?)
}

// ───────────────────────────── общее ─────────────────────────────

fn run_powershell(script: &str) -> Result<String, String> {
    // Принудительно UTF-8 без BOM, иначе кириллица в описаниях служб превратится в «????»
    let full = format!(
        "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false; \
         $ProgressPreference = 'SilentlyContinue'; {script}"
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &full]);
    hide_console(&mut cmd);
    let out = cmd.output().map_err(|e| format!("не удалось запустить PowerShell: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().next().unwrap_or("неизвестная ошибка");
        return Err(format!("PowerShell: {first}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Не показывать мигающее окно консоли при запуске дочернего процесса
pub fn hide_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

fn strip_bom(s: &str) -> &str {
    s.trim_start_matches('\u{feff}')
}

/// PowerShell отдаёт один объект как `{...}`, а несколько как `[{...}, ...]`
fn json_items(json: &str) -> Result<Vec<Value>, String> {
    let text = strip_bom(json).trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("некорректный JSON: {e}"))?;
    Ok(as_items(Some(&v)))
}

fn as_items(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other.clone()],
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_array_with_cyrillic_and_bom() {
        let json = "\u{feff}[{\"Name\":\"Spooler\",\"DisplayName\":\"Диспетчер печати\",\
            \"State\":\"Running\",\"StartMode\":\"Auto\",\"ProcessId\":1234,\"Description\":null},\
            {\"Name\":\"Audiosrv\",\"DisplayName\":\"Windows Audio\",\"State\":\"Stopped\",\
            \"StartMode\":\"Manual\",\"ProcessId\":0,\"Description\":\"Звук\"}]";
        let s = parse_services(json).unwrap();
        assert_eq!(s.len(), 2);
        // сортировка по отображаемому имени: «windows audio» раньше «диспетчер печати»
        assert_eq!(s[0].name, "Audiosrv");
        assert_eq!(s[1].display, "Диспетчер печати");
        assert!(s[1].is_running());
        assert!(!s[0].is_running());
        assert_eq!(s[1].pid, 1234);
        assert_eq!(s[1].description, "");
        assert_eq!(s[0].description, "Звук");
    }

    #[test]
    fn services_single_object_is_not_an_array() {
        let json = r#"{"Name":"Dhcp","DisplayName":null,"State":"Running","StartMode":"Auto","ProcessId":8}"#;
        let s = parse_services(json).unwrap();
        assert_eq!(s.len(), 1);
        // нет DisplayName — подставляется имя службы
        assert_eq!(s[0].display, "Dhcp");
    }

    #[test]
    fn services_empty_and_broken_input() {
        assert!(parse_services("").unwrap().is_empty());
        assert!(parse_services("  \n").unwrap().is_empty());
        assert!(parse_services("not json").is_err());
    }

    #[test]
    fn connections_tcp_and_udp() {
        let json = r#"{"tcp":[{"LocalAddress":"0.0.0.0","LocalPort":135,"RemoteAddress":"0.0.0.0",
            "RemotePort":0,"State":"Listen","OwningProcess":1080},
            {"LocalAddress":"192.168.1.5","LocalPort":50123,"RemoteAddress":"142.250.1.1",
            "RemotePort":443,"State":"Established","OwningProcess":4242}],
            "udp":{"LocalAddress":"::","LocalPort":5353,"OwningProcess":900}}"#;
        let c = parse_connections(json).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].proto, "TCP");
        assert_eq!(c[0].state, "Listen");
        assert_eq!(c[1].remote_port, 443);
        assert_eq!(c[1].pid, 4242);
        // один UDP-объект (не массив) тоже разбирается; состояния у UDP нет
        assert_eq!(c[2].proto, "UDP");
        assert_eq!(c[2].local_port, 5353);
        assert_eq!(c[2].state, "");
    }

    #[test]
    fn connections_empty_lists() {
        let c = parse_connections(r#"{"tcp":[],"udp":[]}"#).unwrap();
        assert!(c.is_empty());
        let c = parse_connections(r#"{"tcp":null,"udp":null}"#).unwrap();
        assert!(c.is_empty());
    }

    #[test]
    fn service_name_validation() {
        assert!(valid_service_name("Spooler"));
        assert!(valid_service_name("MSSQL$SQLEXPRESS"));
        assert!(!valid_service_name(""));
        assert!(!valid_service_name("/y"));
        assert!(!valid_service_name("-x"));
        assert!(!valid_service_name("a\"b"));
    }
}
