use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    Quit,
    /// Esc：先关帮助 / 退出搜索，都没有时才退出程序
    Escape,
    Up,
    Down,
    Enter,
    Space,
    Next,
    Prev,
    Search,
    Add,
    /// 当前歌单内：选中曲上移一位
    MoveSongUp,
    /// 当前歌单内：选中曲下移一位
    MoveSongDown,
    VolumeUp,
    VolumeDown,
    SeekForward,
    SeekBack,
    CycleMode,
    FocusNext,
    /// v：列表模式 ⇄ 播放页
    ToggleView,
    /// b：播放页封面 Unicode 块 ⇄ 纯 ASCII
    ToggleLogoStyle,
    /// Q：播放队列面板
    ToggleQueue,
    /// ?：快捷键浮层
    ToggleHelp,
    Char(char),
    Backspace,
    Tick,
}

pub fn poll_event(timeout: Duration, search_mode: bool) -> std::io::Result<Option<AppEvent>> {
    if event::poll(timeout)? {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                return Ok(Some(AppEvent::Tick));
            }
            return Ok(Some(if search_mode {
                map_key_search(key)
            } else {
                map_key_normal(key)
            }));
        }
    }
    Ok(None)
}

fn map_key_search(key: KeyEvent) -> AppEvent {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return AppEvent::Quit;
    }
    match key.code {
        KeyCode::Esc => AppEvent::Quit,
        KeyCode::Enter => AppEvent::Enter,
        KeyCode::Backspace => AppEvent::Backspace,
        KeyCode::Char(c) => AppEvent::Char(c),
        _ => AppEvent::Tick,
    }
}

fn map_key_normal(key: KeyEvent) -> AppEvent {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return AppEvent::Quit;
    }
    // Ctrl+↑/↓ 或大写 K/J：歌单调序
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Up => return AppEvent::MoveSongUp,
            KeyCode::Down => return AppEvent::MoveSongDown,
            _ => {}
        }
    }
    match key.code {
        KeyCode::Char('q') => AppEvent::Quit,
        KeyCode::Esc => AppEvent::Escape,
        KeyCode::Char('j') | KeyCode::Down => AppEvent::Down,
        KeyCode::Char('k') | KeyCode::Up => AppEvent::Up,
        KeyCode::Char('J') => AppEvent::MoveSongDown,
        KeyCode::Char('K') => AppEvent::MoveSongUp,
        KeyCode::Enter => AppEvent::Enter,
        KeyCode::Char(' ') => AppEvent::Space,
        KeyCode::Char('n') => AppEvent::Next,
        KeyCode::Char('p') => AppEvent::Prev,
        KeyCode::Char('/') => AppEvent::Search,
        KeyCode::Char('a') => AppEvent::Add,
        KeyCode::Char('+') | KeyCode::Char('=') => AppEvent::VolumeUp,
        KeyCode::Char('-') => AppEvent::VolumeDown,
        KeyCode::Right | KeyCode::Char('l') => AppEvent::SeekForward,
        KeyCode::Left | KeyCode::Char('h') => AppEvent::SeekBack,
        KeyCode::Char('m') => AppEvent::CycleMode,
        KeyCode::Char('v') => AppEvent::ToggleView,
        KeyCode::Char('b') => AppEvent::ToggleLogoStyle,
        KeyCode::Char('Q') => AppEvent::ToggleQueue,
        KeyCode::Char('?') => AppEvent::ToggleHelp,
        KeyCode::Tab => AppEvent::FocusNext,
        KeyCode::Backspace => AppEvent::Backspace,
        KeyCode::Char(c) => AppEvent::Char(c),
        _ => AppEvent::Tick,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn new_keys_are_mapped() {
        assert_eq!(map_key_normal(key('v')), AppEvent::ToggleView);
        assert_eq!(map_key_normal(key('Q')), AppEvent::ToggleQueue);
        assert_eq!(map_key_normal(key('?')), AppEvent::ToggleHelp);
        assert_eq!(map_key_normal(key('q')), AppEvent::Quit);
    }

    #[test]
    fn esc_is_escape_not_quit_in_normal_mode() {
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_key_normal(esc), AppEvent::Escape);
        assert_eq!(map_key_search(esc), AppEvent::Quit);
    }
}
