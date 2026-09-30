#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chandir;
mod config;
mod fetch;
mod filter;
mod history;
mod icon;
mod import;
mod peercast;
mod player;
mod update;
mod win;
mod worker;

use config::Config;
use filter::Filters;
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock, mpsc};

/// 日本語のフォントを読み込む。太字のフォントも読めたら true。
fn setup_fonts(ctx: &egui::Context, custom: &str) -> bool {
    let fonts_dir = std::path::Path::new("C:\\Windows\\Fonts");
    let regular: Vec<std::path::PathBuf> = [custom.to_string()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(Into::into)
        .chain(["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"].iter().map(|f| fonts_dir.join(f)))
        .collect();
    let bold = ["YuGothB.ttc", "meiryob.ttc"].map(|f| fonts_dir.join(f));

    let mut fonts = egui::FontDefinitions::default();
    let Some(data) = regular.iter().find_map(|p| std::fs::read(p).ok()) else {
        return false;
    };
    fonts.font_data.insert("jp".into(), Arc::new(jp_font(data)));
    let default_prop = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "jp".into());
    fonts.families.entry(egui::FontFamily::Monospace).or_default().push("jp".into());

    let has_bold = match bold.iter().find_map(|p| std::fs::read(p).ok()) {
        Some(data) => {
            fonts.font_data.insert("jp_bold".into(), Arc::new(jp_font(data)));
            let mut fam = vec!["jp_bold".to_string(), "jp".to_string()];
            fam.extend(default_prop);
            fonts.families.insert(egui::FontFamily::Name(app::BOLD.into()), fam);
            true
        }
        None => false,
    };
    ctx.set_fonts(fonts);
    has_bold
}

/// 日本語のフォントを、字の上下の真ん中が行の真ん中に来るようにずらして読み込む。
/// egui は足りない字を補うフォント (⟳ ⚙ 📋 などの絵文字) を行の真ん中に置くが、
/// 日本語のフォントは行の上寄りに字があるので、そのままだと絵文字より上に見える。
/// ずらす量は字の大きさに比例させるので、文字サイズやフォントを変えてもそろう。
fn jp_font(data: Vec<u8>) -> egui::FontData {
    use skrifa::MetadataProvider;
    let mut font = egui::FontData::from_owned(data);
    let Ok(f) = skrifa::FontRef::from_index(&font.font, 0) else {
        return font;
    };
    // egui と同じ取り方をした行の寸法 (上向きが正、字の単位)
    let m = f.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    let em = m.units_per_em as f32;
    // 字の上下の真ん中は、漢字の外枠で測る
    let gm = f.glyph_metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    let Some(b) = f.charmap().map('国').and_then(|g| gm.bounds(g)) else {
        return font;
    };
    // 行の上端から測った、字の真ん中と行の真ん中
    let glyph_mid = m.ascent - (b.y_min + b.y_max) / 2.0;
    let row_mid = (m.ascent - m.descent + m.leading) / 2.0;
    if em > 0.0 {
        font.tweak.y_offset_factor = (row_mid - glyph_mid) / em;
    }
    font
}

/// 二つ目の起動なら false。
#[cfg(windows)]
fn single_instance() -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = "Local\\pcyp-rust-single-instance".encode_utf16().chain(Some(0)).collect();
    // 閉じずにおく (プロセスが終わるまで持つ)
    let h = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    !(h.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS)
}

#[cfg(not(windows))]
fn single_instance() -> bool {
    true
}

fn main() -> eframe::Result {
    if !single_instance() {
        // すでに動いているほうを前に出す
        let title: Vec<u16> = config::APP_NAME.encode_utf16().chain(Some(0)).collect();
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let h = FindWindowW(std::ptr::null(), title.as_ptr());
            if !h.is_null() {
                ShowWindow(h, SW_RESTORE);
                SetForegroundWindow(h);
            }
        }
        let _ = title;
        return Ok(());
    }

    let shared = Arc::new(Mutex::new(worker::Shared::default()));
    // 読めなかったファイルは消さずに退避し、控えから読む。何があったかは画面の上で知らせる
    let loaded_cfg: config::Loaded<Config> = config::load_json(config::CONFIG_FILE);
    let loaded_filters: config::Loaded<Filters> = config::load_json(config::FILTER_FILE);
    let loaded_history: config::Loaded<history::History> = config::load_json(config::HISTORY_FILE);
    worker::lock(&shared).history = loaded_history.value;
    let notices: Vec<String> =
        loaded_cfg.notices.into_iter().chain(loaded_filters.notices).chain(loaded_history.notices).collect();
    for n in &notices {
        worker::lock(&shared).log(true, n.clone());
    }
    let cfg: Config = loaded_cfg.value;
    let filters: Filters = loaded_filters.value;
    let config = Arc::new(RwLock::new(cfg.clone()));
    let filters = Arc::new(RwLock::new(filters.0));
    let (tx, rx) = mpsc::channel();

    let size = cfg.view.window_size.unwrap_or([760.0, 620.0]);
    let hide_on_start = cfg.tray.enabled && cfg.tray.start_minimized;
    let viewport = egui::ViewportBuilder::default()
        .with_title(config::APP_NAME)
        .with_inner_size(size)
        .with_min_inner_size([360.0, 240.0])
        .with_icon(win::app_icon())
        .with_visible(!hide_on_start);
    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        config::APP_NAME,
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let has_bold = setup_fonts(&ctx, &cfg.view.font_path);
            if let Ok(h) = cc.window_handle()
                && let RawWindowHandle::Win32(w) = h.as_raw()
            {
                win::set_main_window(w.hwnd.get(), ctx.clone());
                win::apply_exe_icon();
                // 前回閉じた位置に開く。モニターを外したなどで画面の外になるなら、使わずにふつうの位置で開く
                if let Some(r) = cfg.view.window_rect {
                    if win::rect_is_on_screen(r) {
                        win::set_window_rect(r);
                    } else {
                        worker::lock(&shared).log(false, "前回の窓の位置は画面の外になるので、ふつうの位置で開きました");
                    }
                }
            }
            let open_settings = Arc::new(AtomicBool::new(false));
            let tray = if cfg.tray.enabled {
                match win::create_tray(tx.clone(), open_settings.clone()) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        worker::lock(&shared).log(true, format!("タスクトレイにアイコンを出せません: {}", e));
                        None
                    }
                }
            } else {
                None
            };

            let repaint_ctx = ctx.clone();
            let (n_config, n_shared) = (config.clone(), shared.clone());
            worker::Worker {
                config: config.clone(),
                filters: filters.clone(),
                shared: shared.clone(),
                repaint: Arc::new(move || repaint_ctx.request_repaint()),
                notifier: Arc::new(move |chans| win::notify_channels(chans, n_config.clone(), n_shared.clone())),
                fetcher: Arc::new(fetch::fetch_index),
            }
            .spawn(rx);

            Ok(Box::new(app::App::new(
                &ctx,
                app::AppInit { config, filters, shared, tx, tray, open_settings, has_bold, notices },
            )))
        }),
    )
}
