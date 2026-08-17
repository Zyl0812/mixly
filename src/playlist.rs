use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{PlayMode, Playlist, Song};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlaylistIndex {
    playlists: Vec<PlaylistMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlaylistMeta {
    id: String,
    name: String,
    file: String,
}

pub struct PlaylistStore {
    dir: PathBuf,
    index_path: PathBuf,
}

impl PlaylistStore {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let index_path = dir.join("index.json");
        let store = Self { dir, index_path };
        if !store.index_path.exists() {
            store.write_index(&PlaylistIndex {
                playlists: Vec::new(),
            })?;
        }
        Ok(store)
    }

    fn read_index(&self) -> Result<PlaylistIndex> {
        let text = fs::read_to_string(&self.index_path)
            .with_context(|| format!("read {}", self.index_path.display()))?;
        serde_json::from_str(&text).context("parse playlist index")
    }

    fn write_index(&self, index: &PlaylistIndex) -> Result<()> {
        let text = serde_json::to_string_pretty(index)?;
        fs::write(&self.index_path, text)?;
        Ok(())
    }

    fn playlist_path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    pub fn list(&self) -> Result<Vec<Playlist>> {
        let index = self.read_index()?;
        let mut out = Vec::new();
        for meta in index.playlists {
            if let Ok(pl) = self.load_file(&self.playlist_path(&meta.file)) {
                out.push(pl);
            }
        }
        Ok(out)
    }

    fn load_file(&self, path: &Path) -> Result<Playlist> {
        let text = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    fn save_playlist(&self, pl: &Playlist) -> Result<()> {
        let file = format!("{}.json", pl.id);
        let path = self.playlist_path(&file);
        let text = serde_json::to_string_pretty(pl)?;
        fs::write(path, text)?;

        let mut index = self.read_index()?;
        if let Some(m) = index.playlists.iter_mut().find(|m| m.id == pl.id) {
            m.name = pl.name.clone();
            m.file = file;
        } else {
            index.playlists.push(PlaylistMeta {
                id: pl.id.clone(),
                name: pl.name.clone(),
                file,
            });
        }
        self.write_index(&index)?;
        Ok(())
    }

    pub fn create(&self, name: &str) -> Result<Playlist> {
        let now = now_ts();
        let pl = Playlist {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            songs: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.save_playlist(&pl)?;
        Ok(pl)
    }

    pub fn find(&self, id_or_name: &str) -> Result<Playlist> {
        let index = self.read_index()?;
        let meta = index
            .playlists
            .iter()
            .find(|m| m.id == id_or_name || m.name.eq_ignore_ascii_case(id_or_name))
            .with_context(|| format!("playlist not found: {id_or_name}"))?;
        self.load_file(&self.playlist_path(&meta.file))
    }

    pub fn delete(&self, id_or_name: &str) -> Result<()> {
        let mut index = self.read_index()?;
        let pos = index
            .playlists
            .iter()
            .position(|m| m.id == id_or_name || m.name.eq_ignore_ascii_case(id_or_name))
            .with_context(|| format!("playlist not found: {id_or_name}"))?;
        let meta = index.playlists.remove(pos);
        let path = self.playlist_path(&meta.file);
        let _ = fs::remove_file(path);
        self.write_index(&index)?;
        Ok(())
    }

    pub fn rename(&self, id_or_name: &str, new_name: &str) -> Result<Playlist> {
        let mut pl = self.find(id_or_name)?;
        pl.name = new_name.to_string();
        pl.updated_at = now_ts();
        self.save_playlist(&pl)?;
        Ok(pl)
    }

    pub fn add_song(&self, id_or_name: &str, song: Song) -> Result<Playlist> {
        let mut pl = self.find(id_or_name)?;
        pl.songs.push(song);
        pl.updated_at = now_ts();
        self.save_playlist(&pl)?;
        Ok(pl)
    }

    /// 将播放时识别到的专辑补回所有同平台、同 ID 的歌单条目。
    /// 只填空值，不覆盖用户已保存的专辑名。
    pub fn backfill_album(&self, song: &Song) -> Result<bool> {
        let Some(album) = song
            .album
            .as_deref()
            .map(str::trim)
            .filter(|album| !album.is_empty())
        else {
            return Ok(false);
        };

        let index = self.read_index()?;
        let mut updated = false;
        for meta in index.playlists {
            let mut pl = self.load_file(&self.playlist_path(&meta.file))?;
            let mut changed = false;
            for stored in &mut pl.songs {
                let missing = stored
                    .album
                    .as_deref()
                    .map(str::trim)
                    .filter(|album| !album.is_empty())
                    .is_none();
                if missing && stored.platform == song.platform && stored.id == song.id {
                    stored.album = Some(album.to_owned());
                    changed = true;
                }
            }
            if changed {
                pl.updated_at = now_ts();
                self.save_playlist(&pl)?;
                updated = true;
            }
        }
        Ok(updated)
    }

    pub fn remove_at(&self, id_or_name: &str, index: usize) -> Result<Playlist> {
        let mut pl = self.find(id_or_name)?;
        if index >= pl.songs.len() {
            bail!("index {index} out of range (len={})", pl.songs.len());
        }
        pl.songs.remove(index);
        pl.updated_at = now_ts();
        self.save_playlist(&pl)?;
        Ok(pl)
    }

    pub fn reorder(&self, id_or_name: &str, from: usize, to: usize) -> Result<Playlist> {
        let mut pl = self.find(id_or_name)?;
        let len = pl.songs.len();
        if len == 0 {
            bail!("歌单为空，无法调序");
        }
        if from >= len || to >= len {
            bail!(
                "调序下标越界：from={from} to={to}，歌单共 {len} 首（下标 0..{}）",
                len - 1
            );
        }
        if from == to {
            return Ok(pl);
        }
        let song = pl.songs.remove(from);
        pl.songs.insert(to, song);
        pl.updated_at = now_ts();
        self.save_playlist(&pl)?;
        Ok(pl)
    }
}

fn now_ts() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Fisher–Yates 打乱，时间种子 LCG（免 rand 依赖）。CLI 歌单与队列洗牌共用。
pub fn shuffle<T>(items: &mut [T]) {
    if items.len() < 2 {
        return;
    }
    let mut state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(1);
    for i in (1..items.len()).rev() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (state as usize) % (i + 1);
        items.swap(i, j);
    }
}

/// In-process play queue: one track loaded at a time via relay.
#[derive(Debug, Clone, Default)]
pub struct PlayQueue {
    pub songs: Vec<Song>,
    pub index: usize,
    pub mode: PlayMode,
    shuffle_order: Vec<usize>,
}

impl PlayQueue {
    pub fn from_songs(songs: Vec<Song>) -> Self {
        let mut q = Self {
            songs,
            index: 0,
            mode: PlayMode::Sequential,
            shuffle_order: Vec::new(),
        };
        q.rebuild_shuffle();
        q
    }

    pub fn current(&self) -> Option<&Song> {
        self.songs.get(self.index)
    }

    /// 预览「切歌后」会播的下一首（不修改队列）。
    /// 单曲循环 / 仅一首的列表循环返回 `None`（无需预取自己）。
    pub fn peek_next(&self) -> Option<&Song> {
        if self.songs.is_empty() {
            return None;
        }
        match self.mode {
            PlayMode::LoopOne => None,
            PlayMode::Sequential => {
                if self.index + 1 < self.songs.len() {
                    self.songs.get(self.index + 1)
                } else {
                    None
                }
            }
            PlayMode::LoopAll => {
                if self.songs.len() == 1 {
                    None
                } else {
                    self.songs.get((self.index + 1) % self.songs.len())
                }
            }
            PlayMode::Shuffle => {
                if self.songs.len() == 1 {
                    return None;
                }
                if let Some(pos) = self.shuffle_order.iter().position(|&i| i == self.index) {
                    let next_pos = (pos + 1) % self.shuffle_order.len();
                    let next_idx = self.shuffle_order[next_pos];
                    self.songs.get(next_idx)
                } else {
                    self.songs.get((self.index + 1) % self.songs.len())
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.songs.len()
    }

    fn rebuild_shuffle(&mut self) {
        self.shuffle_order = (0..self.songs.len()).collect();
        shuffle(&mut self.shuffle_order);
    }

    pub fn set_mode(&mut self, mode: PlayMode) {
        self.mode = mode;
        if mode == PlayMode::Shuffle {
            self.rebuild_shuffle();
        }
    }

    pub fn cycle_mode(&mut self) {
        self.set_mode(self.mode.next());
    }

    /// Advance after eof. Returns false if queue should stop.
    pub fn advance(&mut self) -> bool {
        if self.songs.is_empty() {
            return false;
        }
        match self.mode {
            PlayMode::LoopOne => true,
            PlayMode::Sequential => {
                if self.index + 1 < self.songs.len() {
                    self.index += 1;
                    true
                } else {
                    false
                }
            }
            PlayMode::LoopAll => {
                self.index = (self.index + 1) % self.songs.len();
                true
            }
            PlayMode::Shuffle => {
                // next in shuffle order
                if let Some(pos) = self.shuffle_order.iter().position(|&i| i == self.index) {
                    let next_pos = (pos + 1) % self.shuffle_order.len();
                    self.index = self.shuffle_order[next_pos];
                    true
                } else {
                    self.index = 0;
                    true
                }
            }
        }
    }

    pub fn next_manual(&mut self) -> bool {
        if self.songs.is_empty() {
            return false;
        }
        match self.mode {
            PlayMode::Shuffle => {
                if let Some(pos) = self.shuffle_order.iter().position(|&i| i == self.index) {
                    self.index = self.shuffle_order[(pos + 1) % self.shuffle_order.len()];
                } else {
                    self.index = (self.index + 1) % self.songs.len();
                }
            }
            _ => {
                self.index = (self.index + 1) % self.songs.len();
            }
        }
        true
    }

    pub fn prev_manual(&mut self) -> bool {
        if self.songs.is_empty() {
            return false;
        }
        match self.mode {
            PlayMode::Shuffle => {
                if let Some(pos) = self.shuffle_order.iter().position(|&i| i == self.index) {
                    let prev = if pos == 0 {
                        self.shuffle_order.len() - 1
                    } else {
                        pos - 1
                    };
                    self.index = self.shuffle_order[prev];
                } else if self.index == 0 {
                    self.index = self.songs.len() - 1;
                } else {
                    self.index -= 1;
                }
            }
            _ => {
                if self.index == 0 {
                    self.index = self.songs.len() - 1;
                } else {
                    self.index -= 1;
                }
            }
        }
        true
    }

    pub fn jump(&mut self, index: usize) -> bool {
        if index < self.songs.len() {
            self.index = index;
            true
        } else {
            false
        }
    }

    /// 用补全后的元数据覆盖当前曲（播放时 enrich 后写回队列，供 UI 展示）。
    pub fn update_current(&mut self, song: Song) {
        if let Some(slot) = self.songs.get_mut(self.index) {
            *slot = song;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Platform;
    use tempfile::tempdir;

    fn sample_song(id: &str) -> Song {
        Song {
            platform: Platform::Netease,
            id: id.into(),
            name: format!("Song {id}"),
            artists: vec!["A".into()],
            album: None,
            duration_ms: None,
            cover_url: None,
        }
    }

    #[test]
    fn playlist_crud_roundtrip() {
        let dir = tempdir().unwrap();
        let store = PlaylistStore::new(dir.path()).unwrap();
        let pl = store.create("Mix").unwrap();
        assert_eq!(pl.name, "Mix");
        store.add_song(&pl.id, sample_song("1")).unwrap();
        store.add_song("Mix", sample_song("2")).unwrap();
        let loaded = store.find("Mix").unwrap();
        assert_eq!(loaded.songs.len(), 2);
        store.remove_at("Mix", 0).unwrap();
        let loaded = store.find(&pl.id).unwrap();
        assert_eq!(loaded.songs.len(), 1);
        assert_eq!(loaded.songs[0].id, "2");
        store.rename("Mix", "Mixed").unwrap();
        assert_eq!(store.find(&pl.id).unwrap().name, "Mixed");
        store.delete(&pl.id).unwrap();
        assert!(store.find(&pl.id).is_err());
    }

    #[test]
    fn mixed_playlist_roundtrips_all_platforms() {
        // 三在线平台 + 本地文件混合歌单，往返一致
        let dir = tempdir().unwrap();
        let store = PlaylistStore::new(dir.path()).unwrap();
        let pl = store.create("混排").unwrap();
        store
            .add_song(
                &pl.id,
                Song {
                    platform: Platform::Qq,
                    id: "0039MnYb0qxYhV".into(),
                    name: "晴天".into(),
                    artists: vec!["周杰伦".into()],
                    album: Some("叶惠美".into()),
                    duration_ms: Some(269_000),
                    cover_url: None,
                },
            )
            .unwrap();
        store
            .add_song(
                &pl.id,
                Song {
                    platform: Platform::Netease,
                    id: "347230".into(),
                    name: "海阔天空".into(),
                    artists: vec!["Beyond".into()],
                    album: None,
                    duration_ms: None,
                    cover_url: None,
                },
            )
            .unwrap();
        store
            .add_song(
                &pl.id,
                Song {
                    platform: Platform::Bilibili,
                    id: "BV1xx411c7mD:p2".into(),
                    name: "晴天（P2）".into(),
                    artists: vec!["某某UP".into()],
                    album: Some("音乐".into()),
                    duration_ms: Some(225_000),
                    cover_url: None,
                },
            )
            .unwrap();
        store
            .add_song(
                &pl.id,
                Song {
                    platform: Platform::Local,
                    id: "D:\\music\\demo.flac".into(),
                    name: "demo".into(),
                    artists: vec!["本地".into()],
                    album: Some("本地文件".into()),
                    duration_ms: None,
                    cover_url: None,
                },
            )
            .unwrap();

        let loaded = store.find("混排").unwrap();
        assert_eq!(loaded.songs.len(), 4);
        let platforms: Vec<Platform> = loaded.songs.iter().map(|s| s.platform).collect();
        assert_eq!(
            platforms,
            vec![
                Platform::Qq,
                Platform::Netease,
                Platform::Bilibili,
                Platform::Local
            ]
        );
        let bili = &loaded.songs[2];
        assert_eq!(bili.id, "BV1xx411c7mD:p2");
        assert_eq!(bili.name, "晴天（P2）");
    }

    #[test]
    fn playback_backfills_album_without_overwriting_existing_value() {
        let dir = tempdir().unwrap();
        let store = PlaylistStore::new(dir.path()).unwrap();
        let pl = store.create("Mix").unwrap();
        store.add_song(&pl.id, sample_song("1")).unwrap();
        let mut existing = sample_song("1");
        existing.album = Some("手动专辑".into());
        store.add_song(&pl.id, existing).unwrap();

        let mut enriched = sample_song("1");
        enriched.album = Some("在线专辑".into());
        assert!(store.backfill_album(&enriched).unwrap());

        let loaded = store.find(&pl.id).unwrap();
        assert_eq!(loaded.songs[0].album.as_deref(), Some("在线专辑"));
        assert_eq!(loaded.songs[1].album.as_deref(), Some("手动专辑"));
        assert!(!store.backfill_album(&enriched).unwrap());
    }

    #[test]
    fn queue_sequential_advance() {
        let mut q = PlayQueue::from_songs(vec![sample_song("a"), sample_song("b")]);
        assert_eq!(q.current().unwrap().id, "a");
        assert!(q.advance());
        assert_eq!(q.current().unwrap().id, "b");
        assert!(!q.advance());
    }

    #[test]
    fn peek_next_sequential() {
        let q = PlayQueue::from_songs(vec![sample_song("a"), sample_song("b")]);
        assert_eq!(q.peek_next().unwrap().id, "b");
        let mut q = q;
        q.advance();
        assert!(q.peek_next().is_none());
    }

    #[test]
    fn shuffle_songs_preserves_members() {
        let mut songs = vec![
            sample_song("a"),
            sample_song("b"),
            sample_song("c"),
            sample_song("d"),
        ];
        let before: std::collections::HashSet<_> = songs.iter().map(|s| s.id.clone()).collect();
        shuffle(&mut songs);
        let after: std::collections::HashSet<_> = songs.iter().map(|s| s.id.clone()).collect();
        assert_eq!(before, after);
        assert_eq!(songs.len(), 4);
    }

    #[test]
    fn playlist_reorder_moves_song() {
        let dir = tempdir().unwrap();
        let store = PlaylistStore::new(dir.path()).unwrap();
        let pl = store.create("Ord").unwrap();
        store.add_song(&pl.id, sample_song("a")).unwrap();
        store.add_song(&pl.id, sample_song("b")).unwrap();
        store.add_song(&pl.id, sample_song("c")).unwrap();
        // a b c → move index 2 to 0 → c a b
        let pl = store.reorder(&pl.id, 2, 0).unwrap();
        assert_eq!(
            pl.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
        // move 1 (a) to 2 → c b a
        let pl = store.reorder("Ord", 1, 2).unwrap();
        assert_eq!(
            pl.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );
    }
}
