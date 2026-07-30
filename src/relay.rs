//! Local HTTP audio relay: mpv → 127.0.0.1 → reqwest(+proxy) → CDN
//! （或本地预取缓存文件）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RANGE};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::models::Platform;

#[derive(Clone)]
pub struct TrackSource {
    pub url: String,
    pub platform: Platform,
    /// Optional async refresh: returns a new CDN URL when the old one expires.
    pub refresh: Option<RefreshHook>,
    /// 若已预取到本地文件，中继优先从此文件提供（支持 Range）。
    pub local_cache: Option<PathBuf>,
}

pub type RefreshHook = Arc<dyn Fn() -> RefreshFuture + Send + Sync>;
pub type RefreshFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>;

#[derive(Default)]
struct RelayState {
    current: Option<TrackSource>,
}

#[derive(Clone)]
pub struct AudioRelay {
    state: Arc<Mutex<RelayState>>,
    port: u16,
}

impl AudioRelay {
    /// Bind `127.0.0.1:0` and spawn the accept loop.
    pub async fn start(client: reqwest::Client) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind audio relay")?;
        let port = listener.local_addr()?.port();
        let state = Arc::new(Mutex::new(RelayState::default()));
        let relay = Self {
            state: state.clone(),
            port,
        };
        let serve_client = client;
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        debug!(%addr, "relay connection");
                        let st = state.clone();
                        let cl = serve_client.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_conn(stream, st, cl).await {
                                debug!(error = %e, "relay connection ended");
                            }
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "relay accept failed");
                        break;
                    }
                }
            }
        });
        info!(port, "audio relay listening on 127.0.0.1");
        Ok(relay)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn current_url(&self) -> String {
        format!("http://127.0.0.1:{}/current", self.port)
    }

    pub async fn set_track(&self, track: TrackSource) {
        let mut g = self.state.lock().await;
        g.current = Some(track);
    }
}

/// CDN 反盗链要求的 Referer + 浏览器 UA（中继与预取共用）。
pub fn cdn_headers(platform: Platform) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let referer = match platform {
        Platform::Netease => "https://music.163.com/",
        Platform::Qq => "https://y.qq.com/",
        Platform::Local => "",
    };
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
    headers
}

async fn handle_conn(
    mut stream: TcpStream,
    state: Arc<Mutex<RelayState>>,
    client: reqwest::Client,
) -> Result<()> {
    let mut buf = vec![0u8; 8192];
    let mut got = 0usize;
    loop {
        let n = stream.read(&mut buf[got..]).await?;
        if n == 0 {
            return Ok(());
        }
        got += n;
        if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if got == buf.len() {
            buf.resize(buf.len() * 2, 0);
        }
    }
    let header_bytes = &buf[..got];
    let header_str = String::from_utf8_lossy(header_bytes);
    let mut lines = header_str.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if method != "GET" && method != "HEAD" {
        write_simple(&mut stream, 405, "Method Not Allowed", b"").await?;
        return Ok(());
    }
    if path != "/current" && !path.starts_with("/current?") {
        write_simple(&mut stream, 404, "Not Found", b"no track").await?;
        return Ok(());
    }

    let mut req_headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            req_headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let range = req_headers.get("range").cloned();

    let track = {
        let g = state.lock().await;
        g.current.clone()
    };
    let Some(mut track) = track else {
        write_simple(&mut stream, 404, "Not Found", b"no current track").await?;
        return Ok(());
    };

    match proxy_stream(&mut stream, &client, &track, range.as_deref(), method == "HEAD").await {
        Ok(()) => Ok(()),
        Err(e) => {
            // One refresh retry on failure (expired link / 403).
            warn!(error = %e, "relay upstream failed; trying refresh once");
            if let Some(ref hook) = track.refresh {
                match hook().await {
                    Ok(new_url) if !new_url.is_empty() => {
                        track.url = new_url.clone();
                        {
                            let mut g = state.lock().await;
                            if let Some(cur) = g.current.as_mut() {
                                cur.url = new_url;
                            }
                        }
                        proxy_stream(
                            &mut stream,
                            &client,
                            &track,
                            range.as_deref(),
                            method == "HEAD",
                        )
                        .await
                    }
                    Ok(_) => Err(anyhow!("refresh returned empty url")),
                    Err(re) => Err(re).context("refresh play url"),
                }
            } else {
                Err(e)
            }
        }
    }
}

async fn proxy_stream(
    stream: &mut TcpStream,
    client: &reqwest::Client,
    track: &TrackSource,
    range: Option<&str>,
    head_only: bool,
) -> Result<()> {
    if let Some(path) = track.local_cache.as_ref() {
        return serve_local_file(stream, path, range, head_only).await;
    }

    let mut headers = cdn_headers(track.platform);
    if let Some(r) = range {
        headers.insert(RANGE, HeaderValue::from_str(r).unwrap_or(HeaderValue::from_static("bytes=0-")));
    }

    let resp = client
        .get(&track.url)
        .headers(headers)
        .send()
        .await
        .context("upstream request")?;

    let status = resp.status();
    if status.as_u16() == 403 || status.as_u16() == 401 {
        return Err(anyhow!("upstream status {status}"));
    }
    if !(status.is_success() || status.as_u16() == 206) {
        return Err(anyhow!("upstream status {status}"));
    }

    let status_code = status.as_u16();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let content_length = resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let content_range = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let accept_ranges = resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("bytes")
        .to_string();

    // If upstream ignored Range and we asked for one, synthesize 206 when possible is hard
    // without full body; prefer forwarding upstream semantics.
    let reason = if status_code == 206 {
        "Partial Content"
    } else {
        "OK"
    };

    let mut out = format!("HTTP/1.1 {status_code} {reason}\r\n");
    out.push_str(&format!("Content-Type: {content_type}\r\n"));
    out.push_str(&format!("Accept-Ranges: {accept_ranges}\r\n"));
    if let Some(cl) = &content_length {
        out.push_str(&format!("Content-Length: {cl}\r\n"));
    }
    if let Some(cr) = &content_range {
        out.push_str(&format!("Content-Range: {cr}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    stream.write_all(out.as_bytes()).await?;

    if head_only {
        stream.flush().await?;
        return Ok(());
    }

    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.context("read upstream body")?;
        stream.write_all(&chunk).await?;
    }
    stream.flush().await?;
    Ok(())
}

/// 从本地预取文件提供音频，正确处理 `Range`（与 CDN 中继语义一致）。
async fn serve_local_file(
    stream: &mut TcpStream,
    path: &PathBuf,
    range: Option<&str>,
    head_only: bool,
) -> Result<()> {
    use tokio::io::AsyncSeekExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("open prefetch cache {}", path.display()))?;
    let total = file.metadata().await?.len();
    let (status, content_range, start, length) = range_response_meta(total, range);
    if status == 416 {
        let body = b"range not satisfiable";
        let mut out = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n",
            body.len()
        );
        if let Some(cr) = &content_range {
            out.push_str(&format!("Content-Range: {cr}\r\n"));
        }
        out.push_str("Connection: close\r\n\r\n");
        stream.write_all(out.as_bytes()).await?;
        stream.write_all(body).await?;
        stream.flush().await?;
        return Ok(());
    }

    let reason = if status == 206 {
        "Partial Content"
    } else {
        "OK"
    };
    let mut out = format!("HTTP/1.1 {status} {reason}\r\n");
    out.push_str("Content-Type: application/octet-stream\r\n");
    out.push_str("Accept-Ranges: bytes\r\n");
    out.push_str(&format!("Content-Length: {length}\r\n"));
    if let Some(cr) = &content_range {
        out.push_str(&format!("Content-Range: {cr}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    stream.write_all(out.as_bytes()).await?;
    if head_only {
        stream.flush().await?;
        return Ok(());
    }
    // 按 Range 流式发送，避免把整个音频文件读进内存
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut limited = file.take(length);
    tokio::io::copy(&mut limited, stream).await?;
    stream.flush().await?;
    Ok(())
}

async fn write_simple(
    stream: &mut TcpStream,
    code: u16,
    reason: &str,
    body: &[u8],
) -> Result<()> {
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

/// Pure helper used by tests: build response metadata for a Range request against a known buffer.
/// Mirrors the status / Content-Range / Accept-Ranges semantics the relay must produce.
pub fn range_response_meta(
    total_len: u64,
    range_header: Option<&str>,
) -> (u16, Option<String>, u64, u64) {
    // returns (status, content_range, start, length)
    let Some(range) = range_header else {
        return (200, None, 0, total_len);
    };
    let range = range.trim();
    let rest = range
        .strip_prefix("bytes=")
        .unwrap_or(range);
    let (start_s, end_s) = rest.split_once('-').unwrap_or((rest, ""));
    let start: u64 = start_s.parse().unwrap_or(0);
    let end: u64 = if end_s.is_empty() {
        total_len.saturating_sub(1)
    } else {
        end_s
            .parse::<u64>()
            .unwrap_or(total_len.saturating_sub(1))
            .min(total_len.saturating_sub(1))
    };
    if start >= total_len {
        return (
            416,
            Some(format!("bytes */{total_len}")),
            0,
            0,
        );
    }
    let length = end - start + 1;
    (
        206,
        Some(format!("bytes {start}-{end}/{total_len}")),
        start,
        length,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn range_meta_full() {
        let (st, cr, start, len) = range_response_meta(1000, None);
        assert_eq!(st, 200);
        assert!(cr.is_none());
        assert_eq!((start, len), (0, 1000));
    }

    #[test]
    fn range_meta_from_n() {
        let (st, cr, start, len) = range_response_meta(1000, Some("bytes=100-"));
        assert_eq!(st, 206);
        assert_eq!(cr.as_deref(), Some("bytes 100-999/1000"));
        assert_eq!(start, 100);
        assert_eq!(len, 900);
    }

    #[test]
    fn range_meta_closed() {
        let (st, cr, start, len) = range_response_meta(500, Some("bytes=10-19"));
        assert_eq!(st, 206);
        assert_eq!(cr.as_deref(), Some("bytes 10-19/500"));
        assert_eq!((start, len), (10, 10));
    }

    /// Integration: real relay request path against a local mock CDN.
    #[tokio::test]
    async fn relay_range_returns_206() {
        // mock CDN
        let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock_port = mock.local_addr().unwrap().port();
        let body = vec![b'A'; 1000];
        tokio::spawn(async move {
            let (mut sock, _) = mock.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("Range: bytes=100-") || req.contains("range: bytes=100-"));
            let payload = &body[100..];
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nContent-Range: bytes 100-999/1000\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            sock.write_all(header.as_bytes()).await.unwrap();
            sock.write_all(payload).await.unwrap();
        });

        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let relay = AudioRelay::start(client).await.unwrap();
        relay
            .set_track(TrackSource {
                url: format!("http://127.0.0.1:{mock_port}/audio"),
                platform: Platform::Netease,
                refresh: None,
                local_cache: None,
            })
            .await;

        let mut sock = TcpStream::connect(format!("127.0.0.1:{}", relay.port()))
            .await
            .unwrap();
        let req = format!(
            "GET /current HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=100-\r\nConnection: close\r\n\r\n"
        );
        sock.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(
            text.starts_with("HTTP/1.1 206"),
            "expected 206, got: {}",
            text.lines().next().unwrap_or("")
        );
        assert!(
            text.to_ascii_lowercase().contains("content-range: bytes 100-999/1000"),
            "missing content-range: {text}"
        );
        assert!(
            text.to_ascii_lowercase().contains("accept-ranges: bytes"),
            "missing accept-ranges"
        );
        // body should be 900 A's
        if let Some(pos) = resp.windows(4).position(|w| w == b"\r\n\r\n") {
            let body = &resp[pos + 4..];
            assert_eq!(body.len(), 900);
            assert!(body.iter().all(|&b| b == b'A'));
        } else {
            panic!("no header/body split");
        }
    }
}
