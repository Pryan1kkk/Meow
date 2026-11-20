//! Вкладка «Процессы»: список / дерево / группировка по приложениям, колонки,
//! тепловая карта нагрузки, панель деталей, контекстное меню и действия над процессами.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Key, Layout, Modifiers, OpenUrl, RichText, Sense, Ui, vec2};
use egui_extras::{Column, TableBuilder};
use sysinfo::{Pid, Users};

use crate::metrics::{History, Metrics};
use crate::settings::Settings;
use crate::theme::*;
use crate::widgets::*;
use crate::winproc::{self, Priority};
use crate::winquery::{Job, JobState};

// ───────────────────────────── колонки и режимы ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Col {
    Name,
    Pid,
    Cpu,
    Mem,
    Disk,
    Gpu,
    Status,
    Time,
    User,
    Path,
}

impl Col {
    /// Колонки, которые можно включать и выключать («Имя» есть всегда)
    pub const OPTIONAL: [Col; 9] = [
        Col::Pid,
        Col::Cpu,
        Col::Mem,
        Col::Disk,
        Col::Gpu,
        Col::Status,
        Col::Time,
        Col::User,
        Col::Path,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Col::Name => "name",
            Col::Pid => "pid",
            Col::Cpu => "cpu",
            Col::Mem => "mem",
            Col::Disk => "disk",
            Col::Gpu => "gpu",
            Col::Status => "status",
            Col::Time => "time",
            Col::User => "user",
            Col::Path => "path",
        }
    }

    pub fn from_id(s: &str) -> Option<Col> {
        [Col::Name].iter().chain(Col::OPTIONAL.iter()).copied().find(|c| c.id() == s)
    }

    fn title(self) -> &'static str {
        match self {
            Col::Name => "Имя",
            Col::Pid => "PID",
            Col::Cpu => "ЦП",
            Col::Mem => "Память",
            Col::Disk => "Диск",
            Col::Gpu => "GPU",
            Col::Status => "Состояние",
            Col::Time => "Время работы",
            Col::User => "Пользователь",
            Col::Path => "Путь",
        }
    }

    fn width(self) -> f32 {
        match self {
            Col::Name => 240.0,
            Col::Pid => 70.0,
            Col::Cpu | Col::Gpu => 80.0,
            Col::Mem => 100.0,
            Col::Disk => 110.0,
            Col::Status => 120.0,
            Col::Time => 130.0,
            Col::User => 140.0,
            Col::Path => 320.0,
        }
    }

    /// Числовые колонки: выравнивание вправо, заголовок с итогом, тепловая подсветка
    fn numeric(self) -> bool {
        matches!(self, Col::Cpu | Col::Mem | Col::Disk | Col::Gpu)
    }

    /// По этим колонкам сначала показываем «самые тяжёлые»
    fn desc_first(self) -> bool {
        matches!(self, Col::Cpu | Col::Mem | Col::Disk | Col::Gpu | Col::Time)
    }

    fn sortable(self) -> bool {
        self != Col::Status
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    List,
    Tree,
    Group,
}

impl View {
    pub fn id(self) -> &'static str {
        match self {
            View::List => "list",
            View::Tree => "tree",
            View::Group => "group",
        }
    }
    pub fn from_id(s: &str) -> View {
        match s {
            "list" => View::List,
            "tree" => View::Tree,
            _ => View::Group,
        }
    }
}

// ───────────────────────────── данные ─────────────────────────────

#[derive(Clone)]
pub struct ProcRow {
    pub pid: Pid,
    pub ppid: Option<Pid>,
    pub name: String,
    pub name_lc: String,
    pub exe: Option<PathBuf>,
    pub cmd: String,
    pub cwd: String,
    pub user: String,
    pub cpu: f32,
    pub mem: u64,
    pub virt: u64,
    pub disk_read: f64,
    pub disk_write: f64,
    pub gpu: Option<f32>,
    pub run_time: u64,
}

impl ProcRow {
    fn disk(&self) -> f64 {
        self.disk_read + self.disk_write
    }
    fn path_text(&self) -> String {
        self.exe.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
    }
}

#[derive(Clone)]
struct GroupAgg {
    name: String,
    key: String,
    members: Vec<usize>, // индексы в `rows`
    cpu: f32,
    mem: u64,
    disk: f64,
    gpu: Option<f32>,
}

#[derive(Clone)]
enum DKind {
    Proc(usize),
    Group(Box<GroupAgg>),
}

#[derive(Clone)]
struct DRow {
    kind: DKind,
    depth: usize,
    key: String,
    expandable: bool,
    expanded: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Sel {
    Proc(Pid),
    Group(String),
}

/// Значения для сортировки (одинаковые для процесса и для группы)
struct Vals {
    name: String,
    pid: u32,
    cpu: f32,
    mem: u64,
    disk: f64,
    gpu: f32,
    time: u64,
    user: String,
    path: String,
}

impl Vals {
    fn of_row(r: &ProcRow) -> Self {
        Self {
            name: r.name_lc.clone(),
            pid: r.pid.as_u32(),
            cpu: r.cpu,
            mem: r.mem,
            disk: r.disk(),
            gpu: r.gpu.unwrap_or(0.0),
            time: r.run_time,
            user: r.user.to_lowercase(),
            path: r.path_text().to_lowercase(),
        }
    }
}

fn cmp_vals(a: &Vals, b: &Vals, col: Col, desc: bool) -> Ordering {
    let fcmp = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    let ord = match col {
        Col::Name | Col::Status => a.name.cmp(&b.name),
        Col::Pid => a.pid.cmp(&b.pid),
        Col::Cpu => fcmp(a.cpu as f64, b.cpu as f64),
        Col::Mem => a.mem.cmp(&b.mem),
        Col::Disk => fcmp(a.disk, b.disk),
        Col::Gpu => fcmp(a.gpu as f64, b.gpu as f64),
        Col::Time => a.time.cmp(&b.time),
        Col::User => a.user.cmp(&b.user),
        Col::Path => a.path.cmp(&b.path),
    };
    let ord = if desc { ord.reverse() } else { ord };
    // при равенстве порядок должен быть стабильным, иначе строки «прыгают» между обновлениями
    ord.then_with(|| a.name.cmp(&b.name)).then_with(|| a.pid.cmp(&b.pid))
}

/// Критичные процессы Windows: их завершение приводит к сбою или синему экрану
pub fn is_protected(name: &str) -> bool {
    const LIST: [&str; 11] = [
        "system",
        "registry",
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "winlogon.exe",
        "services.exe",
        "lsass.exe",
        "system idle process",
        "memory compression",
        "secure system",
    ];
    LIST.iter().any(|p| name.eq_ignore_ascii_case(p))
}

#[derive(Clone)]
enum Action {
    Kill(Vec<Pid>),
    KillTree(Vec<Pid>),
    Suspend(Pid),
    Resume(Pid),
    SetPriority(Pid, Priority),
    OpenLocation(Pid),
    SearchWeb(String),
    Copy(String),
    Select(Sel),
    Toggle(String),
}

// ───────────────────────────── вкладка ─────────────────────────────

pub struct ProcessesTab {
    pub view: View,
    pub sort: Col,
    pub sort_desc: bool,
    pub cols: Vec<Col>,
    pub filter: String,
    pub focus_search: bool,
    pub details: bool,
    pub confirm_kill_tree: bool,
    pub status: Option<(String, Instant)>,
    /// Просьба к главному циклу обновить данные раньше срока (после завершения процесса и т. п.)
    pub force_refresh: bool,

    rows: Vec<ProcRow>,
    by_pid: HashMap<Pid, usize>,
    children: HashMap<Pid, Vec<usize>>,
    display: Vec<DRow>,
    dirty: bool,
    selected: Option<Sel>,
    scroll_to: Option<usize>,
    tree_collapsed: HashSet<String>,
    group_expanded: HashSet<String>,
    suspended: HashSet<u32>,
    pending: Option<Action>,
    /// После перестроения списка прокрутить к выбранной строке
    want_scroll_sel: bool,

    users: Option<Users>,
    /// Список пользователей читается в фоне: на доменных ПК без связи с контроллером это может занять секунды
    users_job: Option<Job<Option<Users>>>,
    snapshots: u32,
    sel_hist_cpu: History,
    sel_hist_mem: History,
    sel_hist_pid: Option<Pid>,
    sel_priority: Option<Priority>,
    me: Pid,
}

impl ProcessesTab {
    pub fn new(s: &Settings) -> Self {
        let mut cols: Vec<Col> = s.columns.iter().filter_map(|c| Col::from_id(c)).filter(|c| *c != Col::Name).collect();
        cols.sort_by_key(|c| Col::OPTIONAL.iter().position(|o| o == c));
        cols.dedup();
        Self {
            view: View::from_id(&s.view),
            sort: Col::from_id(&s.sort).unwrap_or(Col::Cpu),
            sort_desc: s.sort_desc,
            cols,
            filter: String::new(),
            focus_search: false,
            details: true,
            confirm_kill_tree: s.confirm_kill_tree,
            status: None,
            force_refresh: false,
            rows: Vec::new(),
            by_pid: HashMap::new(),
            children: HashMap::new(),
            display: Vec::new(),
            dirty: true,
            selected: None,
            scroll_to: None,
            tree_collapsed: HashSet::new(),
            group_expanded: HashSet::new(),
            suspended: HashSet::new(),
            pending: None,
            want_scroll_sel: false,
            users: None,
            users_job: Some(Job::spawn(|| Users::new_with_refreshed_list().ok())),
            snapshots: 0,
            sel_hist_cpu: History::new(),
            sel_hist_mem: History::new(),
            sel_hist_pid: None,
            sel_priority: None,
            me: Pid::from_u32(std::process::id()),
        }
    }

    /// Записывает видимые пользователю настройки обратно в `Settings`
    pub fn sync_to(&self, s: &mut Settings) {
        s.view = self.view.id().to_owned();
        s.sort = self.sort.id().to_owned();
        s.sort_desc = self.sort_desc;
        s.columns = self.cols.iter().map(|c| c.id().to_owned()).collect();
    }

    pub fn process_count(&self) -> usize {
        self.rows.len()
    }

    /// Нужна ли sysinfo загрузка GPU по процессам (это дороже, поэтому только если колонка включена)
    pub fn wants_gpu(&self) -> bool {
        self.cols.contains(&Col::Gpu)
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    // ───────────── снимок процессов ─────────────

    /// Вызывается после каждого обновления `Metrics`
    pub fn on_refresh(&mut self, m: &Metrics) {
        self.snapshots += 1;
        if let Some(job) = &self.users_job {
            match job.poll() {
                JobState::Running => {}
                JobState::Done(list) => {
                    self.users_job = None;
                    if list.is_some() {
                        self.users = list;
                    }
                }
                JobState::Failed => self.users_job = None,
            }
        }
        if self.snapshots % 60 == 0 && self.users_job.is_none() {
            self.users_job = Some(Job::spawn(|| Users::new_with_refreshed_list().ok()));
        }
        let ncpu = m.sys.cpus().len().max(1) as f32;
        let dt = m.dt;

        self.rows = m
            .sys
            .processes()
            .iter()
            .map(|(pid, p)| {
                let name = p.name().to_string_lossy().into_owned();
                let du = p.disk_usage();
                let user = p
                    .user_id()
                    .and_then(|u| self.users.as_ref().and_then(|us| us.get_user_by_id(u)))
                    .map(|u| u.name().to_owned())
                    .unwrap_or_default();
                ProcRow {
                    pid: *pid,
                    ppid: p.parent().filter(|pp| pp != pid),
                    name_lc: name.to_lowercase(),
                    name,
                    exe: p.exe().map(|e| e.to_path_buf()),
                    cmd: p.cmd().iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "),
                    cwd: p.cwd().map(|c| c.display().to_string()).unwrap_or_default(),
                    user,
                    // sysinfo отдаёт загрузку относительно одного ядра — приводим к 0..100% всей системы
                    cpu: p.cpu_usage() / ncpu,
                    mem: p.memory(),
                    virt: p.virtual_memory(),
                    disk_read: du.read_bytes as f64 / dt,
                    disk_write: du.written_bytes as f64 / dt,
                    gpu: p.gpu_usage(),
                    run_time: p.run_time(),
                }
            })
            .collect();

        self.reindex();
        // забываем «приостановлен» для умерших процессов
        let alive: HashSet<u32> = self.rows.iter().map(|r| r.pid.as_u32()).collect();
        self.suspended.retain(|p| alive.contains(p));

        // выбранный процесс мог завершиться
        if let Some(Sel::Proc(pid)) = &self.selected {
            if !self.by_pid.contains_key(pid) {
                self.selected = None;
            }
        }
        // история выбранного процесса для мини-графиков
        let sel_pid = match &self.selected {
            Some(Sel::Proc(p)) => Some(*p),
            _ => None,
        };
        if sel_pid != self.sel_hist_pid {
            self.sel_hist_cpu = History::new();
            self.sel_hist_mem = History::new();
            self.sel_hist_pid = sel_pid;
            self.sel_priority = sel_pid.and_then(|p| winproc::get_priority(p.as_u32()));
        }
        if let Some(i) = sel_pid.and_then(|p| self.by_pid.get(&p)) {
            let r = &self.rows[*i];
            self.sel_hist_cpu.push(r.cpu as f64);
            self.sel_hist_mem.push(r.mem as f64);
        }
        self.dirty = true;
    }

    /// Перестраивает индексы «PID → строка» и «родитель → дети» после смены списка строк
    fn reindex(&mut self) {
        self.by_pid = self.rows.iter().enumerate().map(|(i, r)| (r.pid, i)).collect();
        self.children.clear();
        for (i, r) in self.rows.iter().enumerate() {
            if let Some(pp) = r.ppid {
                if self.by_pid.contains_key(&pp) {
                    self.children.entry(pp).or_default().push(i);
                }
            }
        }
    }

    // ───────────── построение отображаемого списка ─────────────

    fn matches_filter(r: &ProcRow, f: &str) -> bool {
        r.name_lc.contains(f)
            || r.pid.to_string().contains(f)
            || r.user.to_lowercase().contains(f)
            || r.path_text().to_lowercase().contains(f)
            || r.cmd.to_lowercase().contains(f)
    }

    fn rebuild(&mut self) {
        self.dirty = false;
        let filter = self.filter.trim().to_lowercase();
        let (col, desc) = (self.sort, self.sort_desc);
        let vals: Vec<Vals> = self.rows.iter().map(Vals::of_row).collect();
        let sort_idx = |v: &mut Vec<usize>| v.sort_by(|a, b| cmp_vals(&vals[*a], &vals[*b], col, desc));

        let mut out: Vec<DRow> = Vec::new();
        let flat = |i: usize| DRow {
            kind: DKind::Proc(i),
            depth: 0,
            key: format!("p{}", self.rows[i].pid),
            expandable: false,
            expanded: false,
        };

        if !filter.is_empty() || self.view == View::List {
            // при поиске всегда плоский список: так результаты не прячутся в свёрнутых ветках
            let mut idx: Vec<usize> = (0..self.rows.len())
                .filter(|i| filter.is_empty() || Self::matches_filter(&self.rows[*i], &filter))
                .collect();
            sort_idx(&mut idx);
            out.extend(idx.into_iter().map(flat));
        } else if self.view == View::Tree {
            let mut roots: Vec<usize> = (0..self.rows.len())
                .filter(|i| match self.rows[*i].ppid {
                    Some(pp) => !self.by_pid.contains_key(&pp),
                    None => true,
                })
                .collect();
            sort_idx(&mut roots);
            // Что достижимо от корней (независимо от свёрнутости веток). Всё остальное — участники
            // циклов родителей: их показываем отдельно, чтобы они не пропали из списка.
            let mut reachable: HashSet<usize> = HashSet::new();
            let mut walk: Vec<usize> = roots.clone();
            while let Some(i) = walk.pop() {
                if !reachable.insert(i) {
                    continue;
                }
                if let Some(kids) = self.children.get(&self.rows[i].pid) {
                    walk.extend(kids.iter().copied());
                }
            }
            let mut seen: HashSet<usize> = HashSet::new();
            // обход в глубину с явным стеком: (индекс, глубина)
            let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|i| (*i, 0)).collect();
            while let Some((i, depth)) = stack.pop() {
                if !seen.insert(i) {
                    continue;
                }
                let key = format!("p{}", self.rows[i].pid);
                let mut kids = self.children.get(&self.rows[i].pid).cloned().unwrap_or_default();
                sort_idx(&mut kids);
                let expanded = !self.tree_collapsed.contains(&key);
                out.push(DRow { kind: DKind::Proc(i), depth, key, expandable: !kids.is_empty(), expanded });
                if expanded {
                    stack.extend(kids.into_iter().rev().map(|k| (k, depth + 1)));
                }
            }
            // процессы, оказавшиеся в цикле родителей, не должны пропасть из списка
            let mut rest: Vec<usize> = (0..self.rows.len()).filter(|i| !reachable.contains(i)).collect();
            sort_idx(&mut rest);
            out.extend(rest.into_iter().map(flat));
        } else {
            let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
            for (i, r) in self.rows.iter().enumerate() {
                groups.entry(r.name_lc.as_str()).or_default().push(i);
            }
            // элемент верхнего уровня: одиночный процесс или группа
            struct Top {
                vals: Vals,
                kind: DKind,
            }
            let mut tops: Vec<Top> = Vec::new();
            for (key, mut members) in groups {
                if members.len() == 1 {
                    let i = members[0];
                    tops.push(Top { vals: Vals::of_row(&self.rows[i]), kind: DKind::Proc(i) });
                    continue;
                }
                members.sort_by(|a, b| cmp_vals(&vals[*a], &vals[*b], col, desc));
                let gpu_sum: f32 = members.iter().filter_map(|i| self.rows[*i].gpu).sum();
                let any_gpu = members.iter().any(|i| self.rows[*i].gpu.is_some());
                let agg = GroupAgg {
                    name: self.rows[members[0]].name.clone(),
                    key: key.to_owned(),
                    cpu: members.iter().map(|i| self.rows[*i].cpu).sum(),
                    mem: members.iter().map(|i| self.rows[*i].mem).sum(),
                    disk: members.iter().map(|i| self.rows[*i].disk()).sum(),
                    gpu: any_gpu.then_some(gpu_sum),
                    members,
                };
                let first = &self.rows[agg.members[0]];
                let top_vals = Vals {
                    name: key.to_owned(),
                    pid: agg.members.iter().map(|i| self.rows[*i].pid.as_u32()).min().unwrap_or(0),
                    cpu: agg.cpu,
                    mem: agg.mem,
                    disk: agg.disk,
                    gpu: agg.gpu.unwrap_or(0.0),
                    time: agg.members.iter().map(|i| self.rows[*i].run_time).max().unwrap_or(0),
                    user: first.user.to_lowercase(),
                    path: first.path_text().to_lowercase(),
                };
                tops.push(Top { vals: top_vals, kind: DKind::Group(Box::new(agg)) });
            }
            tops.sort_by(|a, b| cmp_vals(&a.vals, &b.vals, col, desc));
            for t in tops {
                match t.kind {
                    DKind::Proc(i) => out.push(flat(i)),
                    DKind::Group(g) => {
                        let key = format!("g{}", g.key);
                        let expanded = self.group_expanded.contains(&key);
                        let members = g.members.clone();
                        out.push(DRow { kind: DKind::Group(g), depth: 0, key, expandable: true, expanded });
                        if expanded {
                            out.extend(members.into_iter().map(|i| DRow { depth: 1, ..flat(i) }));
                        }
                    }
                }
            }
        }
        self.display = out;
    }

    // ───────────── выбор ─────────────

    fn sel_of(d: &DRow, rows: &[ProcRow]) -> Sel {
        match &d.kind {
            DKind::Proc(i) => Sel::Proc(rows[*i].pid),
            DKind::Group(g) => Sel::Group(g.key.clone()),
        }
    }

    fn sel_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.display.iter().position(|d| Self::sel_of(d, &self.rows) == *sel)
    }

    fn sel_pids(&self) -> Vec<Pid> {
        match &self.selected {
            Some(Sel::Proc(p)) => vec![*p],
            Some(Sel::Group(key)) => self.rows.iter().filter(|r| &r.name_lc == key).map(|r| r.pid).collect(),
            None => Vec::new(),
        }
    }

    fn toggle_key(&mut self, key: &str) {
        let set = if key.starts_with('g') { &mut self.group_expanded } else { &mut self.tree_collapsed };
        if !set.remove(key) {
            set.insert(key.to_owned());
        }
        self.dirty = true;
    }

    fn set_expanded(&mut self, idx: usize, expand: bool) {
        let d = &self.display[idx];
        if !d.expandable || d.expanded == expand {
            return;
        }
        let key = d.key.clone();
        self.toggle_key(&key);
    }

    fn select(&mut self, s: Sel) {
        self.selected = Some(s);
    }

    // ───────────── действия ─────────────

    /// Процесс и все его потомки; потомки идут раньше родителя
    fn tree_of(&self, root: Pid) -> Vec<Pid> {
        let mut order = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![root];
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            order.push(pid);
            if let Some(kids) = self.children.get(&pid) {
                stack.extend(kids.iter().map(|i| self.rows[*i].pid));
            }
        }
        order.reverse();
        order
    }

    fn name_of(&self, pid: Pid) -> String {
        self.by_pid.get(&pid).map(|i| self.rows[*i].name.clone()).unwrap_or_else(|| pid.to_string())
    }

    fn kill_one(&self, m: &Metrics, pid: Pid) -> Result<(), String> {
        let Some(p) = m.sys.process(pid) else {
            return Err("процесс уже не существует".to_owned());
        };
        let name = p.name().to_string_lossy().into_owned();
        if pid == self.me {
            return Err("это сам TaskMgr: просто закрой окно".to_owned());
        }
        if is_protected(&name) {
            return Err(format!("«{name}» — защищённый процесс, его завершение приведёт к сбою Windows"));
        }
        if p.kill() {
            Ok(())
        } else {
            Err(format!("не удалось завершить «{name}»: возможно, нужны права администратора"))
        }
    }

    fn do_kill(&mut self, m: &Metrics, pids: &[Pid], tree: bool) -> String {
        let targets: Vec<Pid> = if tree {
            let mut seen = HashSet::new();
            pids.iter().flat_map(|p| self.tree_of(*p)).filter(|p| seen.insert(*p)).collect()
        } else {
            pids.to_vec()
        };
        let (mut ok, mut failed, mut first_err) = (0, 0, None);
        for pid in &targets {
            match self.kill_one(m, *pid) {
                Ok(()) => ok += 1,
                Err(e) => {
                    failed += 1;
                    first_err.get_or_insert(e);
                }
            }
        }
        let what = if pids.len() == 1 { format!("«{}»", self.name_of(pids[0])) } else { format!("{} процессов", pids.len()) };
        match (ok, failed) {
            (0, _) => format!("Не завершено {what}: {}", first_err.unwrap_or_default()),
            (_, 0) if tree => format!("Дерево {what}: завершено {ok}"),
            (_, 0) => format!("Завершено: {what}"),
            _ => format!("Завершено {ok}, не удалось {failed}: {}", first_err.unwrap_or_default()),
        }
    }

    fn open_location(&self, pid: Pid) -> String {
        let Some(path) = self.by_pid.get(&pid).and_then(|i| self.rows[*i].exe.clone()) else {
            return "Путь к файлу недоступен (нужны права администратора)".to_owned();
        };
        let mut cmd = std::process::Command::new(if cfg!(windows) { "explorer" } else { "xdg-open" });
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.raw_arg(format!("/select,\"{}\"", path.display()));
        }
        #[cfg(not(windows))]
        cmd.arg(path.parent().map(|p| p.display().to_string()).unwrap_or_default());
        match cmd.spawn() {
            Ok(_) => format!("Открываю: {}", path.display()),
            Err(e) => format!("Не удалось открыть проводник: {e}"),
        }
    }

    fn exec(&mut self, ctx: &egui::Context, m: &Metrics, a: Action) {
        match a {
            Action::Kill(pids) => {
                let msg = self.do_kill(m, &pids, false);
                self.say(msg);
                self.force_refresh = true;
            }
            Action::KillTree(pids) => {
                // завершение целых деревьев необратимо — по умолчанию спрашиваем подтверждение
                if self.confirm_kill_tree {
                    self.pending = Some(Action::KillTree(pids));
                } else {
                    let msg = self.do_kill(m, &pids, true);
                    self.say(msg);
                    self.force_refresh = true;
                }
            }
            Action::Suspend(pid) => {
                let name = self.name_of(pid);
                match winproc::suspend(pid.as_u32()) {
                    Ok(()) => {
                        self.suspended.insert(pid.as_u32());
                        self.say(format!("«{name}» приостановлен"));
                    }
                    Err(e) => self.say(format!("Не удалось приостановить «{name}»: {e}")),
                }
            }
            Action::Resume(pid) => {
                let name = self.name_of(pid);
                match winproc::resume(pid.as_u32()) {
                    Ok(()) => {
                        self.suspended.remove(&pid.as_u32());
                        self.say(format!("«{name}» возобновлён"));
                    }
                    Err(e) => self.say(format!("Не удалось возобновить «{name}»: {e}")),
                }
            }
            Action::SetPriority(pid, p) => {
                let name = self.name_of(pid);
                match winproc::set_priority(pid.as_u32(), p) {
                    Ok(()) => {
                        self.sel_priority = Some(p);
                        self.say(format!("Приоритет «{name}»: {}", p.label().to_lowercase()));
                    }
                    Err(e) => self.say(format!("Не удалось изменить приоритет «{name}»: {e}")),
                }
            }
            Action::OpenLocation(pid) => {
                let msg = self.open_location(pid);
                self.say(msg);
            }
            Action::SearchWeb(name) => {
                ctx.open_url(OpenUrl::new_tab(format!("https://www.google.com/search?q={}", url_encode(&name))));
            }
            Action::Copy(text) => {
                ctx.copy_text(text);
                self.say("Скопировано в буфер обмена");
            }
            Action::Select(s) => self.select(s),
            Action::Toggle(key) => self.toggle_key(&key),
        }
    }

    // ───────────── клавиатура ─────────────

    /// Обрабатывает горячие клавиши; завершение процесса возвращается как действие,
    /// потому что выполнять его должен тот, у кого есть доступ к `Metrics`
    fn handle_keys(&mut self, ctx: &egui::Context) -> Option<Action> {
        if ctx.egui_wants_keyboard_input() {
            // идёт ввод в строке поиска: Esc очищает её
            if ctx.input(|i| i.key_pressed(Key::Escape)) && !self.filter.is_empty() {
                self.filter.clear();
                self.dirty = true;
            }
            return None;
        }
        let (up, down, left, right, del, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if self.dirty {
            self.rebuild();
        }
        if (up || down) && !self.display.is_empty() {
            let cur = self.sel_index();
            let next = match (cur, up) {
                (None, _) => 0,
                (Some(i), true) => i.saturating_sub(1),
                (Some(i), false) => (i + 1).min(self.display.len() - 1),
            };
            let s = Self::sel_of(&self.display[next], &self.rows);
            self.select(s);
            self.scroll_to = Some(next);
        }
        if let Some(i) = self.sel_index() {
            if left {
                // стрелка влево: свернуть ветку или перейти к родителю
                if self.display[i].expandable && self.display[i].expanded {
                    self.set_expanded(i, false);
                } else if self.display[i].depth > 0 {
                    if let Some(parent) = (0..i).rev().find(|j| self.display[*j].depth < self.display[i].depth) {
                        let s = Self::sel_of(&self.display[parent], &self.rows);
                        self.select(s);
                        self.scroll_to = Some(parent);
                    }
                }
            }
            if right {
                self.set_expanded(i, true);
            }
        }
        if esc {
            self.selected = None;
        }
        if del {
            let pids = self.sel_pids();
            if !pids.is_empty() {
                return Some(Action::Kill(pids));
            }
        }
        None
    }
}

// ───────────────────────────── интерфейс ─────────────────────────────

/// Значения одной строки таблицы, подготовленные для вывода
struct Cells {
    name: String,
    count: usize,
    pid: Option<u32>,
    cpu: f32,
    mem: u64,
    disk: f64,
    gpu: Option<f32>,
    status: &'static str,
    time: Option<u64>,
    user: String,
    path: String,
    suspended: bool,
    is_me: bool,
}

impl ProcessesTab {
    pub fn ui(&mut self, ui: &mut Ui, m: &mut Metrics) {
        let ctx = ui.ctx().clone();
        let mut actions: Vec<Action> = Vec::new();
        if let Some(a) = self.handle_keys(&ctx) {
            actions.push(a);
        }
        if self.dirty {
            self.rebuild();
        }
        if self.want_scroll_sel {
            self.want_scroll_sel = false;
            self.scroll_to = self.sel_index();
        }

        self.toolbar(ui, &mut actions);
        ui.add_space(10.0);

        if self.details && self.selected.is_some() {
            egui::Panel::right("proc_details")
                .resizable(true)
                .default_size(340.0)
                .frame(egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 0, top: 0, bottom: 0 }))
                .show(ui, |ui| self.details_ui(ui, &mut actions));
        }
        self.table(ui, m, &mut actions);

        for a in actions {
            self.exec(&ctx, m, a);
        }
        self.confirm_modal(&ctx, m);
    }

    fn toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal_wrapped(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Поиск по имени, PID, пользователю, пути  (Ctrl+F)")
                    .desired_width(300.0)
                    .margin(egui::Margin::symmetric(10, 7)),
            );
            if self.focus_search {
                resp.request_focus();
                self.focus_search = false;
            }
            if resp.changed() {
                self.dirty = true;
            }

            let mut v = self.view;
            if segmented(ui, &mut v, &[(View::List, "Список"), (View::Tree, "Дерево"), (View::Group, "По приложениям")]) {
                self.view = v;
                self.dirty = true;
            }

            ui.menu_button("Колонки ▾", |ui| {
                for c in Col::OPTIONAL {
                    let mut on = self.cols.contains(&c);
                    if ui.checkbox(&mut on, c.title()).changed() {
                        if on {
                            self.cols.push(c);
                            self.cols.sort_by_key(|x| Col::OPTIONAL.iter().position(|o| o == x));
                        } else {
                            self.cols.retain(|x| *x != c);
                        }
                    }
                }
            });
            ui.toggle_value(&mut self.details, "Подробности");
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            let pids = self.sel_pids();
            let has = !pids.is_empty();
            let btn = |text: &str, color: Color32, enabled: bool| {
                (egui::Button::new(RichText::new(text).color(Color32::WHITE).size(13.0))
                    .fill(if enabled { color } else { BG_CARD })
                    .corner_radius(8.0)
                    .min_size(vec2(0.0, 30.0)), enabled)
            };
            let (b, en) = btn("✖  Завершить задачу", DANGER, has);
            if ui.add_enabled(en, b).on_hover_text("Delete").clicked() {
                actions.push(Action::Kill(pids.clone()));
            }
            let (b, en) = btn("Завершить дерево", DANGER.gamma_multiply(0.65), has);
            if ui
                .add_enabled(en, b)
                .on_hover_text("Завершить процесс вместе со всеми дочерними")
                .clicked()
            {
                actions.push(Action::KillTree(pids.clone()));
            }
            if let Some(Sel::Proc(pid)) = self.selected.clone() {
                let is_susp = self.suspended.contains(&pid.as_u32());
                let (label, act) = if is_susp {
                    ("▶  Возобновить", Action::Resume(pid))
                } else {
                    ("⏸  Приостановить", Action::Suspend(pid))
                };
                let (b, en) = btn(label, BG_HOVER, cfg!(windows));
                if ui.add_enabled(en, b).clicked() {
                    actions.push(act);
                }
            }
            if let Some((msg, t)) = &self.status {
                if t.elapsed() < Duration::from_secs(8) {
                    ui.add_space(8.0);
                    ui.label(dim(msg.clone()));
                }
            }
        });
    }

    fn cells(&self, d: &DRow) -> Cells {
        match &d.kind {
            DKind::Proc(i) => {
                let r = &self.rows[*i];
                let suspended = self.suspended.contains(&r.pid.as_u32());
                Cells {
                    name: r.name.clone(),
                    count: 0,
                    pid: Some(r.pid.as_u32()),
                    cpu: r.cpu,
                    mem: r.mem,
                    disk: r.disk(),
                    gpu: r.gpu,
                    status: if suspended { "Приостановлен" } else { "Работает" },
                    time: Some(r.run_time),
                    user: r.user.clone(),
                    path: r.path_text(),
                    suspended,
                    is_me: r.pid == self.me,
                }
            }
            DKind::Group(g) => Cells {
                name: g.name.clone(),
                count: g.members.len(),
                pid: None,
                cpu: g.cpu,
                mem: g.mem,
                disk: g.disk,
                gpu: g.gpu,
                status: "",
                time: None,
                user: String::new(),
                path: String::new(),
                suspended: false,
                is_me: false,
            },
        }
    }

    fn table(&mut self, ui: &mut Ui, m: &Metrics, actions: &mut Vec<Action>) {
        let total_mem = m.mem_total().max(1) as f64;
        let (sort, sort_desc) = (self.sort, self.sort_desc);
        let sel = self.selected.clone();
        let mut new_sort: Option<Col> = None;
        let mut select: Option<Sel> = None;
        let cols = self.cols.clone();
        let all_cols: Vec<Col> = std::iter::once(Col::Name).chain(cols.iter().copied()).collect();
        let scroll_to = self.scroll_to.take();
        let (tot_cpu, tot_mem, tot_disk, tot_gpu) =
            (m.cpu_total as f64, m.mem_pct(), m.disk_read_rate + m.disk_write_rate, m.gpu_usage);

        let this = &*self;
        egui::Frame::new()
            .fill(BG_CARD)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                let mut table = TableBuilder::new(ui)
                    .striped(false)
                    .resizable(true)
                    .sense(Sense::click())
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .max_scroll_height(10_000.0)
                    .column(Column::remainder().at_least(200.0).clip(true));
                for c in &cols {
                    table = table.column(Column::initial(c.width()).at_least(56.0).clip(true));
                }
                if let Some(i) = scroll_to {
                    table = table.scroll_to_row(i, Some(Align::Center));
                }

                table
                    .header(46.0, |mut header| {
                        for c in &all_cols {
                            let c = *c;
                            header.col(|ui| {
                                let active = sort == c;
                                let arrow = if active { if sort_desc { " ▼" } else { " ▲" } } else { "" };
                                let color = if active { ACCENT } else { TEXT_DIM };
                                let layout = if c.numeric() {
                                    Layout::top_down(Align::Max)
                                } else {
                                    Layout::top_down(Align::Min)
                                };
                                ui.with_layout(layout, |ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    if c.numeric() {
                                        let top = match c {
                                            Col::Cpu => fmt_pct(tot_cpu),
                                            Col::Mem => fmt_pct(tot_mem),
                                            Col::Disk => fmt_rate(tot_disk),
                                            _ => tot_gpu.map(|g| fmt_pct(g as f64)).unwrap_or_else(|| "—".into()),
                                        };
                                        ui.label(RichText::new(top).size(14.0).color(TEXT));
                                    }
                                    let text = RichText::new(format!("{}{arrow}", c.title())).size(12.5).strong().color(color);
                                    if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() && c.sortable() {
                                        new_sort = Some(c);
                                    }
                                });
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(28.0, this.display.len(), |mut row| {
                            let d = &this.display[row.index()];
                            let this_sel = Self::sel_of(d, &this.rows);
                            row.set_selected(sel.as_ref() == Some(&this_sel));
                            let cl = this.cells(d);

                            for c in &all_cols {
                                let c = *c;
                                row.col(|ui| match c {
                                    Col::Name => {
                                        ui.add_space(6.0 + d.depth as f32 * 18.0);
                                        if d.expandable {
                                            let arrow = if d.expanded { "▾" } else { "▸" };
                                            let r = ui.add(
                                                egui::Label::new(RichText::new(arrow).size(14.0).color(TEXT_DIM))
                                                    .sense(Sense::click()),
                                            );
                                            if r.clicked() {
                                                actions.push(Action::Toggle(d.key.clone()));
                                            }
                                        } else {
                                            ui.add_space(14.0);
                                        }
                                        let color = if cl.is_me {
                                            ACCENT
                                        } else if cl.suspended {
                                            AMBER
                                        } else {
                                            TEXT
                                        };
                                        let prefix = if cl.suspended { "⏸ " } else { "" };
                                        ui.label(RichText::new(format!("{prefix}{}", cl.name)).color(color));
                                        if cl.count > 0 {
                                            ui.label(dim(format!("({})", cl.count)));
                                        }
                                    }
                                    Col::Pid => {
                                        if let Some(p) = cl.pid {
                                            ui.label(RichText::new(p.to_string()).monospace().color(TEXT_DIM));
                                        }
                                    }
                                    Col::Cpu => heat_cell(ui, (cl.cpu as f64 / 25.0).min(1.0), &fmt_pct(cl.cpu as f64)),
                                    Col::Mem => heat_cell(ui, cl.mem as f64 / total_mem * 6.0, &fmt_bytes(cl.mem)),
                                    Col::Disk => heat_cell(ui, cl.disk / 50e6, &fmt_rate(cl.disk)),
                                    Col::Gpu => match cl.gpu {
                                        Some(g) => heat_cell(ui, g as f64 / 100.0, &fmt_pct(g as f64)),
                                        None => heat_cell(ui, 0.0, "—"),
                                    },
                                    Col::Status => {
                                        ui.label(dim(cl.status));
                                    }
                                    Col::Time => {
                                        if let Some(t) = cl.time {
                                            ui.label(dim(fmt_duration(t)));
                                        }
                                    }
                                    Col::User => {
                                        ui.label(dim(cl.user.clone()));
                                    }
                                    Col::Path => {
                                        ui.label(dim(cl.path.clone())).on_hover_text(&cl.path);
                                    }
                                });
                            }

                            let resp = row.response();
                            if resp.clicked() || resp.secondary_clicked() {
                                select = Some(this_sel.clone());
                            }
                            if resp.double_clicked() && d.expandable {
                                actions.push(Action::Toggle(d.key.clone()));
                            }
                            resp.context_menu(|ui| {
                                select = Some(this_sel.clone());
                                Self::row_menu(ui, d, &this.rows, &this.suspended, actions);
                            });
                        });
                    });
            });

        if let Some(c) = new_sort {
            if self.sort == c {
                self.sort_desc = !self.sort_desc;
            } else {
                self.sort = c;
                self.sort_desc = c.desc_first();
            }
            self.dirty = true;
        }
        if let Some(s) = select {
            self.selected = Some(s);
        }
    }

    fn row_menu(ui: &mut Ui, d: &DRow, rows: &[ProcRow], suspended: &HashSet<u32>, actions: &mut Vec<Action>) {
        match &d.kind {
            DKind::Proc(i) => {
                let r = &rows[*i];
                let pid = r.pid;
                if ui.button("Завершить задачу").clicked() {
                    actions.push(Action::Kill(vec![pid]));
                    ui.close();
                }
                if ui.button("Завершить дерево процессов").clicked() {
                    actions.push(Action::KillTree(vec![pid]));
                    ui.close();
                }
                ui.separator();
                if suspended.contains(&pid.as_u32()) {
                    if ui.button("Возобновить").clicked() {
                        actions.push(Action::Resume(pid));
                        ui.close();
                    }
                } else if ui.button("Приостановить").clicked() {
                    actions.push(Action::Suspend(pid));
                    ui.close();
                }
                ui.menu_button("Приоритет", |ui| {
                    for p in Priority::ALL {
                        if ui.button(p.label()).clicked() {
                            actions.push(Action::SetPriority(pid, p));
                            ui.close();
                        }
                    }
                });
                if ui.button("Режим эффективности (низкий приоритет)").clicked() {
                    actions.push(Action::SetPriority(pid, Priority::Idle));
                    ui.close();
                }
                ui.separator();
                if ui.add_enabled(r.exe.is_some(), egui::Button::new("Открыть расположение файла")).clicked() {
                    actions.push(Action::OpenLocation(pid));
                    ui.close();
                }
                if ui.button("Поиск в интернете").clicked() {
                    actions.push(Action::SearchWeb(r.name.clone()));
                    ui.close();
                }
                ui.separator();
                for (label, text) in [
                    ("Копировать PID", pid.to_string()),
                    ("Копировать имя", r.name.clone()),
                    ("Копировать путь", r.path_text()),
                    ("Копировать командную строку", r.cmd.clone()),
                ] {
                    if ui.add_enabled(!text.is_empty(), egui::Button::new(label)).clicked() {
                        actions.push(Action::Copy(text));
                        ui.close();
                    }
                }
            }
            DKind::Group(g) => {
                let pids: Vec<Pid> = g.members.iter().map(|i| rows[*i].pid).collect();
                if ui.button(format!("Завершить все ({})", pids.len())).clicked() {
                    actions.push(Action::Kill(pids.clone()));
                    ui.close();
                }
                if ui.button("Завершить деревья всех процессов").clicked() {
                    actions.push(Action::KillTree(pids));
                    ui.close();
                }
                ui.separator();
                if ui.button(if d.expanded { "Свернуть" } else { "Развернуть" }).clicked() {
                    actions.push(Action::Toggle(d.key.clone()));
                    ui.close();
                }
                if ui.button("Поиск в интернете").clicked() {
                    actions.push(Action::SearchWeb(g.name.clone()));
                    ui.close();
                }
            }
        }
    }

    // ───────────── панель деталей ─────────────

    fn details_ui(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::Frame::new()
            .fill(BG_CARD)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match self.selected.clone() {
                        Some(Sel::Proc(pid)) => self.proc_details(ui, pid, actions),
                        Some(Sel::Group(key)) => self.group_details(ui, &key, actions),
                        None => {}
                    }
                });
            });
    }

    fn proc_details(&mut self, ui: &mut Ui, pid: Pid, actions: &mut Vec<Action>) {
        let Some(&i) = self.by_pid.get(&pid) else { return };
        let r = self.rows[i].clone();
        let suspended = self.suspended.contains(&pid.as_u32());
        let width = ui.available_width();

        ui.label(RichText::new(&r.name).size(20.0).strong().color(TEXT));
        ui.horizontal(|ui| {
            ui.label(dim(format!("PID {pid}")));
            if suspended {
                ui.label(RichText::new("⏸ приостановлен").color(AMBER).size(12.0));
            }
        });
        ui.add_space(8.0);

        ui.label(dim("ЦП, % от всей системы"));
        let cpu_max = (self.sel_hist_cpu.max() * 1.3).max(5.0);
        sparkline(ui, self.sel_hist_cpu.values(), vec2(width, 54.0), ACCENT, Some(cpu_max));
        ui.add_space(4.0);
        ui.label(dim("Память"));
        sparkline(ui, self.sel_hist_mem.values(), vec2(width, 54.0), PURPLE, None);
        ui.add_space(10.0);

        let parent = r.ppid.and_then(|pp| self.by_pid.get(&pp).map(|j| (pp, self.rows[*j].name.clone())));
        let kids = self.children.get(&pid).map(Vec::len).unwrap_or(0);

        egui::Grid::new("proc_info").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
            let line = |ui: &mut Ui, k: &str, v: String| {
                ui.label(dim(k));
                ui.label(RichText::new(v).size(13.0).color(TEXT));
                ui.end_row();
            };
            line(ui, "ЦП", fmt_pct(r.cpu as f64));
            line(ui, "Память", fmt_bytes(r.mem));
            line(ui, "Виртуальная", fmt_bytes(r.virt));
            line(ui, "Чтение с диска", fmt_rate(r.disk_read));
            line(ui, "Запись на диск", fmt_rate(r.disk_write));
            if let Some(g) = r.gpu {
                line(ui, "GPU", fmt_pct(g as f64));
            }
            line(ui, "Работает", fmt_duration(r.run_time));
            if !r.user.is_empty() {
                line(ui, "Пользователь", r.user.clone());
            }
            line(ui, "Дочерних", kids.to_string());
            ui.label(dim("Родитель"));
            match &parent {
                Some((pp, name)) => {
                    if ui.link(format!("{name} ({pp})")).clicked() {
                        actions.push(Action::Select(Sel::Proc(*pp)));
                    }
                }
                None => {
                    ui.label(dim("—"));
                }
            }
            ui.end_row();
        });

        if cfg!(windows) {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(dim("Приоритет"));
                let cur = self.sel_priority;
                egui::ComboBox::from_id_salt("prio_combo")
                    .selected_text(cur.map(|p| p.label()).unwrap_or("—"))
                    .show_ui(ui, |ui| {
                        for p in Priority::ALL {
                            if ui.selectable_label(cur == Some(p), p.label()).clicked() {
                                actions.push(Action::SetPriority(pid, p));
                            }
                        }
                    });
            });
        }

        for (title, text) in [("Путь", r.path_text()), ("Командная строка", r.cmd.clone()), ("Рабочая папка", r.cwd.clone())] {
            if text.is_empty() {
                continue;
            }
            ui.add_space(10.0);
            ui.label(dim(title));
            ui.add(egui::Label::new(RichText::new(&text).size(12.5).color(TEXT)).wrap());
        }

        ui.add_space(14.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Завершить").clicked() {
                actions.push(Action::Kill(vec![pid]));
            }
            if ui.button("Завершить дерево").clicked() {
                actions.push(Action::KillTree(vec![pid]));
            }
            if ui.add_enabled(r.exe.is_some(), egui::Button::new("Показать файл")).clicked() {
                actions.push(Action::OpenLocation(pid));
            }
            if ui.button("Поиск в интернете").clicked() {
                actions.push(Action::SearchWeb(r.name.clone()));
            }
        });
    }

    fn group_details(&mut self, ui: &mut Ui, key: &str, actions: &mut Vec<Action>) {
        let members: Vec<&ProcRow> = self.rows.iter().filter(|r| r.name_lc == key).collect();
        let Some(first) = members.first() else { return };
        ui.label(RichText::new(&first.name).size(20.0).strong().color(TEXT));
        ui.label(dim(format!("Процессов в группе: {}", members.len())));
        ui.add_space(10.0);

        let (cpu, mem, disk) = (
            members.iter().map(|r| r.cpu as f64).sum::<f64>(),
            members.iter().map(|r| r.mem).sum::<u64>(),
            members.iter().map(|r| r.disk()).sum::<f64>(),
        );
        ui.horizontal(|ui| {
            stat(ui, "ЦП", &fmt_pct(cpu));
            ui.add_space(14.0);
            stat(ui, "Память", &fmt_bytes(mem));
            ui.add_space(14.0);
            stat(ui, "Диск", &fmt_rate(disk));
        });
        ui.add_space(12.0);
        ui.label(dim("Экземпляры (по убыванию памяти)"));
        let mut sorted = members.clone();
        sorted.sort_by(|a, b| b.mem.cmp(&a.mem));
        for r in sorted.iter().take(40) {
            let text = format!("PID {} · {} · {}", r.pid, fmt_bytes(r.mem), fmt_pct(r.cpu as f64));
            if ui.selectable_label(false, text).clicked() {
                actions.push(Action::Select(Sel::Proc(r.pid)));
            }
        }
        if sorted.len() > 40 {
            ui.label(dim(format!("…и ещё {}", sorted.len() - 40)));
        }
        ui.add_space(12.0);
        let pids: Vec<Pid> = members.iter().map(|r| r.pid).collect();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Завершить все").clicked() {
                actions.push(Action::Kill(pids.clone()));
            }
            if ui.button("Завершить деревья").clicked() {
                actions.push(Action::KillTree(pids.clone()));
            }
        });
    }

    // ───────────── подтверждение ─────────────

    fn confirm_modal(&mut self, ctx: &egui::Context, m: &Metrics) {
        let Some(Action::KillTree(pids)) = self.pending.clone() else { return };
        let mut seen = HashSet::new();
        let total = pids.iter().flat_map(|p| self.tree_of(*p)).filter(|p| seen.insert(*p)).count();
        let what = if pids.len() == 1 { format!("«{}»", self.name_of(pids[0])) } else { format!("{} процессов", pids.len()) };

        let mut decision: Option<bool> = None;
        #[allow(deprecated)]
        let id = egui::Id::new("confirm_kill_tree");
        let resp = egui::Modal::new(id).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Завершить дерево процессов?").size(17.0).strong());
            ui.add_space(8.0);
            ui.label(format!(
                "{what} вместе со всеми дочерними: будет остановлено {total} шт. Несохранённые данные пропадут."
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let kill = egui::Button::new(RichText::new("Завершить").color(Color32::WHITE)).fill(DANGER);
                if ui.add(kill).clicked() {
                    decision = Some(true);
                }
                if ui.button("Отмена").clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision.is_none() && resp.should_close() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                self.pending = None;
                let msg = self.do_kill(m, &pids, true);
                self.say(msg);
                self.force_refresh = true;
            }
            Some(false) => self.pending = None,
            None => {}
        }
    }
}


// ───────────────────────────── переход к процессу и экспорт ─────────────────────────────

impl ProcessesTab {
    /// Показать процесс в списке: сбросить поиск, раскрыть нужные ветки, выбрать и прокрутить
    pub fn reveal(&mut self, pid: u32) {
        let pid = Pid::from_u32(pid);
        let Some(&i) = self.by_pid.get(&pid) else {
            self.say(format!("Процесс {pid} уже не существует"));
            return;
        };
        self.filter.clear();
        match self.view {
            View::Group => {
                self.group_expanded.insert(format!("g{}", self.rows[i].name_lc));
            }
            View::Tree => {
                // раскрываем всех предков
                let mut cur = self.rows[i].ppid;
                let mut guard = 0;
                while let Some(pp) = cur {
                    self.tree_collapsed.remove(&format!("p{pp}"));
                    cur = self.by_pid.get(&pp).and_then(|j| self.rows[*j].ppid);
                    guard += 1;
                    if guard > 256 {
                        break;
                    }
                }
            }
            View::List => {}
        }
        self.selected = Some(Sel::Proc(pid));
        self.want_scroll_sel = true;
        self.dirty = true;
    }

    /// Сохраняет текущий список процессов в CSV (разделитель `;` и BOM — чтобы Excel открыл с кириллицей)
    pub fn export_csv(&mut self) {
        let mut out = String::from("\u{feff}Имя;PID;Родитель;ЦП %;Память байт;Чтение Б/с;Запись Б/с;Пользователь;Путь;Командная строка\r\n");
        let mut sorted: Vec<&ProcRow> = self.rows.iter().collect();
        sorted.sort_by_key(|r| r.pid.as_u32());
        for r in sorted {
            let fields = [
                r.name.clone(),
                r.pid.to_string(),
                r.ppid.map(|p| p.to_string()).unwrap_or_default(),
                format!("{:.1}", r.cpu),
                r.mem.to_string(),
                format!("{:.0}", r.disk_read),
                format!("{:.0}", r.disk_write),
                r.user.clone(),
                r.path_text(),
                r.cmd.clone(),
            ];
            out.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(";"));
            out.push_str("\r\n");
        }
        let dir = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(|h| PathBuf::from(h).join("Desktop"))
            .filter(|d| d.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("TaskMgrRS_processes_{stamp}.csv"));
        match std::fs::write(&path, out) {
            Ok(()) => self.say(format!("Экспортировано: {}", path.display())),
            Err(e) => self.say(format!("Не удалось сохранить CSV: {e}")),
        }
    }
}

/// Поле CSV: если есть разделитель, кавычки или перевод строки — заключаем в кавычки
fn csv_field(s: &str) -> String {
    if s.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(name: &str, pid: u32, cpu: f32, mem: u64) -> Vals {
        Vals { name: name.into(), pid, cpu, mem, disk: 0.0, gpu: 0.0, time: 0, user: String::new(), path: String::new() }
    }

    #[test]
    fn sorting_is_stable_on_ties() {
        let a = vals("a.exe", 10, 5.0, 100);
        let b = vals("a.exe", 20, 5.0, 100);
        // одинаковые значения — порядок определяется PID, а не случайностью
        assert_eq!(cmp_vals(&a, &b, Col::Cpu, true), Ordering::Less);
        assert_eq!(cmp_vals(&b, &a, Col::Cpu, true), Ordering::Greater);
    }

    #[test]
    fn descending_puts_heavy_first() {
        let light = vals("x", 1, 1.0, 10);
        let heavy = vals("y", 2, 50.0, 10);
        assert_eq!(cmp_vals(&heavy, &light, Col::Cpu, true), Ordering::Less);
        assert_eq!(cmp_vals(&heavy, &light, Col::Cpu, false), Ordering::Greater);
    }

    #[test]
    fn csv_escaping() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a;b"), "\"a;b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line1\nline2"), "\"line1\nline2\"");
    }

    #[test]
    fn protected_processes() {
        assert!(is_protected("csrss.exe"));
        assert!(is_protected("LSASS.EXE"));
        assert!(!is_protected("notepad.exe"));
    }

    #[test]
    fn column_ids_roundtrip() {
        for c in Col::OPTIONAL {
            assert_eq!(Col::from_id(c.id()), Some(c));
        }
        assert_eq!(Col::from_id("name"), Some(Col::Name));
        assert_eq!(Col::from_id("nope"), None);
    }
}

#[cfg(test)]
mod structure_tests {
    use super::*;

    fn row(pid: u32, ppid: Option<u32>, name: &str, cpu: f32, mem: u64) -> ProcRow {
        ProcRow {
            pid: Pid::from_u32(pid),
            ppid: ppid.map(Pid::from_u32),
            name: name.to_owned(),
            name_lc: name.to_lowercase(),
            exe: None,
            cmd: String::new(),
            cwd: String::new(),
            user: String::new(),
            cpu,
            mem,
            virt: 0,
            disk_read: 0.0,
            disk_write: 0.0,
            gpu: None,
            run_time: 0,
        }
    }

    fn tab(rows: Vec<ProcRow>, view: View) -> ProcessesTab {
        let mut t = ProcessesTab::new(&Settings::default());
        t.rows = rows;
        t.reindex();
        t.view = view;
        t.sort = Col::Pid;
        t.sort_desc = false;
        t.rebuild();
        t
    }

    fn keys(t: &ProcessesTab) -> Vec<String> {
        t.display.iter().map(|d| d.key.clone()).collect()
    }

    fn depths(t: &ProcessesTab) -> Vec<usize> {
        t.display.iter().map(|d| d.depth).collect()
    }

    /// 1 ─┬─ 2 ── 3
    ///    └─ 4
    /// 5
    fn sample_tree() -> Vec<ProcRow> {
        vec![
            row(1, None, "a.exe", 1.0, 10),
            row(2, Some(1), "b.exe", 2.0, 20),
            row(3, Some(2), "c.exe", 3.0, 30),
            row(4, Some(1), "d.exe", 4.0, 40),
            row(5, None, "e.exe", 5.0, 50),
        ]
    }

    #[test]
    fn tree_order_and_depth() {
        let t = tab(sample_tree(), View::Tree);
        assert_eq!(keys(&t), ["p1", "p2", "p3", "p4", "p5"]);
        assert_eq!(depths(&t), [0, 1, 2, 1, 0]);
        assert!(t.display[0].expandable && t.display[0].expanded);
        assert!(!t.display[2].expandable);
    }

    #[test]
    fn collapsed_branch_hides_descendants() {
        let mut t = tab(sample_tree(), View::Tree);
        t.toggle_key("p2");
        t.rebuild();
        assert_eq!(keys(&t), ["p1", "p2", "p4", "p5"]);
        assert!(!t.display[1].expanded);
        t.toggle_key("p1");
        t.rebuild();
        assert_eq!(keys(&t), ["p1", "p5"]);
    }

    #[test]
    fn parent_cycle_does_not_lose_or_duplicate_processes() {
        let rows = vec![row(10, Some(11), "x.exe", 0.0, 0), row(11, Some(10), "y.exe", 0.0, 0), row(12, None, "z.exe", 0.0, 0)];
        let t = tab(rows, View::Tree);
        let mut k = keys(&t);
        k.sort();
        assert_eq!(k, ["p10", "p11", "p12"]);
    }

    #[test]
    fn orphan_with_missing_parent_becomes_root() {
        let rows = vec![row(7, Some(999), "orphan.exe", 0.0, 0), row(8, Some(7), "child.exe", 0.0, 0)];
        let t = tab(rows, View::Tree);
        assert_eq!(keys(&t), ["p7", "p8"]);
        assert_eq!(depths(&t), [0, 1]);
    }

    #[test]
    fn self_parent_is_treated_as_root() {
        // у процесса «System Idle» в Windows родитель совпадает с ним самим
        let rows = vec![row(0, Some(0), "idle", 0.0, 0)];
        let mut t = ProcessesTab::new(&Settings::default());
        // on_refresh отбрасывает ppid == pid; здесь повторяем это условие вручную
        t.rows = rows.into_iter().map(|mut r| { r.ppid = r.ppid.filter(|p| *p != r.pid); r }).collect();
        t.reindex();
        t.view = View::Tree;
        t.rebuild();
        assert_eq!(keys(&t), ["p0"]);
    }

    #[test]
    fn groups_aggregate_and_expand() {
        let rows = vec![
            row(1, None, "chrome.exe", 1.0, 100),
            row(2, None, "Chrome.exe", 2.0, 200),
            row(3, None, "chrome.exe", 3.0, 300),
            row(4, None, "a.exe", 9.0, 5),
        ];
        let mut t = tab(rows, View::Group);
        // группа из 3 процессов + одиночный a.exe показывается без заголовка группы
        assert_eq!(t.display.len(), 2);
        let group = t.display.iter().find_map(|d| match &d.kind {
            DKind::Group(g) => Some(g.clone()),
            _ => None,
        });
        let g = group.expect("группа chrome.exe");
        assert_eq!(g.members.len(), 3);
        assert_eq!(g.mem, 600);
        assert!((g.cpu - 6.0).abs() < 1e-4);
        // регистр имени не создаёт отдельную группу
        t.toggle_key("gchrome.exe");
        t.rebuild();
        assert_eq!(t.display.len(), 5);
        let members: Vec<usize> = t.display.iter().filter(|d| d.depth == 1).map(|_| 1).collect();
        assert_eq!(members.len(), 3);
    }

    #[test]
    fn group_sort_uses_aggregate() {
        let rows = vec![
            row(1, None, "small.exe", 50.0, 1),
            row(2, None, "big.exe", 10.0, 1),
            row(3, None, "big.exe", 10.0, 1),
            row(4, None, "big.exe", 10.0, 1),
        ];
        let mut t = tab(rows, View::Group);
        t.sort = Col::Cpu;
        t.sort_desc = true;
        t.dirty = true;
        t.rebuild();
        // суммарно у big.exe 30%, у small.exe 50% — small выше; при сортировке по Pid группа идёт по min(PID)
        let first = &t.display[0];
        assert!(matches!(first.kind, DKind::Proc(_)), "одиночный small.exe с 50% должен быть первым");
    }

    #[test]
    fn search_gives_flat_list_even_in_tree_mode() {
        let mut t = tab(sample_tree(), View::Tree);
        t.filter = "c.exe".to_owned();
        t.rebuild();
        assert_eq!(keys(&t), ["p3"]);
        assert_eq!(depths(&t), [0]);
        t.filter = "4".to_owned(); // поиск по PID
        t.rebuild();
        assert_eq!(keys(&t), ["p4"]);
    }

    #[test]
    fn kill_tree_order_children_first() {
        let t = tab(sample_tree(), View::Tree);
        let order = t.tree_of(Pid::from_u32(1));
        assert_eq!(order.len(), 4);
        let pos = |p: u32| order.iter().position(|x| *x == Pid::from_u32(p)).unwrap();
        assert!(pos(3) < pos(2), "внук раньше ребёнка");
        assert!(pos(2) < pos(1) && pos(4) < pos(1), "дети раньше родителя");
        assert!(!order.contains(&Pid::from_u32(5)));
    }

    #[test]
    fn kill_tree_survives_cycles() {
        let rows = vec![row(10, Some(11), "x.exe", 0.0, 0), row(11, Some(10), "y.exe", 0.0, 0)];
        let t = tab(rows, View::Tree);
        assert_eq!(t.tree_of(Pid::from_u32(10)).len(), 2);
    }

    #[test]
    fn reveal_expands_group_and_selects() {
        let rows = vec![row(1, None, "w.exe", 0.0, 0), row(2, None, "w.exe", 0.0, 0), row(3, None, "o.exe", 0.0, 0)];
        let mut t = tab(rows, View::Group);
        assert_eq!(t.display.len(), 2); // группа свёрнута
        t.reveal(2);
        t.rebuild();
        assert_eq!(t.display.len(), 4);
        assert_eq!(t.selected, Some(Sel::Proc(Pid::from_u32(2))));
        assert!(t.sel_index().is_some());
    }

    #[test]
    fn reveal_expands_ancestors_in_tree() {
        let mut t = tab(sample_tree(), View::Tree);
        t.toggle_key("p1");
        t.toggle_key("p2");
        t.rebuild();
        assert_eq!(keys(&t), ["p1", "p5"]);
        t.reveal(3);
        t.rebuild();
        assert!(keys(&t).contains(&"p3".to_owned()));
        assert!(t.sel_index().is_some());
    }

    #[test]
    fn reveal_clears_search_and_ignores_dead_pid() {
        let mut t = tab(sample_tree(), View::List);
        t.filter = "zzz".to_owned();
        t.reveal(5);
        assert!(t.filter.is_empty());
        t.reveal(424242); // такого процесса нет — выбор не меняется
        assert_eq!(t.selected, Some(Sel::Proc(Pid::from_u32(5))));
    }

    #[test]
    fn selected_group_pids() {
        let rows = vec![row(1, None, "w.exe", 0.0, 0), row(2, None, "W.exe", 0.0, 0), row(3, None, "o.exe", 0.0, 0)];
        let mut t = tab(rows, View::Group);
        t.selected = Some(Sel::Group("w.exe".to_owned()));
        let mut pids: Vec<u32> = t.sel_pids().iter().map(|p| p.as_u32()).collect();
        pids.sort();
        assert_eq!(pids, [1, 2]);
    }
}
