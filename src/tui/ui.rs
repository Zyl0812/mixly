//! 合并版渲染：列表页（方案 C · 全宽单表）⇄ 播放页（封面 + 大歌词），共用顶栏与快捷键行。
//!
//! 版式（自上而下）：
//! ```text
//! 顶栏 1 行   MIXLY  网易云 ● QQ ● B站 ○   Exhigh   直连          ⣾ 状态
//! ────────────────────────────────────────────────────────────────
//! 列表页      歌单标签行 ─ 表头 ─ 全宽歌曲表（# 平台 歌名 歌手 专辑 时长）
//! 播放页      封面 │ 歌名/歌手/专辑/进度/音量/歌词，整块垂直居中
//! ────────────────────────────────────────────────────────────────
//! 播放条 2 行 仅列表页：▶ 歌名  歌手 · 专辑                     2 / 24
//!             3:12 ━━━━●────── 5:26   音量 ▊▊▊▊▊▊░░░░ 65%  模式 顺序
//! 快捷键 1 行 j k 移动 · Tab 切歌单 · Enter 播放 · …
//! ```
//!
//! 播放页没有底部播放条：进度、音量、模式都在右栏跟歌名同一块里。

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

/// 平台标签列宽：最宽的「网易云」是 6 格，加左右各一格留白 = 8，再留 1 格列间距。
const TAG_W: usize = 9;
/// 序号列
const NO_W: usize = 3;
/// 时长列
const DUR_W: usize = 6;
/// 音量条格数
const VOL_CELLS: usize = 10;
/// 队列浮层宽度
const QUEUE_W: u16 = 46;
/// 播放页右栏宽度上限。够放下 46 格进度条和绝大多数歌词行；
/// 再宽只会把信息块推离封面，整组内容散架。
const PLAY_INFO_W: u16 = 72;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    // 暖色底：整屏铺一次，之后各区域只覆盖需要的色带。
    f.render_widget(Block::default().style(t::base()), area);

    // 播放页把进度/音量/模式收进了右栏，所以不画底部播放条。
    let player_h = match app.view {
        ViewMode::List => 2,
        ViewMode::Play => 0,
    };

    let rows = Layout::vertical([
        Constraint::Length(1),        // 顶栏
        Constraint::Length(1),        // 分隔
        Constraint::Min(6),           // 主体
        Constraint::Length(1),        // 分隔
        Constraint::Length(player_h), // 播放条（仅列表页）
        Constraint::Length(1),        // 快捷键
    ])
    .split(area);

    draw_header(f, rows[0], app);
    rule(f, rows[1]);
    match app.view {
        ViewMode::List => draw_list_body(f, rows[2], app),
        ViewMode::Play => draw_play_body(f, rows[2], app),
    }
    rule(f, rows[3]);
    if player_h > 0 {
        draw_player(f, rows[4], app);
    }
    draw_hints(f, rows[5], app);

    if app.show_queue && app.view == ViewMode::List && !app.queue.is_empty() {
        draw_queue_overlay(f, area, app);
    }
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

    let mut left: Vec<Span> = vec![Span::styled(
        " MIXLY",
        Style::default()
            .fg(t::ACCENT_DEEP)
            .add_modifier(Modifier::BOLD),
    )];

    if app.input_mode == InputMode::Search {
        left.push(Span::styled("  搜索 › ", t::accent()));
        left.push(Span::styled(
            app.input_buffer.clone(),
            Style::default().fg(t::TEXT),
        ));
        left.push(Span::styled("▏", Style::default().fg(t::ACCENT)));
        left.push(Span::styled("  Enter 搜索 · Esc 取消", t::faint()));
    } else {
        // 三平台登录态压成一个点：● 已登录（鼠尾草绿），○ 未登录（浅灰）。
        left.push(Span::styled("   网易云 ", t::faint()));
        left.push(account_dot(app.login_netease));
        left.push(Span::styled("  QQ ", t::faint()));
        left.push(account_dot(app.login_qq));
        left.push(Span::styled("  B站 ", t::faint()));
        left.push(account_dot(app.login_bilibili));
        left.push(Span::styled(
            format!("    {}", app.quality.as_str()),
            t::faint(),
        ));
        left.push(Span::styled(
            if app.proxy_active {
                "    代理"
            } else {
                "    直连"
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

fn account_dot(ok: bool) -> Span<'static> {
    if ok {
        Span::styled("●", Style::default().fg(t::SAGE))
    } else {
        Span::styled("○", t::faint())
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

// ── 快捷键行 ────────────────────────────────────────────────────────────

fn draw_hints(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Block::default().style(t::strip()), area);
    let text = match app.view {
        ViewMode::List => {
            " j / k 移动   Tab 切歌单   Enter 播放   K / J 调序   a 加入歌单   Q 队列   / 搜索   v 播放页   ? 全部快捷键"
        }
        ViewMode::Play => {
            " 空格 暂停   n / p 切歌   h / l ±5s   + / - 音量   m 模式   v 列表   / 搜索   ? 全部快捷键"
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            trunc(text, area.width as usize),
            t::faint(),
        )))
        .style(t::strip()),
        area,
    );
}

// ── 列表页（方案 C · 全宽单表）──────────────────────────────────────────

/// 表格列宽。marker 固定 1 格，末尾留 1 格给滚动条/呼吸位。
struct Cols {
    name: usize,
    artist: usize,
    album: usize,
}

fn columns(width: usize) -> Cols {
    let fixed = 1 + NO_W + TAG_W + DUR_W + 1;
    let rest = width.saturating_sub(fixed);
    let name = (rest * 40 / 100).max(6);
    let artist = (rest * 24 / 100).max(4);
    let album = rest.saturating_sub(name + artist);
    Cols {
        name,
        artist,
        album,
    }
}

fn draw_list_body(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1), // 歌单标签行
        Constraint::Length(1), // 分隔
        Constraint::Length(1), // 表头
        Constraint::Min(1),    // 表
    ])
    .split(area);

    draw_playlist_tabs(f, rows[0], app);
    rule(f, rows[1]);

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

    let view_h = rows[3].height as usize;
    let needs_bar = songs.len() > view_h;
    let list_w = rows[3].width.saturating_sub(if needs_bar { 1 } else { 0 });
    let cols = columns(list_w as usize);

    draw_table_header(f, rows[2], &cols);

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
            ListItem::new(song_row(i, s, &cols, sel, playing)).style(if sel {
                t::selected()
            } else {
                Style::default()
            })
        })
        .collect();

    let list_area = Rect {
        width: list_w,
        ..rows[3]
    };
    render_list(f, list_area, items, cursor);

    if needs_bar {
        let offset = window_start(cursor, songs.len(), view_h);
        let mut state = ScrollbarState::new(songs.len().saturating_sub(view_h)).position(offset);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("█")
                .track_style(t::line())
                .thumb_style(Style::default().fg(t::ACCENT)),
            rows[3],
            &mut state,
        );
    }
}

/// 歌单标签行；正在看搜索结果时换成返回提示。
fn draw_playlist_tabs(f: &mut Frame, area: Rect, app: &App) {
    if app.focus == FocusPane::Search && !app.search_results.is_empty() {
        let line = spread(
            vec![
                Span::raw(" "),
                Span::styled("搜索结果", t::bold_accent()),
                Span::styled(format!("   {} 首", app.search_results.len()), t::faint()),
            ],
            vec![Span::styled("Esc 返回歌单 ", t::faint()), Span::raw(" ")],
            area.width,
        );
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let hint = "Tab / ⇧Tab 切歌单  ";
    let budget = (area.width as usize).saturating_sub(str_width(hint) + 1);

    let mut spans = vec![Span::raw(" ")];
    let mut used = 0usize;
    for (i, p) in app.playlists.iter().enumerate() {
        let label = format!(" {} {} ", p.name, p.songs.len());
        let w = str_width(&label);
        if used + w > budget {
            spans.push(Span::styled("…", t::faint()));
            break;
        }
        used += w;
        let sel = i == app.playlist_cursor;
        spans.push(Span::styled(
            label,
            if sel {
                t::selected().add_modifier(Modifier::BOLD)
            } else {
                t::dim()
            },
        ));
    }
    if app.playlists.is_empty() {
        spans.push(Span::styled("（还没有歌单）", t::faint()));
    }

    f.render_widget(
        Paragraph::new(spread(
            spans,
            vec![Span::styled(hint, t::faint())],
            area.width,
        )),
        area,
    );
}

fn draw_table_header(f: &mut Frame, area: Rect, cols: &Cols) {
    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(format!("{:>2} ", "#"), t::faint()),
        Span::styled(cell("平台", TAG_W), t::faint()),
        Span::styled(cell("歌名", cols.name), t::faint()),
        Span::styled(cell("歌手", cols.artist), t::faint()),
        Span::styled(cell("专辑", cols.album), t::faint()),
        Span::styled(cell_right("时长", DUR_W), t::faint()),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

/// 一行歌曲：▎ 12 [平台] 歌名 … 歌手 … 专辑 … 时长
fn song_row(i: usize, s: &Song, cols: &Cols, selected: bool, playing: bool) -> Line<'static> {
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
    let sub_style = if selected { t::accent() } else { t::faint() };

    // 平台标签自带底色，宽度不一，右侧补空格把后面的列拉齐。
    let label = format!(" {} ", s.platform.label_zh());
    let pad = TAG_W.saturating_sub(str_width(&label));
    let tag_style = if selected {
        Style::default().bg(t::ACCENT_TAG).fg(t::TEXT)
    } else {
        Style::default().bg(t::TAG_BG).fg(t::DIM)
    };

    let artists = if s.artists.is_empty() {
        "未知歌手".to_string()
    } else {
        s.artists.join("、")
    };

    Line::from(vec![
        marker,
        Span::styled(
            format!("{:>2} ", i + 1),
            if selected { t::accent() } else { t::faint() },
        ),
        Span::styled(label, tag_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(cell(&s.name, cols.name), name_style),
        Span::styled(cell(&artists, cols.artist), t::dim()),
        Span::styled(
            cell(s.album.as_deref().unwrap_or("未知专辑"), cols.album),
            sub_style,
        ),
        Span::styled(cell_right(&fmt_ms(s.duration_ms), DUR_W), sub_style),
    ])
}

/// Q 呼出的播放队列浮层（方案 C 把常驻队列列换成了按需浮层）。
fn draw_queue_overlay(f: &mut Frame, area: Rect, app: &App) {
    let w = QUEUE_W.min(area.width.saturating_sub(4));
    let h = ((app.queue.len() as u16) + 5).min(area.height.saturating_sub(4));
    if w < 20 || h < 6 {
        return;
    }
    let rect = Rect {
        x: area.x + area.width.saturating_sub(w).saturating_sub(2),
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(t::line())
        .title(Span::styled(" 播放队列 ", t::bold_accent()))
        .title_bottom(Span::styled(" Q 关闭 ", t::faint()))
        .style(Style::default().bg(t::RAISED));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).split(inner);
    let cur = app.queue.index;
    let view_h = rows[0].height as usize;
    let start = window_start(cur, app.queue.len(), view_h);
    let items: Vec<ListItem> = app
        .queue
        .songs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let playing = i == cur;
            let width = rows[0].width;
            ListItem::new(spread(
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
                        trunc(&s.name, width.saturating_sub(11) as usize),
                        if playing {
                            t::bold_accent()
                        } else {
                            Style::default().fg(t::TEXT)
                        },
                    ),
                ],
                vec![Span::styled(fmt_ms(s.duration_ms), t::faint())],
                width,
            ))
        })
        .collect();
    render_list_at(f, rows[0], items, cur, start);

    let lyric = app
        .lyrics
        .current_line()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "（暂无歌词）".into());
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(" 当前歌词", t::faint())),
            Line::from(Span::styled(
                format!(
                    " {}",
                    trunc(&lyric, rows[1].width.saturating_sub(1) as usize)
                ),
                t::accent(),
            )),
        ]),
        rows[1],
    );
}

// ── 播放页 ──────────────────────────────────────────────────────────────

fn draw_play_body(f: &mut Frame, area: Rect, app: &App) {
    // 封面最多占一半宽度；由内嵌 PNG 按当前区域实时渲染，任何尺寸比例都正确。
    let art = logo::render(
        area.width / 2,
        area.height.saturating_sub(2),
        app.logo_ascii,
    );
    let art_w = logo::art_width(&art);
    let cover_w = if art_w > 0 && area.width >= art_w + 44 {
        art_w + 6
    } else {
        0
    };

    // 封面 + 信息块作为一个整体水平居中。
    // 右栏之前是 Min(30)，会把剩余宽度全吃掉，宽终端上整组内容就贴在左边、右半屏空着。
    // 光给上限还不够：还得按内容实际宽度收紧，否则只是把一坨靠左的字装进一个居中的空盒子。
    let avail = area.width.saturating_sub(cover_w);
    let inner = PLAY_INFO_W.min(avail).saturating_sub(4) as usize;
    let lines = now_playing_lines(app, area.height as usize, inner);
    let info_w = lines
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(0)
        .clamp(1, avail.max(1));
    let side = area.width.saturating_sub(cover_w + info_w) / 2;

    let cols = Layout::horizontal([
        Constraint::Length(side),
        Constraint::Length(cover_w),
        Constraint::Length(info_w),
        Constraint::Min(0),
    ])
    .split(area);

    if cover_w > 0 {
        draw_cover(f, cols[1], &art);
    }
    f.render_widget(Paragraph::new(lines), cols[2]);
}

fn draw_cover(f: &mut Frame, area: Rect, art: &[String]) {
    let pad_top = area.height.saturating_sub(art.len() as u16) / 2;
    let mut lines: Vec<Line> = (0..pad_top).map(|_| Line::raw("")).collect();
    for row in art {
        lines.push(Line::from(Span::styled(
            format!(" {row}"),
            Style::default().fg(t::ACCENT),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// 右栏的行：歌名 / 歌手 / 专辑 / 进度 / 音量 / 歌词，整块垂直居中。
/// 播放页没有底部播放条，进度和音量就在这里。
/// 只造行不渲染 —— 调用方要先量出实际宽度才能把整组内容摆到屏幕中间。
fn now_playing_lines(app: &App, h: usize, inner: usize) -> Vec<Line<'static>> {
    let (name, artist, album, tag) = match app.queue.current() {
        Some(s) => (
            s.name.clone(),
            if s.artists.is_empty() {
                "未知歌手".to_string()
            } else {
                s.artists.join("、")
            },
            s.album.as_deref().unwrap_or("未知专辑").to_string(),
            Some(format!(" {} ", s.platform.label_zh())),
        ),
        None => (
            "— 未在播放 —".to_string(),
            String::new(),
            String::new(),
            None,
        ),
    };
    let pos = if app.queue.is_empty() {
        String::new()
    } else {
        format!("  ·  {} / {}", app.queue.index + 1, app.queue.len())
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled("正在播放", t::faint()),
            Span::styled(pos, t::faint()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                trunc(&name, inner),
                Style::default().fg(t::TEXT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(trunc(&artist, inner), t::dim()),
        ]),
    ];
    lines.push(match tag {
        Some(tag) => Line::from(vec![
            Span::raw("  "),
            Span::styled(tag, Style::default().bg(t::TAG_BG).fg(t::DIM)),
            Span::styled(
                format!("  {}", trunc(&album, inner.saturating_sub(10))),
                t::faint(),
            ),
        ]),
        None => Line::raw(""),
    });

    // 进度 + 音量：播放页自己的播放条
    lines.push(Line::raw(""));
    let bar_w = inner.saturating_sub(12).min(46);
    // 时间是 `{:>5}` 右对齐的，自带一格前导空格；这里只缩进 1 格，
    // 「2:00」才和上面的歌名、下面的歌词落在同一列。
    let mut prog = vec![Span::raw(" ")];
    prog.extend(progress_spans(app.snap.time_pos, app.snap.duration, bar_w));
    lines.push(Line::from(prog));
    let mut vol = vec![Span::raw("  ")];
    vol.extend(volume_spans(app.snap.volume));
    vol.push(Span::styled(
        format!("      模式 {}", app.queue.mode.label()),
        t::dim(),
    ));
    lines.push(Line::from(vol));

    // 装饰性分隔（够高才画）
    if h >= 16 {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", "─".repeat(inner.min(44))),
            t::line(),
        )));
    }
    lines.push(Line::raw(""));

    // 歌词：n 行 + (n-1) 空行 ≤ 剩余高度，n 取奇数（0 也是偶数，这里必须饱和减）
    let budget = h.saturating_sub(lines.len());
    let mut n = budget.div_ceil(2).min(5);
    if n.is_multiple_of(2) {
        n = n.saturating_sub(1);
    }
    lines.extend(lyric_lines(app, n.max(1), inner));

    // 整块垂直居中
    let pad = h.saturating_sub(lines.len()) / 2;
    let mut out: Vec<Line> = (0..pad).map(|_| Line::raw("")).collect();
    out.extend(lines);
    out
}

/// 当前行居中的 n 行歌词，行间留空；共 2n-1 行。
fn lyric_lines(app: &App, n: usize, width: usize) -> Vec<Line<'static>> {
    if app.lyrics.lines.is_empty() {
        return vec![Line::from(Span::styled("  （暂无歌词）", t::faint()))];
    }
    let cur = app.lyrics.current_index.unwrap_or(0) as isize;
    let half = (n / 2) as isize;
    let mut out = Vec::with_capacity(2 * n);
    for i in (cur - half)..=(cur + half) {
        if i > cur - half {
            out.push(Line::raw(""));
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
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(trunc(&text, width), style),
        ]));
    }
    out
}

// ── 播放条（仅列表页）───────────────────────────────────────────────────

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
    let queue_pos = if app.queue.is_empty() {
        String::new()
    } else {
        format!("{} / {}  ", app.queue.index + 1, app.queue.len())
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
            Span::raw("   "),
            Span::styled(trunc(&sub, (area.width / 2) as usize), t::dim()),
        ],
        vec![Span::styled(queue_pos, t::faint())],
        rows[0].width,
    );
    f.render_widget(Paragraph::new(head).style(t::player_strip()), rows[0]);

    // 进度 ─ 音量 ─ 模式
    let mode = format!("   模式 {}  ", app.queue.mode.label());
    let vol = volume_spans(app.snap.volume);
    let vol_w: usize = vol.iter().map(|s| s.width()).sum();
    let bar_w = (rows[1].width as usize).saturating_sub(1 + 12 + 4 + vol_w + str_width(&mode));

    let mut spans = vec![Span::raw(" ")];
    spans.extend(progress_spans(app.snap.time_pos, app.snap.duration, bar_w));
    spans.push(Span::raw("    "));
    spans.extend(vol);
    spans.push(Span::styled(mode, t::dim()));
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(t::player_strip()),
        rows[1],
    );
}

/// `0:00 ━━━━●────── 4:29`，宽度恒为 `bar_w + 12`。
/// `●` 是当前位置把手 —— 没有它 seek 时看不出落点。
fn progress_spans(pos: f64, dur: f64, bar_w: usize) -> Vec<Span<'static>> {
    let ratio = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut spans = vec![Span::styled(format!("{:>5} ", fmt_time(pos)), t::dim())];
    if bar_w >= 2 {
        let head = ((bar_w - 1) as f64 * ratio).round() as usize;
        spans.push(Span::styled(
            "━".repeat(head),
            Style::default().fg(t::ACCENT),
        ));
        spans.push(Span::styled("●", Style::default().fg(t::ACCENT_DEEP)));
        spans.push(Span::styled("─".repeat(bar_w - 1 - head), t::line()));
    } else {
        spans.push(Span::raw(" ".repeat(bar_w)));
    }
    spans.push(Span::styled(format!(" {:>5}", fmt_time(dur)), t::dim()));
    spans
}

/// `音量 ▊▊▊▊▊▊░░░░  65%`，宽度恒定。
/// `None` 表示 mpv 还没初始化音频输出（首次播放前 `ao-volume` 不存在），
/// 这时显示 `--` 而不是编一个数出来 —— 显示一个跟系统对不上的数字正是原来的毛病。
fn volume_spans(volume: Option<f64>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled("音量 ", t::faint())];
    match volume {
        Some(v) => {
            let on = ((v / 100.0) * VOL_CELLS as f64)
                .round()
                .clamp(0.0, VOL_CELLS as f64) as usize;
            spans.push(Span::styled("▊".repeat(on), Style::default().fg(t::ACCENT)));
            spans.push(Span::styled("░".repeat(VOL_CELLS - on), t::line()));
            spans.push(Span::styled(format!(" {v:>3.0}%"), t::dim()));
        }
        None => {
            spans.push(Span::styled("─".repeat(VOL_CELLS), t::line()));
            spans.push(Span::styled("   --", t::faint()));
        }
    }
    spans
}

// ── 帮助浮层（? 打开 / ? 或 Esc 关闭）────────────────────────────────────

fn draw_help(f: &mut Frame, area: Rect) {
    let w = 64.min(area.width.saturating_sub(4));
    let h = 16.min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);

    let rows: [(&str, &str, &str, &str); 8] = [
        ("j / k", "上下移动", "/", "搜索三平台"),
        ("Tab", "下一个歌单", "a", "加入当前歌单"),
        ("⇧Tab", "上一个歌单", "K / J", "歌单内调序"),
        ("Enter", "播放选中", "Q", "播放队列浮层"),
        ("空格", "暂停 / 继续", "v", "列表 / 播放页"),
        ("n / p", "下一首 / 上一首", "m", "循环模式"),
        ("h / l", "进度 ±5s", "b", "封面 Unicode/ASCII"),
        ("+ / -", "音量 ±5", "Esc / q", "返回歌单 / 退出"),
    ];
    let mut lines: Vec<Line> = vec![Line::raw("")];
    for (k1, d1, k2, d2) in rows {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{k1:<8}"), t::bold_accent()),
            Span::styled(format!("{d1:<18}"), t::dim()),
            Span::styled(
                format!("{k2:<8}"),
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

/// 截断到 `w` 格再右补空格，输出宽度恒为 `w` —— 表格列靠这个对齐。
fn cell(s: &str, w: usize) -> String {
    let text = trunc(s, w);
    let pad = w.saturating_sub(str_width(&text));
    format!("{text}{}", " ".repeat(pad))
}

/// 同上但右对齐。不能用 `{:>w$}`：那个按字符数补，
/// 「时长」是 2 个字符却占 4 格，会把列撑爆。
fn cell_right(s: &str, w: usize) -> String {
    let text = trunc(s, w);
    let pad = w.saturating_sub(str_width(&text));
    format!("{}{text}", " ".repeat(pad))
}

/// 让选中项尽量居中的窗口起点。
fn window_start(cursor: usize, total: usize, view_h: usize) -> usize {
    if total <= view_h || view_h == 0 {
        return 0;
    }
    cursor.saturating_sub(view_h / 2).min(total - view_h)
}

fn render_list(f: &mut Frame, area: Rect, items: Vec<ListItem>, cursor: usize) {
    let start = window_start(cursor, items.len(), area.height as usize);
    render_list_at(f, area, items, cursor, start);
}

fn render_list_at(f: &mut Frame, area: Rect, items: Vec<ListItem>, cursor: usize, start: usize) {
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
    use crate::models::{LyricsState, Platform, Playlist, Quality};
    use crate::playlist::PlayQueue;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn width_of(spans: &[Span]) -> usize {
        spans.iter().map(|s| s.width()).sum()
    }

    fn song(name: &str, platform: Platform) -> Song {
        Song {
            platform,
            id: name.into(),
            name: name.into(),
            artists: vec!["周杰伦".into()],
            album: Some("叶惠美".into()),
            duration_ms: Some(269_000),
            cover_url: None,
        }
    }

    fn demo_app() -> App {
        let songs: Vec<Song> = ["晴天", "海阔天空", "夜空中最亮的星", "My Jinji"]
            .iter()
            .map(|n| song(n, Platform::Netease))
            .collect();
        App {
            playlists: vec![
                Playlist {
                    id: "a".into(),
                    name: "我的收藏".into(),
                    songs: songs.clone(),
                    created_at: 0,
                    updated_at: 0,
                },
                Playlist {
                    id: "b".into(),
                    name: "通勤路上".into(),
                    songs: vec![],
                    created_at: 0,
                    updated_at: 0,
                },
            ],
            playlist_cursor: 0,
            song_cursor: 1,
            search_results: vec![song("搜索命中", Platform::Qq)],
            search_cursor: 0,
            focus: FocusPane::Songs,
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            queue: PlayQueue::from_songs(songs),
            snap: crate::player::PlaybackSnapshot {
                time_pos: 120.0,
                duration: 269.0,
                paused: false,
                volume: Some(65.0),
                eof: false,
            },
            lyrics: LyricsState::default(),
            status_message: "正在播放 晴天".into(),
            proxy_active: false,
            error: None,
            view: ViewMode::List,
            logo_ascii: false,
            show_queue: false,
            show_help: false,
            quality: Quality::Exhigh,
            login_netease: true,
            login_qq: true,
            login_bilibili: false,
        }
    }

    /// ratatui 会在越界的 Rect 上 panic，而这里的布局全是手算的减法。
    /// 两种视图 × 各种终端尺寸 × 几个浮层组合都渲染一遍，跑得完就说明没算穿。
    #[test]
    fn every_view_renders_at_any_terminal_size() {
        let sizes = [
            (120u16, 34u16), // 常规
            (200, 60),       // 大屏
            (80, 24),        // 标准最小
            (60, 15),        // 窄
            (40, 10),        // 极窄
            (20, 8),         // 荒谬
        ];
        for (w, h) in sizes {
            for view in [ViewMode::List, ViewMode::Play] {
                for (queue, help, searching) in [
                    (false, false, false),
                    (true, false, false),
                    (false, true, false),
                    (false, false, true),
                ] {
                    let mut app = demo_app();
                    app.view = view;
                    app.show_queue = queue;
                    app.show_help = help;
                    if searching {
                        app.focus = FocusPane::Search;
                    }
                    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                    term.draw(|f| draw(f, &app))
                        .unwrap_or_else(|e| panic!("{w}x{h} {view:?} 渲染失败: {e}"));
                }
            }
        }
    }

    /// 播放页的封面 + 信息块要作为一个整体水平居中。
    /// 宽终端上尤其重要 —— 右栏一旦无上限就会把内容全顶到左边、右半屏空着。
    #[test]
    fn play_body_is_horizontally_centered() {
        for w in [100u16, 140, 200, 260] {
            let mut app = demo_app();
            app.view = ViewMode::Play;
            let h = 40;
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| draw(f, &app)).unwrap();
            let buf = term.backend().buffer();

            // 只看主体区（跳过顶栏 2 行、底部分隔 + 快捷键 2 行）
            let mut min_x = w;
            let mut max_x = 0u16;
            for y in 2..(h - 2) {
                for x in 0..w {
                    if buf[(x, y)].symbol().trim().is_empty() {
                        continue;
                    }
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                }
            }
            assert!(max_x >= min_x, "宽度 {w}: 主体区什么都没画");
            let left = min_x;
            let right = w - 1 - max_x;
            assert!(
                left.abs_diff(right) <= 4,
                "宽度 {w}: 左右留白差太多 左={left} 右={right}（没居中）"
            );
            assert!(left > 0, "宽度 {w}: 内容贴着左边缘，说明右栏又把宽度吃光了");
        }
    }

    /// 空状态：没有歌单、没有队列、音量未知 —— 首次启动就是这个样子。
    #[test]
    fn empty_state_renders() {
        let mut app = demo_app();
        app.playlists.clear();
        app.search_results.clear();
        app.queue = PlayQueue::default();
        app.snap = crate::player::PlaybackSnapshot::default();
        for view in [ViewMode::List, ViewMode::Play] {
            app.view = view;
            let mut term = Terminal::new(TestBackend::new(120, 34)).unwrap();
            term.draw(|f| draw(f, &app)).expect("空状态渲染失败");
        }
    }

    #[test]
    #[ignore]
    fn dump_frames() {
        for (label, view, queue) in [
            ("列表页", ViewMode::List, false),
            ("列表页 + 队列浮层", ViewMode::List, true),
            ("播放页", ViewMode::Play, false),
        ] {
            let mut app = demo_app();
            app.view = view;
            app.show_queue = queue;
            let mut term = Terminal::new(TestBackend::new(120, 34)).unwrap();
            term.draw(|f| draw(f, &app)).unwrap();
            println!("\n===== {label} (120x34) =====");
            let buf = term.backend().buffer();
            for y in 0..buf.area.height {
                let mut line = String::new();
                let mut x = 0u16;
                while x < buf.area.width {
                    let sym = buf[(x, y)].symbol();
                    line.push_str(sym);
                    x += UnicodeWidthStr::width(sym).max(1) as u16;
                }
                println!("{}", line.trim_end());
            }
        }
    }

    #[test]
    fn trunc_respects_cjk_width() {
        assert_eq!(str_width("晴天"), 4);
        assert_eq!(trunc("晴天", 4), "晴天");
        assert!(str_width(&trunc("周杰伦的歌单", 6)) <= 6);
    }

    #[test]
    fn cell_pads_to_exact_width() {
        // 表格对齐的全部依据：无论中英文、无论截断与否，宽度都必须精确等于列宽。
        for (s, w) in [
            ("晴天", 10),
            ("Beyond", 10),
            ("夜空中最亮的星", 6),
            ("", 4),
            ("A", 1),
        ] {
            assert_eq!(str_width(&cell(s, w)), w, "cell({s:?}, {w})");
        }
    }

    #[test]
    fn columns_fit_the_given_width() {
        for w in [60usize, 80, 100, 120, 200] {
            let c = columns(w);
            let total = 1 + NO_W + TAG_W + c.name + c.artist + c.album + DUR_W + 1;
            assert!(total <= w, "width {w}: 列宽合计 {total} 超了");
        }
    }

    #[test]
    fn progress_bar_has_constant_width_and_a_knob() {
        for bar_w in [2usize, 10, 40] {
            for (pos, dur) in [(0.0, 100.0), (50.0, 100.0), (100.0, 100.0), (0.0, 0.0)] {
                let spans = progress_spans(pos, dur, bar_w);
                assert_eq!(width_of(&spans), bar_w + 12, "bar_w={bar_w} pos={pos}");
                let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(text.contains('●'), "把手不见了: {text}");
            }
        }
    }

    #[test]
    fn volume_gauge_is_constant_width_and_none_shows_dashes() {
        let some = volume_spans(Some(65.0));
        let none = volume_spans(None);
        assert_eq!(width_of(&some), width_of(&none));
        let filled = some
            .iter()
            .map(|s| s.content.matches('▊').count())
            .sum::<usize>();
        assert_eq!(filled, 7, "65% 在 10 格上应该点亮 7 格（四舍五入）");
        let text: String = none.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("--"), "音量未知时要显示 --，实际: {text}");
        assert!(!text.contains('▊'), "音量未知时不能画出任何实心格");
    }

    #[test]
    fn volume_gauge_endpoints() {
        let zero: String = volume_spans(Some(0.0))
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!zero.contains('▊'));
        let full = volume_spans(Some(100.0));
        let filled = full
            .iter()
            .map(|s| s.content.matches('▊').count())
            .sum::<usize>();
        assert_eq!(filled, VOL_CELLS);
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
