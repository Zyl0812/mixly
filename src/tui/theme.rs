//! Organic 配色映射到终端（真彩色）。终端不支持 24bit 时会自动降级到最近的 256 色。

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(0xf5, 0xea, 0xd8); // --color-bg
pub const SURFACE: Color = Color::Rgb(0xeb, 0xdd, 0xc5); // --color-surface
pub const RAISED: Color = Color::Rgb(0xf9, 0xf4, 0xed); // --color-neutral-100
pub const TEXT: Color = Color::Rgb(0x20, 0x1e, 0x1d); // --color-text
pub const DIM: Color = Color::Rgb(0x64, 0x5c, 0x50); // --color-neutral-700
pub const FAINT: Color = Color::Rgb(0xa1, 0x97, 0x86); // --color-neutral-500
pub const LINE: Color = Color::Rgb(0xdc, 0xd3, 0xc4); // --color-neutral-300
pub const ACCENT: Color = Color::Rgb(0xc6, 0x71, 0x39); // --color-accent
pub const ACCENT_DEEP: Color = Color::Rgb(0x8c, 0x49, 0x1a); // --color-accent-700
pub const ACCENT_SOFT: Color = Color::Rgb(0xff, 0xe1, 0xd0); // --color-accent-200
pub const ACCENT_TAG: Color = Color::Rgb(0xf6, 0xa0, 0x6b); // --color-accent-400
pub const SAGE: Color = Color::Rgb(0x8f, 0xa0, 0x73); // --color-accent-2-500
pub const SAGE_DEEP: Color = Color::Rgb(0x56, 0x63, 0x3f); // --color-accent-2-700
pub const TAG_BG: Color = Color::Rgb(0xee, 0xe7, 0xdb); // --color-neutral-200

pub fn base() -> Style {
    Style::default().bg(BG).fg(TEXT)
}
pub fn dim() -> Style {
    Style::default().fg(DIM)
}
pub fn faint() -> Style {
    Style::default().fg(FAINT)
}
pub fn accent() -> Style {
    Style::default().fg(ACCENT_DEEP)
}
pub fn bold_accent() -> Style {
    Style::default().fg(ACCENT_DEEP).add_modifier(Modifier::BOLD)
}
pub fn line() -> Style {
    Style::default().fg(LINE)
}
/// 顶栏 / 底栏的浅色带
pub fn strip() -> Style {
    Style::default().bg(RAISED).fg(DIM)
}
/// 播放条底色
pub fn player_strip() -> Style {
    Style::default().bg(SURFACE).fg(TEXT)
}
/// 列表选中行
pub fn selected() -> Style {
    Style::default().bg(ACCENT_SOFT).fg(TEXT)
}
/// 面板标题：聚焦时用赭石色，否则中性
pub fn pane_title(focused: bool) -> Style {
    if focused {
        Style::default().fg(ACCENT_DEEP).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(FAINT)
    }
}
