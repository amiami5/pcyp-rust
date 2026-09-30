//! 再生の履歴。exe と同じフォルダの `history.json` に置く。
//!
//! チャンネル名ごとに 1 件にまとめ、最後に再生した日時と回数を持つ。新しいものが先頭。

use crate::chandir::Channel;
use crate::config::{self, Config};
use crate::worker::{SharedRef, lock};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryEntry {
    pub name: String,
    /// 最後に再生したときのチャンネル ID (配信し直すと変わる)
    pub id: String,
    pub genre: String,
    pub desc: String,
    pub content_type: String,
    /// コンタクト URL
    pub url: String,
    pub yp: String,
    /// 最後に再生した日時 (`2026-09-30 21:05`)
    pub time: String,
    /// 再生した回数
    pub count: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

impl History {
    /// 再生したことを残す。同じ名前のものは先頭へ移し、回数を増やす。`max` 件を超えた古いものは消す
    pub fn record(&mut self, ch: &Channel, yp: &str, time: String, max: usize) {
        let count = match self.entries.iter().position(|e| e.name == ch.name) {
            Some(i) => self.entries.remove(i).count,
            None => 0,
        };
        self.entries.insert(
            0,
            HistoryEntry {
                name: ch.name.clone(),
                id: ch.id.clone(),
                genre: ch.genre.clone(),
                desc: ch.desc.clone(),
                content_type: ch.content_type.clone(),
                url: ch.url.clone(),
                yp: yp.to_string(),
                time,
                count: count.saturating_add(1),
            },
        );
        self.entries.truncate(max.max(1));
    }

    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|e| e.name != name);
    }
}

impl HistoryEntry {
    /// 残してある項目だけのチャンネル (フィルターに当てるときなどに使う)
    pub fn channel(&self) -> Channel {
        Channel {
            name: self.name.clone(),
            id: self.id.clone(),
            genre: self.genre.clone(),
            desc: self.desc.clone(),
            content_type: self.content_type.clone(),
            url: self.url.clone(),
            ..Default::default()
        }
    }

    /// 「[ジャンル - 詳細]」の形 (一覧の説明と同じ)
    pub fn summary(&self) -> String {
        self.channel().summary()
    }
}

/// 履歴を保存する。
pub fn save(shared: &SharedRef) {
    let h = lock(shared).history.clone();
    if let Err(e) = config::save_json(config::HISTORY_FILE, &h) {
        lock(shared).log(true, format!("履歴を保存できません: {}", e));
    }
}

/// 再生したチャンネルを履歴に残して保存する (履歴を切っていれば何もしない)。
pub fn record_play(shared: &SharedRef, cfg: &Config, ch: &Channel) {
    if !cfg.history.enabled || ch.is_info() {
        return;
    }
    let yp = cfg.yps.iter().find(|y| y.url == ch.feed_url).map(|y| y.name.clone()).unwrap_or_default();
    lock(shared).history.record(ch, &yp, crate::win::now_ymdhm(), cfg.history.max as usize);
    save(shared);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(name: &str, id: &str) -> Channel {
        Channel { name: name.into(), id: id.into(), genre: "g".into(), ..Default::default() }
    }

    #[test]
    fn same_name_moves_to_top() {
        let mut h = History::default();
        h.record(&ch("A", "1"), "SP", "t1".into(), 10);
        h.record(&ch("B", "2"), "SP", "t2".into(), 10);
        // 配信し直して ID が変わっても、名前が同じなら 1 件にまとめる
        h.record(&ch("A", "3"), "平成", "t3".into(), 10);
        let names: Vec<_> = h.entries.iter().map(|e| (e.name.as_str(), e.count, e.id.as_str(), e.yp.as_str())).collect();
        assert_eq!(names, [("A", 2, "3", "平成"), ("B", 1, "2", "SP")]);
        assert_eq!(h.entries[0].time, "t3");
    }

    #[test]
    fn keeps_max_entries() {
        let mut h = History::default();
        for i in 0..5 {
            h.record(&ch(&format!("c{}", i), "1"), "", String::new(), 3);
        }
        let names: Vec<_> = h.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["c4", "c3", "c2"]);
        h.remove("c3");
        assert_eq!(h.entries.len(), 2);
    }

    #[test]
    fn summary_like_list() {
        let e = HistoryEntry { genre: "ゲーム".into(), desc: "RPG".into(), ..Default::default() };
        assert_eq!(e.summary(), "[ゲーム - RPG]");
    }
}
