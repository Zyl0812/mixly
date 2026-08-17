//! Bilibili 网页接口适配（私有协议，全部集中在本模块）。
//!
//! 仅支持普通 UGC 视频音频的 DASH 播放；不下载、不转码、不绕过账号权限。
//! 所有请求都使用已登录账号的 Cookie；CDN 取链只返回签名 URL + Referer。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::models::{Platform, Song};

/// Bilibili WBI 签名键位表（官方网页端混排次序）。
const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

const API: &str = "https://api.bilibili.com";
const PASSPORT: &str = "https://passport.bilibili.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const REFERER: &str = "https://www.bilibili.com/";

/// 会话过期兜底：Web SESSDATA 官方不返回失效时间，按登录时刻 + 30 天近似；
/// 真正的校验以导航接口的只读状态为准。
const SESSION_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug)]
struct StaleWbiSignature;

impl std::fmt::Display for StaleWbiSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bilibili WBI 签名失效")
    }
}

impl std::error::Error for StaleWbiSignature {}

// ── 视频引用解析（纯函数） ───────────────────────────────────────────────

/// 解析后的 Bilibili 视频引用。`page` 从 1 起，缺省为第 1 P。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoRef {
    Bvid { bvid: String, page: u32 },
    Av { aid: u64, page: u32 },
}

/// 解析 BV 号 / AV 号 / bilibili 视频 URL（含 `p=N` 分 P）。
///
/// 接受 `BV1xx411c7mD`、`av123456`、`https://www.bilibili.com/video/BV1xx/?p=2`。
/// 非法 host、非法 page、非视频路径返回 `None`（短链需先展开为完整 URL）。
pub fn parse_video_ref(input: &str) -> Option<VideoRef> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if is_valid_bvid(input) {
        return Some(VideoRef::Bvid {
            bvid: input.to_string(),
            page: 1,
        });
    }
    if let Some(aid) = parse_av_id(input) {
        return Some(VideoRef::Av { aid, page: 1 });
    }
    let url = reqwest::Url::parse(input).ok()?;
    let host = url.host_str()?;
    if !matches!(host, "www.bilibili.com" | "bilibili.com" | "m.bilibili.com") {
        return None;
    }
    let mut segs = url.path_segments()?;
    if segs.next()? != "video" {
        return None;
    }
    let id = segs.next()?;
    let page = parse_page_param(&url)?;
    if let Some(rest) = id.strip_prefix("BV").or_else(|| id.strip_prefix("bv")) {
        let bvid = format!("BV{rest}");
        if is_valid_bvid(&bvid) {
            return Some(VideoRef::Bvid { bvid, page });
        }
        return None;
    }
    if let Some(aid) = parse_av_id(id) {
        return Some(VideoRef::Av { aid, page });
    }
    None
}

fn is_valid_bvid(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("BV").or_else(|| s.strip_prefix("bv")) else {
        return false;
    };
    rest.len() >= 10 && rest.chars().all(|c| c.is_ascii_alphanumeric())
}

fn parse_av_id(s: &str) -> Option<u64> {
    let rest = s.strip_prefix("av").or_else(|| s.strip_prefix("AV"))?;
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rest.parse::<u64>().ok()
}

fn parse_page_param(url: &reqwest::Url) -> Option<u32> {
    // 有 p 参数时必须为 >=1 的数字；否则视为缺省第 1 P
    let mut page = 1u32;
    for (k, v) in url.query_pairs() {
        if k == "p" {
            let Ok(n) = v.parse::<u32>() else {
                return None;
            };
            if n < 1 {
                return None;
            }
            page = n;
        }
    }
    Some(page)
}

/// 歌单持久化用的规范 ID：第 1 P 为 `BV...`，分 P 为 `BV...:pN`。
pub fn canonical_id(bvid: &str, page: u32) -> String {
    if page <= 1 {
        bvid.to_string()
    } else {
        format!("{bvid}:p{page}")
    }
}

/// 从规范 ID（`BV...` / `BV...:pN`）还原 bvid 与分 P。
pub fn parse_canonical_id(id: &str) -> Option<(String, u32)> {
    if let Some((bvid, p)) = id.split_once(":p") {
        if is_valid_bvid(bvid) {
            if let Ok(page) = p.parse::<u32>() {
                if page >= 1 {
                    return Some((bvid.to_string(), page));
                }
            }
        }
        return None;
    }
    if is_valid_bvid(id) {
        return Some((id.to_string(), 1));
    }
    None
}

/// 视频页面 Referer（CDN 反盗链需要精确到视频页）。
pub fn video_referer(id: &str) -> String {
    match parse_canonical_id(id) {
        Some((bvid, page)) if page > 1 => format!("https://www.bilibili.com/video/{bvid}?p={page}"),
        Some((bvid, _)) => format!("https://www.bilibili.com/video/{bvid}"),
        None => REFERER.to_string(),
    }
}

// ── 会话 ─────────────────────────────────────────────────────────────────

/// Bilibili 登录会话。只保存请求必需的最小 Cookie 字段，不保存 refresh token。
#[derive(Serialize, Deserialize)]
pub struct BilibiliSession {
    pub sessdata: String,
    pub dedeuserid: String,
    pub bili_jct: String,
    pub buvid3: String,
    pub login_at_ms: u64,
    pub expires_at_ms: u64,
}

impl std::fmt::Debug for BilibiliSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不派生 Debug：防止格式化输出泄露 Cookie。
        f.debug_struct("BilibiliSession")
            .field("dedeuserid", &self.dedeuserid)
            .field("sessdata", &"<redacted>")
            .field("bili_jct", &"<redacted>")
            .field("buvid3", &"<redacted>")
            .field("login_at_ms", &self.login_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

impl BilibiliSession {
    fn cookie_header(&self) -> String {
        format!(
            "SESSDATA={}; DedeUserID={}; bili_jct={}; buvid3={}",
            self.sessdata, self.dedeuserid, self.bili_jct, self.buvid3
        )
    }
}

/// 导航接口返回的会话状态。
pub enum SessionStatus {
    LoggedIn { uname: String },
    NotLoggedIn,
}

// ── 客户端 ───────────────────────────────────────────────────────────────

/// Bilibili 私有接口客户端。遵循启动时注入的 `ALL_PROXY` 代理环境。
pub struct BilibiliClient {
    http: reqwest::Client,
    token_path: PathBuf,
    /// 内存中缓存的 WBI 密钥 (img_key, sub_key)。
    wbi: Mutex<Option<(String, String)>>,
}

impl BilibiliClient {
    pub fn new(token_path: PathBuf) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            token_path,
            wbi: Mutex::new(None),
        }
    }

    // ── 会话读写 ──

    pub fn load_session(&self) -> Result<Option<BilibiliSession>> {
        if !self.token_path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(&self.token_path)
            .with_context(|| format!("read {}", self.token_path.display()))?;
        let s: BilibiliSession = serde_json::from_str(&data).context("parse bilibili session")?;
        if s.sessdata.is_empty() {
            return Ok(None);
        }
        Ok(Some(s))
    }

    fn save_session(&self, s: &BilibiliSession) -> Result<()> {
        if let Some(parent) = self.token_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(s)?;
        std::fs::write(&self.token_path, data)
            .with_context(|| format!("write {}", self.token_path.display()))?;
        info!("bilibili session saved (no secrets logged)");
        Ok(())
    }

    pub fn delete_session(&self) -> Result<()> {
        if self.token_path.exists() {
            std::fs::remove_file(&self.token_path)
                .with_context(|| format!("remove {}", self.token_path.display()))?;
        }
        Ok(())
    }

    pub fn has_session(&self) -> bool {
        matches!(self.load_session(), Ok(Some(_)))
    }

    // ── 请求基础 ──

    async fn get_json(
        &self,
        url: &str,
        params: &[(String, String)],
        logged_in: bool,
    ) -> Result<Value> {
        let mut req = self
            .http
            .get(url)
            .header("referer", REFERER)
            .header("user-agent", UA);
        if logged_in {
            if let Ok(Some(s)) = self.load_session() {
                req = req.header("cookie", s.cookie_header());
            }
        }
        if !params.is_empty() {
            req = req.query(params);
        }
        let resp = req.send().await.context("bilibili 请求失败")?;
        let status = resp.status();
        if !status.is_success() {
            bail!("bilibili 请求状态 {status}");
        }
        resp.json().await.context("bilibili 响应解析失败")
    }

    fn check_body(&self, v: &Value) -> Result<()> {
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();
        if code == 0 {
            return Ok(());
        }
        bail!(map_api_error(code, &msg));
    }

    // ── 登录状态校验 + WBI key ──

    /// 导航接口：校验会话是否有效，并顺便缓存 WBI 密钥。
    /// 网络错误返回 Err（区分「未登录」与「接口故障」）。
    pub async fn check_session(&self) -> Result<SessionStatus> {
        let v = self
            .get_json(&format!("{API}/x/web-interface/nav"), &[], true)
            .await?;
        if v.get("code").and_then(|c| c.as_i64()) == Some(-101) {
            return Ok(SessionStatus::NotLoggedIn);
        }
        self.check_body(&v)?;
        let data = &v["data"];
        let is_login = data
            .get("isLogin")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if !is_login {
            return Ok(SessionStatus::NotLoggedIn);
        }
        let uname = data
            .get("uname")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(keys) = wbi_keys_from_nav(data) {
            *self.wbi.lock().unwrap() = Some(keys);
        }
        Ok(SessionStatus::LoggedIn { uname })
    }

    /// 搜索/取链前调用；未登录时给出明确指引。
    async fn require_login(&self) -> Result<()> {
        match self.check_session().await? {
            SessionStatus::LoggedIn { uname } => {
                debug!(uname, "bilibili 登录态有效");
                Ok(())
            }
            SessionStatus::NotLoggedIn => {
                bail!("Bilibili 未登录或登录已过期，请先执行 mixly login --platform bilibili")
            }
        }
    }

    // ── 扫码登录 ──

    pub async fn login_qr(&self) -> Result<()> {
        let v = self
            .get_json(
                &format!("{PASSPORT}/x/passport-login/web/qrcode/generate"),
                &[],
                false,
            )
            .await?;
        self.check_body(&v)?;
        let data = &v["data"];
        let url = data
            .get("url")
            .and_then(|u| u.as_str())
            .context("二维码生成响应缺少 url")?
            .to_string();
        let key = data
            .get("qrcode_key")
            .and_then(|k| k.as_str())
            .context("二维码生成响应缺少 qrcode_key")?
            .to_string();

        println!();
        println!("======== 请使用【哔哩哔哩】App 扫码登录 ========");
        if let Err(e) = crate::qr_term::print_qr_payload(&url) {
            warn!(error = %e, "终端打印二维码失败");
            println!("（终端绘制失败: {e}）");
            println!("二维码内容: {url}");
        }
        println!("==============================================");
        println!("等待确认中…（Ctrl+C 取消）");
        println!();

        loop {
            let (v, cookies) = self.poll_qr(&key).await?;
            self.check_body(&v)?;
            let state = v["data"].get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            match state {
                0 => {
                    let session = build_session_from_poll(&v["data"], &cookies)?;
                    self.save_session(&session)?;
                    println!("登录成功（B站）。凭证仅保存在本机。");
                    return Ok(());
                }
                86038 => bail!("二维码已过期，请重新执行 mixly login --platform bilibili"),
                86090 => {
                    println!("已扫码，请在手机上确认登录…");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                _ => {
                    // 86101 未扫码等：继续轮询
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    /// 轮询二维码状态，同时带回响应头里的 Set-Cookie。
    async fn poll_qr(&self, key: &str) -> Result<(Value, Vec<(String, String)>)> {
        let req = self
            .http
            .get(format!("{PASSPORT}/x/passport-login/web/qrcode/poll"))
            .header("referer", REFERER)
            .header("user-agent", UA)
            .query(&[("qrcode_key", key)]);
        let resp = req.send().await.context("bilibili 请求失败")?;
        let cookies = extract_set_cookies(resp.headers());
        let v: Value = resp.json().await.context("bilibili 响应解析失败")?;
        Ok((v, cookies))
    }

    // ── WBI 签名 ──

    async fn sign_params(
        &self,
        mut params: Vec<(String, String)>,
    ) -> Result<Vec<(String, String)>> {
        let cached = self.wbi.lock().unwrap().clone();
        let (img, sub) = match cached {
            Some(keys) => keys,
            None => {
                // 无缓存 key：先校验会话（未登录/过期会明确报错），nav 会顺带缓存 key
                self.require_login().await?;
                self.wbi
                    .lock()
                    .unwrap()
                    .clone()
                    .context("未能取得 WBI 密钥，请重试")?
            }
        };
        let wts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(sign_query(&mut params, &img, &sub, wts))
    }

    // ── 搜索 ──

    pub async fn search(&self, keyword: &str, limit: u64) -> Result<Vec<Song>> {
        self.require_login().await?;
        // 搜索接口明确拒绝 WBI 签名时才刷新 key 重试一次；风控、网络和登录错误直接返回。
        match self.search_once(keyword, limit).await {
            Ok(songs) => Ok(songs),
            Err(e) if e.downcast_ref::<StaleWbiSignature>().is_some() => {
                *self.wbi.lock().unwrap() = None;
                self.search_once(keyword, limit).await
            }
            Err(e) => Err(e),
        }
    }

    async fn search_once(&self, keyword: &str, limit: u64) -> Result<Vec<Song>> {
        let limit = limit.clamp(1, 50);
        let params = vec![
            ("search_type".to_string(), "video".to_string()),
            ("keyword".to_string(), keyword.to_string()),
            ("page".to_string(), "1".to_string()),
            ("page_size".to_string(), limit.to_string()),
        ];
        let signed = self.sign_params(params).await?;
        let v = self
            .get_json(
                &format!("{API}/x/web-interface/wbi/search/type"),
                &signed,
                true,
            )
            .await?;
        if v.get("code").and_then(|c| c.as_i64()) == Some(-403) {
            return Err(anyhow!(StaleWbiSignature));
        }
        self.check_body(&v)?;
        let results = v["data"]["result"].as_array().cloned().unwrap_or_default();
        Ok(map_search_results(&results))
    }

    // ── 视频详情 ──

    async fn view(&self, bvid: &str, aid: Option<u64>) -> Result<Value> {
        let params = if let Some(aid) = aid {
            vec![("aid".to_string(), aid.to_string())]
        } else {
            vec![("bvid".to_string(), bvid.to_string())]
        };
        let v = self
            .get_json(&format!("{API}/x/web-interface/view"), &params, true)
            .await?;
        self.check_body(&v)?;
        Ok(v["data"].clone())
    }

    /// 解析输入（BV/AV/URL/分P 或已规范化的歌单 ID）并定位 cid。
    pub async fn resolve_video(&self, input: &str) -> Result<ResolvedVideo> {
        self.require_login().await?;
        let reference = parse_video_ref(input)
            .or_else(|| parse_canonical_id(input).map(|(bvid, page)| VideoRef::Bvid { bvid, page }))
            .ok_or_else(|| {
                anyhow!("无法识别 Bilibili 视频「{input}」，请使用 BV 号、av 号或视频 URL")
            })?;
        let (bvid, data) = match &reference {
            VideoRef::Bvid { bvid, .. } => {
                let data = self.view(bvid, None).await?;
                (bvid.clone(), data)
            }
            VideoRef::Av { aid, .. } => {
                let data = self.view("", Some(*aid)).await?;
                let bvid = data
                    .get("bvid")
                    .and_then(|b| b.as_str())
                    .unwrap_or("")
                    .to_string();
                (bvid, data)
            }
        };
        if bvid.is_empty() {
            bail!("Bilibili 视频详情缺少 bvid");
        }
        let page = match &reference {
            VideoRef::Bvid { page, .. } | VideoRef::Av { page, .. } => *page,
        };
        map_view(&data, &bvid, page)
    }

    // ── DASH 音频取链 ──

    /// 普通 DASH 音频中取账号当前允许的最高带宽；不请求视频轨。
    pub async fn play_url(&self, bvid: &str, cid: u64) -> Result<String> {
        self.require_login().await?;
        let params = vec![
            ("bvid".to_string(), bvid.to_string()),
            ("cid".to_string(), cid.to_string()),
            ("fnval".to_string(), "16".to_string()),
            ("fourk".to_string(), "1".to_string()),
        ];
        let v = self
            .get_json(&format!("{API}/x/player/playurl"), &params, true)
            .await?;
        self.check_body(&v)?;
        let audio = v["data"]["dash"]["audio"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let best = pick_best_audio(&audio)
            .context("该视频没有可用普通音频流（可能是付费、地区或 DRM 限制）")?;
        abs_url(&best).context("音频链接无效")
    }
}

/// 搜索响应 `data.result` → 公共 Song 列表（纯函数，脱敏夹具可测）。
fn map_search_results(results: &[Value]) -> Vec<Song> {
    let mut songs = Vec::new();
    for item in results {
        let Some(bvid) = item.get("bvid").and_then(|b| b.as_str()) else {
            continue;
        };
        if !is_valid_bvid(bvid) {
            continue;
        }
        let title = item
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let author = item
            .get("author")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        let tname = item
            .get("typename")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string());
        let duration_ms = item.get("duration").and_then(value_duration_ms);
        let cover_url = item.get("pic").and_then(|p| p.as_str()).and_then(abs_url);
        let artists: Vec<String> = if author.is_empty() {
            Vec::new()
        } else {
            vec![author]
        };
        songs.push(Song {
            platform: Platform::Bilibili,
            id: canonical_id(bvid, 1),
            name: strip_html(&title),
            artists,
            album: tname,
            duration_ms,
            cover_url,
        });
    }
    songs
}

/// 视频详情 `data` → 按分 P 定位 cid 的 ResolvedVideo（纯函数，脱敏夹具可测）。
fn map_view(data: &Value, bvid: &str, page: u32) -> Result<ResolvedVideo> {
    let pages = data
        .get("pages")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let page_info = pages
        .get(page.saturating_sub(1) as usize)
        .ok_or_else(|| anyhow!("视频「{bvid}」没有第 {page} P"))?;
    let cid = page_info.get("cid").and_then(|c| c.as_u64()).unwrap_or(0);
    if cid == 0 {
        bail!("视频「{bvid}」第 {page} P 缺少 cid");
    }
    let title = data
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let author = data["owner"]
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let tname = data
        .get("tname")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let duration_ms = page_info
        .get("duration")
        .and_then(|d| d.as_u64())
        .map(|secs| secs.saturating_mul(1000));
    let pic = data.get("pic").and_then(|p| p.as_str()).and_then(abs_url);
    Ok(ResolvedVideo {
        bvid: bvid.to_string(),
        page,
        cid,
        title,
        author,
        tname,
        duration_ms,
        pic,
    })
}

/// 解析视频详情的最终结果（含分 P 定位到的 cid）。
pub struct ResolvedVideo {
    pub bvid: String,
    pub page: u32,
    pub cid: u64,
    pub title: String,
    pub author: String,
    pub tname: Option<String>,
    pub duration_ms: Option<u64>,
    pub pic: Option<String>,
}

// ── 登录 cookie 提取 ─────────────────────────────────────────────────────

/// 从轮询成功的响应头提取 Set-Cookie 键值对。
fn extract_set_cookies(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for value in headers.get_all(reqwest::header::SET_COOKIE) {
        if let Ok(text) = value.to_str() {
            if let Some((name, rest)) = text.split_once('=') {
                let name = name.trim().to_string();
                let value = rest.split(';').next().unwrap_or("").trim().to_string();
                out.push((name, value));
            }
        }
    }
    out
}

fn cookie_value(cookies: &[(String, String)], name: &str) -> Option<String> {
    cookies
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

/// 登录成功后组装会话：SESSDATA 必填，其余缺字段尽量补。
fn build_session_from_poll(data: &Value, cookies: &[(String, String)]) -> Result<BilibiliSession> {
    let url = data.get("url").and_then(|u| u.as_str()).unwrap_or("");
    let sessdata = cookie_value(cookies, "SESSDATA")
        .or_else(|| extract_cookie(url, "SESSDATA"))
        .context("登录成功但未取到 SESSDATA，请重试")?;
    let dedeuserid = cookie_value(cookies, "DedeUserID")
        .or_else(|| extract_cookie(url, "DedeUserID"))
        .unwrap_or_default();
    let bili_jct = cookie_value(cookies, "bili_jct")
        .or_else(|| extract_cookie(url, "bili_jct"))
        .unwrap_or_default();
    let buvid3 = cookie_value(cookies, "buvid3")
        .or_else(|| extract_cookie(url, "buvid3"))
        .or_else(synthesize_buvid3)
        .unwrap_or_default();
    let now = now_ms();
    Ok(BilibiliSession {
        sessdata,
        dedeuserid,
        bili_jct,
        buvid3,
        login_at_ms: now,
        expires_at_ms: now.saturating_add(SESSION_TTL_MS),
    })
}

// ── 纯函数 / 工具 ────────────────────────────────────────────────────────

/// 从 nav 响应提取 WBI 密钥对。密钥 = URL 文件名（去掉扩展名）。
fn wbi_keys_from_nav(data: &Value) -> Option<(String, String)> {
    let img_url = data["wbi_img"]["img_url"].as_str()?;
    let sub_url = data["wbi_img"]["sub_url"].as_str()?;
    let img_key = file_stem(img_url)?;
    let sub_key = file_stem(sub_url)?;
    Some((img_key, sub_key))
}

fn file_stem(url: &str) -> Option<String> {
    let path = reqwest::Url::parse(url).ok()?.path().to_string();
    let name = path.rsplit('/').next()?.split('.').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// WBI mixin key：按表取字符拼接后截取前 32 位。
pub fn mixin_key_from_keys(img_key: &str, sub_key: &str) -> String {
    let combined: Vec<char> = format!("{img_key}{sub_key}").chars().collect();
    MIXIN_KEY_ENC_TAB
        .iter()
        .take_while(|&&i| i < combined.len())
        .map(|&i| combined[i])
        .take(32)
        .collect()
}

/// 近似 `encodeURIComponent`：保留 unreserved + `~`、`*`，其余百分号编码（大写 hex）。
pub fn wbi_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'*' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn clean_wbi_value(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '!' | '\'' | '(' | ')' | '*'))
        .collect()
}

/// 生成带 WBI 签名的完整查询参数列表：加入 `wts` → 规范化/排序 → 编码 → MD5。
pub fn sign_query(
    params: &mut Vec<(String, String)>,
    img_key: &str,
    sub_key: &str,
    wts: u64,
) -> Vec<(String, String)> {
    params.push(("wts".to_string(), wts.to_string()));
    for (_, value) in params.iter_mut() {
        *value = clean_wbi_value(value);
    }
    params.sort_by(|a, b| a.0.cmp(&b.0));
    let mixin = mixin_key_from_keys(img_key, sub_key);
    let mut query = String::new();
    for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
            query.push('&');
        }
        query.push_str(&wbi_encode(k));
        query.push('=');
        query.push_str(&wbi_encode(v));
    }
    let digest = format!("{:x}", md5::compute(format!("{query}{mixin}").as_bytes()));
    params.push(("w_rid".to_string(), digest));
    params.clone()
}

/// 去掉 `<em class="keyword">` 等 HTML 高亮标签。
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// 兼容搜索结果的 `"3:45"` 字符串与秒数，统一为毫秒。
pub fn value_duration_ms(v: &Value) -> Option<u64> {
    if let Some(s) = v.as_str() {
        return parse_duration_to_ms(s);
    }
    if let Some(secs) = v.as_u64() {
        return Some(secs.saturating_mul(1000));
    }
    if let Some(secs) = v.as_f64() {
        if secs.is_finite() && secs >= 0.0 {
            return Some((secs * 1000.0) as u64);
        }
    }
    None
}

fn parse_duration_to_ms(input: &str) -> Option<u64> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if input.contains(':') {
        let parts: Vec<&str> = input.split(':').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }
        let mut total = 0f64;
        for p in parts {
            total = total * 60.0 + p.trim().parse::<f64>().ok()?;
        }
        if total.is_finite() && total >= 0.0 {
            Some((total * 1000.0) as u64)
        } else {
            None
        }
    } else {
        let secs: f64 = input.parse().ok()?;
        if secs.is_finite() && secs >= 0.0 {
            Some((secs * 1000.0) as u64)
        } else {
            None
        }
    }
}

/// 协议相对 URL（`//i0.hdslb.com/...`）统一为 HTTPS；非 http(s) 返回 None。
pub fn abs_url(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    if s.starts_with("https://") || s.starts_with("http://") {
        return Some(s.to_string());
    }
    None
}

/// 选择普通 DASH 音频中带宽最高的候选。特殊音轨（dolby/flac）在真实响应中
/// 位于独立字段，不会出现在普通 `dash.audio` 数组里。
pub fn pick_best_audio(audio: &[Value]) -> Option<String> {
    audio
        .iter()
        .filter(|a| {
            a.get("baseUrl")
                .and_then(|u| u.as_str())
                .map(|u| !u.is_empty())
                .unwrap_or(false)
        })
        .max_by(|a, b| {
            let bw = |x: &Value| x.get("bandwidth").and_then(|v| v.as_u64()).unwrap_or(0);
            bw(a).cmp(&bw(b))
        })
        .and_then(|a| {
            a.get("baseUrl")
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
        })
}

/// 平台错误码 → 可理解提示。
fn map_api_error(code: i64, msg: &str) -> String {
    match code {
        -101 => "Bilibili 登录已失效，请重新扫码登录".to_string(),
        -404 => "Bilibili 视频不存在或已删除".to_string(),
        -400 => format!("Bilibili 请求参数错误（{code}: {msg}）"),
        -403 => "Bilibili 无权访问该内容".to_string(),
        -412 => "Bilibili 风控拦截，请稍后再试或更换网络".to_string(),
        -352 => "Bilibili 需要安全验证，请到网页端处理".to_string(),
        _ => format!("Bilibili 接口错误（{code}: {msg}）"),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 从 URL 参数提取 cookie 键值，保留 Cookie 中原有的百分号编码。
fn extract_cookie(url: &str, name: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    parsed
        .query()?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

fn synthesize_buvid3() -> Option<String> {
    let u = uuid::Uuid::new_v4().to_string().to_uppercase();
    Some(format!("{u}infoc"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ref_parse_forms() {
        assert_eq!(
            parse_video_ref("BV1xx411c7mD"),
            Some(VideoRef::Bvid {
                bvid: "BV1xx411c7mD".into(),
                page: 1
            })
        );
        assert_eq!(
            parse_video_ref("av123456"),
            Some(VideoRef::Av {
                aid: 123456,
                page: 1
            })
        );
        assert_eq!(
            parse_video_ref("AV123456"),
            Some(VideoRef::Av {
                aid: 123456,
                page: 1
            })
        );
        assert_eq!(
            parse_video_ref("https://www.bilibili.com/video/BV1xx411c7mD"),
            Some(VideoRef::Bvid {
                bvid: "BV1xx411c7mD".into(),
                page: 1
            })
        );
        assert_eq!(
            parse_video_ref("https://bilibili.com/video/BV1xx411c7mD?p=2&spm_id_from=333"),
            Some(VideoRef::Bvid {
                bvid: "BV1xx411c7mD".into(),
                page: 2
            })
        );
        assert_eq!(
            parse_video_ref("https://m.bilibili.com/video/av7?p=3"),
            Some(VideoRef::Av { aid: 7, page: 3 })
        );
    }

    #[test]
    fn video_ref_rejects_invalid() {
        assert!(parse_video_ref("").is_none());
        assert!(parse_video_ref("https://evil.com/video/BV1xx411c7mD").is_none());
        assert!(parse_video_ref("https://www.bilibili.com/watch/BV1xx").is_none());
        assert!(parse_video_ref("BV1xx").is_none()); // BV 太短
        assert!(parse_video_ref("https://www.bilibili.com/video/BV1xx411c7mD?p=0").is_none());
        assert!(parse_video_ref("https://www.bilibili.com/video/BV1xx411c7mD?p=abc").is_none());
    }

    #[test]
    fn canonical_id_roundtrip() {
        assert_eq!(canonical_id("BV1xx411c7mD", 1), "BV1xx411c7mD");
        assert_eq!(canonical_id("BV1xx411c7mD", 2), "BV1xx411c7mD:p2");
        assert_eq!(
            parse_canonical_id("BV1xx411c7mD:p2"),
            Some(("BV1xx411c7mD".to_string(), 2))
        );
        assert_eq!(
            parse_canonical_id("BV1xx411c7mD"),
            Some(("BV1xx411c7mD".to_string(), 1))
        );
        assert!(parse_canonical_id("xx:p2").is_none());
        assert!(parse_canonical_id("BV1xx411c7mD:p0").is_none());
    }

    #[test]
    fn video_referer_matches_page() {
        assert_eq!(
            video_referer("BV1xx411c7mD"),
            "https://www.bilibili.com/video/BV1xx411c7mD"
        );
        assert_eq!(
            video_referer("BV1xx411c7mD:p2"),
            "https://www.bilibili.com/video/BV1xx411c7mD?p=2"
        );
    }

    #[test]
    fn strip_html_removes_highlight() {
        assert_eq!(
            strip_html("周杰伦<em class=\"keyword\">晴天</em>现场"),
            "周杰伦晴天现场"
        );
        assert_eq!(strip_html(" 普通 标题  "), "普通 标题");
    }

    #[test]
    fn duration_parsing_ms() {
        assert_eq!(value_duration_ms(&Value::from("3:45")), Some(225_000));
        assert_eq!(value_duration_ms(&Value::from("1:02:03")), Some(3_723_000));
        assert_eq!(value_duration_ms(&Value::from(225u64)), Some(225_000));
        assert_eq!(value_duration_ms(&Value::from(225.5)), Some(225_500));
        assert_eq!(value_duration_ms(&Value::from("abc")), None);
    }

    #[test]
    fn abs_url_normalizes_protocol_relative() {
        assert_eq!(
            abs_url("//i0.hdslb.com/a.png"),
            Some("https://i0.hdslb.com/a.png".into())
        );
        assert_eq!(
            abs_url("https://i0.hdslb.com/a.png"),
            Some("https://i0.hdslb.com/a.png".into())
        );
        assert_eq!(abs_url(""), None);
        assert_eq!(abs_url("ftp://x"), None);
    }

    #[test]
    fn pick_best_audio_prefers_highest_bandwidth() {
        let audio: Vec<Value> = vec![
            serde_json::json!({ "id": 30216, "bandwidth": 314713, "baseUrl": "https://a/mid" }),
            serde_json::json!({ "id": 30232, "bandwidth": 614713, "baseUrl": "https://a/high" }),
            // 带宽更高但 baseUrl 为空 → 跳过
            serde_json::json!({ "id": 30280, "bandwidth": 1204713, "baseUrl": "" }),
        ];
        assert_eq!(pick_best_audio(&audio), Some("https://a/high".to_string()));
        assert_eq!(pick_best_audio(&[]), None);
    }

    #[test]
    fn wbi_encode_matches_encode_uri_component() {
        assert_eq!(
            wbi_encode("晴天 周杰伦"),
            "%E6%99%B4%E5%A4%A9%20%E5%91%A8%E6%9D%B0%E4%BC%A6"
        );
        assert_eq!(wbi_encode("a~b*c"), "a~b*c");
        assert_eq!(wbi_encode("a&b=1"), "a%26b%3D1");
    }

    #[test]
    fn qr_session_falls_back_to_url_cookies_without_decoding() {
        let data = serde_json::json!({
            "url": "https://passport.bilibili.com/crossDomain?SESSDATA=abc%2Bdef&DedeUserID=123&bili_jct=csrf%2Btoken"
        });
        let session = build_session_from_poll(&data, &[]).unwrap();
        assert_eq!(session.sessdata, "abc%2Bdef");
        assert_eq!(session.dedeuserid, "123");
        assert_eq!(session.bili_jct, "csrf%2Btoken");
    }

    #[test]
    fn wbi_sign_vector_fixed_ts_and_key() {
        // 固定 key 与时间戳：验证排序、编码、mixin 截断与 w_rid 算法。
        // 真实 key 各 32 字符（nav 图片文件名），拼起来 64 字符。
        let img = "0123456789abcdef0123456789abcdef".to_string();
        let sub = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef".to_string();
        let wts = 1786000000u64;
        let mut params = vec![
            ("page_size".to_string(), "5".to_string()),
            ("keyword".to_string(), "晴天 周杰伦".to_string()),
        ];
        let signed = sign_query(&mut params, &img, &sub, wts);

        // wts 必须参与排序与摘要；w_rid 在摘要完成后附加。
        let keys: Vec<&str> = signed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["keyword", "page_size", "wts", "w_rid"]);
        assert_eq!(signed[0].1, "晴天 周杰伦");
        assert_eq!(signed[1].1, "5");
        assert_eq!(signed[2].1, wts.to_string());

        // 固定向量避免测试复刻实现后仍同时出错。
        let w_rid = &signed[3].1;
        assert_eq!(w_rid.len(), 32);
        assert!(w_rid.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(w_rid, "f0ff6f186fa09f01ec0301d6d0eb6082");
    }

    #[test]
    fn wbi_sign_strips_forbidden_characters_from_sent_values() {
        let mut params = vec![("keyword".to_string(), "a!'()*b".to_string())];
        let signed = sign_query(&mut params, "img", "sub", 123);
        assert_eq!(signed[0], ("keyword".to_string(), "ab".to_string()));
    }

    #[test]
    fn wbi_sign_deterministic() {
        let mut a = vec![
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "2".to_string()),
        ];
        let mut b = vec![
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "2".to_string()),
        ];
        assert_eq!(
            sign_query(&mut a, "img", "sub", 123),
            sign_query(&mut b, "img", "sub", 123)
        );
    }

    #[test]
    fn session_debug_redacts_secrets() {
        let s = BilibiliSession {
            sessdata: "secret-sessdata".into(),
            dedeuserid: "12345".into(),
            bili_jct: "secret-jct".into(),
            buvid3: "secret-buvid".into(),
            login_at_ms: 0,
            expires_at_ms: 1,
        };
        let text = format!("{s:?}");
        assert!(!text.contains("secret"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn cookie_extraction_from_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static("SESSDATA=abc%2Bdef; Path=/; HttpOnly"),
        );
        headers.append(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static("DedeUserID=12345; Path=/"),
        );
        let cookies = extract_set_cookies(&headers);
        assert_eq!(
            cookie_value(&cookies, "SESSDATA").as_deref(),
            Some("abc%2Bdef")
        );
        assert_eq!(
            cookie_value(&cookies, "DedeUserID").as_deref(),
            Some("12345")
        );
        assert_eq!(cookie_value(&cookies, "missing"), None);
    }

    #[test]
    fn map_error_never_prints_cookies() {
        // 错误信息里不应包含任何会话字段名对应值
        let msg = map_api_error(-412, "risk");
        assert!(!msg.contains("SESSDATA"));
        assert!(!msg.contains("bili_jct"));
        assert!(!msg.contains("sessdata"));
    }

    // ── 脱敏夹具契约测试（tests/fixtures/bilibili） ──────────────────────

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/tests/fixtures/bilibili/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn fixture_search_mapping() {
        let v = fixture("search_success.json");
        let songs = map_search_results(v["data"]["result"].as_array().unwrap());
        assert_eq!(songs.len(), 2);
        let s0 = &songs[0];
        assert_eq!(s0.platform, Platform::Bilibili);
        assert_eq!(s0.id, "BV1xx411c7mD");
        // HTML 高亮被去掉；"3:45" → 225000ms
        assert_eq!(s0.name, "周杰伦晴天 现场版");
        assert_eq!(s0.artists, vec!["示例UP主"]);
        assert_eq!(s0.album.as_deref(), Some("音乐"));
        assert_eq!(s0.duration_ms, Some(225_000));
        assert_eq!(
            s0.cover_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/archive/ffffffff00000000.png")
        );
        // 纯数字秒数、空作者
        assert_eq!(songs[1].duration_ms, Some(600_000));
        assert!(songs[1].artists.is_empty());
    }

    #[test]
    fn fixture_search_empty() {
        let v = fixture("search_empty.json");
        let songs = map_search_results(v["data"]["result"].as_array().unwrap());
        assert!(songs.is_empty());
    }

    #[test]
    fn fixture_view_multi_page() {
        let v = fixture("view_multi_page.json");
        let data = &v["data"];
        let page1 = map_view(data, "BV1xx411c7mD", 1).unwrap();
        assert_eq!(page1.cid, 700001);
        assert_eq!(page1.title, "示例视频标题");
        assert_eq!(page1.author, "示例UP主");
        assert_eq!(page1.tname.as_deref(), Some("音乐"));
        assert_eq!(page1.duration_ms, Some(100_000));
        assert_eq!(
            page1.pic.as_deref(),
            Some("https://i0.hdslb.com/bfs/archive/aaaaaaaa.png")
        );
        let page3 = map_view(data, "BV1xx411c7mD", 3).unwrap();
        assert_eq!(page3.cid, 700003);
        assert_eq!(page3.duration_ms, Some(150_000));
        // 越界分 P
        assert!(map_view(data, "BV1xx411c7mD", 4).is_err());
        // 缺失 pages 的视频
        let mut no_pages = v.clone();
        no_pages["data"].as_object_mut().unwrap().remove("pages");
        assert!(map_view(&no_pages["data"], "BV1xx411c7mD", 1).is_err());
    }

    #[test]
    fn fixture_dash_audio_selection() {
        let v = fixture("playurl_dash.json");
        let audio = v["data"]["dash"]["audio"].as_array().unwrap();
        assert_eq!(
            pick_best_audio(audio).as_deref(),
            Some("https://upcdn.example.invalid/audio_high.m4s")
        );
        let no = fixture("playurl_no_audio.json");
        let audio2 = no["data"]["dash"]["audio"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(pick_best_audio(&audio2), None);
    }

    #[test]
    fn fixture_error_code_mapping() {
        let risk = fixture("risk_limited.json");
        let msg = map_api_error(
            risk["code"].as_i64().unwrap(),
            risk["message"].as_str().unwrap_or(""),
        );
        assert!(msg.contains("风控"), "{msg}");
        let expired = fixture("login_expired.json");
        let msg2 = map_api_error(expired["code"].as_i64().unwrap(), "");
        assert!(msg2.contains("登录已失效"), "{msg2}");
    }
}
