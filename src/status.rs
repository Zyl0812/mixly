//! 播放状态快照与 Claude Code status line 渲染。
//!
//! `play` 循环每 400ms 写入 `%APPDATA%\mixly\now-playing.json`；
//! `mixly status --claude` 读取并渲染两行/三行 ANSI 文本。本模块不含任何网络或播放器初始化。

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// now-playing.json 协议版本。
pub const NOW_PLAYING_VERSION: u32 = 1;
/// 渲染时超过该毫秒数未更新视为过期，不显示旧歌曲。
pub const STALE_MS: u64 = 3000;
/// 轨道最长显示格数。
pub const TRACK_MAX: usize = 28;
/// 两行模式需要轨道至少 12 格；放不下则切三行。
pub const TRACK_MIN_TWO_LINE: usize = 12;

/// 珊瑚橘（spec 指定 #D97757）。
const CORAL: &str = "\x1b[38;2;217;119;87m";
const GRAY: &str = "\x1b[90m";
const RESET: &str = "\x1b[0m";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NowPlaying {
    pub version: u32,
    pub pid: u32,
    pub updated_at_ms: u64,
    pub platform: String,
    pub song_id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub paused: bool,
    pub lyric: Option<String>,
}

impl NowPlaying {
    pub fn stale(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.updated_at_ms) > STALE_MS
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 原子写入状态文件（临时文件 + rename，避免读取方看到半个文件）。
pub fn write_now_playing(path: &Path, np: &NowPlaying) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string(np)?;
    fs::write(&tmp, data).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("commit {}", path.display()))?;
    Ok(())
}

fn read_raw(path: &Path) -> Option<NowPlaying> {
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// 读取未过期的状态；缺失、损坏、正在写入或过期都返回 None（调用方按“无播放”处理）。
pub fn read_now_playing(path: &Path) -> Option<NowPlaying> {
    let np = read_raw(path)?;
    if np.stale(now_ms()) {
        return None;
    }
    Some(np)
}

/// 播放进程正常退出时清理：仅当状态文件的 pid 仍属于当前进程才删除，
/// 避免误删后来启动的播放器状态。
pub fn clear_now_playing(path: &Path, pid: u32) {
    if let Some(np) = read_raw(path) {
        if np.pid == pid {
            let _ = fs::remove_file(path);
        }
    }
}

// ── Claude Code status line 渲染（纯函数） ──────────────────────────────

/// 输入状态和终端列宽，输出两行或三行 ANSI 文本。
pub fn render_claude_status(np: &NowPlaying, columns: usize) -> String {
    let columns = columns.max(1);
    let song = song_line(np);
    let times = times_text(np.position_secs, np.duration_secs);
    let status = if np.paused { "⏸" } else { "▶" };
    let pct = percent(np.position_secs, np.duration_secs);
    let pct_text = format!("{pct}%");

    // 首行始终是「歌曲 + 状态 + 时间」；两行模式额外在首行追加轨道与百分比
    let head = format!("{song}  {status} {times}");
    let lyric = lyric_line(np, columns);

    // 两行模式所需最小宽度：首行 + 2 + 12 格轨道 + 1 + 百分比
    let fixed = str_width(&head) + 2 + TRACK_MIN_TWO_LINE + 1 + str_width(&pct_text);
    if fixed <= columns {
        let track_n =
            TRACK_MAX.min(columns.saturating_sub(str_width(&head) + 2 + 1 + str_width(&pct_text)));
        let line1 = format!(
            "{head}  {} {pct_text}",
            color_track(render_track(np.position_secs, np.duration_secs, track_n))
        );
        format!("{line1}\n{lyric}")
    } else {
        // 三行：首行（必要时截断）/ 轨道+百分比 / 歌词
        let track_n = TRACK_MAX.min(columns.saturating_sub(1 + str_width(&pct_text)));
        let line1 = trunc(&head, columns);
        let line2 = format!(
            "{} {pct_text}",
            color_track(render_track(np.position_secs, np.duration_secs, track_n))
        );
        format!("{line1}\n{line2}\n{lyric}")
    }
}

fn song_line(np: &NowPlaying) -> String {
    let artists = if np.artists.is_empty() {
        String::new()
    } else {
        format!(" — {}", np.artists.join("、"))
    };
    format!("♫ {}{}", np.title, artists)
}

fn lyric_line(np: &NowPlaying, columns: usize) -> String {
    let lyric = np
        .lyric
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("暂无歌词");
    trunc(&format!("♪ {lyric}"), columns.saturating_sub(1))
}

fn times_text(pos: f64, dur: f64) -> String {
    let pos_s = safe_fmt_time(pos);
    let dur_s = if dur.is_finite() && dur > 0.0 {
        safe_fmt_time(dur)
    } else {
        "--:--".to_string()
    };
    format!("{pos_s} / {dur_s}")
}

fn safe_fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "--:--".to_string();
    }
    let total = secs as u64;
    let s = total % 60;
    let m = (total / 60) % 60;
    let h = total / 3600;
    if h > 0 {
        format!("{h}:{:02}:{:02}", m, s)
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn percent(pos: f64, dur: f64) -> u64 {
    if !pos.is_finite() || !dur.is_finite() || dur <= 0.0 {
        return 0;
    }
    ((pos / dur).clamp(0.0, 1.0) * 100.0).round() as u64
}

/// 轨道长度为 N 时，游标位置为 round(progress × (N-1))。
fn render_track(pos: f64, dur: f64, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let progress = if pos.is_finite() && dur.is_finite() && dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cursor = ((progress * (n - 1) as f64).round() as usize).min(n - 1);
    let mut out = String::new();
    for i in 0..n {
        if i == cursor {
            out.push('●');
        } else {
            out.push('━');
        }
    }
    out
}

fn color_track(track: String) -> String {
    if no_color() {
        return track;
    }
    // 已播放部分 + 游标为珊瑚橘；未播放部分为可见灰色（不用纯黑）
    let Some(cursor) = track.find('●') else {
        return format!("{GRAY}{track}{RESET}");
    };
    let (filled, rest) = track.split_at(cursor + '●'.len_utf8());
    format!("{CORAL}{filled}{RESET}{GRAY}{rest}{RESET}")
}

fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}

// ── 宽度工具（复用 unicode-width，不按字符数截断中文） ──────────────────

fn str_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// 按终端列宽截断，超出补 `…`；不会拆坏 UTF-8 字符。
fn trunc(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if str_width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn np() -> NowPlaying {
        NowPlaying {
            version: NOW_PLAYING_VERSION,
            pid: 42,
            updated_at_ms: now_ms(),
            platform: "qq".into(),
            song_id: "x".into(),
            title: "晴天".into(),
            artists: vec!["周杰伦".into()],
            position_secs: 134.0,
            duration_secs: 269.0,
            paused: false,
            lyric: Some("从前从前，有个人爱你很久……".into()),
        }
    }

    #[test]
    fn percent_68_rounds() {
        assert_eq!(percent(183.0, 269.0), 68);
        assert_eq!(percent(0.0, 269.0), 0);
        assert_eq!(percent(269.0, 269.0), 100);
        assert_eq!(percent(500.0, 269.0), 100);
        assert_eq!(percent(-1.0, 269.0), 0);
        assert_eq!(percent(10.0, 0.0), 0);
        assert_eq!(percent(f64::NAN, 269.0), 0);
    }

    #[test]
    fn track_cursor_at_round_position() {
        // 68% on 28 cells → round(0.68 * 27) = round(18.36) = 18
        let t = render_track(183.0, 269.0, 28);
        assert_eq!(t.chars().count(), 28);
        assert_eq!(t.chars().filter(|&c| c == '●').count(), 1);
        assert_eq!(t.chars().position(|c| c == '●'), Some(18));
        // 0% → cursor at 0
        let t0 = render_track(0.0, 269.0, 12);
        assert_eq!(t0.chars().position(|c| c == '●'), Some(0));
        // 100% → cursor at last
        let t100 = render_track(269.0, 269.0, 12);
        assert_eq!(t100.chars().position(|c| c == '●'), Some(11));
        // unknown duration → 0%
        let t_unk = render_track(10.0, 0.0, 12);
        assert_eq!(t_unk.chars().position(|c| c == '●'), Some(0));
        // 0 格 → 空
        assert_eq!(render_track(10.0, 100.0, 0), "");
    }

    #[test]
    fn wide_terminal_two_lines() {
        let s = render_claude_status(&np(), 120);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("♫ 晴天 — 周杰伦  ▶ 02:14 / 04:29  "));
        // 134/269 ≈ 49.8% → 50%
        assert!(lines[0].ends_with(" 50%"), "line was: {}", lines[0]);
        // 时间后固定两空格，进度条立即开始（没有额外的右对齐空白）
        let after_times = "02:14 / 04:29  ";
        let idx = lines[0].find(after_times).unwrap();
        let after = &lines[0][idx + after_times.len()..];
        assert!(!after.starts_with(' ')); // 紧接轨道
                                          // 轨道不超过 28 格（去掉 ANSI 后按显示宽度数）
        let visible = strip_ansi(lines[0]);
        let track_start = visible.find("02:14 / 04:29  ").unwrap() + "02:14 / 04:29  ".len();
        let seg = &visible[track_start..];
        let before_pct = &seg[..seg.find('%').unwrap()];
        let track_seg = before_pct.split_whitespace().next().unwrap();
        assert!(
            UnicodeWidthStr::width(track_seg) <= TRACK_MAX,
            "track display width {}",
            UnicodeWidthStr::width(track_seg)
        );
        assert!(lines[1].starts_with("♪ "));
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\x1b' {
                // 跳过 ESC [ ... m
                while let Some(&n) = it.peek() {
                    it.next();
                    if n == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn narrow_terminal_three_lines() {
        let s = render_claude_status(&np(), 30);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("♫ "));
        assert!(lines[1].contains('%'));
        assert!(lines[2].starts_with("♪ "));
    }

    #[test]
    fn no_color_strips_ansi() {
        let has_color = {
            std::env::remove_var("NO_COLOR");
            let s = render_claude_status(&np(), 120);
            s.contains("\x1b[")
        };
        let no_color = {
            std::env::set_var("NO_COLOR", "1");
            let s = render_claude_status(&np(), 120);
            s.contains("\x1b[")
        };
        std::env::remove_var("NO_COLOR");
        assert!(has_color);
        assert!(!no_color);
    }

    #[test]
    fn paused_uses_pause_symbol() {
        let mut p = np();
        p.paused = true;
        let s = render_claude_status(&p, 120);
        assert!(s.contains("⏸"));
        let p2 = np();
        assert!(render_claude_status(&p2, 120).contains("▶"));
    }

    #[test]
    fn unknown_duration_does_not_panic() {
        let mut p = np();
        p.duration_secs = 0.0;
        p.position_secs = f64::NAN;
        let s = render_claude_status(&p, 40);
        assert!(s.contains("--:--"));
        assert!(s.contains("0%"));
    }

    #[test]
    fn cjk_truncation_keeps_utf8_and_ellipsis() {
        let mut p = np();
        p.title = "周杰伦的一首非常非常长的歌名".into();
        let s = render_claude_status(&p, 10);
        assert!(std::str::from_utf8(s.as_bytes()).is_ok());
        assert!(s.contains('…'));
    }

    #[test]
    fn stale_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("now-playing.json");
        let mut p = np();
        p.updated_at_ms = now_ms().saturating_sub(STALE_MS + 1000);
        write_now_playing(&path, &p).unwrap();
        assert!(read_now_playing(&path).is_none());
    }

    #[test]
    fn clear_only_removes_own_pid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("now-playing.json");
        write_now_playing(&path, &np()).unwrap();
        clear_now_playing(&path, 999); // 别的进程
        assert!(path.exists());
        clear_now_playing(&path, 42); // 本进程
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("now-playing.json");
        std::fs::write(&path, "not json{").unwrap();
        assert!(read_now_playing(&path).is_none());
    }
}
