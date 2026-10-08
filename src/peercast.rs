//! ローカルの `PeerCast` (`PeerCast` YT / `PeerCastStation`) との連携。

use crate::config::PeerCastConfig;
use serde_json::{Value, json};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// `PeerCast` が待ち受けているか (TCP でつながるか)。
pub fn is_running(cfg: &PeerCastConfig) -> bool {
    let Ok(addrs) = (cfg.host().as_str(), cfg.port()).to_socket_addrs() else {
        return false;
    };
    addrs.into_iter().any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
}

/// ブラウザで開く管理画面の URL。
pub fn admin_url(cfg: &PeerCastConfig) -> String {
    format!("{}/", cfg.base_url())
}

/// `PeerCast` の exe を起動する (すでに動いていれば何もしない)。
pub fn launch(cfg: &PeerCastConfig) -> Result<bool, String> {
    if cfg.exe_path.trim().is_empty() || is_running(cfg) {
        return Ok(false);
    }
    let exe = crate::player::resolve_exe(&cfg.exe_path);
    let mut cmd = std::process::Command::new(&exe);
    if let Some(dir) = exe.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(|_| true).map_err(|e| format!("{} を起動できません: {}", exe.display(), e))
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// JSON-RPC 2.0 のクライアント (`/api/1`)。
pub struct Rpc {
    /// POST 用 (パスワードがあれば ?pass= 付き)
    url: String,
    /// 認証なしで聞く GET 用 (?pass= なし)
    get_url: String,
    auth: Option<String>,
    has_password: bool,
    agent: ureq::Agent,
}

impl Rpc {
    pub fn new(cfg: &PeerCastConfig) -> Rpc {
        // 転送には付いていかない (Authorization や ?pass= を別のところへ送らないため)
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .user_agent(crate::fetch::USER_AGENT)
            .build();
        let auth = (!cfg.user.is_empty() || !cfg.password.is_empty())
            .then(|| format!("Basic {}", base64(format!("{}:{}", cfg.user, cfg.password).as_bytes())));
        Rpc {
            // PeerCast YT は、Cookie 認証の設定だと Basic を受け付けないので ?pass= も付ける
            url: if cfg.password.is_empty() {
                format!("{}/api/1", cfg.base_url())
            } else {
                format!("{}/api/1?pass={}", cfg.base_url(), crate::chandir::url_encode(&cfg.password))
            },
            get_url: format!("{}/api/1", cfg.base_url()),
            auth,
            has_password: !cfg.password.is_empty(),
            agent: ureq::Agent::new_with_config(config),
        }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
        let mut req = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json")
            // PeerCastStation は、この見出しのない API 呼び出しを断る
            .header("X-Requested-With", "XMLHttpRequest");
        if let Some(a) = &self.auth {
            req = req.header("Authorization", a);
        }
        let mut resp = req.send(body).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            return Err(if self.has_password {
                format!("認証に失敗しました (HTTP {status})。ユーザー名とパスワードを確かめてください")
            } else {
                format!("PeerCast がログインを求めています (HTTP {status})。localhost 以外の PeerCast でこの操作をするには、PeerCast の管理画面のパスワードを設定の PeerCast タブに入れてください (一覧の取得と再生には要りません)")
            });
        }
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }
        let text = crate::fetch::read_body_string(&mut resp, 8 * 1024 * 1024)?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("応答を読めません: {e}"))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("不明なエラー");
            return Err(format!("{method}: {msg}"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    /// `PeerCast` YT は `GET /api/1` に認証なしで getVersionInfo の結果を返す (LAN からでも)。
    /// POST の JSON-RPC は localhost 以外からだとログインが要るので、まず GET で聞く。
    /// この GET にはパスワードを付けない (見張りで 10 秒ごとに呼ぶので)。答えなければ POST で聞き直す。
    fn get_version(&self) -> Result<Value, String> {
        let mut resp = self.agent.get(&self.get_url).call().map_err(|e| e.to_string())?;
        if resp.status().as_u16() != 200 {
            return Err(format!("HTTP {}", resp.status()));
        }
        let text = crate::fetch::read_body_string(&mut resp, 1024 * 1024)?;
        let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let v = v.get("result").cloned().unwrap_or(v);
        if v.get("agentName").is_none() {
            return Err("agentName がありません".into());
        }
        Ok(v)
    }

    pub fn version_info(&self) -> Result<VersionInfo, String> {
        let v = match self.get_version() {
            Ok(v) => v,
            Err(_) => self.call("getVersionInfo", json!([]))?,
        };
        let agent = agent_name(&v);
        Ok(VersionInfo { kind: detect_kind(&agent), agent })
    }

    pub fn channels(&self) -> Result<Vec<RelayChannel>, String> {
        let v = self.call("getChannels", json!([]))?;
        Ok(v.as_array().map(|a| a.iter().map(RelayChannel::from_json).collect()).unwrap_or_default())
    }

    pub fn stop_channel(&self, id: &str) -> Result<(), String> {
        self.call("stopChannel", json!({"channelId": id})).map(|_| ())
    }

    pub fn bump_channel(&self, id: &str) -> Result<(), String> {
        self.call("bumpChannel", json!({"channelId": id})).map(|_| ())
    }
}

fn agent_name(v: &Value) -> String {
    v.get("agentName").and_then(Value::as_str).unwrap_or("").to_string()
}

/// 見張りが 10 秒ごとに聞く、`PeerCast` の種類と版 (agentName)。
///
/// 毎回パスワードを送らないように、まずパスワードなしで聞く。それで答えない `PeerCast` (LAN の `PeerCastStation` など) には、
/// 動き始めてから 1 回だけパスワード付きで聞き、止まるか設定が変わるまでは、その答えを使う。
#[derive(Default)]
pub struct AgentProbe {
    /// パスワード付きで聞いたときの設定と、その答え
    asked: Option<(PeerCastConfig, String)>,
}

impl AgentProbe {
    /// 動いている `PeerCast` の agentName。わからなければ空。
    pub fn agent(&mut self, cfg: &PeerCastConfig) -> String {
        if let Some((c, agent)) = &self.asked
            && c == cfg
        {
            return agent.clone();
        }
        let anonymous = PeerCastConfig { user: String::new(), password: String::new(), ..cfg.clone() };
        if let Ok(v) = Rpc::new(&anonymous).version_info() {
            return v.agent;
        }
        if cfg.user.is_empty() && cfg.password.is_empty() {
            return String::new();
        }
        let agent = Rpc::new(cfg).call("getVersionInfo", json!([])).map(|v| agent_name(&v)).unwrap_or_default();
        self.asked = Some((cfg.clone(), agent.clone()));
        agent
    }

    /// `PeerCast` が止まったら呼ぶ。次に動いたときに、また聞く
    pub fn reset(&mut self) {
        self.asked = None;
    }
}

#[derive(Clone, Debug)]
pub struct VersionInfo {
    pub agent: String,
    pub kind: PeerCastKind,
}

/// 接続した `PeerCast` の種類 (getVersionInfo の agentName から見分ける)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerCastKind {
    /// `PeerCast` YT (C++ 版と Rust 版)
    PeerCastYt,
    PeerCastStation,
    Unknown,
}

impl PeerCastKind {
    pub fn label(self) -> &'static str {
        match self {
            PeerCastKind::PeerCastYt => "PeerCast YT",
            PeerCastKind::PeerCastStation => "PeerCastStation",
            PeerCastKind::Unknown => "種類は不明",
        }
    }
}

pub fn detect_kind(agent: &str) -> PeerCastKind {
    if agent.contains("PeerCastStation") {
        PeerCastKind::PeerCastStation
    } else if agent.contains("PeerCast") {
        PeerCastKind::PeerCastYt
    } else {
        PeerCastKind::Unknown
    }
}

/// `PeerCast` がつないでいるチャンネル (`getChannels` の 1 件)。
#[derive(Clone, Debug, Default)]
pub struct RelayChannel {
    pub id: String,
    pub name: String,
    pub status: String,
    pub listeners: i64,
    pub relays: i64,
    pub bitrate: i64,
    pub content_type: String,
}

fn get_str(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        match cur.get(p) {
            Some(n) => cur = n,
            None => return String::new(),
        }
    }
    match cur {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn get_i64(v: &Value, path: &[&str]) -> i64 {
    let mut cur = v;
    for p in path {
        match cur.get(p) {
            Some(n) => cur = n,
            None => return 0,
        }
    }
    cur.as_i64().unwrap_or(0)
}

impl RelayChannel {
    fn from_json(v: &Value) -> RelayChannel {
        RelayChannel {
            id: get_str(v, &["channelId"]).to_ascii_uppercase(),
            name: get_str(v, &["info", "name"]),
            status: get_str(v, &["status", "status"]),
            listeners: get_i64(v, &["status", "localDirects"]),
            relays: get_i64(v, &["status", "localRelays"]),
            bitrate: get_i64(v, &["info", "bitrate"]),
            content_type: get_str(v, &["info", "contentType"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_works() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    fn detects_kind() {
        assert_eq!(detect_kind("PeerCastStation/3.1.0"), PeerCastKind::PeerCastStation);
        assert_eq!(detect_kind("PeerCast/0.1218 (YT50-rs2)"), PeerCastKind::PeerCastYt);
    }

    #[test]
    fn parses_relay_channel() {
        let v: Value = serde_json::from_str(
            r#"{"channelId":"abc","info":{"name":"n","bitrate":500,"contentType":"FLV"},"status":{"status":"Receiving","localDirects":2,"localRelays":1}}"#,
        )
        .unwrap();
        let c = RelayChannel::from_json(&v);
        assert_eq!((c.id.as_str(), c.name.as_str(), c.status.as_str()), ("ABC", "n", "Receiving"));
        assert_eq!((c.listeners, c.relays, c.bitrate), (2, 1, 500));
    }

    /// 認証の付いていない要求には 401、付いている要求には getVersionInfo の答えを返すサーバー。
    /// 受け取った要求の 1 行目と、認証が付いていたかを残す
    fn auth_server() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut first = String::new();
                r.read_line(&mut first).unwrap();
                let (mut auth, mut len) = (false, 0);
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    let line = line.to_ascii_lowercase();
                    auth |= line.starts_with("authorization:");
                    if let Some(v) = line.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                // 本文を読み残して閉じると、応答が届かないことがある
                r.read_exact(&mut vec![0; len]).unwrap();
                seen.lock().unwrap().push(format!("{} auth={auth}", first.trim_end()));
                let resp = if auth {
                    let body = r#"{"jsonrpc":"2.0","id":1,"result":{"agentName":"PeerCastStation/3.1.0"}}"#;
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                } else {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                };
                s.write_all(resp.as_bytes()).unwrap();
            }
        });
        (port, log)
    }

    #[test]
    fn monitor_sends_password_once() {
        let (port, log) = auth_server();
        let cfg = PeerCastConfig { address: format!("127.0.0.1:{port}"), user: "u".into(), password: "p".into(), ..Default::default() };
        let mut probe = AgentProbe::default();
        // パスワードなしの GET と POST に答えないので、1 回だけパスワード付きで聞く
        assert_eq!(probe.agent(&cfg), "PeerCastStation/3.1.0");
        assert_eq!(
            *log.lock().unwrap(),
            ["GET /api/1 HTTP/1.1 auth=false", "POST /api/1 HTTP/1.1 auth=false", "POST /api/1?pass=p HTTP/1.1 auth=true"]
        );
        // 次からは聞かずに、その答えを使う
        assert_eq!(probe.agent(&cfg), "PeerCastStation/3.1.0");
        assert_eq!(log.lock().unwrap().len(), 3);
        // 止まってまた動いたときと、設定が変わったときは、また聞く
        probe.reset();
        assert_eq!(probe.agent(&cfg), "PeerCastStation/3.1.0");
        assert_eq!(log.lock().unwrap().len(), 6);
        probe.agent(&PeerCastConfig { password: "q".into(), ..cfg.clone() });
        assert_eq!(log.lock().unwrap().len(), 9);
        // パスワードがなければ、パスワードなしで聞くだけ
        assert_eq!(AgentProbe::default().agent(&PeerCastConfig { user: String::new(), password: String::new(), ..cfg }), "");
        assert!(log.lock().unwrap()[9..].iter().all(|l| l.ends_with("auth=false")));
    }

    /// 実際の `PeerCast` につなぐテスト。`PCYP_TEST_PEERCAST=ホスト:ポート cargo test -- --ignored` で動かす。
    #[test]
    #[ignore = "実際の PeerCast がいる"]
    fn live_peercast() {
        let Ok(address) = std::env::var("PCYP_TEST_PEERCAST") else {
            eprintln!("PCYP_TEST_PEERCAST が未設定なので飛ばします");
            return;
        };
        // パスワードは任意 (localhost 以外の PeerCast YT の getChannels などに要る)
        let password = std::env::var("PCYP_TEST_PEERCAST_PASS").unwrap_or_default();
        let cfg = PeerCastConfig { address, password: password.clone(), ..Default::default() };
        assert!(is_running(&cfg), "{} につながりません", cfg.base_url());
        let rpc = Rpc::new(&cfg);
        let v = rpc.version_info().expect("getVersionInfo");
        eprintln!("agent: {} ({:?})", v.agent, v.kind);
        assert_ne!(v.kind, PeerCastKind::Unknown);
        let chans = match rpc.channels() {
            Ok(c) => c,
            Err(e) if password.is_empty() => {
                eprintln!("getChannels (パスワードなし): {e}");
                return;
            }
            Err(e) => panic!("getChannels: {e}"),
        };
        eprintln!("channels: {}", chans.len());
        for c in &chans {
            eprintln!("  {} {} {} {}/{}", c.id, c.name, c.status, c.listeners, c.relays);
        }
        // ない ID の停止はエラーになるか、何もしないで終わる。どちらでも応答は返る
        let r = rpc.stop_channel("00000000000000000000000000000001");
        eprintln!("stopChannel(dummy): {r:?}");
    }
}
