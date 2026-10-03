//! GitHub Releases に新しい版が出ていないか確かめる。
//!
//! `releases/latest` は下書きとプレリリースを返さないので、公開した版だけを見る。
//! リリースの zip は Actions が `pcyp-rust-<タグ>-windows-x64.zip` の名前で添付する。

use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 最新のリリースを返す API (Cargo.toml の repository から作る)
pub fn latest_api_url() -> String {
    let repo = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/').trim_end_matches(".git");
    let path = repo.strip_prefix("https://github.com/").unwrap_or(repo);
    format!("https://api.github.com/repos/{path}/releases/latest")
}

/// 確かめ直す間隔。うまく確かめられたとき
pub const CHECK_INTERVAL: Duration = Duration::from_hours(24);
/// 確かめられなかったときは、少し早めにやり直す
pub const RETRY_INTERVAL: Duration = Duration::from_hours(1);
/// Actions が添付する zip の名前の終わり
const ZIP_SUFFIX: &str = "-windows-x64.zip";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// タグ (例 `v0.4.0`)
    pub tag: String,
    /// リリースのページ
    pub page_url: String,
    /// 添付の zip を直接落とす URL (まだ添付されていなければ空)
    pub zip_url: String,
}

impl Release {
    /// 今動いている版より新しいか
    pub fn is_newer(&self) -> bool {
        is_newer(&self.tag, env!("CARGO_PKG_VERSION"))
    }
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

/// API の応答を読む。下書きとプレリリースは無いものとみなす。
pub fn parse_release(json: &str) -> Result<Option<Release>, String> {
    let r: ApiRelease = serde_json::from_str(json).map_err(|e| format!("応答を読めません: {e}"))?;
    if r.draft || r.prerelease {
        return Ok(None);
    }
    let zip_url = r.assets.into_iter().find(|a| a.name.ends_with(ZIP_SUFFIX)).map(|a| a.browser_download_url).unwrap_or_default();
    Ok(Some(Release { tag: r.tag_name, page_url: r.html_url, zip_url }))
}

/// `v0.3.1` や `0.3.1` を数の並びにする。数でない部分 (`-beta` など) から後は見ない。
fn version_parts(s: &str) -> Vec<u64> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split(['-', '+']).next().unwrap_or("");
    s.split('.').map_while(|p| p.parse().ok()).collect()
}

/// `tag` が `current` より新しいか。読めないタグは新しくないとみなす。
pub fn is_newer(tag: &str, current: &str) -> bool {
    let (mut a, mut b) = (version_parts(tag), version_parts(current));
    if a.is_empty() {
        return false;
    }
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// 最新のリリースを取ってくる。公開したリリースが 1 つもなければ `None`。
pub fn fetch_latest() -> Result<Option<Release>, String> {
    // 転送には付いていかない
    let config = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(crate::fetch::USER_AGENT)
        .build();
    let mut resp = ureq::Agent::new_with_config(config)
        .get(&latest_api_url())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        s => return Err(format!("HTTP {s}")),
    }
    let body = crate::fetch::read_body_string(&mut resp, 4 * 1024 * 1024)?;
    parse_release(&body)
}

/// 確かめた結果 (画面と、確かめるスレッドとで共有する)
#[derive(Default)]
pub struct UpdateState {
    pub checking: bool,
    /// 最後に確かめ終えた時刻
    pub last_check: Option<Instant>,
    /// 最後に確かめ終えた時刻 (表示用)
    pub checked_at: String,
    /// 公開されている最新のリリース (今の版と同じでも入れる)
    pub latest: Option<Release>,
    pub error: String,
    /// 知らせの帯を閉じた (次の起動まで出さない)
    pub dismissed: bool,
}

impl UpdateState {
    /// 自動で確かめる時期か
    pub fn due(&self) -> bool {
        self.until_due() == Some(Duration::ZERO)
    }

    /// 次に自動で確かめるまでの時間。確かめている最中なら None (終わったら描き直しが来る)
    pub fn until_due(&self) -> Option<Duration> {
        if self.checking {
            return None;
        }
        Some(match self.last_check {
            None => Duration::ZERO,
            Some(t) => (if self.error.is_empty() { CHECK_INTERVAL } else { RETRY_INTERVAL }).saturating_sub(t.elapsed()),
        })
    }
}

pub type UpdateRef = Arc<Mutex<UpdateState>>;

pub fn lock(s: &UpdateRef) -> std::sync::MutexGuard<'_, UpdateState> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 別のスレッドで確かめる。終わったら `done` を呼ぶ (画面の描き直しとログ)。
pub fn spawn_check(state: &UpdateRef, done: impl FnOnce(&Result<Option<Release>, String>) + Send + 'static) {
    {
        let mut s = lock(state);
        if s.checking {
            return;
        }
        s.checking = true;
    }
    let state = state.clone();
    std::thread::spawn(move || {
        let result = fetch_latest();
        {
            let mut s = lock(&state);
            s.checking = false;
            s.last_check = Some(Instant::now());
            s.checked_at = crate::win::now_hms();
            match &result {
                Ok(r) => {
                    s.latest.clone_from(r);
                    s.error.clear();
                }
                Err(e) => s.error.clone_from(e),
            }
        }
        done(&result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.3.2", "0.3.1"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(is_newer("0.3.1.1", "0.3.1"));
        assert!(!is_newer("v0.3.1", "0.3.1"));
        assert!(!is_newer("v0.3.0", "0.3.1"));
        assert!(!is_newer("v0.3", "0.3.0"));
        assert!(!is_newer("nightly", "0.3.1"));
        assert!(is_newer("v0.4.0-beta", "0.3.1"));
    }

    #[test]
    fn parses_release() {
        let json = r#"{
            "tag_name": "v0.4.0",
            "html_url": "https://github.com/amiami5/pcyp-rust/releases/tag/v0.4.0",
            "draft": false, "prerelease": false,
            "assets": [
                {"name": "other.txt", "browser_download_url": "https://x/other.txt"},
                {"name": "pcyp-rust-v0.4.0-windows-x64.zip", "browser_download_url": "https://x/pcyp.zip"}
            ]
        }"#;
        let r = parse_release(json).unwrap().unwrap();
        assert_eq!(r.tag, "v0.4.0");
        assert_eq!(r.zip_url, "https://x/pcyp.zip");
        assert!(r.page_url.ends_with("/v0.4.0"));

        // zip がまだ添付されていないとき
        let r = parse_release(r#"{"tag_name": "v0.4.0", "html_url": "https://x/"}"#).unwrap().unwrap();
        assert!(r.zip_url.is_empty());
        // プレリリースは知らせない
        assert!(parse_release(r#"{"tag_name": "v0.5.0", "html_url": "https://x/", "prerelease": true}"#).unwrap().is_none());
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn api_url_from_repository() {
        assert_eq!(latest_api_url(), "https://api.github.com/repos/amiami5/pcyp-rust/releases/latest");
    }

    #[test]
    fn due_after_interval() {
        let mut s = UpdateState::default();
        assert!(s.due());
        s.last_check = Some(Instant::now());
        assert!(!s.due());
        s.checking = true;
        s.last_check = None;
        assert!(!s.due());
    }
}
