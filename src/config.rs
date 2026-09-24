//! 設定。exe と同じフォルダに JSON で置く (ポータブル版)。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "pcyp-rust";
pub const CONFIG_FILE: &str = "pcyp-rust.json";
pub const FILTER_FILE: &str = "channelFilters.json";

/// 自動更新の間隔の最小 (分)。YP サーバーに負担をかけないため。
pub const MIN_AUTO_INTERVAL_MIN: u32 = 2;
/// 自動更新の間隔の初期値 (分)。
pub const DEFAULT_AUTO_INTERVAL_MIN: u32 = 5;
/// 手動更新は前回から何秒空けるか。
pub const MANUAL_INTERVAL_SEC: u64 = 30;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct YpEntry {
    pub name: String,
    pub url: String,
    pub enabled: bool,
}

impl Default for YpEntry {
    fn default() -> Self {
        YpEntry { name: String::new(), url: String::new(), enabled: true }
    }
}

impl YpEntry {
    fn new(name: &str, url: &str) -> Self {
        YpEntry { name: name.into(), url: url.into(), enabled: true }
    }
}

pub fn default_yps() -> Vec<YpEntry> {
    vec![
        YpEntry::new("SP", "http://bayonet.ddo.jp/sp/index.txt"),
        YpEntry::new("平成", "http://yp.pcgw.pgw.jp/index.txt"),
        YpEntry::new("P@", "https://p-at.net/index.txt"),
        YpEntry::new("YPv6", "http://ypv6.pecastation.org/index.txt"),
        YpEntry::new("Event YP", "http://eventyp.xrea.jp/index.txt"),
    ]
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlayUrlKind {
    /// `/stream/<ID><ext>?tip=`
    Stream,
    /// `/pls/<ID>?tip=`
    Playlist,
    /// `custom_url` の雛形から作る
    Custom,
}

/// 自由に書く再生の URL の初期値 (/stream/ と同じ形)
pub const DEFAULT_CUSTOM_URL: &str = "$BASE/stream/$ID$EXT?tip=$TIP";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PeerCastConfig {
    /// `ホスト:ポート` (例 `127.0.0.1:7144`、`[::1]:7144`)。ポートを省くと 7144。
    pub address: String,
    /// JSON-RPC 用。空なら認証しない。
    pub user: String,
    pub password: String,
    /// PeerCast 本体の exe。空なら起動しない。
    pub exe_path: String,
    pub launch_on_start: bool,
    pub url_kind: PlayUrlKind,
    /// `url_kind` が Custom のときの再生の URL の雛形 (`$BASE` `$ID` `$EXT` `$TIP` など)
    pub custom_url: String,
}

impl Default for PeerCastConfig {
    fn default() -> Self {
        PeerCastConfig {
            address: format!("127.0.0.1:{}", DEFAULT_PEERCAST_PORT),
            user: String::new(),
            password: String::new(),
            exe_path: String::new(),
            launch_on_start: false,
            url_kind: PlayUrlKind::Stream,
            custom_url: DEFAULT_CUSTOM_URL.into(),
        }
    }
}

pub const DEFAULT_PEERCAST_PORT: u16 = 7144;

/// `ホスト:ポート` を分ける。`http://` や末尾の `/` は取り除く。IPv6 は `[addr]:port`。
pub fn parse_address(s: &str) -> Result<(String, u16), String> {
    let t = s.trim();
    let t = t.strip_prefix("http://").unwrap_or(t);
    let t = t.trim_end_matches('/');
    if t.is_empty() {
        return Err("PeerCast のアドレスが空です".into());
    }
    let bad = || format!("PeerCast のアドレスが正しくありません: {} (例 127.0.0.1:7144)", s.trim());
    let (host, port) = if let Some(rest) = t.strip_prefix('[') {
        let (h, after) = rest.split_once(']').ok_or_else(bad)?;
        match after {
            "" => (h, None),
            p => (h, Some(p.strip_prefix(':').ok_or_else(bad)?)),
        }
    } else if t.matches(':').count() > 1 {
        // 角かっこのない IPv6 はポートなしとみなす
        (t, None)
    } else {
        match t.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (t, None),
        }
    };
    if host.is_empty() || host.contains(['/', ' ', '?', '#', '@']) {
        return Err(bad());
    }
    let port = match port {
        Some(p) => p.parse::<u16>().ok().filter(|&n| n != 0).ok_or_else(bad)?,
        None => DEFAULT_PEERCAST_PORT,
    };
    Ok((host.to_string(), port))
}

impl PeerCastConfig {
    pub fn host(&self) -> String {
        parse_address(&self.address).map(|a| a.0).unwrap_or_else(|_| "127.0.0.1".into())
    }

    pub fn port(&self) -> u16 {
        parse_address(&self.address).map(|a| a.1).unwrap_or(DEFAULT_PEERCAST_PORT)
    }

    /// `http://ホスト:ポート` (IPv6 は角かっこで囲む)
    pub fn base_url(&self) -> String {
        let h = self.host();
        let h = if h.contains(':') { format!("[{}]", h) } else { h };
        format!("http://{}:{}", h, self.port())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlayerEntry {
    /// 対象の種類をカンマで区切ったもの (例 `FLV,MKV`)。`*` はすべて。
    pub types: String,
    pub exe: String,
    pub args: String,
}

impl Default for PlayerEntry {
    fn default() -> Self {
        PlayerEntry { types: "*".into(), exe: String::new(), args: "\"$URL\"".into() }
    }
}

impl PlayerEntry {
    pub fn matches(&self, content_type: &str) -> bool {
        self.types
            .split(',')
            .map(str::trim)
            .any(|t| t == "*" || t.eq_ignore_ascii_case(content_type))
    }
}

pub fn default_players() -> Vec<PlayerEntry> {
    vec![PlayerEntry {
        types: "*".into(),
        exe: "mpv.exe".into(),
        args: "--force-media-title=\"$NAME\" \"$URL\"".into(),
    }]
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NotifyConfig {
    /// お気に入りのチャンネルが始まったら通知する
    pub enabled: bool,
    /// 起動して最初の取得でも通知する
    pub on_first_fetch: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        NotifyConfig { enabled: true, on_first_fetch: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TrayConfig {
    /// タスクトレイにアイコンを出す (要再起動)
    pub enabled: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub start_minimized: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        TrayConfig { enabled: true, minimize_to_tray: true, close_to_tray: false, start_minimized: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Columns {
    pub name: bool,
    pub summary: bool,
    pub listeners: bool,
    pub bitrate: bool,
    pub uptime: bool,
    pub content_type: bool,
    pub yp: bool,
    pub track: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Columns {
            name: true,
            summary: true,
            listeners: true,
            bitrate: true,
            uptime: true,
            content_type: true,
            yp: true,
            track: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ViewConfig {
    /// pcyplite のような 2 行表示
    pub two_line: bool,
    pub show_info_panel: bool,
    pub dark: bool,
    pub font_size: f32,
    /// 日本語のフォント (空なら Yu Gothic か Meiryo)
    pub font_path: String,
    pub columns: Columns,
    /// お知らせ (ID が 0) の行を出す
    pub show_info_rows: bool,
    /// 無視のタブを出さない
    pub hide_ignored_tab: bool,
    /// マウスのホイール 1 目盛りで一覧を何行進めるか
    pub scroll_rows: u32,
    /// 一覧で省略された文字に、カーソルを合わせたとき全文を出す
    pub show_tooltips: bool,
    /// 一覧の列の幅 (境目の線を動かして変えたもの)。列の名前ごと
    pub column_widths: std::collections::BTreeMap<String, f32>,
    /// 閉じたときの窓の位置と大きさ (スクリーンのピクセルで [左, 上, 右, 下])。次の起動でここに開く
    pub window_rect: Option<[i32; 4]>,
    /// 並べ替えの基準 (listeners, name, genre, bitrate, uptime, type, yp, random)
    pub sort_key: String,
    /// 大きい順 (文字列なら逆順)
    pub sort_desc: bool,
    /// お気に入りを上にまとめる
    pub favorites_first: bool,
    pub window_size: Option<[f32; 2]>,
}

impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            two_line: true,
            show_info_panel: true,
            dark: false,
            font_size: 14.0,
            font_path: String::new(),
            columns: Columns::default(),
            show_info_rows: true,
            hide_ignored_tab: true,
            scroll_rows: 2,
            show_tooltips: false,
            column_widths: Default::default(),
            window_rect: None,
            sort_key: "listeners".into(),
            sort_desc: true,
            favorites_first: false,
            window_size: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub yps: Vec<YpEntry>,
    pub peercast: PeerCastConfig,
    pub players: Vec<PlayerEntry>,
    pub auto_update: bool,
    pub update_interval_min: u32,
    pub fetch_on_start: bool,
    /// URL を開くブラウザ。空なら既定のブラウザ。
    pub browser: String,
    pub notify: NotifyConfig,
    pub tray: TrayConfig,
    pub view: ViewConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            yps: default_yps(),
            peercast: PeerCastConfig::default(),
            players: default_players(),
            auto_update: true,
            update_interval_min: DEFAULT_AUTO_INTERVAL_MIN,
            fetch_on_start: true,
            browser: String::new(),
            notify: NotifyConfig::default(),
            tray: TrayConfig::default(),
            view: ViewConfig::default(),
        }
    }
}

impl Config {
    pub fn update_interval_sec(&self) -> u64 {
        self.update_interval_min.max(MIN_AUTO_INTERVAL_MIN) as u64 * 60
    }
}

/// 設定を置くフォルダ (exe のあるフォルダ)。
pub fn base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 読み込んだ値と、利用者に知らせること (壊れたファイルを退避した、など)。
pub struct Loaded<T> {
    pub value: T,
    pub notices: Vec<String>,
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// 1 つ前に保存した内容の控え (`pcyp-rust.json.bak`)
pub fn backup_path(path: &Path) -> PathBuf {
    sibling(path, ".bak")
}

/// 読めなかったファイルを退避する先 (`pcyp-rust.json.broken-20260925-001637`)
fn broken_path(path: &Path) -> PathBuf {
    let p = sibling(path, &format!(".broken-{}", crate::win::now_stamp()));
    // 同じ秒に 2 回退避しても上書きしない
    (0..)
        .map(|i| if i == 0 { p.clone() } else { sibling(&p, &format!("-{}", i)) })
        .find(|p| !p.exists())
        .unwrap()
}

enum ReadResult<T> {
    Missing,
    Ok(T),
    Bad(String),
}

/// 読んで解釈する。ほかのソフト (ウイルス対策など) が一瞬つかんでいることがあるので、読めなければ少し待って読み直す。
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> ReadResult<T> {
    let mut last_err = String::new();
    for i in 0..3 {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let s = s.strip_prefix('\u{feff}').unwrap_or(&s);
                return match serde_json::from_str(s) {
                    Ok(v) => ReadResult::Ok(v),
                    Err(e) => ReadResult::Bad(format!("中身を読めません: {}", e)),
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ReadResult::Missing,
            Err(e) => last_err = format!("開けません: {}", e),
        }
    }
    ReadResult::Bad(last_err)
}

/// 設定のファイルを読む。
///
/// - 本体が読めなければ、消さずに `.broken-日時` の名前で退避し、`.bak` (1 つ前の保存) から読む
/// - `.bak` も読めなければ初期値を使う
/// - 本体がなくて `.bak` だけあれば (保存の途中で止まったとき)、`.bak` から読む
pub fn load_json_at<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Loaded<T> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut notices = Vec::new();
    match read_json::<T>(path) {
        ReadResult::Ok(v) => return Loaded { value: v, notices },
        ReadResult::Missing => {}
        ReadResult::Bad(e) => {
            let broken = broken_path(path);
            match std::fs::rename(path, &broken) {
                Ok(()) => notices.push(format!(
                    "{} を読めなかったので、{} という名前で残しました ({})",
                    name,
                    broken.file_name().unwrap_or_default().to_string_lossy(),
                    e
                )),
                Err(re) => notices.push(format!("{} を読めません ({})。退避もできませんでした: {}", name, e, re)),
            }
        }
    }
    let bak = backup_path(path);
    match read_json::<T>(&bak) {
        ReadResult::Ok(v) => {
            // notices が空なら、本体は読めなかったのではなく、なかった
            let why = if notices.is_empty() { "がなかったので" } else { "は" };
            notices.push(format!("{} {}、1 つ前に保存した控え ({}.bak) から読みました", name, why, name));
            Loaded { value: v, notices }
        }
        ReadResult::Missing => {
            if !notices.is_empty() {
                notices.push(format!("{} は初期値にしました", name));
            }
            Loaded { value: T::default(), notices }
        }
        ReadResult::Bad(e) => {
            notices.push(format!("{}.bak も読めないので ({})、{} は初期値にしました", name, e, name));
            Loaded { value: T::default(), notices }
        }
    }
}

pub fn load_json<T: for<'de> Deserialize<'de> + Default>(file: &str) -> Loaded<T> {
    load_json_at(&base_dir().join(file))
}

/// 保存する。
///
/// 1. 一時ファイルに書き、ディスクまで書き出す (停電などで中身が空にならないように)
/// 2. 今の本体を `.bak` にする (本体が正しく読めるときだけ。壊れた本体は `.broken-日時` に退避する)
/// 3. 一時ファイルを本体の名前にする
///
/// どの時点で止まっても、本体か `.bak` のどちらかに正しい内容が残る。
pub fn save_json_at<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write;
    let tmp = sibling(path, ".tmp");
    let s = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(s.as_bytes())?;
        f.sync_all()
    };
    write().map_err(|e| format!("{} に書けません: {}", tmp.display(), e))?;
    if path.exists() {
        let dest = match read_json::<serde_json::Value>(path) {
            ReadResult::Ok(_) => backup_path(path),
            _ => broken_path(path),
        };
        std::fs::rename(path, &dest).map_err(|e| format!("{} を {} にできません: {}", path.display(), dest.display(), e))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{} に書けません: {}", path.display(), e))
}

pub fn save_json<T: Serialize>(file: &str, value: &T) -> Result<(), String> {
    save_json_at(&base_dir().join(file), value)
}

/// YP の URL として受け付けるか (http と https だけ)。
pub fn valid_yp_url(url: &str) -> bool {
    let l = url.trim().to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && url.trim().len() > "https://".len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_json_uses_defaults() {
        let c: Config = serde_json::from_str(r#"{"auto_update": false, "peercast": {"address": "192.0.2.1:7145"}}"#).unwrap();
        assert!(!c.auto_update);
        assert_eq!(c.peercast.port(), 7145);
        assert_eq!(c.peercast.host(), "192.0.2.1");
        assert_eq!(c.peercast.url_kind, PlayUrlKind::Stream);
        assert_eq!(c.yps.len(), 5);
    }

    #[test]
    fn defaults() {
        let c = Config::default();
        let names: Vec<_> = c.yps.iter().map(|y| y.name.as_str()).collect();
        assert_eq!(names, ["SP", "平成", "P@", "YPv6", "Event YP"]);
        assert!(!c.view.show_tooltips);
        assert!(c.view.hide_ignored_tab);
        assert_eq!(c.view.scroll_rows, 2);
    }

    #[test]
    fn old_fields_are_ignored() {
        // 前の版の設定ファイル (種類、ホストとポートが別) も読める
        let c: Config =
            serde_json::from_str(r#"{"peercast": {"kind": "PeerCastStation", "host": "x", "port": 1, "address": "192.0.2.1:7145"}}"#)
                .unwrap();
        assert_eq!(c.peercast.address, "192.0.2.1:7145");
    }

    #[test]
    fn interval_has_minimum() {
        let c = Config { update_interval_min: 1, ..Default::default() };
        assert_eq!(c.update_interval_sec(), 120);
        assert_eq!(Config::default().update_interval_sec(), 300);
    }

    #[test]
    fn player_matches() {
        let p = PlayerEntry { types: "FLV, mkv".into(), ..Default::default() };
        assert!(p.matches("flv") && p.matches("MKV") && !p.matches("WMV"));
        assert!(PlayerEntry::default().matches("anything"));
    }

    #[test]
    fn parses_address() {
        assert_eq!(parse_address("192.0.2.1:60016"), Ok(("192.0.2.1".into(), 60016)));
        assert_eq!(parse_address(" localhost "), Ok(("localhost".into(), 7144)));
        assert_eq!(parse_address("http://pc.local:7145/"), Ok(("pc.local".into(), 7145)));
        assert_eq!(parse_address("[::1]:7145"), Ok(("::1".into(), 7145)));
        assert_eq!(parse_address("[::1]"), Ok(("::1".into(), 7144)));
        assert_eq!(parse_address("fe80::1"), Ok(("fe80::1".into(), 7144)));
        assert!(parse_address("").is_err());
        assert!(parse_address("host:0").is_err());
        assert!(parse_address("host:70000").is_err());
        assert!(parse_address("host:abc").is_err());
        assert!(parse_address("a/b:1").is_err());
        let pc = PeerCastConfig { address: "[::1]:7145".into(), ..Default::default() };
        assert_eq!(pc.base_url(), "http://[::1]:7145");
    }

    /// テスト用の空のフォルダ (終わったら消す)
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let d = std::env::temp_dir().join(format!("pcyp-rust-test-{}-{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            TempDir(d)
        }

        fn files(&self) -> Vec<String> {
            let mut v: Vec<String> =
                std::fs::read_dir(&self.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            v
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cfg_with_interval(n: u32) -> Config {
        Config { update_interval_min: n, ..Default::default() }
    }

    #[test]
    fn save_keeps_previous_as_backup() {
        let d = TempDir::new("backup");
        let p = d.0.join("c.json");
        save_json_at(&p, &cfg_with_interval(7)).unwrap();
        save_json_at(&p, &cfg_with_interval(9)).unwrap();
        assert_eq!(d.files(), ["c.json", "c.json.bak"]);
        let now: Loaded<Config> = load_json_at(&p);
        assert_eq!(now.value.update_interval_min, 9);
        assert!(now.notices.is_empty());
        let bak: Config = serde_json::from_str(&std::fs::read_to_string(backup_path(&p)).unwrap()).unwrap();
        assert_eq!(bak.update_interval_min, 7);
    }

    #[test]
    fn broken_file_is_kept_and_backup_is_used() {
        let d = TempDir::new("broken");
        let p = d.0.join("c.json");
        save_json_at(&p, &cfg_with_interval(7)).unwrap();
        save_json_at(&p, &cfg_with_interval(9)).unwrap();
        std::fs::write(&p, "{ 壊れた").unwrap();
        let l: Loaded<Config> = load_json_at(&p);
        // 控え (1 つ前の保存) から読む
        assert_eq!(l.value.update_interval_min, 7);
        assert_eq!(l.notices.len(), 2);
        // 壊れたファイルは消さずに残す
        let files = d.files();
        let broken: Vec<_> = files.iter().filter(|f| f.starts_with("c.json.broken-")).collect();
        assert_eq!(broken.len(), 1);
        assert_eq!(std::fs::read_to_string(d.0.join(broken[0])).unwrap(), "{ 壊れた");
        assert!(!p.exists());
    }

    #[test]
    fn empty_file_without_backup_uses_default() {
        let d = TempDir::new("empty");
        let p = d.0.join("c.json");
        std::fs::write(&p, "").unwrap();
        let l: Loaded<Config> = load_json_at(&p);
        assert_eq!(l.value, Config::default());
        assert!(l.notices.iter().any(|n| n.contains("初期値")));
        // 次の保存で、退避したファイルを上書きしない
        save_json_at(&p, &l.value).unwrap();
        let files = d.files();
        assert!(files.iter().any(|f| f.starts_with("c.json.broken-")));
        assert!(files.contains(&"c.json".to_string()));
    }

    #[test]
    fn missing_main_reads_backup() {
        let d = TempDir::new("missing");
        let p = d.0.join("c.json");
        save_json_at(&p, &cfg_with_interval(7)).unwrap();
        save_json_at(&p, &cfg_with_interval(9)).unwrap();
        std::fs::remove_file(&p).unwrap();
        let l: Loaded<Config> = load_json_at(&p);
        assert_eq!(l.value.update_interval_min, 7);
        assert!(l.notices[0].contains("なかった"));
    }

    #[test]
    fn first_run_is_quiet() {
        let d = TempDir::new("first");
        let l: Loaded<Config> = load_json_at(&d.0.join("c.json"));
        assert_eq!(l.value, Config::default());
        assert!(l.notices.is_empty());
    }

    #[test]
    fn saving_over_broken_file_moves_it_aside() {
        let d = TempDir::new("overbroken");
        let p = d.0.join("c.json");
        save_json_at(&p, &cfg_with_interval(7)).unwrap();
        save_json_at(&p, &cfg_with_interval(8)).unwrap();
        // 動いている間に、手で書き換えて壊した
        std::fs::write(&p, "not json").unwrap();
        save_json_at(&p, &cfg_with_interval(9)).unwrap();
        // 控えは壊れたものに置き換えない
        let bak: Config = serde_json::from_str(&std::fs::read_to_string(backup_path(&p)).unwrap()).unwrap();
        assert_eq!(bak.update_interval_min, 7);
        assert!(d.files().iter().any(|f| f.starts_with("c.json.broken-")));
        assert!(!d.files().iter().any(|f| f.ends_with(".tmp")));
    }

    #[test]
    fn yp_url_check() {
        assert!(valid_yp_url("http://a/index.txt"));
        assert!(!valid_yp_url("file:///c:/index.txt"));
        assert!(!valid_yp_url("http://"));
    }
}
