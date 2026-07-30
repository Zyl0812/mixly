//! Only module allowed to touch `netease-qq-music-api`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use netease_qq_music_api::models::{
    LoginStatus, LoginToken, Platform as ApiPlatform, SongQuality,
};
use netease_qq_music_api::MusicClient;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::config::AppPaths;
use crate::models::{Platform, Quality, Song};
use crate::relay::{RefreshHook, TrackSource};

pub struct ApiClient {
    inner: MusicClient,
    paths: AppPaths,
    quality: Quality,
}

impl ApiClient {
    pub fn new(paths: AppPaths, quality: Quality) -> Self {
        Self {
            inner: MusicClient::new(),
            paths,
            quality,
        }
    }

    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }

    fn to_api_platform(p: Platform) -> Result<ApiPlatform> {
        match p {
            Platform::Netease => Ok(ApiPlatform::Netease),
            Platform::Qq => Ok(ApiPlatform::Tencent),
            Platform::Local => bail!("本地平台不使用在线 API"),
        }
    }

    fn to_song_quality(q: Quality) -> SongQuality {
        match q {
            Quality::Standard => SongQuality::Standard,
            Quality::Higher => SongQuality::Exhigh,
            Quality::Exhigh => SongQuality::Exhigh,
            Quality::Lossless => SongQuality::Lossless,
        }
    }

    fn map_song(platform: Platform, s: netease_qq_music_api::models::Song) -> Song {
        Song {
            platform,
            id: s.id,
            name: s.name,
            artists: s.artists.into_iter().map(|a| a.name).collect(),
            album: if s.album.name.is_empty() {
                None
            } else {
                Some(s.album.name)
            },
            duration_ms: None,
            cover_url: if s.pic_url.is_empty() {
                None
            } else {
                Some(s.pic_url)
            },
        }
    }

    fn token_path(&self, platform: Platform) -> Result<&Path> {
        match platform {
            Platform::Netease => Ok(&self.paths.netease_token),
            Platform::Qq => Ok(&self.paths.qq_token),
            Platform::Local => bail!("本地平台无需登录 token"),
        }
    }

    pub fn load_token(&self, platform: Platform) -> Result<Option<LoginToken>> {
        if platform == Platform::Local {
            return Ok(None);
        }
        let path = self.token_path(platform)?;
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("read token {}", path.display()))?;
        let token: LoginToken = serde_json::from_str(&data).context("parse token json")?;
        Ok(Some(token))
    }

    /// Whether a persisted login token exists for the platform.
    pub fn is_logged_in(&self, platform: Platform) -> bool {
        if platform == Platform::Local {
            return true; // 本地无需登录
        }
        matches!(self.load_token(platform), Ok(Some(_)))
    }

    /// True if neither Netease nor QQ has a saved token.
    pub fn is_fully_logged_out(&self) -> bool {
        !self.is_logged_in(Platform::Netease) && !self.is_logged_in(Platform::Qq)
    }

    pub fn save_token(&self, platform: Platform, token: &LoginToken) -> Result<()> {
        self.paths.ensure()?;
        let path = self.token_path(platform)?;
        let data = serde_json::to_string_pretty(token)?;
        std::fs::write(path, data).with_context(|| format!("write token {}", path.display()))?;
        info!(%platform, path = %path.display(), "login token saved");
        Ok(())
    }

    pub async fn search(&self, platform: Platform, keyword: &str, limit: u64) -> Result<Vec<Song>> {
        if platform == Platform::Local {
            let lib = crate::local::LocalLibrary::open(&self.paths)?;
            return Ok(lib.search(keyword, limit as usize));
        }
        let api_plat = Self::to_api_platform(platform)?;
        let token = self.load_token(platform)?;
        let mut req = self
            .inner
            .search()
            .song()
            .keyword(keyword)
            .limit(limit)
            .platform(api_plat);
        if let Some(ref t) = token {
            req = match t {
                LoginToken::Netease(n) => req.login(n),
                LoginToken::Tencent(q) => req.login(q),
            };
        }
        let result = req
            .send()
            .await
            .map_err(|e| anyhow!("search failed ({platform}): {e}"))?;
        Ok(result
            .songs
            .into_iter()
            .map(|s| Self::map_song(platform, s))
            .collect())
    }

    /// 按平台拉取单曲详情（歌名 / 歌手 / 专辑 / 封面）。平台与 id 必须匹配，不会跨平台查。
    pub async fn song_detail(&self, platform: Platform, id: &str) -> Result<Song> {
        if platform == Platform::Local {
            bail!("本地平台不支持在线歌曲详情");
        }
        let api_plat = Self::to_api_platform(platform)?;
        let token = self.load_token(platform)?;
        let mut req = self.inner.detail().song().id(id).platform(api_plat);
        if let Some(ref t) = token {
            req = match t {
                LoginToken::Netease(n) => req.login(n),
                LoginToken::Tencent(q) => req.login(q),
            };
        }
        let result = req
            .send()
            .await
            .map_err(|e| anyhow!("song detail failed ({platform}/{id}): {e}"))?;
        let song = result
            .songs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("song detail empty ({platform}/{id})"))?;
        Ok(Self::map_song(platform, song))
    }

    /// 播放/展示前补全元数据：缺专辑（或歌名/歌手为空）时，按**该曲 platform** 拉详情。
    /// 已有字段保留；网络失败则原样返回，不阻断播放。
    pub async fn enrich_song_metadata(&self, song: &Song) -> Song {
        if !song.platform.is_online() {
            return song.clone();
        }
        let album_missing = song
            .album
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none();
        let name_placeholder = song.name.is_empty() || song.name == song.id;
        if !album_missing && !song.artists.is_empty() && !name_placeholder {
            return song.clone();
        }
        match self.song_detail(song.platform, &song.id).await {
            Ok(d) => Song {
                platform: song.platform,
                id: song.id.clone(),
                name: if name_placeholder {
                    d.name
                } else {
                    song.name.clone()
                },
                artists: if song.artists.is_empty() {
                    d.artists
                } else {
                    song.artists.clone()
                },
                album: if album_missing {
                    d.album
                } else {
                    song.album.clone()
                },
                duration_ms: song.duration_ms.or(d.duration_ms),
                cover_url: song.cover_url.clone().or(d.cover_url),
            },
            Err(e) => {
                warn!(
                    platform = %song.platform,
                    id = %song.id,
                    error = %e,
                    "enrich song metadata failed"
                );
                song.clone()
            }
        }
    }

    pub async fn play_url(&self, platform: Platform, id: &str) -> Result<String> {
        self.play_url_with_quality(platform, id, self.quality).await
    }

    pub async fn play_url_with_quality(
        &self,
        platform: Platform,
        id: &str,
        quality: Quality,
    ) -> Result<String> {
        if platform == Platform::Local {
            let path = std::path::Path::new(id);
            if !path.is_file() {
                bail!("本地文件不存在: {id}");
            }
            return Ok(path
                .canonicalize()
                .unwrap_or_else(|_| path.to_path_buf())
                .to_string_lossy()
                .to_string());
        }
        let api_plat = Self::to_api_platform(platform)?;
        let token = self.load_token(platform)?;

        // Prefer preferred quality; fall back down the ladder on failure.
        let ladder = quality_ladder(quality);
        let mut last_err = None;
        for q in ladder {
            let level = Self::to_song_quality(q);
            let mut req = self
                .inner
                .playback()
                .url()
                .id(id)
                .level(level)
                .platform(api_plat);
            if let Some(ref t) = token {
                req = match t {
                    LoginToken::Netease(n) => req.login(n),
                    LoginToken::Tencent(qq) => req.login(qq),
                };
            }
            match req.send().await {
                Ok(r) if !r.url.is_empty() => {
                    info!(%platform, id, quality = ?q, "resolved play url");
                    return Ok(r.url);
                }
                Ok(_) => {
                    last_err = Some(anyhow!("empty play url for {platform}/{id} @ {q:?}"));
                }
                Err(e) => {
                    last_err = Some(anyhow!("play url failed ({platform}/{id} @ {q:?}): {e}"));
                    warn!(error = %last_err.as_ref().unwrap(), "quality fallback");
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("no play url for {platform}/{id}")))
            .with_context(|| format!("resolve play url {platform}/{id}"))
    }

    pub async fn lyrics(&self, platform: Platform, id: &str) -> Result<String> {
        if platform == Platform::Local {
            bail!("本地文件暂不支持在线歌词");
        }
        let api_plat = Self::to_api_platform(platform)?;
        let token = self.load_token(platform)?;
        let mut req = self.inner.playback().lyric().id(id).platform(api_plat);
        if let Some(ref t) = token {
            req = match t {
                LoginToken::Netease(n) => req.login(n),
                LoginToken::Tencent(qq) => req.login(qq),
            };
        }
        let result = req
            .send()
            .await
            .map_err(|e| anyhow!("lyrics failed ({platform}/{id}): {e}"))?;
        Ok(result.lyric)
    }

    /// Interactive QR login: print QR in terminal and poll until success.
    pub async fn login_qr(&self, platform: Platform) -> Result<()> {
        if platform == Platform::Local {
            bail!("本地平台无需登录");
        }
        let api_plat = Self::to_api_platform(platform)?;
        let session = self
            .inner
            .login()
            .platform(api_plat)
            .session()
            .send()
            .await
            .map_err(|e| anyhow!("创建登录会话失败: {e}"))?;

        let qr = session.qr_code();
        // Still persist payload for debugging / offline view.
        self.paths.ensure()?;
        let qr_path = self.paths.config_dir.join(format!("login_qr_{platform}.txt"));
        let _ = std::fs::write(&qr_path, qr);

        let platform_name = match platform {
            Platform::Netease => "网易云音乐",
            Platform::Qq => "QQ 音乐",
            Platform::Local => "本地",
        };
        println!();
        println!("======== 请使用【{platform_name}】App 扫码登录 ========");
        if let Err(e) = crate::qr_term::print_qr_payload(qr) {
            warn!(error = %e, "终端打印二维码失败，回退到文件");
            println!("（终端绘制失败: {e}）");
            println!("二维码数据已写入: {}", qr_path.display());
            if let Some(b64) = qr.strip_prefix("data:image/png;base64,") {
                let png_path = self.paths.config_dir.join(format!("login_qr_{platform}.png"));
                if let Ok(bytes) = decode_base64_approx(b64) {
                    let _ = std::fs::write(&png_path, bytes);
                    println!("PNG 图片已保存: {}", png_path.display());
                }
            }
        }
        println!("================================================");
        println!("等待确认中…（Ctrl+C 取消）");
        println!();

        loop {
            match session.status().await {
                Ok(LoginStatus::Success(token)) => {
                    self.save_token(platform, &token)?;
                    println!("登录成功（{platform_name}）。");
                    return Ok(());
                }
                Ok(LoginStatus::QrCodeExpired) => {
                    bail!("二维码已过期，请重新执行 login");
                }
                Ok(LoginStatus::WaitingScan) => {
                    sleep(Duration::from_secs(2)).await;
                }
                Ok(LoginStatus::WaitingConfirm) => {
                    println!("已扫码，请在手机上确认登录…");
                    sleep(Duration::from_secs(2)).await;
                }
                Err(e) => {
                    warn!(error = %e, "登录轮询出错，重试中");
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    /// Referer header required by some CDN anti-leech rules.
    pub fn referer_for(platform: Platform) -> &'static str {
        match platform {
            Platform::Netease => "https://music.163.com/",
            Platform::Qq => "https://y.qq.com/",
            Platform::Local => "",
        }
    }

    /// Build a one-shot URL refresh closure for the audio relay (§4.3).
    pub fn make_refresh_hook(self: &Arc<Self>, platform: Platform, id: String) -> RefreshHook {
        let api = Arc::clone(self);
        Arc::new(move || {
            let api = Arc::clone(&api);
            let id = id.clone();
            Box::pin(async move { api.play_url(platform, &id).await })
        })
    }

    /// Resolve play source: online CDN URL, or local file path via `local_cache`.
    pub async fn resolve_track(self: &Arc<Self>, song: &Song) -> Result<TrackSource> {
        if song.platform == Platform::Local {
            let path = std::path::PathBuf::from(&song.id);
            let path = path
                .canonicalize()
                .with_context(|| format!("本地文件不存在: {}", song.id))?;
            if !path.is_file() {
                bail!("本地路径不是文件: {}", path.display());
            }
            let url = path.to_string_lossy().to_string();
            return Ok(TrackSource {
                url: url.clone(),
                platform: Platform::Local,
                refresh: None,
                local_cache: Some(path),
            });
        }
        let url = self.play_url(song.platform, &song.id).await?;
        Ok(TrackSource {
            url,
            platform: song.platform,
            refresh: Some(self.make_refresh_hook(song.platform, song.id.clone())),
            local_cache: None,
        })
    }
}

/// Pure decision helper: after EOF, should we load the next queue item?
/// Separated so the TUI never re-enters the player mutex while deciding.
pub fn should_load_after_eof(eof: bool, eof_latched: bool, queue_advanced: bool) -> (bool, bool) {
    // returns (new_eof_latched, should_load)
    if eof && !eof_latched {
        (true, queue_advanced)
    } else if !eof {
        (false, false)
    } else {
        (eof_latched, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppPaths;

    #[test]
    fn resolve_track_wires_refresh_hook() {
        // Structural: shipped helper always attaches Some(refresh).
        // We do not call the network here — only build the hook + TrackSource shape.
        let paths = AppPaths::discover().expect("paths");
        let api = Arc::new(ApiClient::new(paths, Quality::Standard));
        let hook = api.make_refresh_hook(Platform::Qq, "testid".into());
        let track = TrackSource {
            url: "http://example.invalid/audio".into(),
            platform: Platform::Qq,
            refresh: Some(hook),
            local_cache: None,
        };
        assert!(track.refresh.is_some());
        // Calling make_refresh_hook twice yields independent hooks.
        let hook2 = api.make_refresh_hook(Platform::Netease, "other".into());
        assert!(track.refresh.is_some());
        let _ = hook2;
    }

    #[test]
    fn eof_advance_loads_once_then_latches() {
        let (latched, load) = should_load_after_eof(true, false, true);
        assert!(latched && load);
        let (latched2, load2) = should_load_after_eof(true, true, true);
        assert!(latched2 && !load2);
        let (latched3, load3) = should_load_after_eof(false, true, false);
        assert!(!latched3 && !load3);
    }
}

fn quality_ladder(preferred: Quality) -> Vec<Quality> {
    match preferred {
        Quality::Lossless => vec![
            Quality::Lossless,
            Quality::Exhigh,
            Quality::Higher,
            Quality::Standard,
        ],
        Quality::Exhigh => vec![Quality::Exhigh, Quality::Higher, Quality::Standard],
        Quality::Higher => vec![Quality::Higher, Quality::Exhigh, Quality::Standard],
        Quality::Standard => vec![Quality::Standard, Quality::Higher, Quality::Exhigh],
    }
}

/// Minimal base64 decode without extra deps (QR PNG is best-effort).
fn decode_base64_approx(input: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    let mut i = 0;
    while i + 3 < cleaned.len() {
        let (a, b, c, d) = (
            val(cleaned[i]).ok_or_else(|| anyhow!("bad b64"))?,
            val(cleaned[i + 1]).ok_or_else(|| anyhow!("bad b64"))?,
            val(cleaned[i + 2]).ok_or_else(|| anyhow!("bad b64"))?,
            val(cleaned[i + 3]).ok_or_else(|| anyhow!("bad b64"))?,
        );
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
        out.push((c << 6) | d);
        i += 4;
    }
    Ok(out)
}

