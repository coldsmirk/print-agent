use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub port: u16,
    pub auto_start: bool,
    pub bindings: Vec<PrintBinding>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrintBinding {
    pub doc_type: String,
    pub printer: String,
    pub settings: PrintSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintSettings {
    pub duplex: bool,
    pub copies: u32,
    pub orientation: Orientation,
    pub paper_size: PaperSize,
    pub color_mode: ColorMode,
    pub paper_source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl Orientation {
    pub const ALL: [Self; 2] = [Self::Portrait, Self::Landscape];

    pub fn label(self) -> &'static str {
        match self {
            Self::Portrait => "纵向",
            Self::Landscape => "横向",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PaperSize {
    A3,
    A4,
    A5,
    B5,
    Letter,
    Legal,
    Custom { width_mm: f32, height_mm: f32 },
}

impl PaperSize {
    pub const PRESETS: [Self; 6] = [
        Self::A3,
        Self::A4,
        Self::A5,
        Self::B5,
        Self::Letter,
        Self::Legal,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::A3 => "A3 (297×420mm)",
            Self::A4 => "A4 (210×297mm)",
            Self::A5 => "A5 (148×210mm)",
            Self::B5 => "B5 (176×250mm)",
            Self::Letter => "Letter (216×279mm)",
            Self::Legal => "Legal (216×356mm)",
            Self::Custom { .. } => "自定义",
        }
    }

    pub fn short_label(&self) -> String {
        match self {
            Self::Custom { width_mm, height_mm } => {
                format!("自定义 ({width_mm}×{height_mm}mm)")
            }
            other => other.label().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColorMode {
    Color,
    Grayscale,
    Mono,
}

impl ColorMode {
    pub const ALL: [Self; 3] = [Self::Color, Self::Grayscale, Self::Mono];

    pub fn label(self) -> &'static str {
        match self {
            Self::Color => "彩色",
            Self::Grayscale => "灰度",
            Self::Mono => "黑白",
        }
    }
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            duplex: false,
            copies: 1,
            orientation: Orientation::Portrait,
            paper_size: PaperSize::A4,
            color_mode: ColorMode::Mono,
            paper_source: "自动".to_owned(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: 2354,
            auto_start: false,
            bindings: Vec::new(),
        }
    }
}

impl AppConfig {
    fn config_path() -> PathBuf {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("print-agent");
        std::fs::create_dir_all(&dir).ok();
        dir.join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            std::fs::write(path, json).ok();
        }
    }
}
