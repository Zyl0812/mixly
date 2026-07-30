<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
    <img src="assets/logo.png" alt="mixly logo" width="160">
  </picture>
</p>

<h1 align="center">mixly</h1>

<p align="center">用 Rust 写的轻量 <strong>CLI / TUI</strong> 音乐播放器。</p>

<p align="center">
  <img alt="language" src="https://img.shields.io/badge/rust-2021-orange?logo=rust">
  <img alt="license" src="https://img.shields.io/badge/license-MIT-blue">
</p>

<p align="center"><strong>简体中文</strong> | <a href="README.en.md">English</a></p>

## 安装（Windows x86_64）

在 PowerShell 里执行：

```powershell
irm https://raw.githubusercontent.com/Zyl0812/mixly/main/install.ps1 | iex
```

就这一条。脚本会下载预编译好的 `mixly.exe` 到 `%LOCALAPPDATA%\Programs\mixly`，写进用户 `PATH`，
并在你没装 [mpv](https://mpv.io/) 时用 winget / scoop 自动装上。**不需要 Rust 工具链。**
装完开一个新终端，`PATH` 才会生效。

验证：

```powershell
mixly --help
```

## 特性

- 网易云音乐 + QQ 音乐
- 混合歌单（两个平台的歌可以放在同一个列表里）
- 通过外部 **mpv** 播放，音质无损耗
- **API 请求和音频流都真正走代理**（内置本地 HTTP 中继）
- 自带 [agent skill](#agent-skill)，Claude Code / Codex / Grok 可以直接在命令行驱动它

## 使用

```bash
# 搜索
mixly search "海阔天空" --platform all
mixly search "晴天" --platform qq --limit 5

# 播放单曲
mixly play netease <song_id>
mixly play qq <song_id>

# 登录（扫码，用手机 App 扫）
mixly login --platform netease
mixly login --platform qq

# 歌单
mixly playlist create "My Mix"
mixly playlist list
mixly playlist add "My Mix" netease <id> --name "歌名" --artist "歌手"
mixly playlist add "My Mix" qq <id>
mixly playlist show "My Mix"
mixly playlist play "My Mix"
mixly playlist remove "My Mix" 0
mixly playlist rename "My Mix" "Mixed"
mixly playlist delete "Mixed"

# TUI 界面
mixly tui

# 强制指定代理
mixly --proxy socks5://127.0.0.1:7890 search "test"
```

### TUI 快捷键

| 按键 | 操作 |
|---|---|
| `j` / `k` | 上下移动 |
| `Enter` | 播放选中项 |
| `Space` | 暂停 / 继续 |
| `n` / `p` | 下一首 / 上一首 |
| `/` | 搜索 |
| `a` | 把搜索结果加入歌单 |
| `+` / `-` | 音量 |
| `h` / `l` | 快退 / 快进 5 秒 |
| `m` | 切换播放模式 |
| `q` | 退出 |

## Agent skill

mixly 自带一份 [`skills/mixly/SKILL.md`](skills/mixly/SKILL.md)，装进 AI 编码助手（Claude Code、Codex、Grok 等）后，
你就可以用大白话让它替你操作播放器——它在背后调的还是同一个 `mixly` 命令，不碰源码。

安装：把这个目录复制或软链到你的工具的 skill 目录。

```bash
# Claude Code（项目级）
mkdir -p .claude/skills && cp -r skills/mixly .claude/skills/

# Claude Code（全局，所有项目可用）
mkdir -p ~/.claude/skills && cp -r skills/mixly ~/.claude/skills/

# Grok CLI
mkdir -p .grok/skills && cp -r skills/mixly .grok/skills/
```

装好之后直接说人话就行：

> 「放首周杰伦的晴天」→ agent 自动 `search` → 挑一条 → 后台 `play`，然后告诉你在放哪首
>
> 「建个叫『通勤』的歌单，把这五首加进去」→ `playlist create` + 逐首 `search` / `add`，最后 `show` 给你确认
>
> 「把我 D 盘那个音乐文件夹导进来」→ `local import`

skill 里写清楚了这些规矩，所以 agent 不会瞎搞：

- 只走 CLI，**不开 `mixly tui`**（交互式界面会把 agent 卡死）
- 播放是长驻进程，一律**后台运行**，不会前台干等一整首歌
- 不会并行开多个 `play`（会抢 mpv 的 IPC 管道）
- 登录必须你本人扫码，agent 不会假装已登录
- 不会把 token 文件内容贴进对话
- 常见故障有对应处理：拿不到播放链接 → 提示你 `login` 或换平台；搜不到 → 建议 `--proxy` 或换关键词；mpv 报错 → 提示装 mpv

## 配置

配置目录（由 `directories` crate 决定）：

| 系统 | 路径 |
|---|---|
| Windows | `%APPDATA%\mixly\` |
| Linux | `~/.config/mixly/` |
| macOS | `~/Library/Application Support/mixly/` |

`config.toml` 示例：

```toml
[general]
quality = "Exhigh"          # Standard | Higher | Exhigh | Lossless
default_platform = "all"

[proxy]
enabled = false
url = "socks5://127.0.0.1:7890"

[player]
mpv_path = "mpv"
```

登录凭证单独存在 `netease_token.json` / `qq_token.json`。

### 代理优先级

```
--proxy  >  config.toml [proxy]  >  ALL_PROXY/HTTPS_PROXY/HTTP_PROXY  >  直连
```

音频固定走 mixly 的本地中继（`127.0.0.1:<随机端口>/current`），所以即使 mpv / FFmpeg 本身不支持 SOCKS，SOCKS5 和 HTTPS CDN 也能正常工作。

**注意：** `netease-qq-music-api` 0.1.0 内部的 `reqwest::Client` 是用 `.no_proxy()` 构建的，所以在该 crate 修改之前，`ALL_PROXY` **不会**影响上游 API 请求。音频中继（我们自己的 client）仍然遵循代理设置——这才是校园网 SOCKS + HTTPS CDN 场景下的关键路径。

## 架构（简述）

1. **API** —— `netease-qq-music-api =0.1.0`，隔离在 `src/api/client.rs`
2. **API 代理** —— 在 Tokio 启动前注入 `ALL_PROXY`；Cargo 会把 `reqwest` 与 `socks` 特性统一
3. **音频代理** —— 本地 HTTP 中继用 reqwest 拉 CDN 流（Range / Referer / 链接刷新）
4. **播放** —— mpv JSON IPC（`interprocess` 本地套接字 / Windows 命名管道）
5. **播放队列** —— 由 mixly 自己维护（每首歌一次 `loadfile`，因为链接会过期）

## 许可

MIT（见 [Cargo.toml](Cargo.toml) 中的 `license` 字段）。
