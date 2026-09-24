//! Windows 固有の処理: ウィンドウの表示と非表示、タスクトレイ、通知、時刻、アイコン。

use crate::chandir::Channel;
use crate::config::Config;
use crate::worker::{Command, SharedRef, lock};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock, RwLock};

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static EGUI_CTX: OnceLock<eframe::egui::Context> = OnceLock::new();

pub fn set_main_window(hwnd: isize, ctx: eframe::egui::Context) {
    MAIN_HWND.store(hwnd, Ordering::SeqCst);
    let _ = EGUI_CTX.set(ctx);
}

pub fn request_repaint() {
    if let Some(c) = EGUI_CTX.get() {
        c.request_repaint();
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    fn hwnd() -> Option<windows_sys::Win32::Foundation::HWND> {
        let h = MAIN_HWND.load(Ordering::SeqCst);
        (h != 0).then_some(h as _)
    }

    pub fn show_window() {
        if let Some(h) = hwnd() {
            unsafe {
                if IsIconic(h) != 0 {
                    ShowWindow(h, SW_RESTORE);
                } else {
                    ShowWindow(h, SW_SHOW);
                }
                SetForegroundWindow(h);
            }
        }
        request_repaint();
    }

    pub fn hide_window() {
        if let Some(h) = hwnd() {
            unsafe { ShowWindow(h, SW_HIDE) };
        }
    }

    pub fn is_window_visible() -> bool {
        hwnd().is_some_and(|h| unsafe { IsWindowVisible(h) != 0 && IsIconic(h) == 0 })
    }

    /// 閉じる要求を送る (トレイの「終了」から)。
    pub fn post_close() {
        if let Some(h) = hwnd() {
            unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
        }
    }

    /// 今の日時 (年、月、日、時、分、秒)
    pub fn local_time() -> [u16; 6] {
        let mut st = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
        let st: windows_sys::Win32::Foundation::SYSTEMTIME = st;
        [st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond]
    }

    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromRect,
    };

    fn monitor_info(m: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> Option<MONITORINFO> {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        (!m.is_null() && unsafe { GetMonitorInfoW(m, &mut mi) } != 0).then_some(mi)
    }

    /// WINDOWPLACEMENT の座標 (ワークスペース座標) とスクリーン座標のずれ。
    /// タスクバーが画面の上か左にあると、その分だけずれる。
    fn workspace_offset() -> (i32, i32) {
        let m = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        match monitor_info(m) {
            Some(mi) => (mi.rcWork.left - mi.rcMonitor.left, mi.rcWork.top - mi.rcMonitor.top),
            None => (0, 0),
        }
    }

    fn placement(h: windows_sys::Win32::Foundation::HWND) -> Option<WINDOWPLACEMENT> {
        let mut wp: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
        wp.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        (unsafe { GetWindowPlacement(h, &mut wp) } != 0).then_some(wp)
    }

    /// 通常の状態 (最小化・最大化していないとき) の窓の位置と大きさ。スクリーン座標の [左, 上, 右, 下]。
    /// 最小化やトレイに入れた状態でも、元の位置が取れる。
    pub fn window_rect() -> Option<[i32; 4]> {
        let wp = placement(hwnd()?)?;
        let r = wp.rcNormalPosition;
        let (ox, oy) = workspace_offset();
        Some([r.left + ox, r.top + oy, r.right + ox, r.bottom + oy])
    }

    /// 窓の通常の位置と大きさを変える。見えていない窓は見えないまま (表示されたときにその位置に出る)。
    pub fn set_window_rect(r: [i32; 4]) {
        let Some(h) = hwnd() else { return };
        let Some(mut wp) = placement(h) else { return };
        let (ox, oy) = workspace_offset();
        wp.rcNormalPosition = RECT { left: r[0] - ox, top: r[1] - oy, right: r[2] - ox, bottom: r[3] - oy };
        wp.showCmd = if unsafe { IsWindowVisible(h) } != 0 { SW_SHOWNORMAL as u32 } else { SW_HIDE as u32 };
        unsafe { SetWindowPlacement(h, &wp) };
    }

    /// 窓のタイトルバーのあたりが、どれかのモニターの作業領域 (タスクバーを除く範囲) に十分見えているか。
    pub fn rect_is_on_screen(r: [i32; 4]) -> bool {
        let strip = RECT { left: r[0], top: r[1], right: r[2], bottom: r[1] + super::TITLE_STRIP };
        let m = unsafe { MonitorFromRect(&strip, MONITOR_DEFAULTTONULL) };
        match monitor_info(m) {
            Some(mi) => {
                let w = mi.rcWork;
                super::visible_enough(r, [w.left, w.top, w.right, w.bottom])
            }
            None => false,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn show_window() {
        super::request_repaint();
    }
    pub fn hide_window() {}
    pub fn is_window_visible() -> bool {
        true
    }
    pub fn post_close() {}
    /// 今の日時 (UTC。日付は 1970-01-01 からの日数で代用する)
    pub fn local_time() -> [u16; 6] {
        let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        [1970, 1, (s / 86400) as u16, ((s / 3600) % 24) as u16, ((s / 60) % 60) as u16, (s % 60) as u16]
    }
    pub fn window_rect() -> Option<[i32; 4]> {
        None
    }
    pub fn set_window_rect(_r: [i32; 4]) {}
    pub fn rect_is_on_screen(_r: [i32; 4]) -> bool {
        false
    }
}

pub use imp::{hide_window, is_window_visible, post_close, rect_is_on_screen, set_window_rect, show_window, window_rect};

/// 画面の中にあるかを調べるときの、タイトルバーのあたりの高さ (ピクセル)
const TITLE_STRIP: i32 = 30;
/// タイトルバーが、横にこれだけ以上見えていれば、つかんで動かせるとみなす
const MIN_VISIBLE_WIDTH: i32 = 100;
/// 小さすぎる窓 (壊れた値) は使わない
const MIN_WINDOW_SIZE: i32 = 100;

/// 窓 `r` のタイトルバーのあたりが、作業領域 `work` に十分見えているか。どちらも [左, 上, 右, 下]。
fn visible_enough(r: [i32; 4], work: [i32; 4]) -> bool {
    if r[2] - r[0] < MIN_WINDOW_SIZE || r[3] - r[1] < MIN_WINDOW_SIZE {
        return false;
    }
    let strip = [r[0], r[1], r[2], r[1] + TITLE_STRIP];
    let w = strip[2].min(work[2]) - strip[0].max(work[0]);
    let h = strip[3].min(work[3]) - strip[1].max(work[1]);
    // タイトルバーの上端が作業領域の上にはみ出ていると、つかめないことがある
    w >= MIN_VISIBLE_WIDTH && h >= TITLE_STRIP / 2 && r[1] >= work[1]
}

/// 今の時刻 (`hh:mm:ss`)。
pub fn now_hms() -> String {
    let [_, _, _, h, m, s] = imp::local_time();
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// ファイル名に使う今の日時 (`20260925-001637`)。
pub fn now_stamp() -> String {
    let [y, mo, d, h, mi, s] = imp::local_time();
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, mo, d, h, mi, s)
}

/// ウィンドウのアイコン (一度だけ作る)。
pub fn app_icon() -> Arc<eframe::egui::IconData> {
    static ICON: OnceLock<Arc<eframe::egui::IconData>> = OnceLock::new();
    ICON.get_or_init(|| Arc::new(eframe::egui::IconData { rgba: icon_rgba(64), width: 64, height: 64 })).clone()
}

/// アプリのアイコン (青い丸に白い三角) の RGBA。
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut v = vec![0u8; (size * size * 4) as usize];
    let c = size as f32 / 2.0;
    let r = c - 0.5;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt();
            let a = (r - d + 0.5).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            // 再生の三角
            let (tx, ty) = ((fx - c) / r, (fy - c) / r);
            let inside = tx > -0.35 && tx < 0.55 && ty.abs() < (0.55 - tx) * 0.62;
            let (cr, cg, cb) = if inside { (255, 255, 255) } else { (30, 110, 220) };
            let i = ((y * size + x) * 4) as usize;
            v[i..i + 4].copy_from_slice(&[cr, cg, cb, (a * 255.0) as u8]);
        }
    }
    v
}

/// お気に入りのチャンネルが始まったことを通知する。「再生」のボタンを押すと再生する。
pub fn notify_channels(chans: &[Channel], config: Arc<RwLock<Config>>, shared: SharedRef) {
    // 一度にたくさん出さない
    const MAX_TOASTS: usize = 3;
    for c in chans.iter().take(MAX_TOASTS) {
        let body = c.summary();
        let ch = c.clone();
        let (config, shared) = (config.clone(), shared.clone());
        show_toast(&format!("配信開始: {}", c.name), &body, Some(Box::new(move |action: Option<String>| {
            if action.as_deref() == Some("play") {
                let cfg = config.read().unwrap_or_else(|e| e.into_inner()).clone();
                let r = crate::player::play(&cfg, &ch);
                let mut s = lock(&shared);
                match r {
                    Ok(cmd) => s.log(false, format!("再生: {}", cmd)),
                    Err(e) => s.log(true, e),
                }
                drop(s);
                request_repaint();
            } else {
                show_window();
            }
        })));
    }
    if chans.len() > MAX_TOASTS {
        let rest: Vec<&str> = chans[MAX_TOASTS..].iter().map(|c| c.name.as_str()).collect();
        show_toast(&format!("ほかに {} 件の配信が始まりました", rest.len()), &rest.join("、"), None);
    }
}

type OnActivated = Box<dyn Fn(Option<String>) + Send + 'static>;

#[cfg(windows)]
fn show_toast(title: &str, body: &str, on_play: Option<OnActivated>) {
    use tauri_winrt_notification::{Duration, Toast};
    let mut t = Toast::new(Toast::POWERSHELL_APP_ID).title(title).text1(body).duration(Duration::Short);
    match on_play {
        Some(f) => {
            t = t.add_button("再生", "play").on_activated(move |a| {
                f(a);
                Ok(())
            });
        }
        None => {
            t = t.on_activated(|_| {
                show_window();
                Ok(())
            });
        }
    }
    let _ = t.show();
}

#[cfg(not(windows))]
fn show_toast(_title: &str, _body: &str, _on_play: Option<OnActivated>) {}

/// タスクトレイのアイコン。トレイのメニューとクリックを処理する。
pub struct Tray {
    _icon: tray_icon::TrayIcon,
}

pub fn create_tray(tx: Sender<Command>, open_settings: Arc<std::sync::atomic::AtomicBool>) -> Result<Tray, String> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::new("表示", true, None);
    let refresh = MenuItem::new("更新", true, None);
    let settings = MenuItem::new("設定...", true, None);
    let quit = MenuItem::new("終了", true, None);
    let menu = Menu::new();
    menu.append_items(&[&show, &refresh, &settings, &PredefinedMenuItem::separator(), &quit]).map_err(|e| e.to_string())?;

    let icon = tray_icon::Icon::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(crate::config::APP_NAME)
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())?;

    let (show_id, refresh_id, settings_id, quit_id) = (show.id().clone(), refresh.id().clone(), settings.id().clone(), quit.id().clone());
    let tx = std::sync::Mutex::new(tx);
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == show_id {
            show_window();
        } else if e.id == refresh_id {
            let _ = tx.lock().map(|t| t.send(Command::Refresh { manual: true }));
            request_repaint();
        } else if e.id == settings_id {
            open_settings.store(true, Ordering::SeqCst);
            show_window();
        } else if e.id == quit_id {
            crate::app::QUITTING.store(true, Ordering::SeqCst);
            show_window();
            post_close();
        }
    }));
    TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            if is_window_visible() {
                hide_window();
            } else {
                show_window();
            }
        }
    }));
    Ok(Tray { _icon: tray })
}

#[cfg(test)]
mod tests {
    use super::visible_enough;

    const WORK: [i32; 4] = [0, 0, 1920, 1040];

    #[test]
    fn inside_is_visible() {
        assert!(visible_enough([100, 100, 900, 700], WORK));
    }

    #[test]
    fn mostly_outside_is_not_visible() {
        // 右にはみ出て、タイトルバーが 50 ピクセルしか見えない
        assert!(!visible_enough([1870, 100, 2670, 700], WORK));
        // 下にはみ出て、タイトルバーが見えない
        assert!(!visible_enough([100, 1030, 900, 1630], WORK));
        // 外したモニターの位置
        assert!(!visible_enough([-2000, 100, -1200, 700], WORK));
        // タイトルバーが上にはみ出る
        assert!(!visible_enough([100, -20, 900, 580], WORK));
    }

    #[test]
    fn partly_outside_but_grabbable() {
        // 右に大きくはみ出ても、タイトルバーが 200 ピクセル見えていればよい
        assert!(visible_enough([1720, 100, 2520, 700], WORK));
    }

    #[test]
    fn broken_size_is_rejected() {
        assert!(!visible_enough([100, 100, 110, 110], WORK));
        assert!(!visible_enough([100, 100, 50, 700], WORK));
    }
}
