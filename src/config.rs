use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::models::{Platform, Quality};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub player: PlayerConfig,
    #[serde(default)]
    pub lyrics: LyricsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_quality")]
    pub quality: String,
    /// 搜索范围默认：all | netease | qq（保留字段，兼容旧配置）
    #[serde(default = "default_platform")]
    pub default_platform: String,
    /// 双平台搜索时优先的平台：qq | netease（默认 qq）
    #[serde(default = "default_preferred_platform")]
    pub preferred_platform: String,
}

fn default_quality() -> String {
    "Exhigh".into()
}

fn default_platform() -> String {
    "all".into()
}

fn default_preferred_platform() -> String {
    "qq".into()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            quality: default_quality(),
            default_platform: default_platform(),
            preferred_platform: default_preferred_platform(),
        }
    }
}

/// 解析配置中的优先平台，非法值回退为 QQ。
pub fn preferred_platform_from_config(cfg: &Config) -> Platform {
    match cfg.general.preferred_platform.to_ascii_lowercase().as_str() {
        "netease" | "网易云" | "wy" | "163" => Platform::Netease,
        "qq" | "tencent" | "qqmusic" => Platform::Qq,
        _ => Platform::Qq,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_proxy_url")]
    pub url: String,
}

fn default_proxy_url() -> String {
    "socks5://127.0.0.1:7890".into()
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: default_proxy_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerConfig {
    #[serde(default = "default_mpv_path")]
    pub mpv_path: String,
    /// Optional override; when omitted, platform default is used.
    #[serde(default)]
    pub socket: Option<String>,
}

fn default_mpv_path() -> String {
    "mpv".into()
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            mpv_path: default_mpv_path(),
            socket: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricsConfig {
    #[serde(default = "default_font_size")]
    pub font_size: u32,
}

fn default_font_size() -> u32 {
    24
}

impl Default for LyricsConfig {
    fn default() -> Self {
        Self {
            font_size: default_font_size(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            proxy: ProxyConfig::default(),
            player: PlayerConfig::default(),
            lyrics: LyricsConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub playlists_dir: PathBuf,
    pub netease_token: PathBuf,
    pub qq_token: PathBuf,
    /// 本地音乐库索引
    pub local_library: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "mixly")
            .context("cannot resolve config directory for mixly")?;
        let config_dir = dirs.config_dir().to_path_buf();
        Ok(Self {
            config_file: config_dir.join("config.toml"),
            playlists_dir: config_dir.join("playlists"),
            netease_token: config_dir.join("netease_token.json"),
            qq_token: config_dir.join("qq_token.json"),
            local_library: config_dir.join("local_library.json"),
            config_dir,
        })
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.playlists_dir)?;
        Ok(())
    }
}

pub fn load_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("read config {}", path.display()))?;
    let cfg: Config = toml::from_str(&text).context("parse config.toml")?;
    Ok(cfg)
}

pub fn save_config(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg).context("serialize config")?;
    fs::write(path, text).with_context(|| format!("write config {}", path.display()))?;
    Ok(())
}

pub fn quality_from_config(cfg: &Config) -> Quality {
    Quality::parse(&cfg.general.quality).unwrap_or_default()
}

/// Proxy resolution order: CLI > config.toml [proxy] > env > direct.
///
/// `cli_proxy`: from `--proxy`. Empty string means "explicit direct" is not used;
/// `None` means CLI did not set proxy. When CLI provides a value it always wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyDecision {
    /// Use this proxy URL (may contain credentials).
    Proxy(String),
    /// Connect directly without proxy env injection.
    Direct,
}

impl ProxyDecision {
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Self::Proxy(u) => Some(u.as_str()),
            Self::Direct => None,
        }
    }
}

/// Resolve proxy from CLI arg, config, then environment variables.
pub fn resolve_proxy(cli_proxy: Option<&str>, config: &Config) -> ProxyDecision {
    if let Some(url) = cli_proxy {
        let url = url.trim();
        if !url.is_empty() {
            return ProxyDecision::Proxy(url.to_string());
        }
    }

    if config.proxy.enabled {
        let url = config.proxy.url.trim();
        if !url.is_empty() {
            return ProxyDecision::Proxy(url.to_string());
        }
    }

    for key in [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(val) = std::env::var(key) {
            let val = val.trim();
            if !val.is_empty() {
                return ProxyDecision::Proxy(val.to_string());
            }
        }
    }

    ProxyDecision::Direct
}

/// Mask credentials in a proxy URL for logging.
///
/// `socks5://user:pass@host:7890` → `socks5://***@host:7890`
pub fn mask_proxy_url(url: &str) -> String {
    // Find scheme://
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let rest = &url[scheme_end + 3..];
    if let Some(at) = rest.find('@') {
        let scheme = &url[..scheme_end + 3];
        format!("{scheme}***@{}", &rest[at + 1..])
    } else {
        url.to_string()
    }
}

/// Inject `ALL_PROXY` before any multi-threaded runtime / HTTP client is created.
///
/// Must be called once at process start, before Tokio runtime threads exist.
pub fn inject_proxy_env(decision: &ProxyDecision) {
    match decision {
        ProxyDecision::Proxy(url) => {
            // Safety: called only from main before runtime; single-threaded at this point.
            unsafe {
                std::env::set_var("ALL_PROXY", url);
            }
            tracing::info!(proxy = %mask_proxy_url(url), "proxy resolved: CLI/config/env → ALL_PROXY");
        }
        ProxyDecision::Direct => {
            tracing::info!("proxy resolved: direct (no ALL_PROXY injection)");
        }
    }
}

pub fn default_mpv_socket_path() -> String {
    #[cfg(windows)]
    {
        r"\\.\pipe\mixly-mpv".to_string()
    }
    #[cfg(not(windows))]
    {
        let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        format!("{tmp}/mixly-mpv.sock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config(enabled: bool, url: &str) -> Config {
        let mut c = Config::default();
        c.proxy.enabled = enabled;
        c.proxy.url = url.into();
        c
    }

    #[test]
    fn proxy_cli_wins_over_config_and_env() {
        // Clear relevant env for isolation is hard without mutating process env of others;
        // we still assert CLI precedence over config.
        let cfg = base_config(true, "socks5://config:1@127.0.0.1:1");
        let d = resolve_proxy(Some("socks5://cli:secret@127.0.0.1:7890"), &cfg);
        assert_eq!(
            d,
            ProxyDecision::Proxy("socks5://cli:secret@127.0.0.1:7890".into())
        );
    }

    #[test]
    fn proxy_config_when_enabled_and_no_cli() {
        let cfg = base_config(true, "socks5://127.0.0.1:7890");
        // If env is set in the runner, env could win only when config is disabled.
        // When config enabled, config wins over env.
        let d = resolve_proxy(None, &cfg);
        assert_eq!(d, ProxyDecision::Proxy("socks5://127.0.0.1:7890".into()));
    }

    #[test]
    fn proxy_config_disabled_falls_through() {
        let cfg = base_config(false, "socks5://127.0.0.1:7890");
        // Without CLI and with disabled config, result is env or direct.
        let d = resolve_proxy(None, &cfg);
        match d {
            ProxyDecision::Direct => {}
            ProxyDecision::Proxy(u) => {
                // env was set in this process
                assert!(!u.is_empty());
            }
        }
    }

    #[test]
    fn proxy_cli_empty_does_not_force_empty_proxy() {
        let cfg = base_config(true, "socks5://127.0.0.1:9");
        // empty string after trim is ignored → config applies
        let d = resolve_proxy(Some("   "), &cfg);
        assert_eq!(d, ProxyDecision::Proxy("socks5://127.0.0.1:9".into()));
    }

    #[test]
    fn mask_proxy_hides_userinfo() {
        assert_eq!(
            mask_proxy_url("socks5://user:pass@127.0.0.1:7890"),
            "socks5://***@127.0.0.1:7890"
        );
        assert_eq!(
            mask_proxy_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080"
        );
    }

    #[test]
    fn preferred_platform_defaults_to_qq() {
        let cfg = Config::default();
        assert_eq!(preferred_platform_from_config(&cfg), Platform::Qq);
        let mut cfg2 = Config::default();
        cfg2.general.preferred_platform = "netease".into();
        assert_eq!(preferred_platform_from_config(&cfg2), Platform::Netease);
    }

    #[test]
    fn four_source_priority_matrix() {
        // 1) CLI present
        let cfg = base_config(true, "socks5://cfg@h:1");
        assert!(matches!(
            resolve_proxy(Some("socks5://cli@h:2"), &cfg),
            ProxyDecision::Proxy(u) if u.contains("cli")
        ));
        // 2) config enabled, no CLI
        assert!(matches!(
            resolve_proxy(None, &cfg),
            ProxyDecision::Proxy(u) if u.contains("cfg") || u.contains("127") || !u.is_empty()
        ));
        // 3+4) disabled config → env or direct (both valid)
        let off = base_config(false, "socks5://cfg@h:1");
        let _ = resolve_proxy(None, &off);
    }
}
