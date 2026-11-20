//! Сбор системных метрик и их истории для графиков.

use std::collections::VecDeque;
use std::time::Instant;

use sysinfo::{
    DiskRefreshKind, Disks, Gpus, Networks, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind,
};

/// Сколько последних значений хранится для графиков
pub const HISTORY_LEN: usize = 90;

#[derive(Clone)]
pub struct History(VecDeque<f64>);

impl History {
    pub fn new() -> Self {
        Self(VecDeque::from(vec![0.0; HISTORY_LEN]))
    }
    pub fn push(&mut self, v: f64) {
        if self.0.len() >= HISTORY_LEN {
            self.0.pop_front();
        }
        self.0.push_back(v);
    }
    pub fn values(&self) -> &VecDeque<f64> {
        &self.0
    }
    pub fn max(&self) -> f64 {
        self.0.iter().copied().fold(0.0, f64::max)
    }
}

pub struct DiskInfo {
    pub mount: String,
    pub fs: String,
    pub kind: String,
    pub total: u64,
    pub free: u64,
}

pub struct NetInfo {
    pub name: String,
    pub rx_total: u64,
    pub tx_total: u64,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub mac: String,
}

pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub usage: Option<f32>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
}

pub struct Metrics {
    pub sys: System,
    disks: Option<Disks>,
    nets: Option<Networks>,
    disk_ticks: u32,
    gpus: Option<Gpus>,
    last: Instant,
    /// Сколько секунд прошло между двумя последними обновлениями (для перевода дельт в «за секунду»)
    pub dt: f64,

    pub cpu: History,
    pub mem: History,
    pub disk_read: History,
    pub disk_write: History,
    pub net_rx: History,
    pub net_tx: History,
    pub gpu: History,
    pub core_hist: Vec<History>,

    pub cpu_total: f32,
    pub cores: Vec<f32>,
    pub disk_read_rate: f64,
    pub disk_write_rate: f64,
    pub net_rx_rate: f64,
    pub net_tx_rate: f64,
    pub gpu_usage: Option<f32>,

    pub disks_info: Vec<DiskInfo>,
    pub nets_info: Vec<NetInfo>,
    pub gpus_info: Vec<GpuInfo>,
}

impl Metrics {
    pub fn new() -> Result<Self, String> {
        let mut sys = System::new_all()
            .or_else(|_| System::new())
            .map_err(|e| format!("не удалось инициализировать системные счётчики: {e}"))?;
        sys.refresh_all();
        let ncores = sys.cpus().len();
        let mut m = Self {
            sys,
            disks: Disks::new_with_refreshed_list().or_else(|_| Disks::new()).ok(),
            nets: Networks::new_with_refreshed_list().or_else(|_| Networks::new()).ok(),
            disk_ticks: 0,
            gpus: Gpus::new_with_refreshed_list().ok(),
            last: Instant::now(),
            dt: 1.0,
            cpu: History::new(),
            mem: History::new(),
            disk_read: History::new(),
            disk_write: History::new(),
            net_rx: History::new(),
            net_tx: History::new(),
            gpu: History::new(),
            core_hist: vec![History::new(); ncores],
            cpu_total: 0.0,
            cores: Vec::new(),
            disk_read_rate: 0.0,
            disk_write_rate: 0.0,
            net_rx_rate: 0.0,
            net_tx_rate: 0.0,
            gpu_usage: None,
            disks_info: Vec::new(),
            nets_info: Vec::new(),
            gpus_info: Vec::new(),
        };
        m.refresh(false);
        Ok(m)
    }

    pub fn since_refresh(&self) -> f64 {
        self.last.elapsed().as_secs_f64()
    }

    pub fn mem_used(&self) -> u64 {
        self.sys.used_memory()
    }
    pub fn mem_total(&self) -> u64 {
        self.sys.total_memory()
    }
    pub fn mem_pct(&self) -> f64 {
        self.mem_used() as f64 / self.mem_total().max(1) as f64 * 100.0
    }

    /// `proc_gpu` — читать ли загрузку GPU по каждому процессу (дорого, нужно только для колонки GPU)
    pub fn refresh(&mut self, proc_gpu: bool) {
        let now = Instant::now();
        self.dt = now.duration_since(self.last).as_secs_f64().max(0.05);
        self.last = now;

        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let mut kind = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            // эти поля почти не меняются: достаточно прочитать один раз
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet)
            .with_user(UpdateKind::OnlyIfNotSet);
        if proc_gpu {
            kind = kind.with_gpu_usage();
        }
        self.sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

        self.cpu_total = self.sys.global_cpu_usage();
        self.cores = self.sys.cpus().iter().map(|c| c.usage()).collect();
        self.cpu.push(self.cpu_total as f64);
        for (h, c) in self.core_hist.iter_mut().zip(&self.cores) {
            h.push(*c as f64);
        }
        self.mem.push(self.mem_pct());

        // ── диски ──
        // Нагрузка читается каждый тик, а список дисков, объём и тип — раз в ~15 тиков (это дорого и почти не меняется)
        let full = self.disk_ticks % 15 == 0;
        self.disk_ticks = self.disk_ticks.wrapping_add(1);
        let (mut rd, mut wr) = (0u64, 0u64);
        if let Some(disks) = self.disks.as_mut() {
            if full {
                disks.refresh_specifics(
                    true,
                    DiskRefreshKind::nothing().with_io_usage().with_storage().with_kind(),
                );
                self.disks_info.clear();
            } else {
                disks.refresh_specifics(false, DiskRefreshKind::nothing().with_io_usage());
            }
            for d in disks.iter() {
                let u = d.usage();
                rd += u.read_bytes;
                wr += u.written_bytes;
                if full {
                    self.disks_info.push(DiskInfo {
                        mount: d.mount_point().display().to_string(),
                        fs: d.file_system().to_string_lossy().into_owned(),
                        kind: format!("{:?}", d.kind()),
                        total: d.total_space(),
                        free: d.available_space(),
                    });
                }
            }
        }
        self.disk_read_rate = rd as f64 / self.dt;
        self.disk_write_rate = wr as f64 / self.dt;
        self.disk_read.push(self.disk_read_rate);
        self.disk_write.push(self.disk_write_rate);

        // ── сеть ──
        let (mut rx, mut tx) = (0u64, 0u64);
        self.nets_info.clear();
        if let Some(nets) = self.nets.as_mut() {
            nets.refresh(true);
        }
        for (name, n) in self.nets.iter().flat_map(|n| n.iter()) {
            rx += n.received();
            tx += n.transmitted();
            self.nets_info.push(NetInfo {
                name: name.to_string_lossy().into_owned(),
                rx_total: n.total_received(),
                tx_total: n.total_transmitted(),
                rx_rate: n.received() as f64 / self.dt,
                tx_rate: n.transmitted() as f64 / self.dt,
                mac: n.mac_address().to_string(),
            });
        }
        self.nets_info.sort_by(|a, b| a.name.cmp(&b.name));
        self.net_rx_rate = rx as f64 / self.dt;
        self.net_tx_rate = tx as f64 / self.dt;
        self.net_rx.push(self.net_rx_rate);
        self.net_tx.push(self.net_tx_rate);

        // ── видеокарты ──
        self.gpus_info.clear();
        if let Some(g) = self.gpus.as_mut() {
            g.refresh(true);
            for gpu in g.iter() {
                self.gpus_info.push(GpuInfo {
                    name: gpu.model().unwrap_or("Видеокарта").to_owned(),
                    vendor: gpu.vendor().unwrap_or("").to_owned(),
                    usage: gpu.usage(),
                    mem_used: gpu.used_memory(),
                    mem_total: gpu.total_memory(),
                });
            }
        }
        self.gpu_usage = self
            .gpus_info
            .iter()
            .filter_map(|g| g.usage)
            .fold(None, |acc: Option<f32>, u| Some(acc.map_or(u, |a| a.max(u))));
        self.gpu.push(self.gpu_usage.unwrap_or(0.0) as f64);
    }
}
