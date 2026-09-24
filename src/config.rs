//! 設定。exe と同じフォルダに JSON で置く (ポータブル版)。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP_NAME: &str = "pcyp-rust";
pub const CONFIG_FILE: &str = "pcyp-rust.json";
pub const FILTER_FILE: &str = "channelFilters.json";

/// 自動更新の間隔の最小 (分)。YP サーバーに負担をかけないため。
pub const MIN_AUTO_INTERVAL_MIN: u32 = 5;
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
        YpEntry::new("Heisei", "http://yp.pcgw.pgw.jp/index.txt"),
        YpEntry::new("P@", "https://p-at.net/index.txt"),
        YpEntry::new("YPv6", "http://ypv6.pecastation.org/index.txt"),
        YpEntry::new("Event YP", "http://eventyp.xrea.jp/index.txt"),
    ]
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PeerCastKind {
    /// 起動中の PeerCast から自動で判別する
    Auto,
    /// PeerCast YT (C++ 版と Rust 版)
    PeerCastYt,
    PeerCastStation,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlayUrlKind {
    /// `/stream/<ID><ext>?tip=`
    Stream,
    /// `/pls/<ID>?tip=`
    Playlist,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PeerCastConfig {
    pub kind: PeerCastKind,
    /// `ホスト:ポート` (例 `127.0.0.1:7144`、`[::1]:7144`)。ポートを省くと 7144。
    pub address: String,
    /// JSON-RPC 用。空なら認証しない。
    pub user: String,
    pub password: String,
    /// PeerCast 本体の exe。空なら起動しない。
    pub exe_path: String,
    pub launch_on_start: bool,
    pub url_kind: PlayUrlKind,
}

impl Default for PeerCastConfig {
    fn default() -> Self {
        PeerCastConfig {
            kind: PeerCastKind::Auto,
            address: format!("127.0.0.1:{}", DEFAULT_PEERCAST_PORT),
            user: String::new(),
            password: String::new(),
            exe_path: String::new(),
            launch_on_start: false,
            url_kind: PlayUrlKind::Stream,
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
            update_interval_min: MIN_AUTO_INTERVAL_MIN,
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

pub fn load_json<T: for<'de> Deserialize<'de> + Default>(file: &str) -> Result<T, String> {
    let path = base_dir().join(file);
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{} の読み込みに失敗: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{} を開けません: {}", path.display(), e)),
    }
}

/// 一時ファイルに書いてから置き換える。
pub fn save_json<T: Serialize>(file: &str, value: &T) -> Result<(), String> {
    let path = base_dir().join(file);
    let tmp = path.with_extension("json.tmp");
    let s = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, s).map_err(|e| format!("{} に書けません: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{} に書けません: {}", path.display(), e))
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
    fn interval_has_minimum() {
        let c = Config { update_interval_min: 1, ..Default::default() };
        assert_eq!(c.update_interval_sec(), 300);
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

    #[test]
    fn yp_url_check() {
        assert!(valid_yp_url("http://a/index.txt"));
        assert!(!valid_yp_url("file:///c:/index.txt"));
        assert!(!valid_yp_url("http://"));
    }
}
