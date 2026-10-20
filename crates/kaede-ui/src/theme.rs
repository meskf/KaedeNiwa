//! 主题 token。深浅双模式。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Light,
    Dark,
}

impl Mode {
    pub fn as_attr(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub outer_bg: &'static str,
    pub inner_bg: &'static str,
    pub fg: &'static str,
    pub fg_muted: &'static str,
    pub fg_faint: &'static str,
    pub accent_bg: &'static str,
    pub accent_fg: &'static str,
    pub hover_bg: &'static str,
    pub active_bg: &'static str,
    pub positive: &'static str,
    pub negative: &'static str,
}

pub const LIGHT: Palette = Palette {
    outer_bg: "#F5F4F2",
    inner_bg: "#FFFFFF",
    fg: "#0A0A0A",
    fg_muted: "#52525B",
    fg_faint: "#A1A1AA",
    accent_bg: "#0A0A0A",
    accent_fg: "#FFFFFF",
    hover_bg: "#F4F4F5",
    active_bg: "#E4E4E7",
    positive: "#16A34A",
    negative: "#DC2626",
};

pub const DARK: Palette = Palette {
    outer_bg: "#0A0A0A",
    inner_bg: "#141414",
    fg: "#FAFAFA",
    fg_muted: "#A1A1AA",
    fg_faint: "#52525B",
    accent_bg: "#FAFAFA",
    accent_fg: "#0A0A0A",
    hover_bg: "#1F1F1F",
    active_bg: "#2A2A2A",
    positive: "#22C55E",
    negative: "#EF4444",
};

pub const FONT_STACK: &str = r#""Times New Roman", "FangSong", "仿宋", "STFangsong", "KaiTi", "楷体", "Source Han Serif SC", "Noto Serif CJK SC", serif"#;

#[allow(dead_code)]
pub mod radius {
    pub const OUTER: &str = "20px";
    pub const INNER: &str = "16px";
    pub const ITEM: &str = "10px";
    pub const PILL: &str = "9999px";
}

#[allow(dead_code)]
pub mod space {
    pub const XS: &str = "4px";
    pub const SM: &str = "8px";
    pub const MD: &str = "12px";
    pub const LG: &str = "16px";
    pub const XL: &str = "24px";
}

#[allow(unused)]
pub fn palette_for(mode: Mode) -> Palette {
    match mode {
        Mode::Light => LIGHT,
        Mode::Dark => DARK,
    }
}
