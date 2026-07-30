//! Robust LRC parsing and current-line tracking.

use crate::models::LyricsState;

/// Parse LRC text into (timestamp_seconds, text) pairs, sorted by time.
///
/// Tolerates:
/// - blank lines
/// - malformed timestamps (skipped)
/// - multiple time tags on one line
/// - optional hours, fractional seconds of varying precision
pub fn parse_lrc(input: &str) -> Vec<(f64, String)> {
    let mut lines = Vec::new();
    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // metadata tags like [ar:...] [ti:...] — skip if not timestamps
        let mut rest = line;
        let mut tags: Vec<f64> = Vec::new();
        while rest.starts_with('[') {
            if let Some(end) = rest.find(']') {
                let inner = &rest[1..end];
                if let Some(ts) = parse_timestamp(inner) {
                    tags.push(ts);
                } else if tags.is_empty() && !inner.is_empty() {
                    // pure metadata line
                    break;
                }
                rest = &rest[end + 1..];
            } else {
                break;
            }
        }
        let text = rest.trim();
        if tags.is_empty() {
            continue;
        }
        // keep empty text (instrumental markers) if tagged
        for ts in tags {
            lines.push((ts, text.to_string()));
        }
    }
    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    lines
}

/// Parse `mm:ss`, `mm:ss.xx`, `mm:ss.xxx`, or `h:mm:ss.xx`.
fn parse_timestamp(s: &str) -> Option<f64> {
    // Must look like time: contains ':' and starts with digit
    let s = s.trim();
    if s.is_empty() || !s.chars().next()?.is_ascii_digit() {
        return None;
    }
    // Reject pure metadata like `ar:Artist` (no second numeric part pattern)
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    // last part is ss.xx
    let sec_part = parts.last()?;
    if !sec_part
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.')
    {
        return None;
    }
    for p in &parts[..parts.len() - 1] {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }

    let seconds: f64 = sec_part.parse().ok()?;
    if parts.len() == 2 {
        let minutes: f64 = parts[0].parse().ok()?;
        Some(minutes * 60.0 + seconds)
    } else {
        let hours: f64 = parts[0].parse().ok()?;
        let minutes: f64 = parts[1].parse().ok()?;
        Some(hours * 3600.0 + minutes * 60.0 + seconds)
    }
}

pub fn lyrics_state_from_lrc(input: &str) -> LyricsState {
    LyricsState {
        lines: parse_lrc(input),
        current_index: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_lrc() {
        let lrc = r#"
[ti:Test]
[00:01.00]Hello
[00:05.50]World
"#;
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert!((lines[0].0 - 1.0).abs() < 0.001);
        assert_eq!(lines[0].1, "Hello");
        assert!((lines[1].0 - 5.5).abs() < 0.001);
    }

    #[test]
    fn multi_tag_line() {
        let lrc = "[00:10.00][00:20.00]Twice";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].1, "Twice");
        assert_eq!(lines[1].1, "Twice");
        assert!((lines[0].0 - 10.0).abs() < 0.001);
        assert!((lines[1].0 - 20.0).abs() < 0.001);
    }

    #[test]
    fn skips_malformed_and_blank() {
        let lrc = r#"
[bad]
[00:xx.00]nope

[01:02.03]ok
[99:99]also ok-ish
"#;
        let lines = parse_lrc(lrc);
        assert!(lines.iter().any(|(_, t)| t == "ok"));
        // 99:99 → 99*60+99 = 6039
        assert!(lines.iter().any(|(t, text)| text == "also ok-ish" && *t > 6000.0));
    }

    #[test]
    fn current_line_tracking() {
        let mut state = lyrics_state_from_lrc("[00:00.00]a\n[00:10.00]b\n[00:20.00]c\n");
        state.update_index(0.0);
        assert_eq!(state.current_line(), Some("a"));
        state.update_index(10.5);
        assert_eq!(state.current_line(), Some("b"));
        state.update_index(100.0);
        assert_eq!(state.current_line(), Some("c"));
    }

    #[test]
    fn fractional_and_hours() {
        let lines = parse_lrc("[1:02:03.5]long");
        assert_eq!(lines.len(), 1);
        assert!((lines[0].0 - (3600.0 + 120.0 + 3.5)).abs() < 0.001);
    }
}
