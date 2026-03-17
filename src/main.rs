#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};

use eframe::egui;
use tray_icon::menu::{Menu, MenuItem};
use tray_icon::TrayIconBuilder;

mod app;
mod autostart;
mod config;
mod printer;
mod websocket;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Arc::new(Mutex::new(config::AppConfig::load()));
    tracing::info!("Config loaded, port={}", config.lock().unwrap().port);

    let printers = printer::list_printers();
    tracing::info!("Found {} printers", printers.len());

    let ws_config = config.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");
        rt.block_on(websocket::run(ws_config));
    });

    let menu = Menu::new();
    let item_show = MenuItem::new("打开配置", true, None);
    let item_quit = MenuItem::new("退出", true, None);
    menu.append(&item_show).unwrap();
    menu.append(&item_quit).unwrap();

    let show_id = item_show.id().clone();
    let quit_id = item_quit.id().clone();

    let (icon_rgba, icon_w, icon_h) = load_icon_rgba();

    let tray_icon = tray_icon::Icon::from_rgba(icon_rgba.clone(), icon_w, icon_h)
        .expect("Failed to create tray icon");
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(tray_icon)
        .with_tooltip("Print Agent - 打印代理")
        .build()
        .expect("Failed to build tray icon");

    let window_icon = egui::IconData {
        rgba: icon_rgba,
        width: icon_w,
        height: icon_h,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 560.0])
            .with_resizable(false)
            .with_visible(false)
            .with_title("打印代理 - 配置")
            .with_icon(Arc::new(window_icon)),
        ..Default::default()
    };

    eframe::run_native(
        "PrintAgent",
        options,
        Box::new(move |cc| {
            setup_chinese_fonts(&cc.egui_ctx);
            Ok(Box::new(app::PrintAgentApp::new(
                config, printers, show_id, quit_id,
            )))
        }),
    )
}

fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    if let Some(font_data) = find_cjk_font() {
        let cjk_font = egui::FontData::from_owned(font_data);
        fonts
            .font_data
            .insert("chinese".to_owned(), cjk_font.into());
        // CJK as fallback so Latin/digits use the default font metrics
        if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            list.push("chinese".to_owned());
        }
        if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            list.push("chinese".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

fn find_cjk_font() -> Option<Vec<u8>> {
    use font_kit::family_name::FamilyName;
    use font_kit::properties::Properties;
    use font_kit::source::SystemSource;

    let source = SystemSource::new();
    // Platform-preferred CJK fonts listed first to minimize lookups
    let cjk_families: &[&str] = if cfg!(target_os = "windows") {
        &["Microsoft YaHei", "SimHei", "Microsoft JhengHei", "SimSun",
          "Noto Sans CJK SC", "PingFang SC"]
    } else if cfg!(target_os = "macos") {
        &["PingFang SC", "Hiragino Sans GB", "STHeiti",
          "Noto Sans CJK SC", "Microsoft YaHei"]
    } else {
        &["Noto Sans CJK SC", "WenQuanYi Micro Hei", "Droid Sans Fallback",
          "PingFang SC", "Microsoft YaHei"]
    };

    for family in cjk_families {
        if let Ok(handle) = source.select_best_match(
            &[FamilyName::Title(family.to_string())],
            &Properties::new(),
        )
            && let Some(data) = load_font_handle(&handle)
        {
            tracing::info!("Loaded CJK font: {family}");
            return Some(data);
        }
    }

    tracing::warn!("No well-known CJK font found, scanning system fonts...");
    if let Ok(all) = source.all_families() {
        for family_name in &all {
            if let Ok(handle) = source.select_best_match(
                &[FamilyName::Title(family_name.clone())],
                &Properties::new(),
            )
                && let Some(data) = load_font_handle(&handle)
            {
                // Quick check: a CJK font file is typically > 1MB
                if data.len() > 1_000_000 {
                    tracing::info!("Loaded CJK font (by scan): {family_name}");
                    return Some(data);
                }
            }
        }
    }

    tracing::error!("No CJK font found on this system");
    None
}

fn load_font_handle(handle: &font_kit::handle::Handle) -> Option<Vec<u8>> {
    match handle {
        font_kit::handle::Handle::Path { path, .. } => std::fs::read(path).ok(),
        font_kit::handle::Handle::Memory { bytes, .. } => Some(bytes.to_vec()),
    }
}

const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

fn load_icon_rgba() -> (Vec<u8>, u32, u32) {
    let img = image::load_from_memory(ICON_PNG)
        .expect("Failed to decode embedded icon")
        .into_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}
