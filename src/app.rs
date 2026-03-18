use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use tray_icon::menu::{MenuEvent, MenuId};
use tray_icon::TrayIconEvent;

use crate::config::{
    AppConfig, ColorMode, Orientation, PaperSize, PrintBinding, PrintSettings,
};
use crate::autostart;
use crate::printer;

pub struct PrintAgentApp {
    config: Arc<Mutex<AppConfig>>,
    edit_config: AppConfig,
    printers: Vec<String>,
    show_id: MenuId,
    quit_id: MenuId,
    visible: bool,
    status_msg: String,
    style_initialized: bool,
    settings_open: Option<usize>,
    running_port: u16,
    port_text: String,
}

impl PrintAgentApp {
    pub fn new(
        config: Arc<Mutex<AppConfig>>,
        printers: Vec<String>,
        show_id: MenuId,
        quit_id: MenuId,
    ) -> Self {
        let edit_config = config.lock().unwrap().clone();
        let port = edit_config.port;
        Self {
            config,
            edit_config,
            printers,
            show_id,
            quit_id,
            visible: false,
            status_msg: format!("WebSocket 已启动: ws://127.0.0.1:{port}"),
            style_initialized: false,
            settings_open: None,
            running_port: port,
            port_text: port.to_string(),
        }
    }

    fn ensure_style(&mut self, ctx: &egui::Context) {
        if self.style_initialized {
            return;
        }
        self.style_initialized = true;

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(12.0, 10.0);
        style.spacing.button_padding = egui::vec2(16.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(24);
        style.spacing.interact_size = egui::vec2(40.0, 32.0);
        style.spacing.combo_width = 30.0;
        let cr = egui::CornerRadius::same(4);
        style.visuals.widgets.noninteractive.corner_radius = cr;
        style.visuals.widgets.inactive.corner_radius = cr;
        style.visuals.widgets.hovered.corner_radius = cr;
        style.visuals.widgets.active.corner_radius = cr;
        ctx.set_style(style);
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.visible = true;
        self.edit_config = self.config.lock().unwrap().clone();
        self.port_text = self.edit_config.port.to_string();
        if cfg!(target_os = "windows") {
            // On Windows, restore from off-screen position
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(100.0, 100.0)));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.visible = false;
        if cfg!(target_os = "windows") {
            // On Windows, move off-screen instead of Visible(false) to keep the event loop alive
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(-10000.0, -10000.0)));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn save_config(&mut self) {
        let mut config = self.config.lock().unwrap();
        *config = self.edit_config.clone();
        config.save();
        autostart::set_enabled(config.auto_start);
        self.status_msg = format!("WebSocket 已启动: ws://127.0.0.1:{}", self.running_port);
        tracing::info!("Config saved");
    }

    fn draw_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style())
                    .inner_margin(egui::Margin::same(28)),
            )
            .show(ctx, |ui| {
                self.draw_header(ui);
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(16.0);
                self.draw_port_config(ui);
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("文档类型 → 打印机 绑定关系")
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(12.0);
                self.draw_bindings_table(ui);
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);
                self.draw_action_bar(ui, ctx);
            });

        self.draw_settings_window(ctx);
    }

    fn draw_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("打印代理 - 配置").size(22.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(&self.status_msg)
                        .size(13.0)
                        .color(egui::Color32::from_rgb(80, 200, 80)),
                );
            });
        });
    }

    fn draw_port_config(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("监听端口:").size(15.0));
            ui.add_space(4.0);
            ui.add_sized(
                [80.0, 30.0],
                egui::TextEdit::singleline(&mut self.port_text)
                    .horizontal_align(egui::Align::Center)
                    .vertical_align(egui::Align::Center),
            );
            if let Ok(p) = self.port_text.parse::<u16>()
                && (1024..=65535).contains(&p)
            {
                self.edit_config.port = p;
            }
            ui.add_space(8.0);
            if self.edit_config.port != self.running_port {
                ui.label(
                    egui::RichText::new("⚠ 端口已变更")
                        .size(13.0)
                        .color(egui::Color32::from_rgb(255, 180, 60)),
                );
            }

            ui.add_space(20.0);
            let label = if self.edit_config.auto_start { "开" } else { "关" };
            ui.label(egui::RichText::new("开机自启:").size(15.0));
            ui.checkbox(&mut self.edit_config.auto_start, label);
        });
    }

    fn draw_bindings_table(&mut self, ui: &mut egui::Ui) {
        let mut to_remove = None;
        let mut to_open_settings = None;

        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 80.0)
            .show(ui, |ui| {
                egui::Grid::new("bindings_grid")
                    .num_columns(4)
                    .spacing([20.0, 12.0])
                    .striped(true)
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        for label in ["文档类型", "打印机", "打印参数", "操作"] {
                            ui.label(egui::RichText::new(label).size(14.0).strong());
                        }
                        ui.end_row();

                        for (i, binding) in
                            self.edit_config.bindings.iter_mut().enumerate()
                        {
                            draw_binding_row(ui, i, binding, &self.printers,
                                &mut to_remove, &mut to_open_settings);
                        }
                    });

                if let Some(i) = to_remove {
                    self.edit_config.bindings.remove(i);
                    if self.settings_open == Some(i) {
                        self.settings_open = None;
                    }
                }
                if let Some(i) = to_open_settings {
                    self.settings_open = Some(i);
                }

                ui.add_space(14.0);
                if ui
                    .button(egui::RichText::new("  ＋ 添加映射  ").size(14.0))
                    .clicked()
                {
                    self.edit_config.bindings.push(PrintBinding::default());
                }
            });
    }

    fn draw_action_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("   保  存   ")
                            .size(15.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(egui::Color32::from_rgb(55, 120, 230)),
                )
                .clicked()
            {
                self.save_config();
            }

            ui.add_space(12.0);

            if ui
                .button(egui::RichText::new("   取  消   ").size(15.0))
                .clicked()
            {
                self.edit_config = self.config.lock().unwrap().clone();
                self.port_text = self.edit_config.port.to_string();
                self.settings_open = None;
                self.hide_window(ctx);
            }

            if self.edit_config.port != self.running_port {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("端口已变更，需重启生效")
                        .size(13.0)
                        .color(egui::Color32::from_rgb(255, 180, 60)),
                );
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(" 重启应用 ")
                                .size(15.0)
                                .color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(220, 80, 60)),
                    )
                    .clicked()
                {
                    self.save_config();
                    restart_application();
                }
            }
        });
    }

    fn draw_settings_window(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.settings_open else {
            return;
        };
        if idx >= self.edit_config.bindings.len() {
            self.settings_open = None;
            return;
        }

        let binding = &self.edit_config.bindings[idx];
        let title = if binding.doc_type.is_empty() {
            format!("打印参数配置 - (未命名 #{})", idx + 1)
        } else {
            format!("打印参数配置 - {}", binding.doc_type)
        };

        let mut open = true;
        let window_frame = egui::Frame::window(&ctx.style()).inner_margin(egui::Margin {
            left: 20,
            right: 20,
            top: 10,
            bottom: 16,
        });
        egui::Window::new(title)
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .min_width(420.0)
            .frame(window_frame)
            .show(ctx, |ui| {
                let printer_for_bins = self.edit_config.bindings[idx].printer.clone();
                let settings = &mut self.edit_config.bindings[idx].settings;

                ui.add_space(8.0);
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([20.0, 16.0])
                    .min_col_width(100.0)
                    .show(ui, |ui| {
                        draw_paper_size_row(ui, settings);
                        draw_orientation_row(ui, settings);
                        draw_copies_row(ui, settings);
                        draw_duplex_row(ui, settings);
                        draw_color_mode_row(ui, settings);
                        draw_paper_source_row(ui, settings, &printer_for_bins);
                    });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("  确  定  ")
                                    .size(14.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(55, 120, 230)),
                        )
                        .clicked()
                    {
                        self.settings_open = None;
                    }

                    ui.add_space(8.0);

                    if ui
                        .button(egui::RichText::new("  恢复默认  ").size(14.0))
                        .clicked()
                    {
                        self.edit_config.bindings[idx].settings = PrintSettings::default();
                    }
                });
            });

        if !open {
            self.settings_open = None;
        }
    }
}

fn draw_binding_row(
    ui: &mut egui::Ui,
    i: usize,
    binding: &mut PrintBinding,
    printers: &[String],
    to_remove: &mut Option<usize>,
    to_open_settings: &mut Option<usize>,
) {
    ui.add_sized(
        [160.0, 30.0],
        egui::TextEdit::singleline(&mut binding.doc_type)
            .hint_text("如: 麻醉单")
            .font(egui::TextStyle::Body)
            .vertical_align(egui::Align::Center)
            .margin(egui::Margin::symmetric(8, 4)),
    );

    egui::ComboBox::from_id_salt(format!("printer_{i}"))
        .selected_text(if binding.printer.is_empty() {
            "请选择打印机"
        } else {
            &binding.printer
        })
        .width(240.0)
        .show_ui(ui, |ui| {
            for p in printers {
                ui.selectable_value(&mut binding.printer, p.clone(), p);
            }
        });

    let summary = settings_summary(&binding.settings);
    if ui
        .add_sized(
            [200.0, 30.0],
            egui::Button::new(
                egui::RichText::new(&summary)
                    .size(12.0)
                    .color(egui::Color32::from_rgb(160, 180, 220)),
            ),
        )
        .on_hover_text(format!("点击配置打印参数\n{summary}"))
        .clicked()
    {
        *to_open_settings = Some(i);
    }

    if ui
        .button(
            egui::RichText::new(" 删除 ").color(egui::Color32::from_rgb(255, 100, 100)),
        )
        .clicked()
    {
        *to_remove = Some(i);
    }
    ui.end_row();
}

fn draw_paper_size_row(ui: &mut egui::Ui, settings: &mut PrintSettings) {
    ui.label(egui::RichText::new("纸张大小").size(14.0));
    ui.horizontal(|ui| {
        let is_custom = matches!(settings.paper_size, PaperSize::Custom { .. });
        let current_label = settings.paper_size.short_label();

        egui::ComboBox::from_id_salt("paper_size")
            .selected_text(current_label)
            .width(240.0)
            .show_ui(ui, |ui| {
                for preset in &PaperSize::PRESETS {
                    if ui
                        .selectable_label(
                            !is_custom
                                && std::mem::discriminant(&settings.paper_size)
                                    == std::mem::discriminant(preset),
                            preset.label(),
                        )
                        .clicked()
                    {
                        settings.paper_size = preset.clone();
                    }
                }
                if ui
                    .selectable_label(is_custom, "自定义尺寸...")
                    .clicked()
                    && !is_custom
                {
                    settings.paper_size = PaperSize::Custom {
                        width_mm: 210.0,
                        height_mm: 297.0,
                    };
                }
            });
    });
    ui.end_row();

    if let PaperSize::Custom {
        width_mm,
        height_mm,
    } = &mut settings.paper_size
    {
        ui.label("");
        ui.horizontal(|ui| {
            draw_dimension_input(ui, "宽:", width_mm);
            ui.add_space(12.0);
            draw_dimension_input(ui, "高:", height_mm);
        });
        ui.end_row();
    }
}

fn draw_dimension_input(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.label(label);
    let mut text = format!("{}", *value as u32);
    let resp = ui.add_sized(
        [70.0, 28.0],
        egui::TextEdit::singleline(&mut text)
            .horizontal_align(egui::Align::Center)
            .vertical_align(egui::Align::Center),
    );
    if resp.changed()
        && let Ok(v) = text.parse::<f32>()
    {
        *value = v.clamp(50.0, 1000.0);
    }
    ui.label("mm");
}

fn draw_orientation_row(ui: &mut egui::Ui, settings: &mut PrintSettings) {
    ui.label(egui::RichText::new("打印方向").size(14.0));
    egui::ComboBox::from_id_salt("orientation")
        .selected_text(settings.orientation.label())
        .width(240.0)
        .show_ui(ui, |ui| {
            for o in &Orientation::ALL {
                ui.selectable_value(&mut settings.orientation, *o, o.label());
            }
        });
    ui.end_row();
}

fn draw_copies_row(ui: &mut egui::Ui, settings: &mut PrintSettings) {
    ui.label(egui::RichText::new("打印份数").size(14.0));
    let mut copies_str = settings.copies.to_string();
    let resp = ui.add_sized(
        [70.0, 28.0],
        egui::TextEdit::singleline(&mut copies_str)
            .horizontal_align(egui::Align::Center)
            .vertical_align(egui::Align::Center),
    );
    if resp.changed()
        && let Ok(v) = copies_str.parse::<u32>()
    {
        settings.copies = v.clamp(1, 99);
    }
    ui.end_row();
}

fn draw_duplex_row(ui: &mut egui::Ui, settings: &mut PrintSettings) {
    ui.label(egui::RichText::new("双面打印").size(14.0));
    let label = if settings.duplex { "开" } else { "关" };
    ui.checkbox(&mut settings.duplex, label);
    ui.end_row();
}

fn draw_color_mode_row(ui: &mut egui::Ui, settings: &mut PrintSettings) {
    ui.label(egui::RichText::new("色彩模式").size(14.0));
    egui::ComboBox::from_id_salt("color_mode")
        .selected_text(settings.color_mode.label())
        .width(240.0)
        .show_ui(ui, |ui| {
            for cm in &ColorMode::ALL {
                ui.selectable_value(&mut settings.color_mode, *cm, cm.label());
            }
        });
    ui.end_row();
}

fn draw_paper_source_row(ui: &mut egui::Ui, settings: &mut PrintSettings, printer: &str) {
    ui.label(egui::RichText::new("纸盒选择").size(14.0));
    let bins = printer::list_paper_bins(printer);
    let selected = if settings.paper_source.is_empty() {
        "自动"
    } else {
        &settings.paper_source
    };
    egui::ComboBox::from_id_salt("paper_source")
        .selected_text(selected)
        .width(240.0)
        .show_ui(ui, |ui| {
            for bin in &bins {
                ui.selectable_value(&mut settings.paper_source, bin.clone(), bin.as_str());
            }
        });
    ui.end_row();
}

fn settings_summary(s: &PrintSettings) -> String {
    let paper = match &s.paper_size {
        PaperSize::Custom { width_mm, height_mm } => {
            format!("{}×{}", *width_mm as u32, *height_mm as u32)
        }
        other => other.label().split(' ').next().unwrap_or("?").to_owned(),
    };
    let orient = s.orientation.label();
    let duplex = if s.duplex { "双面" } else { "单面" };
    let color = s.color_mode.label();

    format!("{paper}｜{orient}｜{duplex}｜{color}｜{}份", s.copies)
}

impl eframe::App for PrintAgentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_style(ctx);
        ctx.request_repaint_after(Duration::from_millis(200));

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if *event.id() == self.show_id {
                self.show_window(ctx);
            } else if *event.id() == self.quit_id {
                std::process::exit(0);
            }
        }

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                self.show_window(ctx);
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx);
        }

        self.draw_ui(ctx);
    }
}

fn restart_application() {
    let exe = std::env::current_exe().expect("Failed to get current executable path");
    tracing::info!("Restarting application: {}", exe.display());
    std::process::Command::new(exe)
        .spawn()
        .expect("Failed to restart application");
    std::process::exit(0);
}
