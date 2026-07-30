//! 播放预取：进度 ≥70% 或剩余 ≤18s 时预解析并下载下一首到临时文件。

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::api::ApiClient;
use crate::models::{Platform, Song};
use crate::relay::TrackSource;

/// 进度阈值：超过则开始预取。
pub const PREFETCH_PROGRESS_THRESHOLD: f64 = 0.70;
/// 剩余时间阈值（秒）：小于则开始预取（取 15~20 区间中值）。
pub const PREFETCH_REMAIN_SECS: f64 = 18.0;

/// 是否应开始预取下一首。
///
/// 条件（满足其一即可）：
/// - `time_pos / duration >= 0.70`
/// - `duration - time_pos <= 18` 秒
pub fn should_prefetch(time_pos: f64, duration: f64) -> bool {
    if !time_pos.is_finite() || !duration.is_finite() || duration <= 1.0 || time_pos < 0.0 {
        return false;
    }
    let progress = time_pos / duration;
    let remain = duration - time_pos;
    progress >= PREFETCH_PROGRESS_THRESHOLD || remain <= PREFETCH_REMAIN_SECS
}

/// 已预取完成的曲目（本地缓存 + 可交给中继的 TrackSource）。
pub struct PrefetchedTrack {
    pub song: Song,
    pub track: TrackSource,
    pub cache_path: PathBuf,
    /// 仅临时预取文件可删；用户本地曲库文件必须为 false。
    pub ephemeral: bool,
}

impl PrefetchedTrack {
    pub fn matches(&self, song: &Song) -> bool {
        self.song.platform == song.platform && self.song.id == song.id
    }

    /// 删除临时缓存文件（忽略错误）；永不删除用户本地音乐文件。
    pub fn cleanup(&self) {
        if self.ephemeral {
            let _ = std::fs::remove_file(&self.cache_path);
        }
    }
}

impl Drop for PrefetchedTrack {
    fn drop(&mut self) {
        if self.ephemeral {
            let _ = std::fs::remove_file(&self.cache_path);
        }
    }
}

/// 解析播放 URL 并将音频完整下载到系统临时目录。
pub async fn prefetch_song(
    api: Arc<ApiClient>,
    client: &reqwest::Client,
    song: &Song,
) -> Result<PrefetchedTrack> {
    // 本地文件已在磁盘上，直接挂 local_cache，勿复制/勿当临时文件删除
    if song.platform == Platform::Local {
        let track = api
            .resolve_track(song)
            .await
            .with_context(|| format!("预取本地文件失败 {}", song.display_line()))?;
        let path = track
            .local_cache
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from(&song.id));
        return Ok(PrefetchedTrack {
            song: song.clone(),
            track,
            cache_path: path,
            ephemeral: false,
        });
    }

    let mut track = api
        .resolve_track(song)
        .await
        .with_context(|| format!("预取解析链接失败 {}", song.display_line()))?;

    let path = temp_cache_path(song);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    download_to_file(client, &track.url, song.platform, &path)
        .await
        .with_context(|| format!("预取下载失败 {}", song.display_line()))?;

    track.local_cache = Some(path.clone());
    info!(
        song = %song.display_line(),
        path = %path.display(),
        "下一首预取完成"
    );

    Ok(PrefetchedTrack {
        song: song.clone(),
        track,
        cache_path: path,
        ephemeral: true,
    })
}

fn temp_cache_path(song: &Song) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("mixly-prefetch");
    let safe_id: String = song
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let name = format!(
        "{}-{safe_id}-{}.bin",
        song.platform.as_str(),
        std::process::id()
    );
    dir.push(name);
    dir
}

async fn download_to_file(
    client: &reqwest::Client,
    url: &str,
    platform: Platform,
    path: &PathBuf,
) -> Result<()> {
    let mut headers = HeaderMap::new();
    let referer = match platform {
        Platform::Netease => "https://music.163.com/",
        Platform::Qq => "https://y.qq.com/",
        Platform::Local => "",
    };
    if referer.is_empty() {
        // still continue for non-local (shouldn't reach here for Local)
    }
    if !referer.is_empty() {
        headers.insert(
            HeaderName::from_static("referer"),
            HeaderValue::from_static(referer),
        );
    }
    headers.insert(
        HeaderName::from_static("user-agent"),
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        ),
    );

    let resp = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .context("预取 HTTP 请求")?;
    if !resp.status().is_success() {
        return Err(anyhow!("预取 HTTP 状态 {}", resp.status()));
    }

    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("创建缓存文件 {}", path.display()))?;
    let mut stream = resp.bytes_stream();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("预取读 body")?;
        total += chunk.len() as u64;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    if total == 0 {
        let _ = tokio::fs::remove_file(path).await;
        return Err(anyhow!("预取得到空文件"));
    }
    Ok(())
}

/// 后台预取任务句柄。
pub struct PrefetchJob {
    pub platform: Platform,
    pub id: String,
    handle: tokio::task::JoinHandle<Result<PrefetchedTrack>>,
}

impl PrefetchJob {
    pub fn matches_song(&self, song: &Song) -> bool {
        self.platform == song.platform && self.id == song.id
    }

    pub fn spawn(api: Arc<ApiClient>, client: reqwest::Client, song: Song) -> Self {
        let platform = song.platform;
        let id = song.id.clone();
        let handle = tokio::spawn(async move { prefetch_song(api, &client, &song).await });
        Self {
            platform,
            id,
            handle,
        }
    }

    /// 等待结果；失败时返回 Err。
    pub async fn join(self) -> Result<PrefetchedTrack> {
        match self.handle.await {
            Ok(inner) => inner,
            Err(e) => Err(anyhow!("预取任务失败: {e}")),
        }
    }

    pub fn abort(self) {
        self.handle.abort();
    }
}

/// 若任务已完成则取出，否则保留。
pub async fn try_take_finished(
    job: &mut Option<PrefetchJob>,
) -> Option<Result<PrefetchedTrack>> {
    let Some(j) = job.as_ref() else {
        return None;
    };
    if !j.handle.is_finished() {
        return None;
    }
    let j = job.take().unwrap();
    Some(j.join().await)
}

pub fn log_prefetch_start(song: &Song) {
    info!(song = %song.display_line(), "开始预取下一首");
    println!("… 预取下一首: {}", song.display_line());
}

pub fn log_prefetch_fail(err: &anyhow::Error) {
    warn!(error = %err, "预取失败，将回退实时拉取");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefetch_at_70_percent() {
        assert!(should_prefetch(70.0, 100.0));
        assert!(should_prefetch(71.0, 100.0));
        assert!(!should_prefetch(69.0, 100.0));
    }

    #[test]
    fn prefetch_when_remain_le_18() {
        // 200s 总长，182s 进度 → 剩余 18s
        assert!(should_prefetch(182.0, 200.0));
        assert!(should_prefetch(185.0, 200.0));
        // 剩余 19s，进度 181/200=0.905 < 0.70? 0.905 >= 0.70 so true by progress
        assert!(should_prefetch(181.0, 200.0));
        // short song: 50s total, 30s pos → remain 20, progress 0.6 → false
        assert!(!should_prefetch(30.0, 50.0));
        // remain 18 exactly
        assert!(should_prefetch(32.0, 50.0));
    }

    #[test]
    fn prefetch_invalid_duration() {
        assert!(!should_prefetch(10.0, 0.0));
        assert!(!should_prefetch(10.0, f64::NAN));
        assert!(!should_prefetch(f64::NAN, 100.0));
    }
}
