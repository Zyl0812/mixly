use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::Mutex;
use tracing::warn;

use super::events::{poll_event, AppEvent};
use super::ui;
use crate::api::ApiClient;
use crate::config::ProxyDecision;
use crate::lyrics::lyrics_state_from_lrc;
use crate::models::{LyricsState, Platform, Playlist, Quality, Song};
use crate::player::{MpvPlayer, PlaybackSnapshot};
use crate::playlist::{PlayQueue, PlaylistStore};
use crate::relay::AudioRelay;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Playlists,
    Songs,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
}

/// 主体区域的两种视图：列表（歌单/歌曲/队列）与播放页（封面 + 大歌词）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Play,
}

impl ViewMode {
    pub fn toggled(self) -> Self {
        match self {
            Self::List => Self::Play,
            Self::Play => Self::List,
        }
    }
}

pub struct App {
    pub playlists: Vec<Playlist>,
    pub playlist_cursor: usize,
    pub song_cursor: usize,
    pub search_results: Vec<Song>,
    pub search_cursor: usize,
    pub focus: FocusPane,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub queue: PlayQueue,
    pub snap: PlaybackSnapshot,
    pub lyrics: LyricsState,
    pub status_message: String,
    pub proxy_active: bool,
    pub error: Option<String>,
    /// v 切换：列表 ⇄ 播放页
    pub view: ViewMode,
    /// b 切换：播放页封面用纯 ASCII 渲染（块字符错位时的回退），默认 Unicode
    pub logo_ascii: bool,
    /// Q 切换：列表模式右侧的播放队列面板
    pub show_queue: bool,
    /// ? 切换：快捷键浮层
    pub show_help: bool,
    /// 顶栏展示用
    pub quality: Quality,
    pub login_netease: bool,
    pub login_qq: bool,
}

pub struct TuiDeps {
    pub api: Arc<ApiClient>,
    pub store: PlaylistStore,
    pub relay: AudioRelay,
    pub player: Arc<Mutex<MpvPlayer>>,
    pub proxy: ProxyDecision,
    /// 双平台搜索时优先的平台
    pub prefer: Platform,
    /// 顶栏展示的音质档位（来自 config）
    pub quality: Quality,
}

pub async fn run_tui(deps: TuiDeps) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let playlists = deps.store.list().unwrap_or_default();
    let mut app = App {
        playlists,
        playlist_cursor: 0,
        song_cursor: 0,
        search_results: Vec::new(),
        search_cursor: 0,
        focus: FocusPane::Playlists,
        input_mode: InputMode::Normal,
        input_buffer: String::new(),
        queue: PlayQueue::default(),
        snap: PlaybackSnapshot::default(),
        lyrics: LyricsState::default(),
        status_message: "就绪".into(),
        proxy_active: deps.proxy.as_url().is_some(),
        error: None,
        view: ViewMode::List,
        logo_ascii: false,
        show_queue: true,
        show_help: false,
        quality: deps.quality,
        login_netease: deps.api.is_logged_in(Platform::Netease),
        login_qq: deps.api.is_logged_in(Platform::Qq),
    };

    let result = run_loop(&mut terminal, &mut app, &deps).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    deps: &TuiDeps,
) -> Result<()> {
    let mut eof_latched = false;
    loop {
        // Snapshot under lock, but NEVER call load_current while holding the player mutex
        // (load_current re-locks the same non-reentrant tokio Mutex → deadlock on EOF).
        let mut should_load_next = false;
        {
            let mut player = deps.player.lock().await;
            if let Err(e) = player.ensure_alive().await {
                app.error = Some(format!("mpv: {e}"));
            }
            if let Ok(snap) = player.snapshot().await {
                app.snap = snap;
                app.lyrics.update_index(app.snap.time_pos);
                let advanced = if app.snap.eof && !eof_latched {
                    app.queue.advance()
                } else {
                    false
                };
                let (new_latch, load) =
                    crate::api::should_load_after_eof(app.snap.eof, eof_latched, advanced);
                eof_latched = new_latch;
                should_load_next = load;
            }
        }
        if should_load_next {
            if let Err(e) = load_current(app, deps).await {
                app.error = Some(e.to_string());
            }
        }

        terminal.draw(|f| ui::draw(f, app))?;

        let Some(ev) = poll_event(Duration::from_millis(200), app.input_mode == InputMode::Search)?
        else {
            continue;
        };

        if app.input_mode == InputMode::Search {
            match ev {
                AppEvent::Quit => {
                    app.input_mode = InputMode::Normal;
                    app.input_buffer.clear();
                    app.status_message = "已取消搜索".into();
                }
                AppEvent::Enter => {
                    let kw = app.input_buffer.trim().to_string();
                    app.input_mode = InputMode::Normal;
                    app.input_buffer.clear();
                    if !kw.is_empty() {
                        match search_all(&deps.api, &kw, deps.prefer).await {
                            Ok(songs) => {
                                app.search_results = songs;
                                app.search_cursor = 0;
                                app.focus = FocusPane::Search;
                                app.view = ViewMode::List;
                                app.status_message =
                                    format!("找到 {} 首", app.search_results.len());
                            }
                            Err(e) => {
                                app.error = Some(e.to_string());
                                app.status_message = "搜索失败".into();
                            }
                        }
                    }
                }
                AppEvent::Backspace => {
                    app.input_buffer.pop();
                }
                AppEvent::Char(c) => {
                    app.input_buffer.push(c);
                }
                // While searching, j/k etc. become literal chars if Char; map_key sends dedicated events.
                // Treat unhandled navigation as ignore.
                _ => {}
            }
            continue;
        }

        // 帮助浮层打开时吞掉导航键，只留关闭与退出。
        if app.show_help {
            match ev {
                AppEvent::Quit => break,
                AppEvent::Escape | AppEvent::ToggleHelp => app.show_help = false,
                _ => {}
            }
            continue;
        }

        match ev {
            AppEvent::Quit => break,
            AppEvent::Escape => break,
            AppEvent::ToggleHelp => app.show_help = true,
            AppEvent::ToggleQueue => {
                app.show_queue = !app.show_queue;
                app.status_message = if app.show_queue {
                    "队列面板：开".into()
                } else {
                    "队列面板：关".into()
                };
            }
            AppEvent::ToggleView => {
                app.view = app.view.toggled();
            }
            AppEvent::ToggleLogoStyle => {
                app.logo_ascii = !app.logo_ascii;
                app.status_message = if app.logo_ascii {
                    "封面：ASCII".into()
                } else {
                    "封面：Unicode".into()
                };
            }
            AppEvent::FocusNext => {
                app.view = ViewMode::List;
                app.focus = match app.focus {
                    FocusPane::Playlists => FocusPane::Songs,
                    FocusPane::Songs => {
                        if app.search_results.is_empty() {
                            FocusPane::Playlists
                        } else {
                            FocusPane::Search
                        }
                    }
                    FocusPane::Search => FocusPane::Playlists,
                };
            }
            AppEvent::Up => match app.focus {
                FocusPane::Playlists => {
                    if app.playlist_cursor > 0 {
                        app.playlist_cursor -= 1;
                        app.song_cursor = 0;
                    }
                }
                FocusPane::Songs => {
                    if app.song_cursor > 0 {
                        app.song_cursor -= 1;
                    }
                }
                FocusPane::Search => {
                    if app.search_cursor > 0 {
                        app.search_cursor -= 1;
                    }
                }
            },
            AppEvent::Down => match app.focus {
                FocusPane::Playlists => {
                    if app.playlist_cursor + 1 < app.playlists.len() {
                        app.playlist_cursor += 1;
                        app.song_cursor = 0;
                    }
                }
                FocusPane::Songs => {
                    let len = app
                        .playlists
                        .get(app.playlist_cursor)
                        .map(|p| p.songs.len())
                        .unwrap_or(0);
                    if app.song_cursor + 1 < len {
                        app.song_cursor += 1;
                    }
                }
                FocusPane::Search => {
                    if app.search_cursor + 1 < app.search_results.len() {
                        app.search_cursor += 1;
                    }
                }
            },
            AppEvent::Enter => match app.focus {
                FocusPane::Playlists | FocusPane::Songs => {
                    if let Some(pl) = app.playlists.get(app.playlist_cursor) {
                        app.queue = PlayQueue::from_songs(pl.songs.clone());
                        app.queue.jump(app.song_cursor);
                        if let Err(e) = load_current(app, deps).await {
                            app.error = Some(e.to_string());
                        }
                    }
                }
                FocusPane::Search => {
                    if let Some(song) = app.search_results.get(app.search_cursor).cloned() {
                        app.queue = PlayQueue::from_songs(vec![song]);
                        if let Err(e) = load_current(app, deps).await {
                            app.error = Some(e.to_string());
                        }
                    }
                }
            },
            AppEvent::Space => {
                let player = deps.player.lock().await;
                let _ = player.toggle_pause().await;
            }
            AppEvent::Next => {
                if app.queue.next_manual() {
                    if let Err(e) = load_current(app, deps).await {
                        app.error = Some(e.to_string());
                    }
                }
            }
            AppEvent::Prev => {
                if app.queue.prev_manual() {
                    if let Err(e) = load_current(app, deps).await {
                        app.error = Some(e.to_string());
                    }
                }
            }
            AppEvent::Search => {
                app.input_mode = InputMode::Search;
                app.input_buffer.clear();
                app.status_message = "输入关键词后按 Enter 搜索，Esc 取消".into();
            }
            AppEvent::Add => {
                if let (Some(song), Some(pl)) = (
                    app.search_results.get(app.search_cursor).cloned(),
                    app.playlists.get(app.playlist_cursor),
                ) {
                    match deps.store.add_song(&pl.id, song) {
                        Ok(_) => {
                            app.playlists = deps.store.list().unwrap_or_default();
                            app.status_message = "已加入歌单".into();
                        }
                        Err(e) => app.error = Some(e.to_string()),
                    }
                } else {
                    app.status_message = "请先选中搜索结果与目标歌单".into();
                }
            }
            AppEvent::VolumeUp => {
                let mut player = deps.player.lock().await;
                let v = (app.snap.volume + 5.0).min(100.0);
                let _ = player.set_volume(v).await;
            }
            AppEvent::VolumeDown => {
                let mut player = deps.player.lock().await;
                let v = (app.snap.volume - 5.0).max(0.0);
                let _ = player.set_volume(v).await;
            }
            AppEvent::SeekForward => {
                let player = deps.player.lock().await;
                let _ = player.seek_relative(5.0).await;
            }
            AppEvent::SeekBack => {
                let player = deps.player.lock().await;
                let _ = player.seek_relative(-5.0).await;
            }
            AppEvent::CycleMode => {
                app.queue.cycle_mode();
                app.status_message = format!("模式: {}", app.queue.mode.label());
            }
            AppEvent::MoveSongUp => {
                if let Err(e) = move_selected_song(app, deps, -1) {
                    app.error = Some(e.to_string());
                }
            }
            AppEvent::MoveSongDown => {
                if let Err(e) = move_selected_song(app, deps, 1) {
                    app.error = Some(e.to_string());
                }
            }
            AppEvent::Char(_) | AppEvent::Backspace | AppEvent::Tick => {}
        }

        if let Some(err) = app.error.take() {
            app.status_message = err;
        }
    }
    Ok(())
}

async fn search_all(api: &ApiClient, keyword: &str, prefer: Platform) -> Result<Vec<Song>> {
    let order = match prefer {
        Platform::Qq | Platform::Local => [Platform::Qq, Platform::Netease],
        Platform::Netease => [Platform::Netease, Platform::Qq],
    };
    let mut all = Vec::new();
    for p in order {
        match api.search(p, keyword, 15).await {
            Ok(mut s) => all.append(&mut s),
            Err(e) => warn!(%p, error = %e, "search platform failed"),
        }
    }
    Ok(all)
}

/// `delta`: -1 上移，+1 下移。仅在焦点为歌曲列表时生效。
fn move_selected_song(app: &mut App, deps: &TuiDeps, delta: i32) -> Result<()> {
    if app.focus != FocusPane::Songs {
        app.status_message = "请先 Tab 到「歌曲」列表再调序（K/J 或 Ctrl+↑/↓）".into();
        return Ok(());
    }
    let Some(pl) = app.playlists.get(app.playlist_cursor) else {
        app.status_message = "没有选中的歌单".into();
        return Ok(());
    };
    let from = app.song_cursor;
    let len = pl.songs.len();
    if len < 2 {
        app.status_message = "歌曲不足，无需调序".into();
        return Ok(());
    }
    let to = from as i32 + delta;
    if to < 0 || to >= len as i32 {
        app.status_message = "已经到顶/底了".into();
        return Ok(());
    }
    let to = to as usize;
    let pl_id = pl.id.clone();
    deps.store.reorder(&pl_id, from, to)?;
    app.playlists = deps.store.list().unwrap_or_default();
    // 光标跟随被移动的那一首
    app.song_cursor = to;
    app.status_message = format!("已调序：{from} → {to}");
    Ok(())
}

async fn load_current(app: &mut App, deps: &TuiDeps) -> Result<()> {
    let Some(mut song) = app.queue.current().cloned() else {
        return Ok(());
    };
    // 缺专辑等元数据时按歌曲所属平台补全，并写回队列供播放条/列表展示
    song = deps.api.enrich_song_metadata(&song).await;
    app.queue.update_current(song.clone());

    app.status_message = format!("加载中 {}", song.name);
    let track = deps
        .api
        .resolve_track(&song)
        .await
        .with_context(|| format!("获取播放链接失败 {}", song.id))?;
    deps.relay.set_track(track).await;

    {
        let mut player = deps.player.lock().await;
        player.loadfile(&deps.relay.current_url()).await?;
        player.set_pause(false).await?;
    }

    match deps.api.lyrics(song.platform, &song.id).await {
        Ok(lrc) => {
            app.lyrics = lyrics_state_from_lrc(&lrc);
            app.status_message = format!("正在播放 {}", song.name);
        }
        Err(e) => {
            app.lyrics = LyricsState::default();
            app.status_message = format!("正在播放 {}（无歌词: {e}）", song.name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_mode_toggles() {
        assert_eq!(ViewMode::List.toggled(), ViewMode::Play);
        assert_eq!(ViewMode::Play.toggled(), ViewMode::List);
    }
}
