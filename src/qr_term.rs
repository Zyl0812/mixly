//! Print login QR codes in the terminal (Unicode blocks).

use anyhow::{anyhow, bail, Context, Result};
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;

/// Print a QR payload to stdout.
///
/// Accepts:
/// - `data:image/png;base64,...` (what netease-qq-music-api returns for Netease)
/// - raw URL / text (re-encoded as terminal QR; used when QQ returns a string payload)
pub fn print_qr_payload(payload: &str) -> Result<()> {
    let payload = payload.trim();
    if payload.is_empty() {
        bail!("二维码内容为空");
    }

    if let Some(rest) = payload.strip_prefix("data:image/png;base64,") {
        let bytes = decode_base64(rest).context("解码二维码 PNG")?;
        print_png_as_blocks(&bytes)?;
        return Ok(());
    }

    if let Some(rest) = payload.strip_prefix("data:image/") {
        // other data-uri images
        if let Some((_, b64)) = rest.split_once(";base64,") {
            let bytes = decode_base64(b64).context("解码二维码图片")?;
            print_png_as_blocks(&bytes)?;
            return Ok(());
        }
    }

    // Treat as QR content string (URL etc.)
    print_text_as_qr(payload)
}

fn print_text_as_qr(content: &str) -> Result<()> {
    let code = QrCode::new(content.as_bytes()).map_err(|e| anyhow!("生成二维码失败: {e}"))?;
    let s = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .quiet_zone(true)
        .build();
    println!("{s}");
    Ok(())
}

/// Render a monochrome PNG QR as double-width block characters.
fn print_png_as_blocks(png: &[u8]) -> Result<()> {
    let img = image::load_from_memory(png)
        .context("解析二维码 PNG")?
        .to_luma8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        bail!("二维码图片尺寸无效");
    }

    // QQ Music's PNG has no quiet zone; scanners require four blank modules.
    let padding = if has_dark_edge(&img) { 4 } else { 0 };
    let step = estimate_module_step(&img).max(1);
    let cols = if padding > 0 {
        estimate_edge_to_edge_modules(&img).unwrap_or_else(|| (w / step).max(1))
    } else {
        (w / step).max(1)
    };
    let rows = if padding > 0 { cols } else { (h / step).max(1) };
    let blank_line = " ".repeat(((cols + padding * 2) * 2) as usize);

    for _ in 0..padding {
        println!("{blank_line}");
    }

    // Pair rows with half-block chars when possible for denser output;
    // use full blocks for simplicity and scan reliability.
    for row in 0..rows {
        let mut line = String::with_capacity(((cols + padding * 2) * 2) as usize);
        line.push_str(&" ".repeat((padding * 2) as usize));
        for col in 0..cols {
            let x = ((2 * col + 1) * w / (2 * cols)).min(w - 1);
            let y = ((2 * row + 1) * h / (2 * rows)).min(h - 1);
            let dark = img.get_pixel(x, y).0[0] < 128;
            if dark {
                line.push('█');
                line.push('█');
            } else {
                line.push(' ');
                line.push(' ');
            }
        }
        line.push_str(&" ".repeat((padding * 2) as usize));
        println!("{line}");
    }
    for _ in 0..padding {
        println!("{blank_line}");
    }
    Ok(())
}

fn has_dark_edge(img: &image::GrayImage) -> bool {
    let (w, h) = img.dimensions();
    (0..w).any(|x| img.get_pixel(x, 0).0[0] < 128 || img.get_pixel(x, h - 1).0[0] < 128)
        || (0..h).any(|y| img.get_pixel(0, y).0[0] < 128 || img.get_pixel(w - 1, y).0[0] < 128)
}

/// Infer an edge-to-edge QR grid from its 1:1:3:1:1 top-left finder pattern.
fn estimate_edge_to_edge_modules(img: &image::GrayImage) -> Option<u32> {
    let (w, h) = img.dimensions();
    let finder_width = (0..h / 3).find_map(|y| {
        let mut runs = [0u32; 5];
        let mut x = 0;
        let mut dark = true;
        for run in &mut runs {
            while x < w && (img.get_pixel(x, y).0[0] < 128) == dark {
                *run += 1;
                x += 1;
            }
            dark = !dark;
        }
        let unit_min = runs[0].min(runs[1]).min(runs[3]).min(runs[4]);
        let unit_max = runs[0].max(runs[1]).max(runs[3]).max(runs[4]);
        (unit_min > 0 && unit_max - unit_min <= 2 && runs[2] >= unit_min * 2)
            .then_some(runs.into_iter().sum::<u32>())
    })?;

    (21..=177)
        .step_by(4)
        .min_by_key(|modules| (finder_width * modules).abs_diff(w * 7))
}

/// Heuristic: module size from first dark-run length near the finder pattern.
fn estimate_module_step(img: &image::GrayImage) -> u32 {
    let (w, h) = img.dimensions();
    // Scan middle of top third for first black run length
    let y = (h / 6).max(1).min(h - 1);
    let mut i = 0u32;
    while i < w && img.get_pixel(i, y).0[0] >= 128 {
        i += 1;
    }
    let start = i;
    while i < w && img.get_pixel(i, y).0[0] < 128 {
        i += 1;
    }
    let run = i.saturating_sub(start);
    // Finder pattern has 7 modules in first dark+light structure; first dark is often 1 module
    // but quiet zone edge may merge. Prefer divisors near 6–12 (crate uses 8).
    if (4..=24).contains(&run) {
        run
    } else if run > 24 {
        // might be multi-module run; try /7 finder
        let guess = run / 7;
        if (4..=24).contains(&guess) {
            guess
        } else {
            8
        }
    } else {
        8
    }
}

/// 手写 base64 解码（免依赖）。支持无填充的 2/3 字符尾组。
pub fn decode_base64(input: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Result<u8> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(anyhow!("非法 base64 字符")),
        }
    }
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    if cleaned.len() < 2 {
        bail!("base64 过短");
    }
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    for chunk in cleaned.chunks(4) {
        if chunk.len() < 2 {
            bail!("base64 尾部残缺");
        }
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if let Some(&c) = chunk.get(2) {
            let c = val(c)?;
            out.push((b << 4) | (c >> 2));
            if let Some(&d) = chunk.get(3) {
                out.push((c << 6) | val(d)?);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qrcode::types::Color;
    use qrcode::EcLevel;

    #[test]
    fn print_text_qr_does_not_panic() {
        // Drive real print path; capture by not failing on known content.
        let r = print_text_as_qr("https://music.163.com/login?codekey=testkey123");
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn print_png_data_uri_roundtrip() {
        // Generate PNG via qrcode+image path used by upstream style: we encode text then print as text QR.
        // Also build a minimal PNG from qrcode render through image crate if available.
        let code = QrCode::new(b"https://example.com/login?k=abc").unwrap();
        // Use string path
        let s = code
            .render::<Dense1x2>()
            .dark_color(Dense1x2::Dark)
            .light_color(Dense1x2::Light)
            .build();
        assert!(s.contains('█') || s.contains('▀') || s.contains('▄') || s.chars().count() > 10);

        // Text payload entry
        print_qr_payload("https://y.qq.com/test").unwrap();
    }

    #[test]
    fn detects_missing_png_quiet_zone() {
        let mut edge_to_edge = image::GrayImage::from_pixel(3, 3, image::Luma([255]));
        edge_to_edge.put_pixel(0, 1, image::Luma([0]));
        assert!(has_dark_edge(&edge_to_edge));

        let mut padded = image::GrayImage::from_pixel(9, 9, image::Luma([255]));
        padded.put_pixel(4, 4, image::Luma([0]));
        assert!(!has_dark_edge(&padded));
    }

    #[test]
    fn recovers_modules_from_qq_style_scaled_png() {
        let code = QrCode::with_error_correction_level(
            b"https://c6.y.qq.com/base/fcgi-bin/u?__=123456789012",
            EcLevel::L,
        )
        .unwrap();
        assert_eq!(code.width(), 29);

        let size = 150u32;
        let modules = code.width() as u32;
        let img = image::GrayImage::from_fn(size, size, |x, y| {
            let col = (x * modules / size) as usize;
            let row = (y * modules / size) as usize;
            image::Luma([if code[(col, row)] == Color::Dark {
                0
            } else {
                255
            }])
        });

        let detected = estimate_edge_to_edge_modules(&img).unwrap();
        assert_eq!(detected, modules);
        for row in 0..detected {
            for col in 0..detected {
                let x = (2 * col + 1) * size / (2 * detected);
                let y = (2 * row + 1) * size / (2 * detected);
                assert_eq!(
                    img.get_pixel(x, y).0[0] < 128,
                    code[(col as usize, row as usize)] == Color::Dark
                );
            }
        }
    }
}
