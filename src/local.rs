//! 本地音乐库：导入音频文件，作为混合歌单中的 `Platform::Local` 曲目。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::AppPaths;
use crate::models::{Platform, Song};

const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "wav", "ogg", "opus", "wma", "ape", "aiff", "aif", "alac",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LibraryFile {
    songs: Vec<Song>,
}

/// 本地曲库（JSON 索引，路径在 song.id 中存绝对路径）。
pub struct LocalLibrary {
    path: PathBuf,
    songs: Vec<Song>,
}

impl LocalLibrary {
    pub fn open(paths: &AppPaths) -> Result<Self> {
        paths.ensure()?;
        let path = paths.local_library.clone();
        let songs = if path.exists() {
            let text =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            let file: LibraryFile =
                serde_json::from_str(&text).context("parse local_library.json")?;
            file.songs
        } else {
            Vec::new()
        };
        Ok(Self { path, songs })
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = LibraryFile {
            songs: self.songs.clone(),
        };
        let text = serde_json::to_string_pretty(&file)?;
        fs::write(&self.path, text).with_context(|| format!("write {}", self.path.display()))?;
        Ok(())
    }

    pub fn list(&self) -> &[Song] {
        &self.songs
    }

    pub fn get(&self, id_or_path: &str) -> Option<&Song> {
        let canon = canonicalize_str(id_or_path);
        self.songs.iter().find(|s| {
            s.id == id_or_path
                || canon
                    .as_ref()
                    .map(|c| s.id == *c || Path::new(&s.id) == Path::new(c))
                    .unwrap_or(false)
        })
    }

    pub fn search(&self, keyword: &str, limit: usize) -> Vec<Song> {
        let kw = keyword.to_lowercase();
        self.songs
            .iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&kw)
                    || s.artists.iter().any(|a| a.to_lowercase().contains(&kw))
                    || s.album
                        .as_ref()
                        .map(|a| a.to_lowercase().contains(&kw))
                        .unwrap_or(false)
                    || s.id.to_lowercase().contains(&kw)
            })
            .take(limit)
            .cloned()
            .collect()
    }

    /// 导入文件或目录（递归音频）。返回新加入的曲目。
    pub fn import_path(&mut self, path: &Path) -> Result<Vec<Song>> {
        let path = path
            .canonicalize()
            .with_context(|| format!("路径无效: {}", path.display()))?;
        let mut added = Vec::new();
        if path.is_file() {
            if let Some(song) = self.import_file(&path)? {
                added.push(song);
            }
        } else if path.is_dir() {
            self.import_dir_recursive(&path, &mut added)?;
        } else {
            bail!("不是文件或目录: {}", path.display());
        }
        if !added.is_empty() {
            self.save()?;
        }
        Ok(added)
    }

    fn import_dir_recursive(&mut self, dir: &Path, added: &mut Vec<Song>) -> Result<()> {
        let rd = fs::read_dir(dir).with_context(|| format!("读取目录 {}", dir.display()))?;
        for ent in rd {
            let ent = ent?;
            let p = ent.path();
            if p.is_dir() {
                self.import_dir_recursive(&p, added)?;
            } else if p.is_file() {
                if let Some(song) = self.import_file(&p)? {
                    added.push(song);
                }
            }
        }
        Ok(())
    }

    fn import_file(&mut self, path: &Path) -> Result<Option<Song>> {
        if !is_audio_file(path) {
            return Ok(None);
        }
        let id = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string();
        if self.songs.iter().any(|s| s.id == id) {
            return Ok(None); // 已在库中
        }
        let song = song_from_path(path, &id);
        self.songs.push(song.clone());
        Ok(Some(song))
    }

    pub fn remove(&mut self, id_or_path: &str) -> Result<Song> {
        let canon = canonicalize_str(id_or_path);
        let pos = self
            .songs
            .iter()
            .position(|s| s.id == id_or_path || canon.as_ref().map(|c| s.id == *c).unwrap_or(false))
            .with_context(|| format!("本地库中未找到: {id_or_path}"))?;
        let song = self.songs.remove(pos);
        self.save()?;
        Ok(song)
    }
}

fn canonicalize_str(s: &str) -> Option<String> {
    Path::new(s)
        .canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        .unwrap_or(false)
}

/// 从文件路径构造本地 Song（id = 绝对路径字符串）。
pub fn song_from_path(path: &Path, id: &str) -> Song {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("未知曲目")
        .to_string();
    let parent_album = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());
    Song {
        platform: Platform::Local,
        id: id.to_string(),
        name,
        artists: vec!["本地".into()],
        album: parent_album.or_else(|| Some("本地文件".into())),
        duration_ms: None,
        cover_url: None,
    }
}

/// 将路径解析为可播放的本地 Song（不必已在库中）。
pub fn song_for_play_path(path: &Path) -> Result<Song> {
    let path = path
        .canonicalize()
        .with_context(|| format!("本地文件不存在: {}", path.display()))?;
    if !path.is_file() {
        bail!("不是文件: {}", path.display());
    }
    if !is_audio_file(&path) {
        bail!("不是支持的音频格式: {}", path.display());
    }
    let id = path.to_string_lossy().to_string();
    Ok(song_from_path(&path, &id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn import_file_and_search() {
        let dir = tempdir().unwrap();
        let music = dir.path().join("track.mp3");
        fs::write(&music, b"fake").unwrap();
        let lib_path = dir.path().join("lib.json");
        let mut lib = LocalLibrary {
            path: lib_path,
            songs: Vec::new(),
        };
        let added = lib.import_path(&music).unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].platform, Platform::Local);
        assert_eq!(added[0].name, "track");
        let hits = lib.search("track", 10);
        assert_eq!(hits.len(), 1);
        // second import no dup
        assert!(lib.import_path(&music).unwrap().is_empty());
    }
}
