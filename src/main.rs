#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chandir;
mod config;
mod fetch;
mod filter;
mod peercast;
mod player;
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
    fonts.font_data.insert("jp".into(), Arc::new(egui::FontData::from_owned(data)));
    let default_prop = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "jp".into());
    fonts.families.entry(egui::FontFamily::Monospace).or_default().push("jp".into());

    let has_bold = match bold.iter().find_map(|p| std::fs::read(p).ok()) {
        Some(data) => {
            fonts.font_data.insert("jp_bold".into(), Arc::new(egui::FontData::from_owned(data)));
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
    let cfg: Config = config::load_json(config::CONFIG_FILE).unwrap_or_else(|e| {
        worker::lock(&shared).log(true, format!("{} (初期値を使います)", e));
        Config::default()
    });
    let filters: Filters = config::load_json(config::FILTER_FILE).unwrap_or_else(|e| {
        worker::lock(&shared).log(true, e);
        Filters::default()
    });
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
                app::AppInit { config, filters, shared, tx, tray, open_settings, has_bold },
            )))
        }),
    )
}
