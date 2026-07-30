//! 合并版渲染：列表模式（密度优先）⇄ 播放页（大歌词），共用顶栏、播放条与快捷键行。
//!
//! 版式（自上而下）：
//! ```text
//! 顶栏 1 行   MIXLY · 代理 · 音质 · 账号 · 视图        ⣾ 状态
//! ────────────────────────────────────────────────────────────
//! 主体        列表：歌单 | 歌曲(+滚动条) | 队列        播放页：封面 | 大歌词
//! ────────────────────────────────────────────────────────────
//! 播放条 2 行 ▶ 歌名 歌手·专辑                          当前歌词
//!             0:00 ━━━━━━━━━━━━━━ 4:29  音量 ▉▉▉░ 65%  模式 列表循环
//! ────────────────────────────────────────────────────────────
//! 快捷键 2 行 j k 移动 / Tab 焦点 / …
//! ```

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, FocusPane, InputMode, ViewMode};
use super::logo;
use super::theme as t;
use crate::models::Song;

const QUEUE_W: u16 = 30;
const PLAYLIST_W: u16 = 24;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    // 暖色底：整屏铺一次，之后各区域只覆盖需要的色带。
    f.render_widget(Block::default().style(t::base()), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // 顶栏
        Constraint::Length(1), // 分隔
        Constraint::Min(6),    // 主体
        Constraint::Length(1), // 分隔
        Constraint::Length(2), // 播放条
    ])
    .split(area);

    draw_header(f, rows[0], app);
    rule(f, rows[1]);
    match app.view {
        ViewMode::List => draw_list_body(f, rows[2], app),
        ViewMode::Play => draw_play_body(f, rows[2], app),
    }
    rule(f, rows[3]);
    draw_player(f, rows[4], app);

    if app.show_help {
        draw_help(f, area);
    }
}

fn rule(f: &mut Frame, area: Rect) {
    let text = "─".repeat(area.width as usize);
    f.render_widget(Paragraph::new(text).style(t::line()), area);
}

// ── 顶栏 ────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Block::default().style(t::strip()), area);

    let mut left: Vec<Span> = vec![
        Span::styled(
            " MIXLY",
            Style::default()
                .fg(t::ACCENT_DEEP)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │ ", t::line()),
    ];

    if app.input_mode == InputMode::Search {
        left.push(Span::styled("搜索 › ", t::accent()));
        left.push(Span::styled(app.input_buffer.clone(), Style::default().fg(t::TEXT)));
        left.push(Span::styled("▏", Style::default().fg(t::ACCENT)));
        left.push(Span::styled("  Enter 搜索 · Esc 取消", t::faint()));
    } else {
        let (dot, proxy) = if app.proxy_active {
            (Span::styled("● ", Style::default().fg(t::SAGE)), "代理 已启用")
        } else {
            (Span::styled("● ", t::faint()), "代理 关闭")
        };
        left.push(dot);
        left.push(Span::styled(proxy, t::dim()));
        left.push(Span::styled("   音质 ", t::faint()));
        left.push(Span::styled(app.quality.as_str().to_string(), t::dim()));
        left.push(Span::styled("   网易云 ", t::faint()));
        left.push(account_span(app.login_netease));
        left.push(Span::styled(" · QQ ", t::faint()));
        left.push(account_span(app.login_qq));
        left.push(Span::styled(
            match app.view {
                ViewMode::List => "   列表 · v 切播放页 · ? 快捷键",
                ViewMode::Play => "   播放页 · v 切列表 · ? 快捷键",
            },
            t::faint(),
        ));
    }

    let right = vec![
        Span::styled(format!("{} ", spinner(app)), t::accent()),
        Span::styled(trunc(&app.status_message, 46), t::accent()),
        Span::raw(" "),
    ];
    f.render_widget(
        Paragraph::new(spread(left, right, area.width)).style(t::strip()),
        area,
    );
}

fn account_span(ok: bool) -> Span<'static> {
    if ok {
        Span::styled("✓ 已登录", Style::default().fg(t::SAGE_DEEP))
    } else {
        Span::styled("未登录", t::faint())
    }
}

/// 加载中/播放中转圈，暂停时为静止点。
fn spinner(app: &App) -> &'static str {
    const FRAMES: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];
    if app.snap.paused || app.queue.current().is_none() {
        return "·";
    }
    let i = ((app.snap.time_pos * 5.0) as usize) % FRAMES.len();
    FRAMES[i]
}

// ── 列表模式 ────────────────────────────────────────────────────────────

fn draw_list_body(f: &mut Frame, area: Rect, app: &App) {
    let queue_w = if app.show_queue && area.width >= 96 && !app.queue.is_empty() {
        QUEUE_W
    } else {
        0
    };
    let cols = Layout::horizontal([
        Constraint::Length(PLAYLIST_W),
        Constraint::Min(30),
        Constraint::Length(queue_w),
    ])
    .split(area);

    draw_playlists(f, cols[0], app);
    draw_songs(f, cols[1], app);
    if queue_w > 0 {
        draw_queue(f, cols[2], app);
    }
}

/// 竖分隔线代替四面边框：只在右侧留一条 1px 细线。
fn divider_right(f: &mut Frame, area: Rect) -> Rect {
    f.render_widget(
        Block::default()
            .borders(Borders::RIGHT)
            .border_style(t::line()),
        area,
    );
    Rect {
        width: area.width.saturating_sub(1),
        ..area
    }
}

fn pane_header(f: &mut Frame, area: Rect, title: &str, right: &str, focused: bool) {
    let line = spread(
        vec![
            Span::raw(" "),
            Span::styled(title.to_string(), t::pane_title(focused)),
        ],
        vec![Span::styled(right.to_string(), t::faint()), Span::raw(" ")],
        area.width,
    );
    f.render_widget(Paragraph::new(line), area);
}

fn draw_playlists(f: &mut Frame, area: Rect, app: &App) {
    let inner = divider_right(f, area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    let focused = app.focus == FocusPane::Playlists;
    pane_header(
        f,
        rows[0],
        "歌单",
        &app.playlists.len().to_string(),
        focused,
    );

    let width = rows[1].width;
    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let sel = i == app.playlist_cursor;
            let line = spread(
                vec![
                    Span::styled(
                        if sel { "▎" } else { " " },
                        Style::default().fg(t::ACCENT),
                    ),
                    Span::styled(
                        trunc(&p.name, width.saturating_sub(7) as usize),
                        if sel {
                            Style::default().fg(t::TEXT).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(t::TEXT)
                        },
                    ),
                ],
                vec![
                    Span::styled(
                        p.songs.len().to_string(),
                        if sel { t::accent() } else { t::faint() },
                    ),
                    Span::raw(" "),
                ],
                width,
            );
            ListItem::new(line).style(if sel { t::selected() } else { Style::default() })
        })
        .collect();
    render_list(f, rows[1], items, app.playlist_cursor);
}

fn draw_songs(f: &mut Frame, area: Rect, app: &App) {
    let inner = divider_right(f, area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let searching = app.focus == FocusPane::Search && !app.search_results.is_empty();
    let songs: &[Song] = if searching {
        &app.search_results
    } else {
        app.playlists
            .get(app.playlist_cursor)
            .map(|p| p.songs.as_slice())
            .unwrap_or(&[])
    };
    let cursor = if searching {
        app.search_cursor
    } else {
        app.song_cursor
    };
    let title = if searching {
        "搜索结果".to_string()
    } else {
        app.playlists
            .get(app.playlist_cursor)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "（没有歌单）".into())
    };

    let view_h = rows[1].height as usize;
    let offset = window_start(cursor, songs.len(), view_h);
    let meta = if songs.is_empty() {
        "0 首".to_string()
    } else {
        format!(
            "{} 首 · {}–{}",
            songs.len(),
            offset + 1,
            (offset + view_h).min(songs.len())
        )
    };
    pane_header(
        f,
        rows[0],
        &title,
        &meta,
        matches!(app.focus, FocusPane::Songs | FocusPane::Search),
    );

    let needs_bar = songs.len() > view_h;
    let list_w = rows[1].width.saturating_sub(if needs_bar { 1 } else { 0 });
    let playing_id = app.queue.current().map(|s| (s.platform, s.id.clone()));

    let items: Vec<ListItem> = songs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let sel = i == cursor;
            let playing = playing_id
                .as_ref()
                .map(|(p, id)| *p == s.platform && *id == s.id)
                .unwrap_or(false);
            ListItem::new(song_line(i, s, list_w, sel, playing))
                .style(if sel { t::selected() } else { Style::default() })
        })
        .collect();

    let list_area = Rect {
        width: list_w,
        ..rows[1]
    };
    render_list(f, list_area, items, cursor);

    if needs_bar {
        let mut state = ScrollbarState::new(songs.len().saturating_sub(view_h)).position(offset);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("█")
                .track_style(t::line())
                .thumb_style(Style::default().fg(t::ACCENT)),
            rows[1],
            &mut state,
        );
    }
}

fn draw_queue(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(2)])
        .split(area);
    pane_header(f, rows[0], "播放队列", "Q 收起", false);

    let cur = app.queue.index;
    let view_h = rows[1].height as usize;
    let start = window_start(cur, app.queue.len(), view_h);
    let items: Vec<ListItem> = app
        .queue
        .songs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let playing = i == cur;
            let width = rows[1].width;
            let line = spread(
                vec![
                    Span::styled(
                        if playing {
                            " ▶ ".to_string()
                        } else {
                            format!("{:>2} ", i + 1)
                        },
                        if playing { t::accent() } else { t::faint() },
                    ),
                    Span::styled(
                        trunc(&s.name, width.saturating_sub(12) as usize),
                        if playing {
                            t::bold_accent()
                        } else {
                            Style::default().fg(t::TEXT)
                        },
                    ),
                ],
                vec![
                    Span::styled(fmt_ms(s.duration_ms), t::faint()),
                    Span::raw(" "),
                ],
                width,
            );
            ListItem::new(line)
        })
        .collect();
    render_list_at(f, rows[1], items, cur, start);

    let lyric = app
        .lyrics
        .current_line()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "（暂无歌词）".into());
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(" 当前歌词", t::faint())),
            Line::from(Span::styled(
                format!(" {}", trunc(&lyric, rows[2].width.saturating_sub(2) as usize)),
                t::accent(),
            )),
        ]),
        rows[2],
    );
}

// ── 播放页 ──────────────────────────────────────────────────────────────

fn draw_play_body(f: &mut Frame, area: Rect, app: &App) {
    // 封面最多占一半宽度；由内嵌 PNG 按当前区域实时渲染，任何尺寸比例都正确。
    let art = logo::render(area.width / 2, area.height.saturating_sub(2), app.logo_ascii);
    let art_w = logo::art_width(&art);
    let cover_w = if art_w > 0 && area.width >= art_w + 40 {
        art_w + 6
    } else {
        0
    };
    let cols = Layout::horizontal([Constraint::Length(cover_w), Constraint::Min(30)]).split(area);

    if cover_w > 0 {
        draw_cover(f, cols[0], app, &art);
    }
    draw_lyrics(f, cols[1], app);
}

fn draw_cover(f: &mut Frame, area: Rect, app: &App, art: &[String]) {
    let art_h = art.len() as u16;
    let pad_top = area.height.saturating_sub(art_h + 2) / 2;
    let mut lines: Vec<Line> = Vec::new();
    for _ in 0..pad_top {
        lines.push(Line::raw(""));
    }
    for row in art {
        lines.push(Line::from(Span::styled(
            format!(" {row}"),
            Style::default().fg(t::ACCENT),
        )));
    }
    lines.push(Line::raw(""));
    if let Some(song) = app.queue.current() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                format!(" {} ", song.platform.label_zh()),
                Style::default().bg(t::TAG_BG).fg(t::DIM),
            ),
            Span::raw(" "),
            Span::styled(
                trunc(
                    song.album.as_deref().unwrap_or("未知专辑"),
                    area.width.saturating_sub(12) as usize,
                ),
                t::faint(),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_lyrics(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1), // kicker
        Constraint::Length(1), // 歌名
        Constraint::Length(1), // 歌手
        Constraint::Length(1), // 空
        Constraint::Min(3),    // 歌词窗口
    ])
    .split(area);

    let (name, artist) = match app.queue.current() {
        Some(s) => (
            s.name.clone(),
            if s.artists.is_empty() {
                "未知歌手".to_string()
            } else {
                s.artists.join("、")
            },
        ),
        None => ("— 未在播放 —".to_string(), String::new()),
    };
    let pos = if app.queue.is_empty() {
        String::new()
    } else {
        format!("{} / {}", app.queue.index + 1, app.queue.len())
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("正在播放", t::faint()),
            Span::styled(format!("  {pos}"), t::faint()),
        ])),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                trunc(&name, area.width.saturating_sub(4) as usize),
                Style::default().fg(t::TEXT).add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(trunc(&artist, area.width.saturating_sub(4) as usize), t::dim()),
        ])),
        rows[2],
    );

    // 歌词窗口：奇数行数、行间留空，总高不超过窗口，当前行正居中。
    let win = rows[4];
    let h = win.height as usize;
    let mut lines: Vec<Line> = Vec::new();
    if app.lyrics.lines.is_empty() {
        lines.push(Line::from(Span::styled("  （暂无歌词）", t::faint())));
    } else {
        // n 行歌词 + (n-1) 空行 = 2n-1 ≤ h，n 取奇数且 ≤ 5
        let mut n = ((h + 1) / 2).min(5);
        if n % 2 == 0 {
            n -= 1;
        }
        let n = n.max(1);
        let cur = app.lyrics.current_index.unwrap_or(0) as isize;
        let half = (n / 2) as isize;
        for i in (cur - half)..=(cur + half) {
            if i > cur - half {
                lines.push(Line::raw(""));
            }
            let text = app
                .lyrics
                .lines
                .get(i.max(0) as usize)
                .filter(|_| i >= 0)
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            let dist = (i - cur).abs();
            let style = if dist == 0 {
                t::bold_accent()
            } else if dist == 1 {
                t::dim()
            } else {
                t::faint()
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(trunc(&text, win.width.saturating_sub(4) as usize), style),
            ]));
        }
    }
    let pad = h.saturating_sub(lines.len()) / 2;
    let mut out: Vec<Line> = (0..pad).map(|_| Line::raw("")).collect();
    out.extend(lines);
    f.render_widget(Paragraph::new(out), win);
}

// ── 播放条 ──────────────────────────────────────────────────────────────

fn draw_player(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Block::default().style(t::player_strip()), area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    let (name, sub) = match app.queue.current() {
        Some(s) => (
            s.name.clone(),
            format!(
                "{} · {}",
                if s.artists.is_empty() {
                    "未知歌手".to_string()
                } else {
                    s.artists.join("、")
                },
                s.album.as_deref().unwrap_or("未知专辑")
            ),
        ),
        None => ("— 未在播放 —".into(), String::new()),
    };
    let head = spread(
        vec![
            Span::styled(
                if app.snap.paused { " ⏸  " } else { " ▶  " },
                Style::default().fg(t::ACCENT_DEEP),
            ),
            Span::styled(
                trunc(&name, (area.width / 3) as usize),
                Style::default().fg(t::TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(trunc(&sub, (area.width / 2) as usize), t::dim()),
        ],
        Vec::new(),
        rows[0].width,
    );
    f.render_widget(Paragraph::new(head).style(t::player_strip()), rows[0]);

    // 进度：时间 ─ 进度条 ─ 时间 ─ 模式
    let tail: Vec<Span> = vec![
        Span::styled(format!("   模式 {}  ", app.queue.mode.label()), t::dim()),
    ];
    let tail_w: usize = tail.iter().map(|s| s.width()).sum();
    let time_w = 6 + 6 + 1;
    let bar_w = (rows[1].width as usize).saturating_sub(tail_w + time_w + 2);
    let ratio = if app.snap.duration > 0.0 {
        (app.snap.time_pos / app.snap.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = (bar_w as f64 * ratio).round() as usize;

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("{:>5} ", fmt_time(app.snap.time_pos)), t::dim()),
        Span::styled("━".repeat(filled), Style::default().fg(t::ACCENT)),
        Span::styled("━".repeat(bar_w.saturating_sub(filled)), t::line()),
        Span::styled(format!(" {:>5}", fmt_time(app.snap.duration)), t::dim()),
    ];
    spans.extend(tail);
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(t::player_strip()),
        rows[1],
    );
}

// ── 帮助浮层（? 打开 / ? 或 Esc 关闭）────────────────────────────────────

fn draw_help(f: &mut Frame, area: Rect) {
    let w = 62.min(area.width.saturating_sub(4));
    let h = 16.min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);

    let rows: [(&str, &str, &str, &str); 8] = [
        ("j / k", "上下移动", "/", "搜索双平台"),
        ("Tab", "切换焦点", "a", "加入当前歌单"),
        ("Enter", "播放选中", "K / J", "歌单内调序"),
        ("空格", "暂停 / 继续", "Q", "播放队列面板"),
        ("n / p", "下一首 / 上一首", "v", "列表 / 播放页"),
        ("h / l", "进度 ±5s", "m", "循环模式"),
        ("+ / -", "音量 ±5", "b", "封面 Unicode/ASCII"),
        ("?", "开 / 关帮助", "q", "退出"),
    ];
    let mut lines: Vec<Line> = vec![Line::raw("")];
    for (k1, d1, k2, d2) in rows {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{k1:<7}"), t::bold_accent()),
            Span::styled(format!("{d1:<18}"), t::dim()),
            Span::styled(
                format!("{k2:<7}"),
                Style::default()
                    .fg(t::SAGE_DEEP)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(d2.to_string(), t::dim()),
        ]));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(t::line())
        .title(Span::styled(" 快捷键 ", t::bold_accent()))
        .title_bottom(Span::styled(" ? 或 Esc 关闭 ", t::faint()))
        .style(Style::default().bg(t::RAISED));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

// ── 工具 ────────────────────────────────────────────────────────────────

/// 左右两组 span 两端对齐，中间补空格。
fn spread<'a>(left: Vec<Span<'a>>, right: Vec<Span<'a>>, width: u16) -> Line<'a> {
    let used: usize = left.iter().chain(right.iter()).map(|s| s.width()).sum();
    let pad = (width as usize).saturating_sub(used);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(right);
    Line::from(spans)
}

/// 一行歌曲：▎ 01 [平台] 歌名 歌手 · 专辑 …… 时长
fn song_line(i: usize, s: &Song, width: u16, selected: bool, playing: bool) -> Line<'static> {
    let dur = fmt_ms(s.duration_ms);
    let tag = format!(" {} ", s.platform.label_zh());
    let tag_style = if selected {
        Style::default().bg(t::ACCENT_TAG).fg(t::TEXT)
    } else {
        Style::default().bg(t::TAG_BG).fg(t::DIM)
    };
    let fixed = 1 + 3 + str_width(&tag) + 1 + dur.len() + 2;
    let rest = (width as usize).saturating_sub(fixed);
    let name_max = (rest * 45 / 100).max(8);
    let sub_max = rest.saturating_sub(name_max);

    let artists = if s.artists.is_empty() {
        "未知歌手".to_string()
    } else {
        s.artists.join("、")
    };
    let sub = format!("{} · {}", artists, s.album.as_deref().unwrap_or("未知专辑"));

    let marker = if selected {
        Span::styled("▎", Style::default().fg(t::ACCENT))
    } else if playing {
        Span::styled("▸", Style::default().fg(t::ACCENT))
    } else {
        Span::raw(" ")
    };
    let name_style = if selected || playing {
        Style::default().fg(t::TEXT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t::TEXT)
    };

    spread(
        vec![
            marker,
            Span::styled(
                format!("{:>2} ", i + 1),
                if selected { t::accent() } else { t::faint() },
            ),
            Span::styled(tag, tag_style),
            Span::raw(" "),
            Span::styled(trunc(&s.name, name_max), name_style),
            Span::raw("  "),
            Span::styled(
                trunc(&sub, sub_max),
                if selected { t::accent() } else { t::faint() },
            ),
        ],
        vec![
            Span::styled(dur, if selected { t::accent() } else { t::faint() }),
            Span::raw(" "),
        ],
        width,
    )
}

/// 让选中项尽量居中的窗口起点。
fn window_start(cursor: usize, total: usize, view_h: usize) -> usize {
    if total <= view_h || view_h == 0 {
        return 0;
    }
    cursor
        .saturating_sub(view_h / 2)
        .min(total - view_h)
}

fn render_list(f: &mut Frame, area: Rect, items: Vec<ListItem>, cursor: usize) {
    let start = window_start(cursor, items.len(), area.height as usize);
    render_list_at(f, area, items, cursor, start);
}

fn render_list_at(
    f: &mut Frame,
    area: Rect,
    items: Vec<ListItem>,
    cursor: usize,
    start: usize,
) {
    let mut state = ListState::default();
    state.select(Some(cursor));
    *state.offset_mut() = start;
    f.render_stateful_widget(List::new(items), area, &mut state);
}

fn fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "0:00".into();
    }
    let s = secs as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn fmt_ms(ms: Option<u64>) -> String {
    match ms {
        Some(ms) if ms > 0 => fmt_time(ms as f64 / 1000.0),
        _ => "--:--".into(),
    }
}

fn str_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// 按终端列宽截断，超出补 `…`。
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

    #[test]
    fn trunc_respects_cjk_width() {
        assert_eq!(str_width("晴天"), 4);
        assert_eq!(trunc("晴天", 4), "晴天");
        assert!(str_width(&trunc("周杰伦的歌单", 6)) <= 6);
    }

    #[test]
    fn window_start_centers_cursor() {
        assert_eq!(window_start(0, 24, 10), 0);
        assert_eq!(window_start(12, 24, 10), 7);
        assert_eq!(window_start(23, 24, 10), 14);
        assert_eq!(window_start(3, 4, 10), 0);
    }

    #[test]
    fn fmt_time_and_ms() {
        assert_eq!(fmt_time(73.4), "1:13");
        assert_eq!(fmt_ms(Some(269_000)), "4:29");
        assert_eq!(fmt_ms(None), "--:--");
    }
}
