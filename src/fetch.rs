//! YP の index.txt を HTTP で取ってくる。

use std::time::Duration;

pub const USER_AGENT: &str = concat!("YPBrowser/", env!("CARGO_PKG_VERSION"), " (pcyp-rust)");
/// 本体の上限 (gzip などを展開した後の大きさ)
pub const MAX_BODY: u64 = 4 * 1024 * 1024;

/// 転送には自動で付いていかない。付いていくと、YP がローカルの PeerCast
/// (`http://127.0.0.1:7144/admin?...`) などへ要求を打たせられるため。
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(USER_AGENT)
        .build();
    ureq::Agent::new_with_config(config)
}

/// `feed_url` に `host=localhost:<port>` を付けたもの。
pub fn request_url(feed_url: &str, peercast_port: u16) -> String {
    let sep = if feed_url.contains('?') { '&' } else { '?' };
    format!("{}{}host=localhost%3A{}", feed_url, sep, peercast_port)
}

/// UTF-8 として読めなければ Shift_JIS として読む。
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned(),
    }
}

/// 本文を `max` バイトまで読む。超えたら失敗。
/// `limit()` は圧縮されたままの大きさにしかかからないので、展開した後の大きさでも打ち切る。
pub fn read_body(resp: &mut ureq::http::Response<ureq::Body>, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut buf = Vec::new();
    resp.body_mut()
        .with_config()
        .limit(max)
        .reader()
        .take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > max {
        return Err(format!("応答が大きすぎます ({} バイトまで)", max));
    }
    Ok(buf)
}

/// [`read_body`] を UTF-8 の文字列として読む。
pub fn read_body_string(resp: &mut ureq::http::Response<ureq::Body>, max: u64) -> Result<String, String> {
    String::from_utf8(read_body(resp, max)?).map_err(|e| e.to_string())
}

fn get(agent: &ureq::Agent, url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    agent.get(url).header("Connection", "close").call().map_err(|e| e.to_string())
}

/// `Location` を `base` からの絶対 URL にする。
fn resolve_location(base: &str, loc: &str) -> Option<String> {
    if loc.contains("://") {
        return Some(loc.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(l) = loc.strip_prefix("//") {
        return Some(format!("{}://{}", scheme, l));
    }
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let origin = &base[..scheme.len() + 3 + auth_end];
    if loc.starts_with('/') {
        return Some(format!("{}{}", origin, loc));
    }
    let path = &rest[auth_end..];
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    let dir = if dir.is_empty() { "/" } else { dir };
    if loc.starts_with('?') {
        Some(format!("{}{}{}", origin, path, loc))
    } else {
        Some(format!("{}{}{}", origin, dir, loc))
    }
}

/// 取り直してよい転送先か。同じホスト・同じポートだけ (http→https で既定ポート同士は可)。
fn same_origin_redirect(from: &str, to: &str) -> bool {
    let (Ok(a), Ok(b)) = (from.parse::<ureq::http::Uri>(), to.parse::<ureq::http::Uri>()) else {
        return false;
    };
    let (Some(sa), Some(sb)) = (a.scheme_str(), b.scheme_str()) else {
        return false;
    };
    let (sa, sb) = (sa.to_ascii_lowercase(), sb.to_ascii_lowercase());
    let (Some(ha), Some(hb)) = (a.host(), b.host()) else {
        return false;
    };
    if !ha.eq_ignore_ascii_case(hb) {
        return false;
    }
    let default_port = |s: &str| match s {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    let (Some(da), Some(db)) = (default_port(&sa), default_port(&sb)) else {
        return false;
    };
    let (pa, pb) = (a.port_u16().unwrap_or(da), b.port_u16().unwrap_or(db));
    if sa == sb {
        pa == pb
    } else {
        sa == "http" && sb == "https" && pa == da && pb == db
    }
}

/// index.txt を取ってきて、文字列にして返す。200 以外は失敗。
/// 転送は、同じホスト・同じポートへのものだけ 1 回取り直す。
pub fn fetch_index(feed_url: &str, peercast_port: u16) -> Result<String, String> {
    let url = request_url(feed_url, peercast_port);
    let agent = agent();
    let mut resp = get(&agent, &url)?;
    if resp.status().is_redirection() {
        let loc = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| format!("HTTP {} (転送先がありません)", resp.status()))?;
        let next = resolve_location(&url, loc.trim()).ok_or_else(|| format!("転送先が読めません: {}", loc))?;
        if !same_origin_redirect(&url, &next) {
            return Err(format!("別のホストへの転送は取りに行きません: {}", next));
        }
        resp = get(&agent, &next)?;
    }
    let status = resp.status();
    if status.as_u16() != 200 {
        return Err(format!("HTTP {}", status));
    }
    let body = read_body(&mut resp, MAX_BODY)?;
    Ok(decode_text(&body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// 1 回だけ応答するローカルの HTTP サーバー。受け取った要求の 1 行目を返す。
    fn serve_once(response: Vec<u8>) -> (u16, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            r.read_line(&mut first).unwrap();
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            s.write_all(&response).unwrap();
            first
        });
        (port, h)
    }

    #[test]
    fn fetches_chunked_body() {
        let body = "a<>b\n";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            body.len(),
            body
        );
        let (port, h) = serve_once(resp.into_bytes());
        let text = fetch_index(&format!("http://127.0.0.1:{}/sp/index.txt", port), 7144).unwrap();
        assert_eq!(text, body);
        assert_eq!(h.join().unwrap(), "GET /sp/index.txt?host=localhost%3A7144 HTTP/1.1\r\n");
    }

    #[test]
    fn non_200_is_error() {
        let (port, h) = serve_once(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
        assert!(fetch_index(&format!("http://127.0.0.1:{}/index.txt", port), 7144).is_err());
        h.join().unwrap();
    }

    #[test]
    fn follows_same_host_redirect_once() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let mut firsts = Vec::new();
            for resp in [
                "HTTP/1.1 302 Found\r\nLocation: /new/index.txt\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\na<>b\n",
            ] {
                let (mut s, _) = l.accept().unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut first = String::new();
                r.read_line(&mut first).unwrap();
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                s.write_all(resp.as_bytes()).unwrap();
                firsts.push(first);
            }
            firsts
        });
        let text = fetch_index(&format!("http://127.0.0.1:{}/index.txt", port), 7144).unwrap();
        assert_eq!(text, "a<>b\n");
        assert_eq!(h.join().unwrap()[1], "GET /new/index.txt HTTP/1.1\r\n");
    }

    #[test]
    fn refuses_redirect_to_other_port() {
        // 転送先 (PeerCast のつもり) に要求が届かないことを確かめる
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let tport = target.local_addr().unwrap().port();
        let resp = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/admin?cmd=stop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            tport
        );
        let (port, h) = serve_once(resp.into_bytes());
        let err = fetch_index(&format!("http://127.0.0.1:{}/index.txt", port), tport).unwrap_err();
        assert!(err.contains("転送"), "{}", err);
        h.join().unwrap();
        assert!(target.accept().is_err());
    }

    #[test]
    fn stops_gzip_bomb_after_decompression() {
        use flate2::{Compression, write::GzEncoder};
        let mut gz = GzEncoder::new(Vec::new(), Compression::best());
        gz.write_all(&vec![b'\n'; (MAX_BODY * 4) as usize]).unwrap();
        let gz = gz.finish().unwrap();
        assert!((gz.len() as u64) < MAX_BODY);
        let mut resp = format!(
            "HTTP/1.1 200 OK
Content-Encoding: gzip
Content-Length: {}
Connection: close

",
            gz.len()
        )
        .into_bytes();
        resp.extend_from_slice(&gz);
        let (port, h) = serve_once(resp);
        let err = fetch_index(&format!("http://127.0.0.1:{}/index.txt", port), 7144).unwrap_err();
        assert!(err.contains("大きすぎ"), "{}", err);
        h.join().unwrap();
    }

    #[test]
    fn rejects_too_large_body() {
        let n = MAX_BODY as usize + 1;
        let mut resp = format!("HTTP/1.1 200 OK
Content-Length: {}
Connection: close

", n).into_bytes();
        resp.extend(std::iter::repeat_n(b'\n', n));
        let (port, h) = serve_once(resp);
        assert!(fetch_index(&format!("http://127.0.0.1:{}/index.txt", port), 7144).is_err());
        h.join().unwrap();
    }

    #[test]
    fn redirect_rules() {
        assert!(same_origin_redirect("http://yp.example/i.txt", "http://YP.example/j.txt"));
        assert!(same_origin_redirect("http://yp.example/i.txt", "https://yp.example/i.txt"));
        assert!(same_origin_redirect("http://yp.example:8080/i", "http://yp.example:8080/j"));
        assert!(!same_origin_redirect("http://yp.example/i", "http://127.0.0.1:7144/admin"));
        assert!(!same_origin_redirect("http://yp.example/i", "http://yp.example:7144/admin"));
        assert!(!same_origin_redirect("https://yp.example/i", "http://yp.example/i"));
        assert!(!same_origin_redirect("http://yp.example:8080/i", "https://yp.example/i"));
        assert!(!same_origin_redirect("http://yp.example/i", "http://yp.example.evil/i"));
        assert!(!same_origin_redirect("http://yp.example/i", "http://yp.example@127.0.0.1/i"));
        assert!(!same_origin_redirect("http://yp.example/i", "file:///etc/passwd"));
    }

    #[test]
    fn resolves_location() {
        let b = "http://a:81/sp/index.txt?host=x";
        assert_eq!(resolve_location(b, "/x").unwrap(), "http://a:81/x");
        assert_eq!(resolve_location(b, "y.txt").unwrap(), "http://a:81/sp/y.txt");
        assert_eq!(resolve_location(b, "?q=1").unwrap(), "http://a:81/sp/index.txt?q=1");
        assert_eq!(resolve_location(b, "//b/z").unwrap(), "http://b/z");
        assert_eq!(resolve_location(b, "https://c/").unwrap(), "https://c/");
    }

    #[test]
    fn decodes_sjis() {
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("予定地");
        assert_eq!(decode_text(&sjis), "予定地");
        assert_eq!(decode_text("\u{feff}abc".as_bytes()), "abc");
    }

    #[test]
    fn host_param() {
        assert_eq!(request_url("http://a/index.txt", 7144), "http://a/index.txt?host=localhost%3A7144");
        assert_eq!(request_url("http://a/i?x=1", 7145), "http://a/i?x=1&host=localhost%3A7145");
    }
}
