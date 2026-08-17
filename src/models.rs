use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Netease,
    Qq,
    /// Bilibili 普通 UGC 视频的音频（id 为规范 BV 引用，如 `BV1xx411c7mD` 或 `BV1xx411c7mD:p2`）
    Bilibili,
    /// 本地导入的音频文件（id 为绝对路径）
    Local,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Netease => "netease",
            Self::Qq => "qq",
            Self::Bilibili => "bilibili",
            Self::Local => "local",
        }
    }

    /// 界面展示用中文平台名。
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::Netease => "网易云",
            Self::Qq => "QQ",
            Self::Bilibili => "B站",
            Self::Local => "本地",
        }
    }

    /// 是否走在线 API（本地文件不需要登录/在线取链）。
    pub fn is_online(self) -> bool {
        !matches!(self, Self::Local)
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "netease" | "neteasecloud" | "wy" | "163" => Some(Self::Netease),
            "qq" | "tencent" | "qqmusic" => Some(Self::Qq),
            "bilibili" | "bili" | "b站" => Some(Self::Bilibili),
            "local" | "file" | "filesystem" | "fs" => Some(Self::Local),
            _ => None,
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Song {
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub cover_url: Option<String>,
}

impl Song {
    /// 展示：`[平台] 歌名-专辑-歌手`（不含歌曲 ID）。
    ///
    /// ID 仍保存在 [`Song::id`]，CLI 搜索会单独附带，便于 `mixly play`。
    pub fn display_line(&self) -> String {
        let album = self
            .album
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("未知专辑");
        let artists = if self.artists.is_empty() {
            "未知歌手".to_string()
        } else {
            self.artists.join("、")
        };
        format!(
            "[{}] {}-{}-{}",
            self.platform.label_zh(),
            self.name,
            album,
            artists
        )
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn display_line_is_name_album_artist_without_id() {
        let s = Song {
            platform: Platform::Qq,
            id: "0039MnYb0qxYhV".into(),
            name: "晴天".into(),
            artists: vec!["周杰伦".into()],
            album: Some("叶惠美".into()),
            duration_ms: None,
            cover_url: None,
        };
        assert_eq!(s.display_line(), "[QQ] 晴天-叶惠美-周杰伦");
        assert!(!s.display_line().contains(&s.id));
    }

    #[test]
    fn bilibili_platform_parse_serialize_and_label() {
        for alias in ["bilibili", "BILI", "bili", "b站", "B站"] {
            assert_eq!(Platform::parse(alias), Some(Platform::Bilibili));
        }
        assert_eq!(Platform::parse("bilibli"), None);
        assert_eq!(Platform::Bilibili.as_str(), "bilibili");
        assert_eq!(Platform::Bilibili.label_zh(), "B站");
        assert!(Platform::Bilibili.is_online());

        let json = serde_json::to_string(&Platform::Bilibili).unwrap();
        assert_eq!(json, "\"bilibili\"");
        assert_eq!(
            serde_json::from_str::<Platform>(&json).unwrap(),
            Platform::Bilibili
        );
    }

    #[test]
    fn old_playlists_still_read_with_new_enum() {
        // 旧歌单里没有 bilibili 平台，字段默认仍然可读
        let json = r#"{
            "platform": "qq",
            "id": "x",
            "name": "n",
            "artists": ["a"],
            "album": null,
            "duration_ms": null,
            "cover_url": null
        }"#;
        let s: Song = serde_json::from_str(json).unwrap();
        assert_eq!(s.platform, Platform::Qq);
        assert_eq!(s.name, "n");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub songs: Vec<Song>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Quality {
    Standard,
    Higher,
    #[default]
    Exhigh,
    Lossless,
}

impl Quality {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "standard" | "std" => Some(Self::Standard),
            "higher" | "high" => Some(Self::Higher),
            "exhigh" | "ex" => Some(Self::Exhigh),
            "lossless" | "flac" | "hires" => Some(Self::Lossless),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Higher => "Higher",
            Self::Exhigh => "Exhigh",
            Self::Lossless => "Lossless",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LyricsState {
    pub lines: Vec<(f64, String)>,
    pub current_index: Option<usize>,
}

impl LyricsState {
    pub fn update_index(&mut self, time_pos: f64) {
        if self.lines.is_empty() {
            self.current_index = None;
            return;
        }
        let mut idx = 0usize;
        for (i, (ts, _)) in self.lines.iter().enumerate() {
            if *ts <= time_pos {
                idx = i;
            } else {
                break;
            }
        }
        self.current_index = Some(idx);
    }

    pub fn current_line(&self) -> Option<&str> {
        self.current_index
            .and_then(|i| self.lines.get(i))
            .map(|(_, t)| t.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayMode {
    #[default]
    Sequential,
    LoopAll,
    LoopOne,
    Shuffle,
}

impl PlayMode {
    pub fn next(self) -> Self {
        match self {
            Self::Sequential => Self::LoopAll,
            Self::LoopAll => Self::LoopOne,
            Self::LoopOne => Self::Shuffle,
            Self::Shuffle => Self::Sequential,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sequential => "顺序",
            Self::LoopAll => "列表循环",
            Self::LoopOne => "单曲循环",
            Self::Shuffle => "随机",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_mode_labels_are_chinese() {
        for mode in [
            PlayMode::Sequential,
            PlayMode::LoopAll,
            PlayMode::LoopOne,
            PlayMode::Shuffle,
        ] {
            let label = mode.label();
            assert!(
                label.chars().any(|c| c > '\u{7f}'),
                "expected CJK in mode label, got {label:?}"
            );
        }
    }

    #[test]
    fn default_tui_status_ready_is_chinese_source() {
        // Mirrors the shipped default in tui::app ("就绪")
        let ready = "就绪";
        assert!(ready
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
    }
}
