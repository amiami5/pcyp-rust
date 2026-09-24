//! YP の index.txt の解析。

pub const NUM_FIELDS: usize = 19;
pub const ZERO_ID: &str = "00000000000000000000000000000000";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Channel {
    pub name: String,
    pub id: String,
    pub tip: String,
    pub url: String,
    pub genre: String,
    pub desc: String,
    pub listeners: i32,
    pub relays: i32,
    pub bitrate: i32,
    pub content_type: String,
    pub track_artist: String,
    pub track_album: String,
    pub track_title: String,
    pub track_contact: String,
    pub encoded_name: String,
    pub uptime: String,
    pub status: String,
    pub comment: String,
    pub direct: i32,
    pub feed_url: String,
}

impl Channel {
    /// ID が全部 0 の行 (YP からのお知らせなど)。再生できない。
    pub fn is_info(&self) -> bool {
        self.id == ZERO_ID
    }

    /// 配信時間を分にしたもの。読めなければ 0。
    pub fn uptime_minutes(&self) -> i32 {
        match self.uptime.split_once(':') {
            Some((h, m)) => atoi(h).saturating_mul(60).saturating_add(atoi(m)),
            None => atoi(&self.uptime).saturating_mul(60),
        }
    }

    /// 同じチャンネルかどうかを見分けるためのキー。
    pub fn key(&self) -> String {
        if self.is_info() { format!("name:{}", self.name) } else { self.id.clone() }
    }

    pub fn chat_url(&self) -> String {
        side_url(&self.feed_url, "chat.php", &self.encoded_name)
    }

    pub fn stat_url(&self) -> String {
        side_url(&self.feed_url, "getgmt.php", &self.encoded_name)
    }

    /// 「[ジャンル - 詳細] コメント」の形の説明。
    pub fn summary(&self) -> String {
        let mut s = String::new();
        match (self.genre.is_empty(), self.desc.is_empty()) {
            (false, false) => s.push_str(&format!("[{} - {}]", self.genre, self.desc)),
            (false, true) => s.push_str(&format!("[{}]", self.genre)),
            (true, false) => s.push_str(&format!("[{}]", self.desc)),
            (true, true) => {}
        }
        if !self.comment.is_empty() {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(&format!("「{}」", self.comment));
        }
        s
    }

    pub fn track_text(&self) -> String {
        [&self.track_artist, &self.track_title, &self.track_album]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" - ")
    }
}

/// C の `atoi` と同じく、先頭の数字だけを読む。読めなければ 0。
pub fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i32, |a, d| a.wrapping_mul(10).wrapping_add((d - b'0') as i32));
    if neg { n.wrapping_neg() } else { n }
}

/// http(s) の URL ならそのまま、それ以外は空にする。
pub fn http_url_or_empty(s: &str) -> String {
    if is_http_url(s) { s.to_string() } else { String::new() }
}

pub fn is_http_url(s: &str) -> bool {
    let l = s.trim_start().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// 32 桁の 16 進数なら大文字にして返す。それ以外は全部 0。
fn normalize_id(s: &str) -> String {
    if s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()) {
        s.to_ascii_uppercase()
    } else {
        ZERO_ID.to_string()
    }
}

/// 解析できた行と、解析できなかった行の番号 (1 から数える)。
pub fn parse_index(text: &str, feed_url: &str) -> (Vec<Channel>, Vec<usize>) {
    let (mut chans, mut errors) = (Vec::new(), Vec::new());
    if text.is_empty() {
        return (chans, errors);
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    for (i, line) in body.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let f: Vec<&str> = line.split("<>").collect();
        if f.len() != NUM_FIELDS {
            errors.push(i + 1);
            continue;
        }
        chans.push(Channel {
            name: unescape_html(f[0]),
            id: normalize_id(f[1]),
            tip: f[2].to_string(),
            url: http_url_or_empty(f[3]),
            genre: unescape_html(f[4]),
            desc: unescape_html(f[5]),
            listeners: atoi(f[6]),
            relays: atoi(f[7]),
            bitrate: atoi(f[8]),
            content_type: f[9].to_string(),
            track_artist: unescape_html(f[10]),
            track_album: unescape_html(f[11]),
            track_title: unescape_html(f[12]),
            track_contact: http_url_or_empty(f[13]),
            encoded_name: f[14].to_string(),
            uptime: f[15].to_string(),
            status: f[16].to_string(),
            comment: unescape_html(f[17]),
            direct: atoi(f[18]),
            feed_url: feed_url.to_string(),
        });
    }
    // 聴取者の多い順 (同じ数なら元の順)
    chans.sort_by_key(|c| std::cmp::Reverse(c.listeners));
    (chans, errors)
}

/// `feed_url` の最後の `/` の手前に `<file>?cn=<encoded name>` を付けた URL。
pub fn side_url(feed_url: &str, file: &str, encoded_name: &str) -> String {
    if encoded_name.is_empty() {
        return String::new();
    }
    match feed_url.rfind('/') {
        Some(i) if is_http_url(feed_url) => format!("{}/{}?cn={}", &feed_url[..i], file, encoded_name),
        _ => String::new(),
    }
}

/// コンテンツの種類から、ストリームの URL に付ける拡張子。
pub fn type_ext(content_type: &str) -> &'static str {
    match content_type.to_ascii_uppercase().as_str() {
        "FLV" => ".flv",
        "MKV" => ".mkv",
        "WEBM" => ".webm",
        "MP4" => ".mp4",
        "MP3" => ".mp3",
        "OGG" | "OGM" => ".ogg",
        "MOV" => ".mov",
        "WMV" => ".wmv",
        "WMA" => ".wma",
        "ASX" => ".asx",
        _ => "",
    }
}

/// HTML の文字参照を一度だけ戻す。知らない参照はそのまま残す。
pub fn unescape_html(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..].find(';').filter(|&n| n <= 10).and_then(|n| {
            let ent = &rest[1..1 + n];
            decode_entity(ent).map(|c| (c, n + 2))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(ent: &str) -> Option<char> {
    if let Some(num) = ent.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse::<u32>().ok()?,
        };
        return char::from_u32(code).filter(|&c| c != '\0');
    }
    Some(match ent {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "copy" => '©',
        "reg" => '®',
        "hellip" => '…',
        "yen" => '¥',
        _ => return None,
    })
}

/// URL のクエリの値として使えるように、英数字と一部の記号以外を %XX にする。
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'[' | b']' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "予定地<>97968780D09CC97BB98D4A2BF221EDE7<>127.0.0.1:7144<>http://www.example.com/<>プログラミング<>peercastをいじる - &lt;Free&gt;<>-1<>-1<>428<>FLV<><><><><>%E4%BA%88%E5%AE%9A%E5%9C%B0<>1:14<>click<><>1";

    #[test]
    fn parses_fixed_line() {
        let (c, e) = parse_index(LINE, "http://yp/sp/index.txt");
        assert!(e.is_empty());
        assert_eq!(c.len(), 1);
        let c = &c[0];
        assert_eq!(c.name, "予定地");
        assert_eq!(c.id, "97968780D09CC97BB98D4A2BF221EDE7");
        assert_eq!(c.tip, "127.0.0.1:7144");
        assert_eq!(c.url, "http://www.example.com/");
        assert_eq!(c.desc, "peercastをいじる - <Free>");
        assert_eq!((c.listeners, c.relays, c.bitrate), (-1, -1, 428));
        assert_eq!(c.content_type, "FLV");
        assert_eq!(c.uptime_minutes(), 74);
        assert_eq!(c.direct, 1);
        assert_eq!(c.chat_url(), "http://yp/sp/chat.php?cn=%E4%BA%88%E5%AE%9A%E5%9C%B0");
        assert_eq!(c.stat_url(), "http://yp/sp/getgmt.php?cn=%E4%BA%88%E5%AE%9A%E5%9C%B0");
    }

    #[test]
    fn drops_non_http_urls_and_counts_lines() {
        let bad = LINE.replace("http://www.example.com/", "javascript:alert(1)");
        let text = format!("{}\r\nbroken<>line\n{}\nfile<>x\n", LINE, bad);
        let (c, e) = parse_index(&text, "http://yp/index.txt");
        assert_eq!(c.len(), 2);
        assert_eq!(e, vec![2, 4]);
        assert!(c.iter().any(|c| c.url.is_empty()));
        let file = LINE.replace("http://www.example.com/", "file:///C:/Windows");
        let (c, _) = parse_index(&file, "");
        assert_eq!(c[0].url, "");
    }

    #[test]
    fn empty_and_no_trailing_newline() {
        assert_eq!(parse_index("", "").0.len(), 0);
        assert_eq!(parse_index(LINE, "").0.len(), 1);
        assert_eq!(parse_index(&format!("{}\n", LINE), "").1.len(), 0);
    }

    #[test]
    fn invalid_id_becomes_zero() {
        let l = LINE.replace("97968780D09CC97BB98D4A2BF221EDE7", "xyz");
        let (c, _) = parse_index(&l, "");
        assert!(c[0].is_info());
        let l = LINE.replace("97968780D09CC97BB98D4A2BF221EDE7", "97968780d09cc97bb98d4a2bf221ede7");
        assert_eq!(parse_index(&l, "").0[0].id, "97968780D09CC97BB98D4A2BF221EDE7");
    }

    #[test]
    fn sorts_by_listeners() {
        let a = LINE.replace("<>-1<>-1<>428", "<>5<>0<>428").replace("予定地", "A");
        let b = LINE.replace("<>-1<>-1<>428", "<>20<>0<>428").replace("予定地", "B");
        let c = LINE.replace("<>-1<>-1<>428", "<>5<>0<>428").replace("予定地", "C");
        let (ch, _) = parse_index(&format!("{a}\n{b}\n{c}\n"), "");
        let names: Vec<_> = ch.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["B", "A", "C"]);
    }

    #[test]
    fn atoi_like_c() {
        assert_eq!(atoi("12abc"), 12);
        assert_eq!(atoi("  -3"), -3);
        assert_eq!(atoi("x"), 0);
        assert_eq!(atoi(""), 0);
    }

    #[test]
    fn unescape() {
        assert_eq!(unescape_html("&lt;a&gt; &amp;amp; &quot;&#39;&#x41;&#12354;"), "<a> &amp; \"'Aあ");
        assert_eq!(unescape_html("A & B &unknown; &"), "A & B &unknown; &");
    }

    #[test]
    fn side_url_rules() {
        assert_eq!(side_url("http://yp/sp/index.txt", "chat.php", "abc"), "http://yp/sp/chat.php?cn=abc");
        assert_eq!(side_url("http://yp/sp/index.txt", "chat.php", ""), "");
    }

    #[test]
    fn encodes_tip() {
        assert_eq!(url_encode("1.2.3.4:7144"), "1.2.3.4:7144");
        assert_eq!(url_encode("[::1]:7144"), "[::1]:7144");
        assert_eq!(url_encode("a b&c"), "a%20b%26c");
    }
}
