//! 画面 (eframe/egui)。

use crate::chandir::Channel;
use crate::config::{self, Config, PeerCastKind, PlayUrlKind, PlayerEntry, YpEntry};
use crate::filter::{self, CompiledFilter, FIELDS, Filter, Filters, Search};
use crate::peercast::{self, RelayChannel, Rpc};
use crate::player;
use crate::win;
use crate::worker::{Command, FetchState, SharedRef, lock};
use eframe::egui::{self, Align, Color32, FontFamily, Key, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// トレイの「終了」から閉じるとき true
pub static QUITTING: AtomicBool = AtomicBool::new(false);

pub const BOLD: &str = "bold";

#[derive(Clone, PartialEq, Eq, Debug)]
enum Tab {
    Favorite,
    All,
    Yp(String),
    Ignored,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortKey {
    Listeners,
    Name,
    Bitrate,
    Uptime,
    Type,
    Yp,
    Genre,
}

impl SortKey {
    const ALL: [(SortKey, &'static str); 7] = [
        (SortKey::Listeners, "リスナー"),
        (SortKey::Name, "名前"),
        (SortKey::Genre, "ジャンル"),
        (SortKey::Bitrate, "ビットレート"),
        (SortKey::Uptime, "配信時間"),
        (SortKey::Type, "種類"),
        (SortKey::Yp, "YP"),
    ];

    /// 最初に押したときは、数は多い順、文字列は昇順
    fn default_desc(self) -> bool {
        matches!(self, SortKey::Listeners | SortKey::Bitrate | SortKey::Uptime)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Col {
    Name,
    Summary,
    Listeners,
    Bitrate,
    Uptime,
    Type,
    Yp,
    Track,
    TwoName,
    TwoStats,
    TwoRate,
}

impl Col {
    fn header(self) -> &'static str {
        match self {
            Col::Name | Col::TwoName => "チャンネル",
            Col::Summary => "ジャンル・詳細",
            Col::Listeners => "リスナー/リレー",
            Col::Bitrate => "Bitrate",
            Col::Uptime => "時間",
            Col::Type => "種類",
            Col::Yp => "YP",
            Col::Track => "トラック",
            Col::TwoStats => "リスナー/時間",
            Col::TwoRate => "Bitrate/種類",
        }
    }

    fn sort_key(self) -> Option<SortKey> {
        Some(match self {
            Col::Name | Col::TwoName => SortKey::Name,
            Col::Summary => SortKey::Genre,
            Col::Listeners | Col::TwoStats => SortKey::Listeners,
            Col::Bitrate | Col::TwoRate => SortKey::Bitrate,
            Col::Uptime => SortKey::Uptime,
            Col::Type => SortKey::Type,
            Col::Yp => SortKey::Yp,
            Col::Track => return None,
        })
    }

    fn column(self) -> Column {
        match self {
            Col::TwoName | Col::Summary => Column::remainder().at_least(120.0).clip(true),
            Col::Name => Column::initial(180.0).at_least(60.0).clip(true).resizable(true),
            Col::Listeners | Col::TwoStats => Column::initial(88.0).at_least(40.0).resizable(true),
            Col::Bitrate | Col::TwoRate => Column::initial(60.0).at_least(36.0).resizable(true),
            Col::Uptime => Column::initial(50.0).at_least(30.0).resizable(true),
            Col::Type => Column::initial(48.0).at_least(30.0).resizable(true),
            Col::Yp => Column::initial(64.0).at_least(30.0).clip(true).resizable(true),
            Col::Track => Column::initial(150.0).at_least(40.0).clip(true).resizable(true),
        }
    }
}

fn columns(v: &config::ViewConfig) -> Vec<Col> {
    let c = &v.columns;
    let mut out = Vec::new();
    if v.two_line {
        out.push(Col::TwoName);
        if c.listeners || c.uptime {
            out.push(Col::TwoStats);
        }
        if c.bitrate || c.content_type {
            out.push(Col::TwoRate);
        }
    } else {
        out.push(Col::Name);
        if c.summary {
            out.push(Col::Summary);
        }
        if c.listeners {
            out.push(Col::Listeners);
        }
        if c.bitrate {
            out.push(Col::Bitrate);
        }
        if c.uptime {
            out.push(Col::Uptime);
        }
        if c.content_type {
            out.push(Col::Type);
        }
    }
    if c.track {
        out.push(Col::Track);
    }
    if c.yp {
        out.push(Col::Yp);
    }
    out
}

struct Row {
    ch: Channel,
    yp: String,
    favorite: bool,
    ignore: bool,
    color: Option<Color32>,
    is_new: bool,
    /// 別の YP にも同じチャンネルがある (すべて・お気に入りのタブでは出さない)
    duplicate: bool,
}

#[derive(Default)]
struct Counts {
    favorite: usize,
    all: usize,
    ignored: usize,
    per_yp: Vec<usize>,
}

enum Action {
    Select(usize),
    Play(usize),
    OpenUrl(String),
    AddFilter(String, bool),
    RemoveFilter(usize),
    Bump(String),
    Stop(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Yp,
    Update,
    PeerCast,
    Player,
    Notify,
    View,
}

struct SettingsDlg {
    cfg: Config,
    tab: SettingsTab,
    error: String,
    test: Arc<Mutex<String>>,
}

struct FilterDlg {
    filters: Vec<Filter>,
    sel: usize,
}

#[derive(Default)]
struct PcStatus {
    running: bool,
    agent: String,
}

#[derive(Default)]
struct RelayState {
    channels: Vec<RelayChannel>,
    error: String,
    loading: bool,
}

pub struct App {
    config: Arc<RwLock<Config>>,
    filters: Arc<RwLock<Vec<Filter>>>,
    compiled: Vec<CompiledFilter>,
    filter_errors: Vec<String>,
    shared: SharedRef,
    tx: Sender<Command>,
    tray: Option<win::Tray>,
    open_settings: Arc<AtomicBool>,
    has_bold: bool,

    tab: Tab,
    search: String,
    sort: (SortKey, bool),
    selected: Option<String>,
    rows: Vec<Row>,
    view: Vec<usize>,
    counts: Counts,
    rows_gen: u64,
    dirty: bool,
    view_key: String,
    scroll_to: Option<usize>,
    hovered_url: String,

    settings: Option<SettingsDlg>,
    filter_dlg: Option<FilterDlg>,
    show_log: bool,
    show_relay: bool,
    relay: Arc<Mutex<RelayState>>,
    relay_last: Option<Instant>,
    pc_status: Arc<Mutex<PcStatus>>,

    was_minimized: bool,
    last_size: Option<[f32; 2]>,
    closing: bool,
    first_frame: bool,
}

fn read<T: Clone>(l: &RwLock<T>) -> T {
    l.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn color_of(colors: &[[u8; 3]]) -> Option<Color32> {
    if colors.is_empty() {
        return None;
    }
    let n = colors.len() as u32;
    let sum = colors.iter().fold([0u32; 3], |a, c| [a[0] + c[0] as u32, a[1] + c[1] as u32, a[2] + c[2] as u32]);
    Some(Color32::from_rgba_unmultiplied((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 130))
}

fn listeners_text(n: i32) -> String {
    if n < 0 { "-".into() } else { n.to_string() }
}

pub struct AppInit {
    pub config: Arc<RwLock<Config>>,
    pub filters: Arc<RwLock<Vec<Filter>>>,
    pub shared: SharedRef,
    pub tx: Sender<Command>,
    pub tray: Option<win::Tray>,
    pub open_settings: Arc<AtomicBool>,
    pub has_bold: bool,
}

impl App {
    pub fn new(ctx: &egui::Context, init: AppInit) -> App {
        let cfg = read(&init.config);
        apply_style(ctx, &cfg);
        let mut app = App {
            config: init.config,
            filters: init.filters,
            compiled: Vec::new(),
            filter_errors: Vec::new(),
            shared: init.shared,
            tx: init.tx,
            tray: init.tray,
            open_settings: init.open_settings,
            has_bold: init.has_bold,
            tab: Tab::All,
            search: String::new(),
            sort: (SortKey::Listeners, true),
            selected: None,
            rows: Vec::new(),
            view: Vec::new(),
            counts: Counts::default(),
            rows_gen: u64::MAX,
            dirty: true,
            view_key: String::new(),
            scroll_to: None,
            hovered_url: String::new(),
            settings: None,
            filter_dlg: None,
            show_log: false,
            show_relay: false,
            relay: Arc::new(Mutex::new(RelayState::default())),
            relay_last: None,
            pc_status: Arc::new(Mutex::new(PcStatus::default())),
            was_minimized: false,
            last_size: None,
            closing: false,
            first_frame: true,
        };
        app.recompile_filters();
        app.start_peercast_monitor(ctx.clone(), &cfg);
        app
    }

    fn cfg(&self) -> Config {
        read(&self.config)
    }

    fn log(&self, error: bool, text: impl Into<String>) {
        lock(&self.shared).log(error, text);
    }

    fn recompile_filters(&mut self) {
        let f = read(&self.filters);
        let (c, errors) = filter::compile(&f);
        for e in &errors {
            if !self.filter_errors.contains(e) {
                self.log(true, e.clone());
            }
        }
        self.compiled = c;
        self.filter_errors = errors;
        self.dirty = true;
    }

    /// PeerCast の起動 (設定されていれば) と、動いているかの定期的な確認。
    fn start_peercast_monitor(&self, ctx: egui::Context, cfg: &Config) {
        if cfg.peercast.launch_on_start {
            match peercast::launch(&cfg.peercast) {
                Ok(true) => self.log(false, format!("PeerCast を起動しました: {}", cfg.peercast.exe_path)),
                Ok(false) => {}
                Err(e) => self.log(true, e),
            }
        }
        let (config, status) = (self.config.clone(), self.pc_status.clone());
        std::thread::Builder::new()
            .name("peercast-monitor".into())
            .spawn(move || {
                loop {
                    let pc = read(&config).peercast;
                    let running = peercast::is_running(&pc);
                    let agent = if running { Rpc::new(&pc).version_info().map(|v| v.agent).unwrap_or_default() } else { String::new() };
                    {
                        let mut s = status.lock().unwrap_or_else(|e| e.into_inner());
                        s.running = running;
                        s.agent = agent;
                    }
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_secs(10));
                }
            })
            .ok();
    }

    // ------------------------------------------------------------------ 一覧の組み立て

    fn rebuild_rows(&mut self) {
        let s = lock(&self.shared);
        let generation = s.generation;
        let loading = s.fetching;
        if !self.dirty && generation == self.rows_gen && !loading {
            return;
        }
        let mut rows = Vec::new();
        let mut counts = Counts { per_yp: vec![0; s.yps.len()], ..Default::default() };
        let mut seen = HashSet::new();
        for (yi, y) in s.yps.iter().enumerate() {
            for c in &y.channels {
                let m = filter::apply(&self.compiled, c, &y.name);
                let key = c.key();
                let duplicate = !c.is_info() && !seen.insert(key.clone());
                if m.ignore {
                    if !duplicate {
                        counts.ignored += 1;
                    }
                } else {
                    counts.per_yp[yi] += 1;
                    if !duplicate {
                        counts.all += 1;
                        if m.favorite {
                            counts.favorite += 1;
                        }
                    }
                }
                rows.push(Row {
                    ch: c.clone(),
                    yp: y.name.clone(),
                    favorite: m.favorite,
                    ignore: m.ignore,
                    color: color_of(&m.colors),
                    is_new: s.new_keys.contains(&key),
                    duplicate,
                });
            }
        }
        drop(s);
        self.rows = rows;
        self.counts = counts;
        self.rows_gen = generation;
        self.dirty = false;
        self.view_key.clear();
    }

    fn rebuild_view(&mut self, cfg: &Config) {
        let key = format!("{:?}|{}|{:?}|{}", self.tab, self.search, self.sort, cfg.view.show_info_rows);
        if key == self.view_key {
            return;
        }
        self.view_key = key;
        let q = self.search.trim().to_lowercase();
        let yp_name = match &self.tab {
            Tab::Yp(url) => cfg.yps.iter().find(|y| &y.url == url).map(|y| y.name.clone()),
            _ => None,
        };
        let mut v: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                let r = &self.rows[i];
                let tab_ok = match &self.tab {
                    Tab::Favorite => r.favorite && !r.ignore && !r.duplicate,
                    Tab::All => !r.ignore && !r.duplicate,
                    Tab::Yp(url) => !r.ignore && &r.ch.feed_url == url && yp_name.is_some(),
                    Tab::Ignored => r.ignore && !r.duplicate,
                };
                if !tab_ok || (!cfg.view.show_info_rows && r.ch.is_info() && self.tab != Tab::Ignored) {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                let c = &r.ch;
                [&c.name, &c.genre, &c.desc, &c.comment, &c.content_type, &r.yp, &c.track_title, &c.track_artist]
                    .iter()
                    .any(|s| s.to_lowercase().contains(&q))
            })
            .collect();
        let (key, desc) = self.sort;
        let rows = &self.rows;
        v.sort_by(|&a, &b| {
            let (a, b) = (&rows[a], &rows[b]);
            let o = match key {
                SortKey::Listeners => a.ch.listeners.cmp(&b.ch.listeners).then(a.ch.relays.cmp(&b.ch.relays)),
                SortKey::Name => a.ch.name.to_lowercase().cmp(&b.ch.name.to_lowercase()),
                SortKey::Genre => a.ch.genre.to_lowercase().cmp(&b.ch.genre.to_lowercase()),
                SortKey::Bitrate => a.ch.bitrate.cmp(&b.ch.bitrate),
                SortKey::Uptime => a.ch.uptime_minutes().cmp(&b.ch.uptime_minutes()),
                SortKey::Type => a.ch.content_type.cmp(&b.ch.content_type),
                SortKey::Yp => a.yp.cmp(&b.yp),
            };
            if desc { o.reverse() } else { o }
        });
        self.view = v;
    }

    fn selected_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.view.iter().copied().find(|&i| self.rows[i].ch.key() == *sel)
    }

    // ------------------------------------------------------------------ 操作

    fn refresh(&self) {
        let _ = self.tx.send(Command::Refresh { manual: true });
    }

    fn play(&self, ch: &Channel) {
        match player::play(&self.cfg(), ch) {
            Ok(cmd) => self.log(false, format!("再生: {}", cmd)),
            Err(e) => self.log(true, e),
        }
    }

    fn open_url(&self, url: &str) {
        if let Err(e) = player::open_url(&self.cfg().browser, url) {
            self.log(true, e);
        }
    }

    fn save_filters(&mut self, filters: Vec<Filter>) {
        *self.filters.write().unwrap_or_else(|e| e.into_inner()) = filters.clone();
        if let Err(e) = config::save_json(config::FILTER_FILE, &Filters(filters)) {
            self.log(true, e);
        }
        self.recompile_filters();
    }

    fn apply_config(&mut self, ctx: &egui::Context, new: Config) {
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = new.clone();
        if let Err(e) = config::save_json(config::CONFIG_FILE, &new) {
            self.log(true, e);
        }
        let _ = self.tx.send(Command::ConfigChanged);
        apply_style(ctx, &new);
        if let Tab::Yp(url) = &self.tab
            && !new.yps.iter().any(|y| y.enabled && &y.url == url)
        {
            self.tab = Tab::All;
        }
        self.dirty = true;
    }

    fn spawn_rpc(&self, what: &'static str, f: impl FnOnce(&Rpc) -> Result<(), String> + Send + 'static) {
        let pc = self.cfg().peercast;
        let shared = self.shared.clone();
        let relay_dirty = self.relay.clone();
        std::thread::spawn(move || {
            let r = f(&Rpc::new(&pc));
            let mut s = lock(&shared);
            match r {
                Ok(()) => s.log(false, format!("PeerCast: {}しました", what)),
                Err(e) => s.log(true, format!("PeerCast: {}できません: {}", what, e)),
            }
            drop(s);
            relay_dirty.lock().unwrap_or_else(|e| e.into_inner()).loading = false;
            win::request_repaint();
        });
    }

    fn load_relays(&mut self) {
        {
            let mut r = self.relay.lock().unwrap_or_else(|e| e.into_inner());
            if r.loading {
                return;
            }
            r.loading = true;
        }
        self.relay_last = Some(Instant::now());
        let pc = self.cfg().peercast;
        let relay = self.relay.clone();
        std::thread::spawn(move || {
            let res = Rpc::new(&pc).channels();
            let mut r = relay.lock().unwrap_or_else(|e| e.into_inner());
            r.loading = false;
            match res {
                Ok(c) => {
                    r.channels = c;
                    r.error.clear();
                }
                Err(e) => r.error = e,
            }
            win::request_repaint();
        });
    }

    fn exact_filter_index(&self, name: &str) -> Option<usize> {
        let target = Filter::exact_name(name, false).base_search.search;
        read(&self.filters).iter().position(|f| f.base_search.search == target && f.base_search.fields == ["name"])
    }

    fn do_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Select(i) => self.selected = Some(self.rows[i].ch.key()),
                Action::Play(i) => {
                    self.selected = Some(self.rows[i].ch.key());
                    let ch = self.rows[i].ch.clone();
                    self.play(&ch);
                }
                Action::OpenUrl(u) => self.open_url(&u),
                Action::AddFilter(name, ignore) => {
                    let mut f = read(&self.filters);
                    f.push(Filter::exact_name(&name, ignore));
                    self.log(false, format!("{}に追加しました: {}", if ignore { "無視" } else { "お気に入り" }, name));
                    self.save_filters(f);
                }
                Action::RemoveFilter(i) => {
                    let mut f = read(&self.filters);
                    if i < f.len() {
                        let removed = f.remove(i);
                        self.log(false, format!("フィルターを削除しました: {}", removed.title()));
                        self.save_filters(f);
                    }
                }
                Action::Bump(id) => self.spawn_rpc("再接続", move |r| r.bump_channel(&id)),
                Action::Stop(id) => self.spawn_rpc("チャンネルを停止", move |r| r.stop_channel(&id)),
            }
        }
    }

    // ------------------------------------------------------------------ 画面の部品

    fn toolbar(&mut self, ui: &mut egui::Ui, cfg: &Config) {
        let (fetching, wait) = {
            let s = lock(&self.shared);
            (s.fetching, s.manual_wait())
        };
        ui.horizontal(|ui| {
            let r = ui.add_enabled(!fetching && wait == 0, egui::Button::new(if fetching { "⟳ 取得中…" } else { "⟳ 更新" }));
            let r = if wait > 0 { r.on_disabled_hover_text(format!("あと {} 秒で更新できます", wait)) } else { r.on_hover_text("すべての YP を更新 (F5)") };
            if r.clicked() {
                self.refresh();
            }
            if ui.button("⚙ 設定").clicked() {
                self.open_settings_dialog(cfg);
            }
            if ui.button("★ フィルター").on_hover_text("お気に入り・無視・色分け").clicked() && self.filter_dlg.is_none() {
                self.filter_dlg = Some(FilterDlg { filters: read(&self.filters), sel: 0 });
            }
            self.view_menu(ui, cfg);
            self.peercast_menu(ui, cfg);
            if ui.selectable_label(self.show_log, "📋 ログ").clicked() {
                self.show_log = !self.show_log;
            }
            ui.separator();
            let id = egui::Id::new("search_box");
            let r = ui.add(egui::TextEdit::singleline(&mut self.search).id(id).hint_text("🔍 検索 (Ctrl+F)").desired_width(180.0));
            if ui.input(|i| i.modifiers.command && i.key_pressed(Key::F)) {
                r.request_focus();
            }
            if !self.search.is_empty() && ui.small_button("✖").clicked() {
                self.search.clear();
            }
        });
    }

    fn view_menu(&mut self, ui: &mut egui::Ui, cfg: &Config) {
        let mut v = cfg.view.clone();
        ui.menu_button("表示", |ui| {
            ui.checkbox(&mut v.two_line, "2 行で表示 (pcyplite 風)");
            ui.checkbox(&mut v.show_info_panel, "チャンネル情報の欄");
            ui.checkbox(&mut v.show_info_rows, "YP のお知らせの行");
            ui.checkbox(&mut v.dark, "ダークモード");
            ui.separator();
            ui.label("列");
            let c = &mut v.columns;
            if !v.two_line {
                ui.checkbox(&mut c.summary, "ジャンル・詳細");
            }
            ui.checkbox(&mut c.listeners, "リスナー/リレー");
            ui.checkbox(&mut c.uptime, "配信時間");
            ui.checkbox(&mut c.bitrate, "ビットレート");
            ui.checkbox(&mut c.content_type, "種類");
            ui.checkbox(&mut c.track, "トラック");
            ui.checkbox(&mut c.yp, "YP");
            ui.separator();
            ui.menu_button("並べ替え", |ui| {
                for (k, label) in SortKey::ALL {
                    let mark = if self.sort.0 == k { if self.sort.1 { " ▼" } else { " ▲" } } else { "" };
                    if ui.button(format!("{}{}", label, mark)).clicked() {
                        self.set_sort(k);
                        ui.close();
                    }
                }
            });
        });
        if v != cfg.view {
            let mut new = cfg.clone();
            new.view = v;
            self.apply_config(ui.ctx(), new);
        }
    }

    fn peercast_menu(&mut self, ui: &mut egui::Ui, cfg: &Config) {
        ui.menu_button("PeerCast", |ui| {
            if ui.button("接続中のチャンネル…").clicked() {
                self.show_relay = true;
                self.load_relays();
                ui.close();
            }
            if ui.button("管理画面を開く").clicked() {
                self.open_url(&peercast::admin_url(&cfg.peercast));
                ui.close();
            }
            if ui.add_enabled(!cfg.peercast.exe_path.is_empty(), egui::Button::new("PeerCast を起動")).clicked() {
                match peercast::launch(&cfg.peercast) {
                    Ok(true) => self.log(false, "PeerCast を起動しました"),
                    Ok(false) => self.log(false, "PeerCast はすでに動いています"),
                    Err(e) => self.log(true, e),
                }
                ui.close();
            }
        });
    }

    fn set_sort(&mut self, k: SortKey) {
        self.sort = if self.sort.0 == k { (k, !self.sort.1) } else { (k, k.default_desc()) };
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui, cfg: &Config) {
        let yps: Vec<(String, String, FetchState)> =
            lock(&self.shared).yps.iter().map(|y| (y.name.clone(), y.url.clone(), y.state.clone())).collect();
        egui::ScrollArea::horizontal().id_salt("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                let c = &self.counts;
                let mut tab = self.tab.clone();
                ui.selectable_value(&mut tab, Tab::Favorite, format!("★ お気に入り ({})", c.favorite));
                ui.selectable_value(&mut tab, Tab::All, format!("すべて ({})", c.all));
                for (i, (name, url, state)) in yps.iter().enumerate() {
                    let n = c.per_yp.get(i).copied().unwrap_or(0);
                    let (label, tip) = match state {
                        FetchState::Error(e) => (RichText::new(format!("⚠ {} ({})", name, n)).color(Color32::from_rgb(220, 60, 60)), e.clone()),
                        FetchState::Loading => (RichText::new(format!("{} …", name)), "取得中".to_string()),
                        _ => (RichText::new(format!("{} ({})", name, n)), url.clone()),
                    };
                    let r = ui.selectable_label(tab == Tab::Yp(url.clone()), label).on_hover_text(tip);
                    if r.clicked() {
                        tab = Tab::Yp(url.clone());
                    }
                }
                ui.selectable_value(&mut tab, Tab::Ignored, format!("🚫 無視 ({})", c.ignored));
                self.tab = tab;
            });
        });
        let _ = cfg;
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, cfg: &Config) {
        let s = lock(&self.shared);
        let last_update = s.yps.iter().filter_map(|y| y.updated.clone()).max();
        let next = s.next_auto.map(|t| t.saturating_duration_since(Instant::now()).as_secs());
        let errors = s.yps.iter().filter(|y| matches!(y.state, FetchState::Error(_))).count();
        let last_log = s.log.back().cloned();
        drop(s);
        let pc = self.pc_status.lock().unwrap_or_else(|e| e.into_inner());
        let (running, agent) = (pc.running, pc.agent.clone());
        drop(pc);
        ui.horizontal(|ui| {
            ui.label(format!("{} ch", self.view.len()));
            ui.separator();
            if let Some(t) = last_update {
                ui.label(format!("更新 {}", t));
            }
            if let Some(n) = next.filter(|_| cfg.auto_update) {
                ui.label(format!("次 {}:{:02}", n / 60, n % 60));
            }
            if errors > 0 {
                ui.colored_label(Color32::from_rgb(220, 60, 60), format!("⚠ YP エラー {}", errors));
            }
            ui.separator();
            let (dot, text) = if running {
                (Color32::from_rgb(40, 170, 70), if agent.is_empty() { format!("PeerCast {}", cfg.peercast.address) } else { agent })
            } else {
                (Color32::from_rgb(200, 60, 60), format!("PeerCast 未起動 ({})", cfg.peercast.address))
            };
            ui.colored_label(dot, "●");
            ui.label(text);
            ui.separator();
            if !self.hovered_url.is_empty() {
                ui.add(egui::Label::new(RichText::new(&self.hovered_url).weak()).truncate());
            } else if let Some(l) = last_log {
                let t = RichText::new(format!("{} {}", l.time, l.text));
                let t = if l.error { t.color(Color32::from_rgb(220, 60, 60)) } else { t.weak() };
                if ui.add(egui::Label::new(t).truncate().sense(Sense::click())).on_hover_text("クリックでログを表示").clicked() {
                    self.show_log = true;
                }
            }
        });
    }

    fn name_text(&self, ui: &egui::Ui, r: &Row) -> RichText {
        let dark = ui.visuals().dark_mode;
        let color = if r.ch.is_info() {
            ui.visuals().weak_text_color()
        } else if dark {
            Color32::from_rgb(120, 175, 255)
        } else {
            Color32::from_rgb(0, 60, 200)
        };
        let t = RichText::new(&r.ch.name).color(color);
        if self.has_bold { t.family(FontFamily::Name(BOLD.into())) } else { t.strong() }
    }

    fn info_panel(&mut self, ui: &mut egui::Ui, idx: usize) {
        let c = self.rows[idx].ch.clone();
        let yp = self.rows[idx].yp.clone();
        let name = self.name_text(ui, &self.rows[idx]);
        let mut actions = Vec::new();
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("チャンネル情報").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("✖").on_hover_text("閉じる").clicked() {
                    close = true;
                }
                if ui.add_enabled(!c.is_info(), egui::Button::new("▶ 再生")).clicked() {
                    actions.push(Action::Play(idx));
                }
            });
        });
        if close {
            let mut cfg = self.cfg();
            cfg.view.show_info_panel = false;
            self.apply_config(ui.ctx(), cfg);
        }
        egui::ScrollArea::vertical().id_salt("info_scroll").show(ui, |ui| {
            ui.label(name);
            let summary = c.summary();
            if !summary.is_empty() {
                ui.label(summary);
            }
            let track = c.track_text();
            if !track.is_empty() {
                ui.label(format!("♪ {}", track));
            }
            ui.horizontal_wrapped(|ui| {
                if !c.url.is_empty() && ui.link(&c.url).on_hover_text("コンタクト URL を開く").clicked() {
                    actions.push(Action::OpenUrl(c.url.clone()));
                }
                let chat = c.chat_url();
                if !chat.is_empty() && ui.link("チャット").clicked() {
                    actions.push(Action::OpenUrl(chat));
                }
                let stat = c.stat_url();
                if !stat.is_empty() && ui.link("統計").clicked() {
                    actions.push(Action::OpenUrl(stat));
                }
            });
            ui.label(
                RichText::new(format!(
                    "{}  {} kbps  {}/{}  {}  YP: {}  tip: {}  ID: {}",
                    c.content_type,
                    c.bitrate,
                    listeners_text(c.listeners),
                    listeners_text(c.relays),
                    c.uptime,
                    yp,
                    c.tip,
                    c.id
                ))
                .small()
                .weak(),
            );
        });
        self.do_actions(actions);
    }

    fn channel_menu(&self, ui: &mut egui::Ui, i: usize, actions: &mut Vec<Action>) {
        let r = &self.rows[i];
        let c = &r.ch;
        let cfg = self.cfg();
        if ui.add_enabled(!c.is_info(), egui::Button::new("▶ 再生")).clicked() {
            actions.push(Action::Play(i));
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(!c.url.is_empty(), egui::Button::new("コンタクト URL を開く")).clicked() {
            actions.push(Action::OpenUrl(c.url.clone()));
            ui.close();
        }
        let chat = c.chat_url();
        if ui.add_enabled(!chat.is_empty(), egui::Button::new("チャットを開く")).clicked() {
            actions.push(Action::OpenUrl(chat));
            ui.close();
        }
        let stat = c.stat_url();
        if ui.add_enabled(!stat.is_empty(), egui::Button::new("統計を開く")).clicked() {
            actions.push(Action::OpenUrl(stat));
            ui.close();
        }
        ui.menu_button("コピー", |ui| {
            let items = [
                ("名前", c.name.clone()),
                ("ストリーム URL", player::stream_url(&cfg.peercast, c)),
                ("プレイリスト URL", player::playlist_url(&cfg.peercast, c)),
                ("コンタクト URL", c.url.clone()),
                ("チャンネル ID", c.id.clone()),
                ("名前と詳細", format!("{} {}", c.name, c.summary())),
            ];
            for (label, value) in items {
                if ui.add_enabled(!value.is_empty(), egui::Button::new(label)).clicked() {
                    ui.ctx().copy_text(value);
                    ui.close();
                }
            }
        });
        ui.separator();
        match self.exact_filter_index(&c.name) {
            Some(fi) => {
                let ignore = read(&self.filters).get(fi).is_some_and(|f| f.ignore);
                if ui.button(if ignore { "無視をやめる" } else { "お気に入りから外す" }).clicked() {
                    actions.push(Action::RemoveFilter(fi));
                    ui.close();
                }
            }
            None => {
                if ui.button("★ お気に入りに追加").clicked() {
                    actions.push(Action::AddFilter(c.name.clone(), false));
                    ui.close();
                }
                if ui.button("🚫 無視に追加").clicked() {
                    actions.push(Action::AddFilter(c.name.clone(), true));
                    ui.close();
                }
            }
        }
        ui.separator();
        ui.add_enabled_ui(!c.is_info(), |ui| {
            if ui.button("PeerCast: 再接続").clicked() {
                actions.push(Action::Bump(c.id.clone()));
                ui.close();
            }
            if ui.button("PeerCast: 停止").clicked() {
                actions.push(Action::Stop(c.id.clone()));
                ui.close();
            }
        });
    }

    fn table(&mut self, ui: &mut egui::Ui, cfg: &Config) {
        let cols = columns(&cfg.view);
        let line = ui.text_style_height(&egui::TextStyle::Body);
        let row_h = if cfg.view.two_line { line * 2.0 + 8.0 } else { line + 6.0 };
        let selected = self.selected_index();
        let mut actions = Vec::new();
        let mut sort_click = None;
        let mut hovered = String::new();
        ui.style_mut().interaction.selectable_labels = false;

        let mut tb = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .auto_shrink(false)
            .cell_layout(Layout::left_to_right(Align::Center));
        for c in &cols {
            tb = tb.column(c.column());
        }
        if let Some(i) = self.scroll_to.take() {
            tb = tb.scroll_to_row(i, None);
        }
        let this = &*self;
        tb.header(22.0, |mut h| {
            for c in &cols {
                h.col(|ui| {
                    let mark = match c.sort_key() {
                        Some(k) if k == this.sort.0 => if this.sort.1 { " ▼" } else { " ▲" },
                        _ => "",
                    };
                    let r = ui.add(egui::Label::new(RichText::new(format!("{}{}", c.header(), mark)).strong()).sense(Sense::click()));
                    if r.clicked() {
                        sort_click = c.sort_key();
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, this.view.len(), |mut row| {
                let vi = row.index();
                let i = this.view[vi];
                let r = &this.rows[i];
                let c = &r.ch;
                row.set_selected(selected == Some(i));
                let bg = if r.ch.is_info() { Some(Color32::from_rgba_unmultiplied(128, 128, 128, 40)) } else { r.color };
                for col in &cols {
                    row.col(|ui| {
                        if let Some(bg) = bg {
                            let rect = ui.max_rect().expand2(egui::vec2(4.0, 2.0));
                            ui.painter().rect_filled(rect, 0.0, bg);
                        }
                        ui.spacing_mut().item_spacing.y = 0.0;
                        match col {
                            Col::TwoName => {
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        if r.is_new {
                                            ui.label(RichText::new("NEW").small().color(Color32::from_rgb(230, 120, 0)));
                                        }
                                        ui.add(egui::Label::new(this.name_text(ui, r)).truncate());
                                    });
                                    ui.add(egui::Label::new(c.summary()).truncate());
                                });
                            }
                            Col::Name => {
                                if r.is_new {
                                    ui.label(RichText::new("NEW").small().color(Color32::from_rgb(230, 120, 0)));
                                }
                                ui.add(egui::Label::new(this.name_text(ui, r)).truncate());
                            }
                            Col::Summary => {
                                ui.add(egui::Label::new(c.summary()).truncate());
                            }
                            Col::Listeners => {
                                right(ui, format!("{}/{}", listeners_text(c.listeners), listeners_text(c.relays)));
                            }
                            Col::Bitrate => right(ui, c.bitrate.to_string()),
                            Col::Uptime => right(ui, c.uptime.clone()),
                            Col::Type => {
                                ui.label(&c.content_type);
                            }
                            Col::Yp => {
                                ui.add(egui::Label::new(RichText::new(&r.yp).weak()).truncate());
                            }
                            Col::Track => {
                                ui.add(egui::Label::new(c.track_text()).truncate());
                            }
                            Col::TwoStats => {
                                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                                    if cfg.view.columns.listeners {
                                        ui.label(format!("{}/{}", listeners_text(c.listeners), listeners_text(c.relays)));
                                    }
                                    if cfg.view.columns.uptime {
                                        ui.label(&c.uptime);
                                    }
                                });
                            }
                            Col::TwoRate => {
                                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                                    if cfg.view.columns.bitrate {
                                        ui.label(c.bitrate.to_string());
                                    }
                                    if cfg.view.columns.content_type {
                                        ui.label(&c.content_type);
                                    }
                                });
                            }
                        }
                    });
                }
                let resp = row.response();
                if resp.hovered() {
                    hovered = c.url.clone();
                }
                if resp.double_clicked() {
                    actions.push(Action::Play(i));
                } else if resp.clicked() || resp.secondary_clicked() {
                    actions.push(Action::Select(i));
                }
                let summary = c.summary();
                let resp = if summary.is_empty() { resp } else { resp.on_hover_text_at_pointer(summary) };
                resp.context_menu(|ui| this.channel_menu(ui, i, &mut actions));
            });
        });
        self.hovered_url = hovered;
        if let Some(k) = sort_click {
            self.set_sort(k);
        }
        self.do_actions(actions);
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (f5, enter, up, down) = ctx.input(|i| {
            (i.key_pressed(Key::F5), i.key_pressed(Key::Enter), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown))
        });
        if f5 {
            self.refresh();
        }
        if enter && let Some(i) = self.selected_index() {
            let ch = self.rows[i].ch.clone();
            self.play(&ch);
        }
        if (up || down) && !self.view.is_empty() {
            let pos = self.selected_index().and_then(|i| self.view.iter().position(|&v| v == i));
            let next = match (pos, down) {
                (None, _) => 0,
                (Some(p), true) => (p + 1).min(self.view.len() - 1),
                (Some(p), false) => p.saturating_sub(1),
            };
            self.selected = Some(self.rows[self.view[next]].ch.key());
            self.scroll_to = Some(next);
        }
    }

    // ------------------------------------------------------------------ ウィンドウ

    fn log_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_log;
        sub_window(ctx, "log", "ログ", [640.0, 360.0], &mut open, |ui| {
            let mut s = lock(&self.shared);
            if ui.button("クリア").clicked() {
                s.log.clear();
            }
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                for l in &s.log {
                    let t = RichText::new(format!("{} {}", l.time, l.text));
                    ui.label(if l.error { t.color(Color32::from_rgb(220, 60, 60)) } else { t });
                }
            });
        });
        self.show_log = open;
    }

    fn relay_window(&mut self, ctx: &egui::Context) {
        if !self.show_relay {
            return;
        }
        if self.relay_last.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
            self.load_relays();
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        let mut open = true;
        let mut actions = Vec::new();
        let mut reload = false;
        sub_window(ctx, "relay", "接続中のチャンネル (PeerCast)", [600.0, 300.0], &mut open, |ui| {
            let r = self.relay.lock().unwrap_or_else(|e| e.into_inner());
            ui.horizontal(|ui| {
                if ui.button("⟳ 再読み込み").clicked() {
                    reload = true;
                }
                if r.loading {
                    ui.spinner();
                }
            });
            if !r.error.is_empty() {
                ui.colored_label(Color32::from_rgb(220, 60, 60), &r.error);
            }
            if r.channels.is_empty() && r.error.is_empty() {
                ui.label("つないでいるチャンネルはありません");
            }
            egui::Grid::new("relays").striped(true).num_columns(5).show(ui, |ui| {
                for c in &r.channels {
                    ui.label(&c.name);
                    ui.label(&c.status);
                    ui.label(format!("視聴 {} / 中継 {}", c.listeners, c.relays));
                    ui.label(format!("{} kbps {}", c.bitrate, c.content_type));
                    ui.horizontal(|ui| {
                        if ui.small_button("再接続").clicked() {
                            actions.push(Action::Bump(c.id.clone()));
                        }
                        if ui.small_button("停止").clicked() {
                            actions.push(Action::Stop(c.id.clone()));
                        }
                    });
                    ui.end_row();
                }
            });
        });
        self.show_relay = open;
        if reload || !actions.is_empty() {
            self.relay_last = None;
        }
        self.do_actions(actions);
    }

    fn open_settings_dialog(&mut self, cfg: &Config) {
        if self.settings.is_none() {
            self.settings = Some(SettingsDlg { cfg: cfg.clone(), tab: SettingsTab::Yp, error: String::new(), test: Arc::new(Mutex::new(String::new())) });
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.settings.as_mut() else { return };
        let mut open = true;
        let mut result: Option<bool> = None; // Some(true) で閉じる、Some(false) で適用だけ
        let mut cancel = false;
        sub_window(ctx, "settings", "設定", [720.0, 560.0], &mut open, |ui| {
            ui.horizontal(|ui| {
                for (t, label) in [
                    (SettingsTab::Yp, "YP"),
                    (SettingsTab::Update, "更新"),
                    (SettingsTab::PeerCast, "PeerCast"),
                    (SettingsTab::Player, "プレイヤー"),
                    (SettingsTab::Notify, "通知・トレイ"),
                    (SettingsTab::View, "表示・その他"),
                ] {
                    ui.selectable_value(&mut dlg.tab, t, label);
                }
            });
            ui.separator();
            let avail = ui.available_height() - 48.0;
            egui::ScrollArea::vertical().max_height(avail.max(100.0)).auto_shrink(false).show(ui, |ui| match dlg.tab {
                SettingsTab::Yp => settings_yp(ui, &mut dlg.cfg),
                SettingsTab::Update => settings_update(ui, &mut dlg.cfg),
                SettingsTab::PeerCast => settings_peercast(ui, &mut dlg.cfg, &dlg.test),
                SettingsTab::Player => settings_player(ui, &mut dlg.cfg),
                SettingsTab::Notify => settings_notify(ui, &mut dlg.cfg),
                SettingsTab::View => settings_view(ui, &mut dlg.cfg),
            });
            ui.separator();
            if !dlg.error.is_empty() {
                ui.colored_label(Color32::from_rgb(220, 60, 60), &dlg.error);
            }
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    result = Some(true);
                }
                if ui.button("適用").clicked() {
                    result = Some(false);
                }
                if ui.button("キャンセル").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            open = false;
        }
        if let Some(close) = result {
            let dlg = self.settings.as_mut().unwrap();
            match validate(&mut dlg.cfg) {
                Ok(()) => {
                    dlg.error.clear();
                    let new = dlg.cfg.clone();
                    self.apply_config(ctx, new);
                    if close {
                        open = false;
                    }
                }
                Err(e) => dlg.error = e,
            }
        }
        if !open {
            self.settings = None;
        }
    }

    fn filter_window(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.filter_dlg.as_mut() else { return };
        let mut open = true;
        let mut apply = None;
        let mut cancel = false;
        sub_window(ctx, "filters", "フィルター (お気に入り・無視・色分け)", [760.0, 520.0], &mut open, |ui| {
            ui.horizontal(|ui| {
                if ui.button("追加").clicked() {
                    dlg.filters.push(Filter::default());
                    dlg.sel = dlg.filters.len() - 1;
                }
                let n = dlg.filters.len();
                if ui.add_enabled(n > 0, egui::Button::new("削除")).clicked() && dlg.sel < n {
                    dlg.filters.remove(dlg.sel);
                    dlg.sel = dlg.sel.min(dlg.filters.len().saturating_sub(1));
                }
                if ui.add_enabled(dlg.sel > 0 && dlg.sel < n, egui::Button::new("↑")).clicked() {
                    dlg.filters.swap(dlg.sel, dlg.sel - 1);
                    dlg.sel -= 1;
                }
                if ui.add_enabled(dlg.sel + 1 < n, egui::Button::new("↓")).clicked() {
                    dlg.filters.swap(dlg.sel, dlg.sel + 1);
                    dlg.sel += 1;
                }
            });
            ui.separator();
            let height = ui.available_height() - 44.0;
            ui.horizontal_top(|ui| {
                ui.set_max_height(height);
                egui::ScrollArea::vertical().id_salt("flist").max_height(height).max_width(200.0).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(190.0);
                    for (i, f) in dlg.filters.iter().enumerate() {
                        let mark = if f.ignore { "🚫" } else if f.favorite { "★" } else { "🎨" };
                        let title = if f.title().is_empty() { "(新しいフィルター)" } else { f.title() };
                        let mut text = RichText::new(format!("{} {}", mark, title));
                        if !f.enabled {
                            text = text.weak().strikethrough();
                        }
                        if ui.selectable_label(dlg.sel == i, text).clicked() {
                            dlg.sel = i;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    if let Some(f) = dlg.filters.get_mut(dlg.sel) {
                        filter_editor(ui, f);
                    } else {
                        ui.label("「追加」でフィルターを作ります");
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    apply = Some(true);
                }
                if ui.button("適用").clicked() {
                    apply = Some(false);
                }
                if ui.button("キャンセル").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            open = false;
        }
        if let Some(close) = apply {
            let filters = self.filter_dlg.as_ref().unwrap().filters.clone();
            self.save_filters(filters);
            if close {
                open = false;
            }
        }
        if !open {
            self.filter_dlg = None;
        }
    }

    // ------------------------------------------------------------------ ウィンドウの状態

    fn window_state(&mut self, ctx: &egui::Context, cfg: &Config) {
        if std::mem::take(&mut self.first_frame) && cfg.tray.start_minimized && self.tray.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        let (close, minimized, rect) = ctx.input(|i| {
            let v = i.viewport();
            (v.close_requested(), v.minimized.unwrap_or(false), v.inner_rect)
        });
        if !minimized && let Some(r) = rect {
            self.last_size = Some([r.width(), r.height()]);
        }
        let tray = self.tray.is_some();
        if minimized && !self.was_minimized && tray && cfg.tray.minimize_to_tray {
            win::hide_window();
        }
        self.was_minimized = minimized;
        if close && !self.closing {
            if tray && cfg.tray.close_to_tray && !QUITTING.load(Ordering::SeqCst) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                win::hide_window();
            } else {
                self.closing = true;
                let mut c = self.cfg();
                if let Some(s) = self.last_size {
                    c.view.window_size = Some(s);
                }
                if let Err(e) = config::save_json(config::CONFIG_FILE, &c) {
                    eprintln!("{}", e);
                }
                let _ = self.tx.send(Command::Quit);
            }
        }
        if self.open_settings.swap(false, Ordering::SeqCst) {
            self.open_settings_dialog(cfg);
        }
    }
}

/// メインとは別の OS のウィンドウを開く (自由に移動・サイズ変更できる)。閉じるボタンで `open` を false にする。
fn sub_window(ctx: &egui::Context, id: &str, title: &str, size: [f32; 2], open: &mut bool, mut add: impl FnMut(&mut egui::Ui)) {
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_min_inner_size([320.0, 200.0])
        .with_icon(win::app_icon());
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(id), builder, |ui, _class| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            *open = false;
        }
        egui::CentralPanel::default().show(ui, |ui| add(ui));
    });
}

fn right(ui: &mut egui::Ui, text: String) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(text);
    });
}

pub fn apply_style(ctx: &egui::Context, cfg: &Config) {
    ctx.set_theme(if cfg.view.dark { egui::Theme::Dark } else { egui::Theme::Light });
    let size = cfg.view.font_size.clamp(9.0, 32.0);
    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        for (style, font) in s.text_styles.iter_mut() {
            font.size = match style {
                Small => size * 0.8,
                Heading => size * 1.3,
                _ => size,
            };
        }
    });
}

fn validate(cfg: &mut Config) -> Result<(), String> {
    for y in &mut cfg.yps {
        y.url = y.url.trim().to_string();
        y.name = y.name.trim().to_string();
        if !config::valid_yp_url(&y.url) {
            return Err(format!("YP「{}」の URL は http:// か https:// で始めてください", y.name));
        }
        if y.name.is_empty() {
            return Err(format!("YP ({}) の名前が空です", y.url));
        }
    }
    let mut urls = HashSet::new();
    for y in &cfg.yps {
        if !urls.insert(y.url.clone()) {
            return Err(format!("同じ URL の YP が 2 つあります: {}", y.url));
        }
    }
    config::parse_address(&cfg.peercast.address)?;
    cfg.peercast.address = cfg.peercast.address.trim().to_string();
    cfg.update_interval_min = cfg.update_interval_min.max(config::MIN_AUTO_INTERVAL_MIN);
    Ok(())
}

fn pick_exe(current: &mut String, filter_name: &str) {
    let mut d = rfd::FileDialog::new().add_filter(filter_name, &["exe"]).add_filter("すべて", &["*"]);
    let p = crate::player::resolve_exe(current);
    if let Some(dir) = p.parent().filter(|d| d.is_dir()) {
        d = d.set_directory(dir);
    }
    if let Some(p) = d.pick_file() {
        // exe のフォルダの下なら相対パスにする (ポータブル版のため)
        let base = config::base_dir();
        *current = match p.strip_prefix(&base) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => p.display().to_string(),
        };
    }
}

fn settings_yp(ui: &mut egui::Ui, cfg: &mut Config) {
    ui.label("index.txt を取得する YP。URL は http と https だけ使えます。");
    let mut remove = None;
    let mut swap = None;
    let n = cfg.yps.len();
    egui::Grid::new("yps").num_columns(4).striped(true).show(ui, |ui| {
        ui.label("有効");
        ui.label("名前");
        ui.label("index.txt の URL");
        ui.label("");
        ui.end_row();
        for (i, y) in cfg.yps.iter_mut().enumerate() {
            ui.checkbox(&mut y.enabled, "");
            ui.add_sized([90.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut y.name));
            let ok = config::valid_yp_url(&y.url);
            let mut te = egui::TextEdit::singleline(&mut y.url);
            if !ok {
                te = te.text_color(Color32::from_rgb(220, 60, 60));
            }
            ui.add_sized([320.0, ui.spacing().interact_size.y], te);
            ui.horizontal(|ui| {
                if ui.add_enabled(i > 0, egui::Button::new("↑").small()).clicked() {
                    swap = Some((i, i - 1));
                }
                if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).clicked() {
                    swap = Some((i, i + 1));
                }
                if ui.small_button("削除").clicked() {
                    remove = Some(i);
                }
            });
            ui.end_row();
        }
    });
    if let Some((a, b)) = swap {
        cfg.yps.swap(a, b);
    }
    if let Some(i) = remove {
        cfg.yps.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button("追加").clicked() {
            cfg.yps.push(YpEntry { name: "新しい YP".into(), url: "http://".into(), enabled: true });
        }
        if ui.button("初期値に戻す").clicked() {
            cfg.yps = config::default_yps();
        }
    });
}

fn settings_update(ui: &mut egui::Ui, cfg: &mut Config) {
    ui.checkbox(&mut cfg.fetch_on_start, "起動したら一覧を取得する");
    ui.checkbox(&mut cfg.auto_update, "自動で更新する");
    ui.horizontal(|ui| {
        ui.label("更新の間隔");
        ui.add(egui::DragValue::new(&mut cfg.update_interval_min).range(config::MIN_AUTO_INTERVAL_MIN..=120).suffix(" 分"));
    });
    ui.label(
        RichText::new(format!(
            "YP サーバーに負担をかけないよう、自動更新は {} 分以上、手動更新は前回から {} 秒以上空けます。",
            config::MIN_AUTO_INTERVAL_MIN,
            config::MANUAL_INTERVAL_SEC
        ))
        .weak(),
    );
}

fn settings_peercast(ui: &mut egui::Ui, cfg: &mut Config, test: &Arc<Mutex<String>>) {
    let pc = &mut cfg.peercast;
    egui::Grid::new("pc").num_columns(2).show(ui, |ui| {
        ui.label("種類");
        egui::ComboBox::from_id_salt("pckind")
            .selected_text(match pc.kind {
                PeerCastKind::Auto => "自動",
                PeerCastKind::PeerCastYt => "PeerCast YT (C++ / Rust)",
                PeerCastKind::PeerCastStation => "PeerCastStation",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut pc.kind, PeerCastKind::Auto, "自動");
                ui.selectable_value(&mut pc.kind, PeerCastKind::PeerCastYt, "PeerCast YT (C++ / Rust)");
                ui.selectable_value(&mut pc.kind, PeerCastKind::PeerCastStation, "PeerCastStation");
            });
        ui.end_row();
        ui.label("アドレス");
        ui.horizontal(|ui| {
            let ok = config::parse_address(&pc.address).is_ok();
            let mut te = egui::TextEdit::singleline(&mut pc.address).hint_text("127.0.0.1:7144");
            if !ok {
                te = te.text_color(Color32::from_rgb(220, 60, 60));
            }
            ui.add_sized([220.0, ui.spacing().interact_size.y], te);
            ui.label(RichText::new("ホスト:ポート").weak());
        });
        ui.end_row();
        ui.label("再生の URL");
        ui.vertical(|ui| {
            ui.radio_value(&mut pc.url_kind, PlayUrlKind::Stream, "/stream/<ID>.<拡張子>?tip=… (推奨)");
            ui.radio_value(&mut pc.url_kind, PlayUrlKind::Playlist, "/pls/<ID>?tip=… (プレイリスト)");
        });
        ui.end_row();
        ui.label("ユーザー名");
        ui.add_sized([200.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut pc.user));
        ui.end_row();
        ui.label("パスワード");
        ui.add_sized([200.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut pc.password).password(true));
        ui.end_row();
        ui.label("PeerCast 本体");
        ui.horizontal(|ui| {
            ui.add_sized([300.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut pc.exe_path).hint_text("PeerCastStation.exe など"));
            if ui.button("参照…").clicked() {
                pick_exe(&mut pc.exe_path, "PeerCast");
            }
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut pc.launch_on_start, "起動時に PeerCast 本体も起動する (動いていなければ)");
        ui.end_row();
    });
    ui.label(
        RichText::new("ユーザー名とパスワードは、PeerCast が別の PC にあるときの「接続中のチャンネル」「停止」「再接続」にだけ使います (PeerCast の管理画面のパスワード)。一覧の取得と再生には要りません。")
            .weak(),
    );
    ui.horizontal(|ui| {
        if ui.button("接続を確認").clicked() {
            let pc = pc.clone();
            let test = test.clone();
            *test.lock().unwrap_or_else(|e| e.into_inner()) = "確認中…".into();
            std::thread::spawn(move || {
                let msg = if !peercast::is_running(&pc) {
                    format!("{} につながりません。PeerCast が起動しているか確かめてください", pc.base_url())
                } else {
                    match Rpc::new(&pc).version_info() {
                        Ok(v) => format!("OK: {} ({:?})", v.agent, v.kind),
                        Err(e) => format!("ポートは開いていますが、JSON-RPC に失敗しました: {}", e),
                    }
                };
                *test.lock().unwrap_or_else(|e| e.into_inner()) = msg;
                win::request_repaint();
            });
        }
        ui.label(test.lock().unwrap_or_else(|e| e.into_inner()).as_str());
    });
}

fn settings_player(ui: &mut egui::Ui, cfg: &mut Config) {
    ui.label("種類 (FLV、MKV など。カンマ区切り、* はすべて) ごとのプレイヤー。上から順に、最初に当てはまったものを使います。");
    let mut remove = None;
    let mut swap = None;
    let n = cfg.players.len();
    for (i, p) in cfg.players.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::Grid::new(("player", i)).num_columns(2).show(ui, |ui| {
                ui.label("種類");
                ui.horizontal(|ui| {
                    ui.add_sized([140.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut p.types));
                    if ui.add_enabled(i > 0, egui::Button::new("↑").small()).clicked() {
                        swap = Some((i, i - 1));
                    }
                    if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).clicked() {
                        swap = Some((i, i + 1));
                    }
                    if ui.small_button("削除").clicked() {
                        remove = Some(i);
                    }
                });
                ui.end_row();
                ui.label("プレイヤー");
                ui.horizontal(|ui| {
                    ui.add_sized([360.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut p.exe));
                    if ui.button("参照…").clicked() {
                        pick_exe(&mut p.exe, "プレイヤー");
                    }
                });
                ui.end_row();
                ui.label("引数");
                ui.add_sized([420.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut p.args));
                ui.end_row();
            });
        });
    }
    if let Some((a, b)) = swap {
        cfg.players.swap(a, b);
    }
    if let Some(i) = remove {
        cfg.players.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button("追加").clicked() {
            cfg.players.push(PlayerEntry::default());
        }
        ui.menu_button("雛形から追加…", |ui| {
            let presets: [(&str, &str, &str); 5] = [
                ("mpv", "mpv.exe", "--force-media-title=\"$NAME\" \"$URL\""),
                ("MPC-BE", "mpc-be64.exe", "\"$URL\""),
                ("MPC-HC", "mpc-hc64.exe", "\"$URL\""),
                ("VLC", "C:\\Program Files\\VideoLAN\\VLC\\vlc.exe", "--meta-title=\"$NAME\" \"$URL\""),
                ("PeerstPlayer", "PeerstPlayer\\PeerstPlayer.exe", "\"$URL\" \"$NAME\" \"$CONTACT\""),
            ];
            for (label, exe, args) in presets {
                if ui.button(label).clicked() {
                    cfg.players.push(PlayerEntry { types: "*".into(), exe: exe.into(), args: args.into() });
                    ui.close();
                }
            }
        });
    });
    ui.separator();
    ui.label("引数で使える置き換え (\" で囲むと空白を含む値も 1 つの引数になります):");
    egui::Grid::new("ph").num_columns(2).show(ui, |ui| {
        for (k, d) in player::PLACEHOLDERS {
            ui.monospace(*k);
            ui.label(*d);
            ui.end_row();
        }
    });
    ui.label(RichText::new("pcyplite 形式の <stream/> <channelname/> <contact/> も使えます。").weak());
}

fn settings_notify(ui: &mut egui::Ui, cfg: &mut Config) {
    ui.heading("通知");
    ui.checkbox(&mut cfg.notify.enabled, "お気に入りのチャンネルが始まったら通知する");
    ui.checkbox(&mut cfg.notify.on_first_fetch, "起動して最初の取得でも通知する");
    ui.label(RichText::new("通知するかどうかは、フィルターごとの「通知する」でも選べます。通知の「再生」で再生します。").weak());
    ui.separator();
    ui.heading("タスクトレイ");
    ui.checkbox(&mut cfg.tray.enabled, "タスクトレイにアイコンを出す (再起動で反映)");
    ui.add_enabled_ui(cfg.tray.enabled, |ui| {
        ui.checkbox(&mut cfg.tray.minimize_to_tray, "最小化したらタスクトレイに格納する");
        ui.checkbox(&mut cfg.tray.close_to_tray, "閉じるボタンでタスクトレイに格納する (終了はトレイのメニューから)");
        ui.checkbox(&mut cfg.tray.start_minimized, "起動時にタスクトレイに格納する");
    });
}

fn settings_view(ui: &mut egui::Ui, cfg: &mut Config) {
    egui::Grid::new("view").num_columns(2).show(ui, |ui| {
        ui.label("文字の大きさ");
        ui.add(egui::Slider::new(&mut cfg.view.font_size, 10.0..=24.0).step_by(1.0));
        ui.end_row();
        ui.label("フォント (再起動で反映)");
        ui.horizontal(|ui| {
            ui.add_sized([300.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut cfg.view.font_path).hint_text("空なら Yu Gothic か Meiryo"));
            if ui.button("参照…").clicked()
                && let Some(p) = rfd::FileDialog::new().add_filter("フォント", &["ttf", "ttc", "otf"]).set_directory("C:\\Windows\\Fonts").pick_file()
            {
                cfg.view.font_path = p.display().to_string();
            }
        });
        ui.end_row();
        ui.label("URL を開くブラウザ");
        ui.horizontal(|ui| {
            ui.add_sized([300.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut cfg.browser).hint_text("空なら既定のブラウザ"));
            if ui.button("参照…").clicked() {
                pick_exe(&mut cfg.browser, "ブラウザ");
            }
        });
        ui.end_row();
    });
    ui.checkbox(&mut cfg.view.two_line, "2 行で表示 (pcyplite 風)");
    ui.checkbox(&mut cfg.view.dark, "ダークモード");
    ui.checkbox(&mut cfg.view.show_info_rows, "YP のお知らせの行を表示する");
    ui.separator();
    ui.label(RichText::new(format!("設定の保存先: {}", config::base_dir().display())).weak());
}

fn search_editor(ui: &mut egui::Ui, id: &str, s: &mut Search, toggle: Option<&str>) {
    ui.horizontal(|ui| {
        match toggle {
            Some(label) => {
                ui.checkbox(&mut s.enabled, label);
            }
            None => {
                ui.label("条件");
            }
        }
        ui.add_enabled(toggle.is_none() || s.enabled, egui::TextEdit::singleline(&mut s.search).desired_width(260.0).hint_text("正規表現"));
    });
    ui.add_enabled_ui(toggle.is_none() || s.enabled, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label("  対象:");
            for (key, label) in FIELDS {
                let mut on = s.fields.iter().any(|f| f == key);
                if ui.checkbox(&mut on, *label).changed() {
                    if on {
                        s.fields.push(key.to_string());
                    } else {
                        s.fields.retain(|f| f != key);
                    }
                }
            }
        });
    });
    let _ = id;
}

fn filter_editor(ui: &mut egui::Ui, f: &mut Filter) {
    egui::ScrollArea::vertical().id_salt("fedit").auto_shrink(false).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("名前");
            ui.add_sized([200.0, ui.spacing().interact_size.y], egui::TextEdit::singleline(&mut f.name).hint_text("空なら条件を表示"));
            ui.checkbox(&mut f.enabled, "有効");
        });
        ui.horizontal(|ui| {
            if ui.radio(f.favorite && !f.ignore, "★ お気に入り").clicked() {
                f.favorite = true;
                f.ignore = false;
            }
            if ui.radio(f.ignore, "🚫 無視").clicked() {
                f.ignore = true;
                f.favorite = false;
            }
            if ui.radio(!f.favorite && !f.ignore, "🎨 色分けだけ").clicked() {
                f.favorite = false;
                f.ignore = false;
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut f.enable_color, "色をつける");
            ui.color_edit_button_srgb(&mut f.color);
            ui.add_enabled(f.favorite, egui::Checkbox::new(&mut f.notify, "始まったら通知する"));
            ui.checkbox(&mut f.ignore_case, "大文字と小文字を区別しない");
        });
        ui.separator();
        search_editor(ui, "base", &mut f.base_search, None);
        search_editor(ui, "and", &mut f.and_search, Some("かつ"));
        search_editor(ui, "not", &mut f.not_search, Some("ただし次は除く"));
        let (_, errors) = filter::compile(std::slice::from_ref(&Filter { enabled: true, ..f.clone() }));
        for e in errors {
            ui.colored_label(Color32::from_rgb(220, 60, 60), e);
        }
        ui.label(RichText::new("正規表現は Rust の regex の書き方です (JavaScript と少し違います)。").weak());
    });
}

impl eframe::App for App {
    /// 最小化中や非表示中も呼ばれる
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let cfg = self.cfg();
        self.window_state(ctx, &cfg);
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let cfg = self.cfg();
        self.rebuild_rows();
        self.rebuild_view(&cfg);
        self.keyboard(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(2.0);
            self.toolbar(ui, &cfg);
            self.tab_bar(ui, &cfg);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &cfg));
        if cfg.view.show_info_panel
            && let Some(i) = self.selected_index()
        {
            egui::Panel::bottom("info").resizable(true).default_size(96.0).show(ui, |ui| self.info_panel(ui, i));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.view.is_empty() {
                let fetching = lock(&self.shared).fetching;
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(if fetching { "取得中…" } else { "チャンネルはありません" }).weak());
                });
            } else {
                self.table(ui, &cfg);
            }
        });

        self.settings_window(&ctx);
        self.filter_window(&ctx);
        self.log_window(&ctx);
        self.relay_window(&ctx);

        // 次の自動更新までの表示のため
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}
