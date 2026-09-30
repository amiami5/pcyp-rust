//! お気に入り・無視・色分けのフィルター。

use crate::chandir::Channel;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

pub const FIELDS: &[(&str, &str)] = &[
    ("name", "名前"),
    ("genre", "ジャンル"),
    ("desc", "詳細"),
    ("comment", "コメント"),
    ("url", "コンタクト"),
    ("type", "種類"),
    ("tip", "トラッカー"),
    ("id", "ID"),
    ("yp", "YP"),
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Search {
    pub enabled: bool,
    pub search: String,
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Filter {
    /// 表示用の名前 (空なら検索の文字列)
    pub name: String,
    pub enabled: bool,
    pub favorite: bool,
    pub ignore: bool,
    /// 当たったチャンネルが始まったら通知する
    pub notify: bool,
    pub enable_color: bool,
    pub color: [u8; 3],
    pub ignore_case: bool,
    pub base_search: Search,
    pub and_search: Search,
    pub not_search: Search,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            name: String::new(),
            enabled: true,
            favorite: true,
            ignore: false,
            notify: true,
            enable_color: true,
            color: [255, 230, 150],
            ignore_case: true,
            base_search: Search { enabled: true, search: String::new(), fields: vec!["name".into()] },
            and_search: Search::default(),
            not_search: Search::default(),
        }
    }
}

impl Filter {
    pub fn title(&self) -> &str {
        if self.name.is_empty() { &self.base_search.search } else { &self.name }
    }

    /// 名前がちょうど一致するお気に入り / 無視を作る。
    pub fn exact_name(name: &str, ignore: bool) -> Filter {
        Filter {
            name: name.to_string(),
            favorite: !ignore,
            ignore,
            notify: !ignore,
            enable_color: !ignore,
            ignore_case: false,
            base_search: Search {
                enabled: true,
                search: format!("^{}$", regex::escape(name)),
                fields: vec!["name".into()],
            },
            ..Default::default()
        }
    }
}

/// チャンネルからフィルターを作るときの雛形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    /// 名前がちょうど同じ
    Exact,
    /// 名前 (括弧の中を除いた部分) を含む
    Contains,
    /// コンタクト URL が同じ
    Contact,
    /// 名前が同じか、コンタクト URL が同じ
    NameOrContact,
}

impl Template {
    pub const ALL: [Template; 4] = [Template::Contains, Template::Exact, Template::Contact, Template::NameOrContact];

    pub fn label(self) -> &'static str {
        match self {
            Template::Exact => "名前がちょうど同じ",
            Template::Contains => "名前を含む",
            Template::Contact => "コンタクト URL が同じ",
            Template::NameOrContact => "名前か URL が同じ",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Template::Exact => "この名前のチャンネルだけに当てます",
            Template::Contains => "【】や () の中を除いた名前を含むチャンネルに当てます。名前の後ろに回数や告知を付ける配信者向け",
            Template::Contact => "コンタクト URL (掲示板など) が同じチャンネルに当てます。名前をよく変える配信者向け",
            Template::NameOrContact => "名前か、コンタクト URL のどちらかが同じなら当てます",
        }
    }

    /// このチャンネルでこの雛形が使えるか (コンタクト URL がなければ URL の雛形は使えない)
    pub fn usable(self, c: &Channel) -> bool {
        match self {
            Template::Contact | Template::NameOrContact => !contact_key(&c.url).is_empty(),
            _ => !c.name.trim().is_empty(),
        }
    }

    /// 雛形の検索条件を作る。
    pub fn search(self, c: &Channel) -> Search {
        let exact = format!("^{}$", regex::escape(&c.name));
        let url = format!("^https?://{}/?$", regex::escape(contact_key(&c.url)));
        let (search, fields) = match self {
            Template::Exact => (exact, vec!["name"]),
            Template::Contains => (regex::escape(name_core(&c.name)), vec!["name"]),
            Template::Contact => (url, vec!["url"]),
            Template::NameOrContact => (format!("{}|{}", exact, url), vec!["name", "url"]),
        };
        Search { enabled: true, search, fields: fields.into_iter().map(String::from).collect() }
    }

    /// 雛形から新しいフィルターを作る (お気に入り)。
    pub fn filter(self, c: &Channel) -> Filter {
        Filter { name: name_core(&c.name).to_string(), base_search: self.search(c), ..Default::default() }
    }
}

/// 名前から、【】 () [] などで囲んだ部分を除いた、はじめのまとまり。
/// 「〇〇の雑談【初見歓迎】」なら「〇〇の雑談」。何も残らなければ名前をそのまま返す。
pub fn name_core(name: &str) -> &str {
    const OPEN: &[char] = &['(', '（', '[', '［', '【', '〔', '「', '『', '<', '＜', '《', '〈'];
    let name = name.trim();
    let core = match name.find(OPEN) {
        Some(0) => {
            // 先頭が括弧なら、閉じたあとの部分を使う
            const CLOSE: &[char] = &[')', '）', ']', '］', '】', '〕', '」', '』', '>', '＞', '》', '〉'];
            match name.find(CLOSE) {
                Some(i) => {
                    let rest = &name[i..];
                    let rest = rest[rest.chars().next().map_or(0, char::len_utf8)..].trim_start();
                    rest.find(OPEN).map_or(rest, |j| &rest[..j])
                }
                None => name,
            }
        }
        Some(i) => &name[..i],
        None => name,
    };
    let core = core.trim();
    if core.is_empty() { name } else { core }
}

/// コンタクト URL の比べる部分 (http(s):// と末尾の / を除く)。
fn contact_key(url: &str) -> &str {
    let u = url.trim();
    let u = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).unwrap_or("");
    u.trim_end_matches('/')
}

/// 1 つのフィルターがどのチャンネルに当たるかを調べるためのもの (有効かどうかは見ない)。
pub fn compile_one(f: &Filter) -> Result<CompiledFilter, String> {
    if f.base_search.search.is_empty() {
        return Err("条件が空です".into());
    }
    let (mut c, mut e) = compile(std::slice::from_ref(&Filter { enabled: true, ..f.clone() }));
    match c.pop() {
        Some(c) => Ok(c),
        None => Err(e.pop().unwrap_or_default()),
    }
}

/// 保存するファイルの形。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(transparent)]
pub struct Filters(pub Vec<Filter>);

struct CompiledSearch {
    re: Regex,
    fields: Vec<String>,
}

pub struct CompiledFilter {
    /// 元のフィルターの並びの中での位置
    pub index: usize,
    /// 一覧に出す名前 (フィルターの名前、なければ条件)
    pub title: String,
    pub favorite: bool,
    pub ignore: bool,
    pub notify: bool,
    pub color: Option<[u8; 3]>,
    base: CompiledSearch,
    and: Option<CompiledSearch>,
    not: Option<CompiledSearch>,
}

/// フィルターを当てた結果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchResult {
    pub favorite: bool,
    pub ignore: bool,
    pub notify: bool,
    pub colors: Vec<[u8; 3]>,
    /// 当たったフィルターの名前
    pub names: Vec<String>,
    /// 当たったお気に入り / 無視のフィルターの位置 (元の並びの中での番号)
    pub favorite_filters: Vec<usize>,
    pub ignore_filters: Vec<usize>,
}

fn compile_search(s: &Search, ignore_case: bool) -> Result<CompiledSearch, String> {
    let re = RegexBuilder::new(&s.search)
        .case_insensitive(ignore_case)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| e.to_string())?;
    let fields = if s.fields.is_empty() { vec!["name".to_string()] } else { s.fields.clone() };
    Ok(CompiledSearch { re, fields })
}

/// 有効なフィルターを正規表現にする。失敗したものはエラーを返して飛ばす。
pub fn compile(filters: &[Filter]) -> (Vec<CompiledFilter>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (index, f) in filters.iter().enumerate() {
        if !f.enabled {
            continue;
        }
        if f.base_search.search.is_empty() {
            continue;
        }
        let r = (|| -> Result<CompiledFilter, String> {
            let base = compile_search(&f.base_search, f.ignore_case)?;
            let and = if f.and_search.enabled { Some(compile_search(&f.and_search, f.ignore_case)?) } else { None };
            let not = if f.not_search.enabled { Some(compile_search(&f.not_search, f.ignore_case)?) } else { None };
            Ok(CompiledFilter {
                index,
                title: f.title().to_string(),
                favorite: f.favorite,
                ignore: f.ignore,
                notify: f.notify,
                color: f.enable_color.then_some(f.color),
                base,
                and,
                not,
            })
        })();
        match r {
            Ok(c) => out.push(c),
            Err(e) => errors.push(format!("フィルター「{}」を無効にしました: {}", f.title(), e)),
        }
    }
    (out, errors)
}

fn field<'a>(c: &'a Channel, yp: &'a str, name: &str) -> &'a str {
    match name {
        "name" => &c.name,
        "genre" => &c.genre,
        "desc" => &c.desc,
        "comment" => &c.comment,
        "url" => &c.url,
        "type" => &c.content_type,
        "tip" => &c.tip,
        "id" => &c.id,
        "yp" => yp,
        _ => "",
    }
}

fn hit(s: &CompiledSearch, c: &Channel, yp: &str) -> bool {
    s.fields.iter().any(|f| s.re.is_match(field(c, yp, f)))
}

impl CompiledFilter {
    pub fn is_match(&self, c: &Channel, yp: &str) -> bool {
        hit(&self.base, c, yp)
            && self.and.as_ref().is_none_or(|s| hit(s, c, yp))
            && self.not.as_ref().is_none_or(|s| !hit(s, c, yp))
    }
}

pub fn apply(filters: &[CompiledFilter], c: &Channel, yp: &str) -> MatchResult {
    let mut r = MatchResult::default();
    for f in filters {
        if f.is_match(c, yp) {
            r.favorite |= f.favorite;
            r.ignore |= f.ignore;
            r.notify |= f.favorite && f.notify;
            if let Some(col) = f.color {
                r.colors.push(col);
            }
            if !r.names.contains(&f.title) {
                r.names.push(f.title.clone());
            }
            if f.favorite {
                r.favorite_filters.push(f.index);
            }
            if f.ignore {
                r.ignore_filters.push(f.index);
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(name: &str, genre: &str) -> Channel {
        Channel { name: name.into(), genre: genre.into(), ..Default::default() }
    }

    #[test]
    fn base_and_not() {
        let f = Filter {
            base_search: Search { enabled: true, search: "game".into(), fields: vec!["genre".into()] },
            and_search: Search { enabled: true, search: "PS".into(), fields: vec!["genre".into()] },
            not_search: Search { enabled: true, search: "ネタバレ".into(), fields: vec!["genre".into()] },
            ..Default::default()
        };
        let (c, e) = compile(&[f]);
        assert!(e.is_empty());
        assert!(apply(&c, &ch("a", "GAME ps3"), "").favorite);
        assert!(!apply(&c, &ch("a", "game pc"), "").favorite);
        assert!(!apply(&c, &ch("a", "game ps ネタバレ"), "").favorite);
    }

    #[test]
    fn bad_regex_is_disabled() {
        let bad = Filter { base_search: Search { enabled: true, search: "(".into(), fields: vec![] }, ..Default::default() };
        let good = Filter::exact_name("a.b", true);
        let (c, e) = compile(&[bad, good]);
        assert_eq!(c.len(), 1);
        assert_eq!(e.len(), 1);
        assert!(apply(&c, &ch("a.b", ""), "").ignore);
        assert!(!apply(&c, &ch("axb", ""), "").ignore);
        assert_eq!(apply(&c, &ch("a.b", ""), "").names, ["a.b"]);
        // 無効な 1 件目を飛ばしても、番号は元の並びのまま
        assert_eq!(apply(&c, &ch("a.b", ""), "").ignore_filters, [1]);
    }

    #[test]
    fn reads_peercast_yt_format() {
        let json = r#"[{"enabled":true,"favorite":true,"ignore":false,"enable_color":true,"color":[255,200,0],
            "ignore_case":true,"base_search":{"search":"x","fields":["name","genre"]},
            "and_search":{"enabled":false,"search":"","fields":[]},"not_search":{"enabled":false,"search":"","fields":[]}}]"#;
        let f: Filters = serde_json::from_str(json).unwrap();
        assert_eq!(f.0[0].color, [255, 200, 0]);
        assert!(f.0[0].notify);
    }

    #[test]
    fn name_core_strips_brackets() {
        assert_eq!(name_core("〇〇の雑談【初見歓迎】"), "〇〇の雑談");
        assert_eq!(name_core("abc (test)"), "abc");
        assert_eq!(name_core("【告知】ゲーム実況(PS5)"), "ゲーム実況");
        assert_eq!(name_core("【だけ】"), "【だけ】");
        assert_eq!(name_core("そのまま"), "そのまま");
    }

    #[test]
    fn templates_match_the_source() {
        let c = Channel { name: "a.b【雑談】".into(), url: "https://example.com/bbs/".into(), ..Default::default() };
        for t in Template::ALL {
            assert!(t.usable(&c));
            let f = compile_one(&t.filter(&c)).unwrap();
            assert!(f.is_match(&c, ""), "{:?}", t);
        }
        let other = |name: &str, url: &str| Channel { name: name.into(), url: url.into(), ..Default::default() };
        let m = |t: Template, o: &Channel| compile_one(&t.filter(&c)).unwrap().is_match(o, "");
        assert!(m(Template::Contains, &other("a.b 2 回目", "")));
        assert!(!m(Template::Contains, &other("axb", "")));
        assert!(!m(Template::Exact, &other("a.b", "")));
        assert!(m(Template::Contact, &other("別の名前", "http://example.com/bbs")));
        assert!(!m(Template::Contact, &other("別の名前", "http://example.com/bbs/2")));
        assert!(m(Template::NameOrContact, &other("a.b【雑談】", "")));
        assert!(m(Template::NameOrContact, &other("x", "https://example.com/bbs/")));
        let no_url = Channel { name: "x".into(), ..Default::default() };
        assert!(!Template::Contact.usable(&no_url));
        assert!(compile_one(&Filter::default()).is_err());
    }
}
