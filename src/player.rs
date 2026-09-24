//! 再生用の URL を作り、プレイヤーやブラウザを起動する。

use crate::chandir::{self, Channel};
use crate::config::{Config, PeerCastConfig, PlayUrlKind};
use crate::peercast::host_for_url;
use std::path::PathBuf;
use std::process::Command;

fn base(pc: &PeerCastConfig) -> String {
    format!("http://{}:{}", host_for_url(&pc.host), pc.port)
}

fn tip_query(c: &Channel) -> String {
    if c.tip.is_empty() { String::new() } else { format!("?tip={}", chandir::url_encode(&c.tip)) }
}

/// `http://127.0.0.1:7144/stream/<ID><ext>?tip=<tip>`
pub fn stream_url(pc: &PeerCastConfig, c: &Channel) -> String {
    format!("{}/stream/{}{}{}", base(pc), c.id, chandir::type_ext(&c.content_type), tip_query(c))
}

/// `http://127.0.0.1:7144/pls/<ID>?tip=<tip>`
pub fn playlist_url(pc: &PeerCastConfig, c: &Channel) -> String {
    format!("{}/pls/{}{}", base(pc), c.id, tip_query(c))
}

pub fn play_url(pc: &PeerCastConfig, c: &Channel) -> String {
    match pc.url_kind {
        PlayUrlKind::Stream => stream_url(pc, c),
        PlayUrlKind::Playlist => playlist_url(pc, c),
    }
}

/// 引数の雛形を、Windows のコマンドラインと同じ規則 (空白で区切る、`"` で囲む) で分ける。
pub fn split_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut has_token = false;
    for ch in s.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quote => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(cur);
    }
    args
}

/// 雛形で使える置き換え。pcyplite 形式のタグも受け付ける。
pub const PLACEHOLDERS: &[(&str, &str)] = &[
    ("$URL", "再生用の URL (設定で /stream/ か /pls/)"),
    ("$STREAM", "/stream/ の URL"),
    ("$PLS", "/pls/ の URL"),
    ("$NAME", "チャンネル名"),
    ("$ID", "チャンネル ID"),
    ("$TIP", "トラッカー"),
    ("$CONTACT", "コンタクト URL"),
    ("$TYPE", "種類"),
    ("$GENRE", "ジャンル"),
    ("$DESC", "詳細"),
    ("$COMMENT", "コメント"),
    ("$BITRATE", "ビットレート"),
];

pub fn expand_args(template: &str, pc: &PeerCastConfig, c: &Channel) -> Vec<String> {
    let url = play_url(pc, c);
    let stream = stream_url(pc, c);
    let pls = playlist_url(pc, c);
    let bitrate = c.bitrate.to_string();
    // 長い名前から置き換える
    let table: [(&str, &str); 17] = [
        ("$CONTACT", &c.url),
        ("$COMMENT", &c.comment),
        ("$BITRATE", &bitrate),
        ("$STREAM", &stream),
        ("$GENRE", &c.genre),
        ("$NAME", &c.name),
        ("$TYPE", &c.content_type),
        ("$DESC", &c.desc),
        ("$URL", &url),
        ("$PLS", &pls),
        ("$TIP", &c.tip),
        ("$ID", &c.id),
        ("<stream/>", &url),
        ("<channelname/>", &c.name),
        ("<contact/>", &c.url),
        ("<id/>", &c.id),
        ("<tip/>", &c.tip),
    ];
    split_args(template)
        .into_iter()
        .map(|arg| {
            // 置き換えた値の中は、もう一度置き換えない
            let mut out = String::new();
            let mut rest = arg.as_str();
            'outer: while !rest.is_empty() {
                for (k, v) in &table {
                    if let Some(r) = rest.strip_prefix(k) {
                        out.push_str(v);
                        rest = r;
                        continue 'outer;
                    }
                }
                let ch = rest.chars().next().unwrap();
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
            out
        })
        .collect()
}

/// 相対パスなら、exe のフォルダからの位置として探す。なければそのまま (PATH から探す)。
pub fn resolve_exe(path: &str) -> PathBuf {
    let p = PathBuf::from(path.trim().trim_matches('"'));
    if p.is_relative() {
        let local = crate::config::base_dir().join(&p);
        if local.exists() {
            return local;
        }
    }
    p
}

/// チャンネルを再生する。起動したコマンドの説明を返す。
pub fn play(cfg: &Config, c: &Channel) -> Result<String, String> {
    if c.is_info() {
        return Err("このチャンネルは再生できません (ID がありません)".into());
    }
    let entry = cfg
        .players
        .iter()
        .find(|p| !p.exe.trim().is_empty() && p.matches(&c.content_type))
        .ok_or_else(|| format!("種類 {} のプレイヤーが設定されていません", c.content_type))?;
    let exe = resolve_exe(&entry.exe);
    let args = expand_args(&entry.args, &cfg.peercast, c);
    let mut cmd = Command::new(&exe);
    cmd.args(&args);
    if let Some(dir) = exe.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    // 終わるのを待たない
    cmd.spawn().map_err(|e| format!("{} を起動できません: {}", exe.display(), e))?;
    Ok(format!("{} {}", exe.display(), args.join(" ")))
}

/// http(s) の URL だけをブラウザで開く。
pub fn open_url(browser: &str, url: &str) -> Result<(), String> {
    if !chandir::is_http_url(url) {
        return Err(format!("http(s) でない URL は開きません: {}", url));
    }
    if !browser.trim().is_empty() {
        let exe = resolve_exe(browser);
        return Command::new(&exe).arg(url).spawn().map(|_| ()).map_err(|e| format!("{} を起動できません: {}", exe.display(), e));
    }
    shell_open(url)
}

#[cfg(windows)]
fn shell_open(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (op, file) = (wide("open"), wide(url));
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    if r as isize > 32 { Ok(()) } else { Err(format!("URL を開けません: {}", url)) }
}

#[cfg(not(windows))]
fn shell_open(url: &str) -> Result<(), String> {
    Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch() -> Channel {
        Channel {
            name: "予定地 \"x\"".into(),
            id: "97968780D09CC97BB98D4A2BF221EDE7".into(),
            tip: "127.0.0.1:7144".into(),
            content_type: "FLV".into(),
            url: "http://example.com/".into(),
            ..Default::default()
        }
    }

    #[test]
    fn urls() {
        let pc = PeerCastConfig::default();
        assert_eq!(
            stream_url(&pc, &ch()),
            "http://127.0.0.1:7144/stream/97968780D09CC97BB98D4A2BF221EDE7.flv?tip=127.0.0.1:7144"
        );
        assert_eq!(playlist_url(&pc, &ch()), "http://127.0.0.1:7144/pls/97968780D09CC97BB98D4A2BF221EDE7?tip=127.0.0.1:7144");
        let mut c = ch();
        c.tip.clear();
        c.content_type = "WMV2".into();
        assert_eq!(stream_url(&pc, &c), "http://127.0.0.1:7144/stream/97968780D09CC97BB98D4A2BF221EDE7");
    }

    #[test]
    fn splits_like_windows() {
        assert_eq!(split_args(r#"--title "$NAME" "$URL""#), ["--title", "$NAME", "$URL"]);
        assert_eq!(split_args(r#"a "" b"#), ["a", "", "b"]);
        assert_eq!(split_args(r#"--x="a b" c"#), ["--x=a b", "c"]);
    }

    #[test]
    fn expands_without_recursion() {
        let pc = PeerCastConfig::default();
        let mut c = ch();
        c.name = "$URL".into();
        let a = expand_args(r#"--force-media-title="$NAME" "<stream/>" $CONTACT"#, &pc, &c);
        assert_eq!(a[0], "--force-media-title=$URL");
        assert!(a[1].starts_with("http://127.0.0.1:7144/stream/"));
        assert_eq!(a[2], "http://example.com/");
    }

    #[test]
    fn refuses_non_http() {
        assert!(open_url("", "javascript:alert(1)").is_err());
        assert!(open_url("", "file:///C:/").is_err());
    }
}
