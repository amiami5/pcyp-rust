//! YP の index.txt を HTTP で取ってくる。

use std::time::Duration;

pub const USER_AGENT: &str = concat!("YPBrowser/", env!("CARGO_PKG_VERSION"), " (pcyp-rust)");
/// 本体の上限
pub const MAX_BODY: u64 = 32 * 1024 * 1024;

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .max_redirects(1)
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

/// index.txt を取ってきて、文字列にして返す。200 以外は失敗。
pub fn fetch_index(feed_url: &str, peercast_port: u16) -> Result<String, String> {
    let url = request_url(feed_url, peercast_port);
    let mut resp = agent()
        .get(&url)
        .header("Connection", "close")
        .call()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.as_u16() != 200 {
        return Err(format!("HTTP {}", status));
    }
    let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string())?;
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
