//! pcyplite の設定 (`pcypLite.ini` と `Favorite.ini`) から、YP、プレイヤー、お気に入りを取り込む。
//!
//! pcyplite は Delphi 製で、ini は UTF-8 (BOM 付き) か `Shift_JIS`。
//!
//! `Favorite.ini` の `Flags` は、お気に入りの画面のチェックをビットにしたもの。
//! 画面の「チャンネル名(1)」〜「ビットレート(9)」の番号が、そのままビットの位置になっている。
//!
//! | ビット | 意味 |
//! |---|---|
//! | 0 | 有効 (一覧のチェック) |
//! | 1〜4 | 探す対象: チャンネル名 / 詳細 + Playing / コメント / コンタクト URL |
//! | 5〜9 | 探す対象: YP 名 / ストリーム URL / 種別 / リスナー / ビットレート |
//! | 10 | リストに表示しない (無視) |
//! | 11 | 通知しない |
//! | 12 | お気に入りフィルタに追加する |
//! | 13 | 検索文字に正規表現を使う |

use crate::config::{PlayerEntry, YpEntry};
use crate::filter::{Filter, Search};
use std::collections::HashMap;
use std::path::Path;

pub const MAIN_INI: &str = "pcypLite.ini";
pub const FAVORITE_INI: &str = "Favorite.ini";

/// ini の 1 つのセクション (キーの大文字と小文字は区別しない)
#[derive(Default, Debug)]
pub struct Section(HashMap<String, String>);

impl Section {
    pub fn get(&self, key: &str) -> &str {
        self.0.get(&key.to_ascii_lowercase()).map_or("", String::as_str)
    }
}

/// ini を読む。セクションは出てきた順に並べる。
pub fn parse_ini(text: &str) -> Vec<(String, Section)> {
    let mut out: Vec<(String, Section)> = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push((name.to_string(), Section::default()));
        } else if let (Some((k, v)), Some(last)) = (line.split_once('='), out.last_mut()) {
            last.1 .0.insert(k.trim().to_ascii_lowercase(), v.to_string());
        }
    }
    out
}

fn section<'a>(ini: &'a [(String, Section)], name: &str) -> Option<&'a Section> {
    ini.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, s)| s)
}

/// 取り込む候補 1 つ。`checked` は一覧のチェック (取り込むかどうか)
#[derive(Clone, Debug)]
pub struct Item<T> {
    pub value: T,
    pub checked: bool,
    /// 一覧の「メモ」の欄に出すこと (同じものがある、など)
    pub note: String,
}

impl<T> Item<T> {
    fn new(value: T) -> Self {
        Item { value, checked: true, note: String::new() }
    }

    fn skip(mut self, note: impl Into<String>) -> Self {
        self.checked = false;
        self.note = note.into();
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub filters: Vec<Item<Filter>>,
    pub yps: Vec<Item<YpEntry>>,
    pub players: Vec<Item<PlayerEntry>>,
    /// `PeerCast` のアドレス (YP の `Host=`)
    pub peercast: Option<Item<String>>,
    /// URL を開くブラウザ
    pub browser: Option<Item<String>>,
    /// 読めなかったファイルなど
    pub warnings: Vec<String>,
}

/// Delphi の `TColor` (`0x00BBGGRR`)。負の数はシステムの色 (窓の背景など) なので色なしとみなす。
fn delphi_color(s: &str) -> Option<[u8; 3]> {
    let n: i64 = s.trim().parse().ok()?;
    if !(0..=0xFF_FFFF).contains(&n) {
        return None;
    }
    Some([(n & 0xFF) as u8, (n >> 8 & 0xFF) as u8, (n >> 16 & 0xFF) as u8])
}

/// 探す対象のビット (1〜9) と、pcyp-rust の項目。リスナーとビットレートは文字で探せないので無い
const FIELD_BITS: &[(u32, &[&str], &str)] = &[
    (1, &["name"], "チャンネル名"),
    (2, &["genre", "desc"], "詳細"),
    (3, &["comment"], "コメント"),
    (4, &["url"], "コンタクトURL"),
    (5, &["yp"], "YP名"),
    (6, &["id"], "ストリームURL"),
    (7, &["type"], "種別"),
    (8, &[], "リスナー"),
    (9, &[], "ビットレート"),
];

/// `Favorite.ini` の 1 項目をフィルターにする。取り込めない条件はメモに書く。
pub fn favorite_to_filter(sec: &Section) -> Item<Filter> {
    let title = sec.get("Title").trim().to_string();
    let word = sec.get("Word").trim();
    let flags: u32 = sec.get("Flags").trim().parse().unwrap_or(0x3003);
    let bit = |n: u32| flags & (1 << n) != 0;
    let regex = bit(13);
    // 検索文字が空なら、名前で探す
    let src = if word.is_empty() { title.as_str() } else { word };
    let search = if regex { src.to_string() } else { regex::escape(src) };

    let mut fields: Vec<String> = Vec::new();
    let mut lost = Vec::new();
    for &(b, names, label) in FIELD_BITS {
        if bit(b) {
            if names.is_empty() {
                lost.push(label);
            }
            fields.extend(names.iter().map(std::string::ToString::to_string));
        }
    }
    if fields.is_empty() {
        fields.push("name".into());
    }
    let ignore = bit(10);
    let favorite = bit(12) && !ignore;
    let color = delphi_color(sec.get("Color"));
    let f = Filter {
        name: title.clone(),
        enabled: bit(0),
        favorite,
        ignore,
        notify: favorite && !bit(11),
        // pcyplite は色を決めていなければ名前を赤くするだけなので、お気に入りには初期値の色を付ける
        enable_color: color.is_some() || favorite,
        color: color.unwrap_or(Filter::default().color),
        // pcyplite は大文字と小文字を区別する (MORI|mori のように両方書く)
        ignore_case: false,
        base_search: Search { enabled: true, search, fields },
        ..Default::default()
    };
    let mut item = Item::new(f);
    let mut notes = Vec::new();
    if !lost.is_empty() {
        notes.push(format!("{} では探せません", lost.join("・")));
    }
    if !sec.get("FileName").trim().is_empty() {
        notes.push("通知の音は取り込みません".into());
    }
    if let Err(e) = regex::Regex::new(&item.value.base_search.search) {
        return item.skip(format!("正規表現が読めません: {}", e.to_string().lines().last().unwrap_or("")));
    }
    item.note = notes.join("。");
    item
}

/// YP の URL を index.txt の URL にする (pcyplite はフォルダの形で持っている)
pub fn yp_index_url(url: &str) -> String {
    let u = url.trim();
    if u.ends_with('/') { format!("{u}index.txt") } else { u.to_string() }
}

/// URL を比べるための形 (http と https、大文字と小文字、末尾の / は同じとみなす)
fn url_key(url: &str) -> String {
    let l = url.trim().to_ascii_lowercase();
    let l = l.strip_prefix("https://").or_else(|| l.strip_prefix("http://")).unwrap_or(&l).to_string();
    l.trim_end_matches('/').to_string()
}

/// pcyplite のフォルダを読む。今の設定と比べて、同じものはチェックを外しておく。
pub fn load(dir: &Path, cur_filters: &[Filter], cur: &crate::config::Config) -> Imported {
    let mut out = Imported::default();
    let read = |name: &str| -> Option<Vec<(String, Section)>> {
        std::fs::read(dir.join(name)).ok().map(|b| parse_ini(&crate::fetch::decode_text(&b)))
    };

    match read(FAVORITE_INI) {
        Some(ini) => {
            for (name, sec) in &ini {
                if !name.to_ascii_lowercase().starts_with("fav_") {
                    continue;
                }
                let mut item = favorite_to_filter(sec);
                let dup = cur_filters
                    .iter()
                    .any(|f| f.base_search.search == item.value.base_search.search && f.ignore == item.value.ignore);
                if dup {
                    item = item.skip("同じ条件のフィルターがあります");
                }
                out.filters.push(item);
            }
        }
        None => out.warnings.push(format!("{FAVORITE_INI} がありません")),
    }

    let Some(ini) = read(MAIN_INI) else {
        out.warnings.push(format!("{MAIN_INI} がありません"));
        return out;
    };

    let mut hosts: Vec<String> = Vec::new();
    if let Some(sec) = section(&ini, "YP") {
        for i in 0.. {
            let url = sec.get(&format!("Url{i}"));
            let title = sec.get(&format!("Title{i}"));
            if url.is_empty() && title.is_empty() {
                break;
            }
            let y = YpEntry { name: title.to_string(), url: yp_index_url(url), enabled: sec.get(&format!("Enabled{i}")) != "0" };
            let host = sec.get(&format!("Host{i}")).trim();
            if !host.is_empty() {
                hosts.push(host.to_string());
            }
            let mut item = Item::new(y);
            if !crate::config::valid_yp_url(&item.value.url) {
                item = item.skip("http と https の URL だけ使えます");
            } else if let Some(c) = cur.yps.iter().find(|c| url_key(&c.url) == url_key(&item.value.url)) {
                item = item.skip(format!("同じ URL の YP があります ({})", c.name));
            }
            out.yps.push(item);
        }
    }

    // YP ごとに持っている Host のうち、いちばん多いもの
    if let Some(host) = hosts.iter().max_by_key(|h| hosts.iter().filter(|x| x == h).count()) {
        let mut item = Item::new(host.clone());
        if hosts.iter().any(|h| h != host) {
            item.note = "YP によって違うので、いちばん多いものにしました".into();
        }
        if crate::config::parse_address(host).ok() == crate::config::parse_address(&cur.peercast.address).ok() {
            item = item.skip("今の設定と同じです");
        }
        out.peercast = Some(item);
    }

    if let Some(sec) = section(&ini, "Etc_Player") {
        for i in 0.. {
            let ext = sec.get(&format!("Extension{i}")).trim();
            let exe = sec.get(&format!("FileName{i}")).trim();
            if ext.is_empty() && exe.is_empty() {
                break;
            }
            let args = sec.get(&format!("Arguments{i}")).trim();
            let p = PlayerEntry { types: ext.to_string(), exe: exe.to_string(), args: convert_args(args) };
            let mut item = Item::new(p);
            let mut notes = Vec::new();
            if args.contains("<stream/>&pls=m3u") {
                notes.push("<stream/>&pls=m3u は $URL にしました");
            }
            if args.contains("<direct/>") {
                notes.push("<direct/> は消しました");
            }
            item.note = notes.join("。");
            if exe.is_empty() {
                item = item.skip("exe が空です");
            } else if !Path::new(exe).exists() {
                item = item.skip("exe が見つかりません");
            }
            out.players.push(item);
        }
    }

    if let Some(sec) = section(&ini, "Etc") {
        let b = sec.get("Browser").trim();
        if !b.is_empty() {
            let mut item = Item::new(b.to_string());
            if b == cur.browser {
                item = item.skip("今の設定と同じです");
            }
            out.browser = Some(item);
        }
    }
    out
}

/// pcyplite の引数を pcyp-rust の書き方にする。
///
/// pcyplite の `<stream/>` は /pls/ の URL なので、`<stream/>&pls=m3u` (m3u のプレイリスト) と書いてあることが多い。
/// pcyp-rust の `<stream/>` は `PeerCast` タブで選んだ再生の URL (初期値は /stream/) なので、`$URL` だけにする。
/// `<direct/>` は pcyp-rust に無いので消す。
pub fn convert_args(args: &str) -> String {
    args.replace("<stream/>&pls=m3u", "$URL").replace("<direct/>", "")
}

/// プレイヤーを取り込むときのやり方
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerMode {
    /// 今のプレイヤーを消して、取り込んだものだけにする
    Replace,
    /// 今のプレイヤーの前に足す (種類の合うものを上から探すので、取り込んだものが先に使われる)
    Prepend,
}

/// チェックしたものを設定とフィルターに足す。取り込んだ件数の説明を返す。
pub fn apply(data: &Imported, mode: PlayerMode, cfg: &mut crate::config::Config, filters: &mut Vec<Filter>) -> String {
    let mut done = Vec::new();

    let fs: Vec<Filter> = data.filters.iter().filter(|i| i.checked).map(|i| i.value.clone()).collect();
    if !fs.is_empty() {
        done.push(format!("お気に入り {} 件", fs.len()));
        filters.extend(fs);
    }
    let ys: Vec<YpEntry> = data.yps.iter().filter(|i| i.checked).map(|i| i.value.clone()).collect();
    if !ys.is_empty() {
        done.push(format!("YP {} 件", ys.len()));
        cfg.yps.extend(ys);
    }
    let ps: Vec<PlayerEntry> = data
        .players
        .iter()
        .filter(|i| i.checked)
        .map(|i| i.value.clone())
        .collect();
    if !ps.is_empty() {
        done.push(format!("プレイヤー {} 件", ps.len()));
        match mode {
            PlayerMode::Replace => cfg.players = ps,
            PlayerMode::Prepend => {
                let rest = std::mem::take(&mut cfg.players);
                cfg.players = ps;
                cfg.players.extend(rest);
            }
        }
    }
    if let Some(p) = data.peercast.as_ref().filter(|i| i.checked) {
        cfg.peercast.address.clone_from(&p.value);
        done.push("PeerCast のアドレス".into());
    }
    if let Some(b) = data.browser.as_ref().filter(|i| i.checked) {
        cfg.browser.clone_from(&b.value);
        done.push("ブラウザ".into());
    }
    if done.is_empty() { "取り込むものがありませんでした".into() } else { format!("pcyplite から {} を取り込みました", done.join("、")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fav(text: &str) -> Item<Filter> {
        let ini = parse_ini(text);
        favorite_to_filter(&ini[0].1)
    }

    #[test]
    fn reads_favorite_flags() {
        // 名前で探すお気に入り (有効、通知あり)
        let f = fav("\u{feff}[Fav_0]\r\nTitle=はじ\r\nWord=^はじ\r\nFileName=\r\nColor=-16777211\r\nColorText=-16777208\r\nFlags=12291\r\n");
        let v = &f.value;
        assert!(f.checked);
        assert_eq!(v.name, "はじ");
        assert_eq!(v.base_search.search, "^はじ");
        assert_eq!(v.base_search.fields, ["name"]);
        assert!(v.enabled && v.favorite && v.notify && !v.ignore && !v.ignore_case);

        // チェックの外れたもの (12290)
        assert!(!fav("[Fav_0]\nTitle=a\nWord=a\nFlags=12290").value.enabled);

        // 詳細とコメントで探す、無効のもの (12300)
        let v = fav("[Fav_0]\nTitle=DQ\nWord=DQ|dq\nFlags=12300").value;
        assert!(!v.enabled);
        assert_eq!(v.base_search.fields, ["genre", "desc", "comment"]);

        // NG (15379): 名前とコンタクト URL、無視、通知しない
        let v = fav("[Fav_0]\nTitle=NG\nWord=TERUMI|\\(　´ω｀\\)だんごむし\nFlags=15379").value;
        assert!(v.ignore && !v.favorite && !v.notify && v.enabled && !v.enable_color);
        assert_eq!(v.base_search.fields, ["name", "url"]);
    }

    #[test]
    fn favorite_details() {
        // 検索文字が空なら名前で探す
        assert_eq!(fav("[Fav_0]\nTitle=きりたんぽ\nWord=\nFlags=12291").value.base_search.search, "きりたんぽ");
        // 正規表現を使わないなら、記号をそのまま探す
        assert_eq!(fav("[Fav_0]\nTitle=a\nWord=a.b\nFlags=3").value.base_search.search, "a\\.b");
        // 音は取り込まない
        assert!(fav("[Fav_0]\nTitle=a\nWord=a\nFileName=c:\\x.wav\nFlags=12291").note.contains("音"));
        // 読めない正規表現はチェックを外す
        assert!(!fav("[Fav_0]\nTitle=a\nWord=(a\nFlags=12291").checked);
        // 色を決めてあれば使う (Delphi の TColor は BGR)
        let v = fav("[Fav_0]\nTitle=a\nWord=a\nColor=255\nFlags=12291").value;
        assert!(v.enable_color);
        assert_eq!(v.color, [255, 0, 0]);
    }

    #[test]
    fn loads_folder() {
        let d = std::env::temp_dir().join(format!("pcyp-rust-test-import-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FAVORITE_INI), "[Fav_0]\nTitle=A\nWord=A\nFlags=12291\n\n[Fav_1]\nTitle=B\nWord=B\nFlags=12291\n").unwrap();
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode(concat!(
            "[YP]\nEnabled0=1\nTitle0=sp\nUrl0=http://bayonet.ddo.jp/sp/\nHost0=192.0.2.1:7145\n",
            "Enabled1=0\nTitle1=新しい\nUrl1=http://example.com/yp/\nHost1=192.0.2.1:7145\n",
            "[Etc_Player]\nExtension0=flv\nFileName0=C:\\nowhere\\mpv.exe\nArguments0=\"<stream/>\"\n",
            "[Etc]\nBrowser=\n"
        ));
        std::fs::write(d.join(MAIN_INI), &sjis).unwrap();

        let cur = crate::config::Config::default();
        let data = load(&d, &[Filter::exact_name("x", false), Filter { base_search: Search { search: "B".into(), ..Default::default() }, ..Default::default() }], &cur);
        let _ = std::fs::remove_dir_all(&d);

        assert!(data.warnings.is_empty());
        assert_eq!(data.filters.len(), 2);
        assert!(data.filters[0].checked);
        assert!(!data.filters[1].checked, "同じ条件のものは外す");
        // SP は初期設定にあるので外す。新しい YP は index.txt を付けて、無効のまま取り込む
        assert!(!data.yps[0].checked);
        assert!(data.yps[1].checked);
        assert_eq!(data.yps[1].value.url, "http://example.com/yp/index.txt");
        assert!(!data.yps[1].value.enabled);
        assert_eq!(data.peercast.as_ref().unwrap().value, "192.0.2.1:7145");
        assert!(!data.players[0].checked, "exe がないものは外す");
        assert!(data.browser.is_none());

        let mut cfg = cur.clone();
        let mut filters = vec![];
        let msg = apply(&data, PlayerMode::Prepend, &mut cfg, &mut filters);
        assert_eq!(filters.len(), 1);
        assert_eq!(cfg.yps.len(), cur.yps.len() + 1);
        assert_eq!(cfg.peercast.address, "192.0.2.1:7145");
        assert_eq!(cfg.players, cur.players);
        assert!(msg.contains("お気に入り 1 件"));
    }

    /// 実際の pcyplite のフォルダを読んで、一覧に出す内容を表示する。フォルダを環境変数で渡したときだけ動く。
    /// `PCYP_TEST_PCYPLITE=フォルダ cargo test live_pcyplite -- --ignored --nocapture`
    #[test]
    #[ignore = "実際の PCYP Lite のフォルダがいる"]
    fn live_pcyplite() {
        let Ok(dir) = std::env::var("PCYP_TEST_PCYPLITE") else { return };
        let data = load(Path::new(&dir), &[], &crate::config::Config::default());
        let mark = |c: bool| if c { "☑" } else { "☐" };
        for i in &data.filters {
            let f = &i.value;
            let kind = if f.ignore { "無視" } else if f.favorite { "お気に入り" } else { "色分け" };
            println!("{} {} | {} | {:?} | {} 通知:{} 有効:{} | {}", mark(i.checked), f.name, f.base_search.search, f.base_search.fields, kind, f.notify, f.enabled, i.note);
        }
        for i in &data.yps {
            println!("{} YP {} {} 有効:{} | {}", mark(i.checked), i.value.name, i.value.url, i.value.enabled, i.note);
        }
        for i in &data.players {
            println!("{} {} {} {} | {}", mark(i.checked), i.value.types, i.value.exe, i.value.args, i.note);
        }
        for (l, i) in [("PeerCast", &data.peercast), ("ブラウザ", &data.browser)] {
            if let Some(i) = i {
                println!("{} {} {} | {}", mark(i.checked), l, i.value, i.note);
            }
        }
        println!("{:?}", data.warnings);
    }

    #[test]
    fn player_args() {
        assert_eq!(convert_args(r#""<stream/>&pls=m3u" --title="<channelname/>""#), r#""$URL" --title="<channelname/>""#);
        assert_eq!(convert_args(r#""<stream/>" "<channelname/>" "<direct/>""#), r#""<stream/>" "<channelname/>" """#);
    }

    #[test]
    fn yp_urls() {
        assert_eq!(yp_index_url("https://p-at.net/"), "https://p-at.net/index.txt");
        assert_eq!(yp_index_url("http://a/index.txt"), "http://a/index.txt");
        assert_eq!(url_key("https://P-AT.net/index.txt"), url_key("http://p-at.net/index.txt"));
    }
}
