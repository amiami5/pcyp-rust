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
}
