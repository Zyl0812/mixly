//! 播放页封面：内嵌 logo PNG，按终端可用区域实时缩放渲染成半格块字符画。
//!
//! 终端格宽高比约 1:2，因此每格对应图像里 1×2 的像素块，上下半格分别用
//! ▀ ▄ █ 空格表示。任意终端尺寸下图形比例都正确。

use std::sync::OnceLock;

static PNG: &[u8] = include_bytes!("logo.png");

/// 半格判定为「有墨」的最低覆盖率。线条显示不全就调低，噪点多就调高。
const INK_COVERAGE: f64 = 0.30; // ponytail: 经验值旋钮，观感不对时微调

/// 渲染出的字符画最大列数（再宽也没有更多细节，只会占掉歌词空间）。
const MAX_COLS: u16 = 44;

struct Mask {
    w: u32,
    h: u32,
    ink: Vec<bool>,
}

fn build_mask() -> Option<Mask> {
    let img = image::load_from_memory(PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    // 墨 = 不透明且偏暗（黑色线稿；透明/白底都不算）
    let ink: Vec<bool> = img
        .pixels()
        .map(|p| p.0[3] >= 128 && (p.0[0] as u32 + p.0[1] as u32 + p.0[2] as u32) / 3 < 128)
        .collect();

    // 裁掉四周空白，让可用面积全部留给图形本体
    let mut x0 = w;
    let mut y0 = h;
    let mut x1 = 0u32;
    let mut y1 = 0u32;
    for y in 0..h {
        for x in 0..w {
            if ink[(y * w + x) as usize] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < x0 {
        return None; // 全空图
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut cropped = Vec::with_capacity((cw * ch) as usize);
    for y in y0..=y1 {
        for x in x0..=x1 {
            cropped.push(ink[(y * w + x) as usize]);
        }
    }
    Some(Mask {
        w: cw,
        h: ch,
        ink: cropped,
    })
}

fn mask() -> Option<&'static Mask> {
    static CELL: OnceLock<Option<Mask>> = OnceLock::new();
    CELL.get_or_init(build_mask).as_ref()
}

/// 在 `max_w` 列 × `max_h` 行内渲染 logo；区域太小返回空。
/// `ascii` 为 true 时用纯 ASCII 字符（等宽字体块字符错位时的回退）。
pub fn render(max_w: u16, max_h: u16, ascii: bool) -> Vec<String> {
    let Some(m) = mask() else {
        return Vec::new();
    };
    if max_w < 16 || max_h < 8 {
        return Vec::new();
    }
    let max_w = max_w.min(MAX_COLS) as f64;
    let max_h = max_h as f64;
    // s = 每列对应的图像像素数；每行对应 2s（格子 1:2）
    let s = (m.w as f64 / max_w)
        .max(m.h as f64 / (2.0 * max_h))
        .max(1.0);
    let cols = ((m.w as f64 / s) as u32).max(1);
    let rows = ((m.h as f64 / (2.0 * s)).ceil() as u32).max(1);

    let mut out = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut line = String::with_capacity(cols as usize * 3);
        for col in 0..cols {
            let xa = col as f64 * s;
            let xb = xa + s;
            let ya = row as f64 * 2.0 * s;
            let top = coverage(m, xa, xb, ya, ya + s) >= INK_COVERAGE;
            let bot = coverage(m, xa, xb, ya + s, ya + 2.0 * s) >= INK_COVERAGE;
            line.push(match (top, bot, ascii) {
                (true, true, false) => '█',
                (true, false, false) => '▀',
                (false, true, false) => '▄',
                (true, true, true) => '#',
                (true, false, true) => '"',
                (false, true, true) => '_',
                (false, false, _) => ' ',
            });
        }
        out.push(line);
    }
    out
}

/// 像素块内的墨覆盖率（越界部分按图像边缘截断）。
fn coverage(m: &Mask, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    let xa = x0 as u32;
    let xb = (x1.ceil() as u32).min(m.w);
    let ya = y0 as u32;
    let yb = (y1.ceil() as u32).min(m.h);
    if xa >= xb || ya >= yb {
        return 0.0;
    }
    let mut hit = 0u32;
    let mut total = 0u32;
    for y in ya..yb {
        for x in xa..xb {
            total += 1;
            if m.ink[(y * m.w + x) as usize] {
                hit += 1;
            }
        }
    }
    hit as f64 / total as f64
}

/// 字符画实际宽度（列数），用于布局分配。
pub fn art_width(art: &[String]) -> u16 {
    art.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_fits_and_uses_half_blocks() {
        let art = render(60, 30, false);
        assert!(!art.is_empty());
        for row in &art {
            assert!(row.chars().count() <= 60, "row too wide: {}", row.len());
            assert!(row.chars().all(|c| matches!(c, ' ' | '▀' | '▄' | '█')));
        }
        // 至少要画出点东西
        assert!(art.iter().any(|r| r.chars().any(|c| c != ' ')));
    }

    #[test]
    fn ascii_mode_is_pure_ascii() {
        let art = render(60, 30, true);
        assert!(!art.is_empty());
        for row in &art {
            assert!(row.is_ascii(), "非 ASCII 行: {row}");
        }
    }

    #[test]
    fn tiny_area_renders_nothing() {
        assert!(render(10, 5, false).is_empty());
        assert!(render(0, 0, true).is_empty());
    }

    #[test]
    fn aspect_roughly_preserved() {
        // 原图接近方形 → 列数 ≈ 2 × 行数
        let art = render(60, 30, false);
        let w = art_width(&art) as f64;
        let h = art.len() as f64;
        assert!((w / (2.0 * h) - 1.0).abs() < 0.4, "w={w} h={h}");
    }
}
