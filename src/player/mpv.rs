//! mpv process management + JSON IPC over interprocess local sockets.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use interprocess::local_socket::tokio::prelude::*;
#[cfg(not(windows))]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Default)]
pub struct PlaybackSnapshot {
    pub time_pos: f64,
    pub duration: f64,
    pub paused: bool,
    /// 输出设备音量（mpv `ao-volume`），也就是系统音量合成器里 mixly 那一条。
    /// 首次 loadfile 之前 AO 还没初始化，属性不可用，此时为 None。
    pub volume: Option<f64>,
    pub eof: bool,
}

pub struct MpvPlayer {
    child: Child,
    socket_path: String,
    mpv_path: String,
    /// Last successfully loaded URL (local relay), for crash recovery.
    last_url: Option<String>,
}

impl MpvPlayer {
    pub async fn spawn(mpv_path: &str, socket_path: &str) -> Result<Self> {
        // Clean stale unix socket if present (no-op on Windows pipes).
        #[cfg(not(windows))]
        {
            let _ = std::fs::remove_file(socket_path);
        }

        let mut cmd = Command::new(mpv_path);
        cmd.args([
            "--no-video",
            "--idle=yes",
            // 播完停在文件末尾而不是卸载进 idle；否则 eof-reached 属性瞬间不可用，
            // 上层轮询观察不到 eof=true，顺序播放无法推进。
            "--keep-open=yes",
            "--force-window=no",
            "--no-terminal",
            &format!("--input-ipc-server={socket_path}"),
            "--ytdl=no",
            "--cache=yes",
            "--demuxer-max-bytes=50M",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);

        let child = cmd
            .spawn()
            .with_context(|| format!("spawn mpv at '{mpv_path}' (is mpv installed?)"))?;

        let player = Self {
            child,
            socket_path: socket_path.to_string(),
            mpv_path: mpv_path.to_string(),
            last_url: None,
        };

        // Wait until IPC is ready.
        let mut last_err = None;
        for _ in 0..50 {
            match player.ping().await {
                Ok(()) => {
                    info!(socket = %socket_path, "mpv IPC ready");
                    return Ok(player);
                }
                Err(e) => {
                    last_err = Some(e);
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("mpv IPC not ready"))).context("waiting for mpv IPC")
    }

    async fn connect(&self) -> Result<LocalSocketStream> {
        connect_local(&self.socket_path).await
    }

    async fn ping(&self) -> Result<()> {
        let _ = self.command(json!(["get_property", "idle-active"])).await?;
        Ok(())
    }

    pub async fn command(&self, command: Value) -> Result<Value> {
        let mut stream = self.connect().await?;
        let req = json!({ "command": command });
        let line = format!("{req}\n");
        stream.write_all(line.as_bytes()).await?;
        stream.flush().await?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        // Skip event-only lines until we get an error field.
        for _ in 0..20 {
            response.clear();
            let n = reader.read_line(&mut response).await?;
            if n == 0 {
                bail!("mpv IPC closed");
            }
            let v: Value = serde_json::from_str(response.trim())
                .with_context(|| format!("parse mpv response: {response}"))?;
            if v.get("error").is_some() {
                let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                if err != "success" {
                    bail!("mpv error: {err} (cmd={command})");
                }
                return Ok(v);
            }
            // event — ignore
            debug!(event = %response.trim(), "mpv event");
        }
        bail!("no mpv command response")
    }

    pub async fn loadfile(&mut self, url: &str) -> Result<()> {
        self.command(json!(["loadfile", url, "replace"])).await?;
        self.last_url = Some(url.to_string());
        Ok(())
    }

    pub async fn set_pause(&self, paused: bool) -> Result<()> {
        self.command(json!(["set_property", "pause", paused]))
            .await?;
        Ok(())
    }

    pub async fn toggle_pause(&self) -> Result<bool> {
        let snap = self.snapshot().await?;
        let next = !snap.paused;
        self.set_pause(next).await?;
        Ok(next)
    }

    /// 写 `ao-volume` 而不是 `volume`：前者就是系统音量合成器里 mixly 那一条，
    /// 双向同步；后者只是 mpv 内部的软件增益，跟系统完全无关，界面上永远对不上。
    /// AO 未初始化（首次 loadfile 之前）时 mpv 会拒绝该属性，这里如实返回错误。
    pub async fn set_volume(&self, volume: f64) -> Result<()> {
        let volume = volume.clamp(0.0, 100.0);
        self.command(json!(["set_property", "ao-volume", volume]))
            .await?;
        Ok(())
    }

    pub async fn seek_relative(&self, delta: f64) -> Result<()> {
        self.command(json!(["seek", delta, "relative"])).await?;
        Ok(())
    }

    pub async fn snapshot(&self) -> Result<PlaybackSnapshot> {
        let time_pos = self.get_number("time-pos").await.unwrap_or(0.0);
        let duration = self.get_number("duration").await.unwrap_or(0.0);
        let paused = self.get_bool("pause").await.unwrap_or(false);
        // ponytail: ao-volume 只在 AO 支持时可用（Windows wasapi、Linux pulse 都行，
        // ALSA 不行）。拿不到就是 None，界面显示「--」而不是编一个数出来。
        let volume = self.get_number("ao-volume").await.ok();
        let eof = self.get_bool("eof-reached").await.unwrap_or(false);
        Ok(PlaybackSnapshot {
            time_pos,
            duration,
            paused,
            volume,
            eof,
        })
    }

    async fn get_number(&self, prop: &str) -> Result<f64> {
        let v = self.command(json!(["get_property", prop])).await?;
        v.get("data")
            .and_then(|d| d.as_f64().or_else(|| d.as_i64().map(|i| i as f64)))
            .ok_or_else(|| anyhow!("property {prop} not a number: {v}"))
    }

    async fn get_bool(&self, prop: &str) -> Result<bool> {
        let v = self.command(json!(["get_property", prop])).await?;
        v.get("data")
            .and_then(|d| d.as_bool())
            .ok_or_else(|| anyhow!("property {prop} not a bool: {v}"))
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Restart mpv if the process died; reload last URL.
    ///
    /// 音量不需要在这里恢复：`ao-volume` 是系统按可执行文件记住的会话音量，
    /// mpv 重启后 Windows 会自己带回来，我们再写一遍反而会覆盖用户在合成器里的改动。
    pub async fn ensure_alive(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }
        warn!("mpv process died; restarting");
        let last = self.last_url.clone();
        let path = self.mpv_path.clone();
        let sock = self.socket_path.clone();
        // Drop old child
        let _ = self.child.kill().await;
        *self = Self::spawn(&path, &sock).await?;
        if let Some(url) = last {
            self.loadfile(&url).await?;
        }
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.command(json!(["quit"])).await;
        let _ = self.child.kill().await;
        Ok(())
    }
}

async fn connect_local(path: &str) -> Result<LocalSocketStream> {
    // Windows named pipes use namespaced names; Unix uses filesystem paths.
    #[cfg(windows)]
    {
        let name = path
            .trim_start_matches(r"\\.\pipe\")
            .to_ns_name::<GenericNamespaced>()
            .context("mpv pipe name")?;
        LocalSocketStream::connect(name)
            .await
            .with_context(|| format!("connect mpv pipe {path}"))
    }
    #[cfg(not(windows))]
    {
        use std::path::PathBuf;
        let name = PathBuf::from(path)
            .to_fs_name::<GenericFilePath>()
            .context("mpv socket path")?;
        LocalSocketStream::connect(name)
            .await
            .with_context(|| format!("connect mpv socket {path}"))
    }
}
