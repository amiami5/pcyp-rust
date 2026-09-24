//! YP の取得を GUI とは別のスレッドで行う。結果は共有の状態に書き、GUI はそれを読む。

use crate::chandir::{Channel, parse_index};
use crate::config::{Config, MANUAL_INTERVAL_SEC};
use crate::filter::{self, Filter};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub const MAX_LOG: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
pub enum FetchState {
    Idle,
    Loading,
    Ok,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct YpStatus {
    pub name: String,
    pub url: String,
    pub state: FetchState,
    pub channels: Vec<Channel>,
    /// 解析できなかった行の番号
    pub bad_lines: Vec<usize>,
    pub updated: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub time: String,
    pub error: bool,
    pub text: String,
}

#[derive(Default)]
pub struct Shared {
    pub yps: Vec<YpStatus>,
    /// 取得が終わるたびに増える
    pub generation: u64,
    pub fetching: bool,
    pub last_fetch: Option<Instant>,
    pub next_auto: Option<Instant>,
    /// 直前の取得で新しく現れたチャンネルのキー
    pub new_keys: HashSet<String>,
    pub log: VecDeque<LogLine>,
}

impl Shared {
    pub fn log(&mut self, error: bool, text: impl Into<String>) {
        if self.log.len() >= MAX_LOG {
            self.log.pop_front();
        }
        self.log.push_back(LogLine { time: crate::win::now_hms(), error, text: text.into() });
    }

    /// 手動更新をあと何秒待つ必要があるか。
    pub fn manual_wait(&self) -> u64 {
        match self.last_fetch {
            Some(t) => MANUAL_INTERVAL_SEC.saturating_sub(t.elapsed().as_secs()),
            None => 0,
        }
    }
}

pub enum Command {
    Refresh { manual: bool },
    ConfigChanged,
    Quit,
}

pub type SharedRef = Arc<Mutex<Shared>>;

/// 通知の方法 (テストでは差し替える)。
pub type Notifier = Arc<dyn Fn(&[Channel]) + Send + Sync>;
/// index.txt を取る方法 (テストでは差し替える)。
pub type Fetcher = Arc<dyn Fn(&str, u16) -> Result<String, String> + Send + Sync>;

pub struct Worker {
    pub config: Arc<RwLock<Config>>,
    pub filters: Arc<RwLock<Vec<Filter>>>,
    pub shared: SharedRef,
    pub repaint: Arc<dyn Fn() + Send + Sync>,
    pub notifier: Notifier,
    pub fetcher: Fetcher,
}

pub fn lock(s: &SharedRef) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// 設定の YP の並びに合わせる。残った YP の一覧はそのまま持つ。
pub fn sync_yps(shared: &mut Shared, cfg: &Config) {
    let mut old: HashMap<String, YpStatus> = shared.yps.drain(..).map(|y| (y.url.clone(), y)).collect();
    for e in cfg.yps.iter().filter(|e| e.enabled) {
        let mut y = old.remove(&e.url).unwrap_or(YpStatus {
            name: String::new(),
            url: e.url.clone(),
            state: FetchState::Idle,
            channels: Vec::new(),
            bad_lines: Vec::new(),
            updated: None,
        });
        y.name = e.name.clone();
        shared.yps.push(y);
    }
}

impl Worker {
    pub fn spawn(self, rx: Receiver<Command>) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new().name("yp-worker".into()).spawn(move || self.run(rx)).expect("spawn worker")
    }

    fn cfg(&self) -> Config {
        self.config.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn run(self, rx: Receiver<Command>) {
        let cfg = self.cfg();
        {
            let mut s = lock(&self.shared);
            sync_yps(&mut s, &cfg);
        }
        // YP ごとに、前回の取得で見えたチャンネルのキー
        let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
        let mut next_auto = Instant::now() + if cfg.fetch_on_start { Duration::ZERO } else { Duration::from_secs(cfg.update_interval_sec()) };
        loop {
            let cfg = self.cfg();
            lock(&self.shared).next_auto = cfg.auto_update.then_some(next_auto);
            (self.repaint)();
            // 自動更新が切れていても、起動時の取得は 1 回だけ行う
            let pending_first = cfg.fetch_on_start && lock(&self.shared).last_fetch.is_none();
            let timed = cfg.auto_update || pending_first;
            let timeout = if timed { next_auto.saturating_duration_since(Instant::now()) } else { Duration::from_secs(3600) };
            match rx.recv_timeout(timeout) {
                Ok(Command::Refresh { manual }) => {
                    let wait = lock(&self.shared).manual_wait();
                    if manual && wait > 0 {
                        lock(&self.shared).log(false, format!("手動更新は前回から {} 秒空けてください (あと {} 秒)", MANUAL_INTERVAL_SEC, wait));
                        (self.repaint)();
                        continue;
                    }
                }
                Ok(Command::ConfigChanged) => {
                    let mut s = lock(&self.shared);
                    sync_yps(&mut s, &cfg);
                    if let Some(t) = s.last_fetch {
                        next_auto = t + Duration::from_secs(cfg.update_interval_sec());
                    }
                    continue;
                }
                Ok(Command::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) if !timed => continue,
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.fetch_all(&cfg, &mut seen);
            next_auto = Instant::now() + Duration::from_secs(cfg.update_interval_sec());
        }
    }

    /// 有効な YP を並列に取る。1 つの YP が失敗しても、ほかの YP の一覧は使う。
    pub fn fetch_all(&self, cfg: &Config, seen: &mut HashMap<String, HashSet<String>>) {
        let targets: Vec<(String, String)> = {
            let mut s = lock(&self.shared);
            sync_yps(&mut s, cfg);
            s.fetching = true;
            s.last_fetch = Some(Instant::now());
            for y in s.yps.iter_mut() {
                y.state = FetchState::Loading;
            }
            s.yps.iter().map(|y| (y.name.clone(), y.url.clone())).collect()
        };
        (self.repaint)();
        let port = cfg.peercast.port;
        std::thread::scope(|scope| {
            for (name, url) in &targets {
                let fetcher = self.fetcher.clone();
                let shared = self.shared.clone();
                let repaint = self.repaint.clone();
                scope.spawn(move || {
                    let result = fetcher(url, port);
                    let mut s = lock(&shared);
                    let msg = match result {
                        Ok(text) => {
                            let (chans, bad) = parse_index(&text, url);
                            let msg = if bad.is_empty() {
                                (false, format!("{}: {} チャンネル", name, chans.len()))
                            } else {
                                (true, format!("{}: {} チャンネル、解析できない行 {:?}", name, chans.len(), bad))
                            };
                            if let Some(y) = s.yps.iter_mut().find(|y| &y.url == url) {
                                y.channels = chans;
                                y.bad_lines = bad;
                                y.state = FetchState::Ok;
                                y.updated = Some(crate::win::now_hms());
                            }
                            msg
                        }
                        Err(e) => {
                            if let Some(y) = s.yps.iter_mut().find(|y| &y.url == url) {
                                y.state = FetchState::Error(e.clone());
                            }
                            (true, format!("{}: 取得に失敗しました: {}", name, e))
                        }
                    };
                    s.log(msg.0, msg.1);
                    drop(s);
                    repaint();
                });
            }
        });
        let to_notify = self.finish_fetch(cfg, seen);
        if !to_notify.is_empty() {
            (self.notifier)(&to_notify);
        }
        (self.repaint)();
    }

    /// 新しく現れたチャンネルを調べ、通知するものを返す。
    fn finish_fetch(&self, cfg: &Config, seen: &mut HashMap<String, HashSet<String>>) -> Vec<Channel> {
        let filters = self.filters.read().unwrap_or_else(|e| e.into_inner()).clone();
        let (compiled, _) = filter::compile(&filters);
        let mut s = lock(&self.shared);
        let mut new_keys = HashSet::new();
        let mut notify = Vec::new();
        let mut notified = HashSet::new();
        for y in &s.yps {
            if y.state != FetchState::Ok {
                continue;
            }
            let keys: HashSet<String> = y.channels.iter().map(Channel::key).collect();
            let prev = seen.get(&y.url);
            for c in &y.channels {
                let key = c.key();
                let is_new = prev.is_some_and(|p| !p.contains(&key));
                if is_new {
                    new_keys.insert(key.clone());
                }
                let first = prev.is_none();
                if cfg.notify.enabled && !c.is_info() && (is_new || first && cfg.notify.on_first_fetch) && !notified.contains(&key) {
                    let m = filter::apply(&compiled, c, &y.name);
                    if m.notify && !m.ignore {
                        notified.insert(key);
                        notify.push(c.clone());
                    }
                }
            }
            seen.insert(y.url.clone(), keys);
        }
        s.new_keys = new_keys;
        s.fetching = false;
        s.generation += 1;
        for c in &notify {
            s.log(false, format!("お気に入りの配信が始まりました: {}", c.name));
        }
        notify
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn line(name: &str, id: &str) -> String {
        format!("{name}<>{id}<>1.2.3.4:7144<><>g<>d<>1<>0<>500<>FLV<><><><><><>0:10<>click<><>0")
    }

    #[test]
    fn notifies_new_favorites_only() {
        let text = Arc::new(Mutex::new(line("A", "11111111111111111111111111111111")));
        let notified = Arc::new(Mutex::new(Vec::<String>::new()));
        let cfg = Config {
            yps: vec![crate::config::YpEntry { name: "T".into(), url: "http://t/index.txt".into(), enabled: true }],
            ..Default::default()
        };
        let t2 = text.clone();
        let n2 = notified.clone();
        let w = Worker {
            config: Arc::new(RwLock::new(cfg.clone())),
            filters: Arc::new(RwLock::new(vec![Filter::exact_name("B", false), Filter::exact_name("A", false)])),
            shared: Arc::new(Mutex::new(Shared::default())),
            repaint: Arc::new(|| {}),
            notifier: Arc::new(move |c: &[Channel]| n2.lock().unwrap().extend(c.iter().map(|c| c.name.clone()))),
            fetcher: Arc::new(move |_, _| Ok(t2.lock().unwrap().clone())),
        };
        let mut seen = HashMap::new();
        w.fetch_all(&cfg, &mut seen);
        // 最初の取得では通知しない
        assert!(notified.lock().unwrap().is_empty());
        *text.lock().unwrap() = format!("{}\n{}\n{}", line("A", "11111111111111111111111111111111"), line("B", "22222222222222222222222222222222"), line("C", "33333333333333333333333333333333"));
        w.fetch_all(&cfg, &mut seen);
        assert_eq!(*notified.lock().unwrap(), ["B"]);
        let s = lock(&w.shared);
        assert_eq!(s.new_keys.len(), 2);
        assert_eq!(s.yps[0].channels.len(), 3);
    }

    #[test]
    fn failed_yp_keeps_others() {
        let cfg = Config {
            yps: vec![
                crate::config::YpEntry { name: "ok".into(), url: "http://ok/index.txt".into(), enabled: true },
                crate::config::YpEntry { name: "ng".into(), url: "http://ng/index.txt".into(), enabled: true },
            ],
            ..Default::default()
        };
        let w = Worker {
            config: Arc::new(RwLock::new(cfg.clone())),
            filters: Arc::new(RwLock::new(vec![])),
            shared: Arc::new(Mutex::new(Shared::default())),
            repaint: Arc::new(|| {}),
            notifier: Arc::new(|_: &[Channel]| {}),
            fetcher: Arc::new(|url: &str, _| {
                if url.contains("ok") { Ok(line("A", "11111111111111111111111111111111")) } else { Err("boom".into()) }
            }),
        };
        w.fetch_all(&cfg, &mut HashMap::new());
        let s = lock(&w.shared);
        assert_eq!(s.yps[0].state, FetchState::Ok);
        assert_eq!(s.yps[0].channels.len(), 1);
        assert!(matches!(s.yps[1].state, FetchState::Error(_)));
    }

    #[test]
    fn manual_refresh_is_throttled() {
        let cfg = Config { yps: vec![], auto_update: false, fetch_on_start: false, ..Default::default() };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel();
        let h = Worker {
            config: Arc::new(RwLock::new(cfg)),
            filters: Arc::new(RwLock::new(vec![])),
            shared: shared.clone(),
            repaint: Arc::new(|| {}),
            notifier: Arc::new(|_: &[Channel]| {}),
            fetcher: Arc::new(|_, _| Ok(String::new())),
        }
        .spawn(rx);
        tx.send(Command::Refresh { manual: true }).unwrap();
        tx.send(Command::Refresh { manual: true }).unwrap();
        tx.send(Command::Quit).unwrap();
        h.join().unwrap();
        let s = lock(&shared);
        assert_eq!(s.generation, 1);
        assert!(s.log.iter().any(|l| l.text.contains("30 秒")));
    }
}
