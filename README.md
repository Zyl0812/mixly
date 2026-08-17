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

验证：

```powershell
mixly --help
```

## 特性

- 网易云音乐 + QQ 音乐 + Bilibili 视频音频（仅普通 UGC 视频音频，无下载）
- 混合歌单（不同平台的歌可以放在同一个列表里）
- 通过外部 **mpv** 播放，音质无损耗
- **API 请求和音频流都真正走代理**（内置本地 HTTP 中继）
- 自带 [agent skill](#agent-skill)，Claude Code / Codex / Grok 可以直接在命令行驱动它
- Claude Code 播放状态插件：底部状态栏实时显示歌曲、进度与歌词（`mixly status --claude`）

## 使用

```bash
# 搜索
mixly search "海阔天空" --platform all
mixly search "晴天" --platform qq --limit 5
mixly search "周杰伦" --platform bilibili     # 需先扫码登录

# 播放单曲
mixly play netease <song_id>
mixly play qq <song_id>
mixly play bilibili BV1xx411c7mD            # BV 号
mixly play bilibili av123456                # AV 号
mixly play bilibili https://www.bilibili.com/video/BV1xx/?p=2   # URL + 分 P

# 登录（扫码，用手机 App 扫）
mixly login --platform netease
mixly login --platform qq
mixly login --platform bilibili

# 退出登录（仅删除指定平台的本机凭证）
mixly logout --platform bilibili

# 歌单
mixly playlist create "My Mix"
mixly playlist list
mixly playlist add "My Mix" netease <id> --name "歌名" --artist "歌手"
mixly playlist add "My Mix" qq <id>
mixly playlist add "My Mix" bilibili BV1xx411c7mD --name "视频音频"
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

### 播放状态（Claude Code 状态栏）

`mixly play` 每 400ms 把当前歌曲、进度、暂停状态和歌词写入 `%APPDATA%\mixly\now-playing.json`。

```bash
mixly status --json     # 机器可读状态（无播放时输出空）
mixly status --claude   # 渲染 Claude Code status line（宽终端两行、窄终端三行）
```

安装 Claude Code 插件（需要 `claude` CLI；命令会注册本地 marketplace、安装插件并配置状态栏）:

```powershell
mixly skill install claude --global
```

如果已经有 Orca、Starship 等 `statusLine`，安装器会把 `mixly status --claude` 放到原命令前面：Mixly 播放信息显示在状态栏块顶部，原内容继续显示在后面。原对象完整备份到 `~/.claude/mixly-statusline-backup.json`，卸载时精确恢复；若组合配置后来被修改，默认拒绝覆盖，`--force` 也只移除 Mixly 前缀并保留其他修改。

### Bilibili 说明与限制

- 只播放**普通 UGC 视频的音频**（DASH 音频流）：不请求视频轨、不下载、不转码、不导出。
- 必须用你自己的账号扫码登录；凭证只保存在本机，不写入日志。
- 不支持：番剧/影视 PGC、课程、付费内容、直播；大会员/地区/DRM 限制不绕过；Dolby/Hi-Res 特殊音轨；自动续期（过期后重新扫码）。
- 播放视频时取账号当前允许的最高普通音频带宽，不做音质档位映射。
- Bilibili 曲目不做整段预取，只按需流式播放（Range）。
- 网页接口属实验性能力，可能随平台变化失效；届时该平台会给出明确错误，不影响网易云/QQ/本地。

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

Skill 已编译进 `mixly.exe`，无需克隆仓库或手动复制。安装到用户目录（所有项目可用）：

```powershell
mixly skill install claude --global
mixly skill install codex --global
mixly skill install grok --global
```

只安装到当前目录：

```powershell
mixly skill install claude --project
mixly skill install codex --project
mixly skill install grok --project
```

Claude Code 全局安装会把内嵌插件生成到 `~/.claude/mixly-marketplace/`，通过 `claude plugin` 注册 `mixly-local` marketplace 并安装 `mixly@mixly-local`，随后配置 `mixly status --claude`。已有状态栏会自动组合而不是覆盖，原命令仍接收 Claude 的 stdin；安装器只在缺失时补充 1 秒刷新，并在卸载时恢复原值。Skill 激活期间仅预授权 `mixly` CLI 命令，避免每次播放都弹确认；用户配置中的 `deny` / `ask` 规则仍优先。

查看路径或卸载：

```powershell
mixly skill path codex --global
mixly skill uninstall codex --global
mixly skill uninstall claude --global
```

若目标文件已被修改，安装或卸载会拒绝操作；确认覆盖或删除时加 `--force`。只需为实际使用的 Agent 安装，安装后若未自动识别，请重启 Agent。

装好之后直接说人话就行：

> 「放首周杰伦的晴天」→ agent 自动 `search` → 挑一条 → 后台 `play`，然后告诉你在放哪首
>
> 「不知道听什么，随便放一首」→ agent 结合最近对话和已有歌单推荐一首新歌，验证正式版本后后台播放
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
preferred_platform = "qq"   # qq | netease | bilibili

[proxy]
enabled = false
url = "socks5://127.0.0.1:7890"

[player]
mpv_path = "mpv"
```

登录凭证单独存在 `netease_token.json` / `qq_token.json` / `bilibili_token.json`（Bilibili 只保存最小 Cookie 字段）。播放状态快照在 `now-playing.json`，不含任何 token 或 URL。

### 代理优先级

```
--proxy  >  config.toml [proxy]  >  ALL_PROXY/HTTPS_PROXY/HTTP_PROXY  >  直连
```

音频固定走 mixly 的本地中继（`127.0.0.1:<随机端口>/current`），所以即使 mpv / FFmpeg 本身不支持 SOCKS，SOCKS5 和 HTTPS CDN 也能正常工作。

API 客户端与音频中继读取同一套代理设置。项目对锁定的 `netease-qq-music-api` 0.1.0 做了最小本地补丁，移除了其内部客户端的强制 `.no_proxy()`。

## 架构（简述）

1. **API** —— 网易云/QQ 走 `netease-qq-music-api =0.1.0`（隔离在 `src/api/client.rs`）；Bilibili 私有协议集中在 `src/api/bilibili.rs`（WBI 签名、二维码登录、DASH 音频取链）
2. **API 代理** —— 在 Tokio 启动前注入 `ALL_PROXY`；本地补丁让上游 API 客户端遵循该设置
3. **音频代理** —— 本地 HTTP 中继用 reqwest 拉 CDN 流（Range / Referer / 链接刷新；Bilibili 逐曲精确 Referer）
4. **播放** —— mpv JSON IPC（`interprocess` 本地套接字 / Windows 命名管道）
5. **播放队列** —— 由 mixly 自己维护（每首歌一次 `loadfile`，因为链接会过期）
6. **状态快照** —— 播放循环写入 `now-playing.json`，`mixly status --claude` 无网络渲染为 Claude Code 状态栏

## 许可

MIT（见 [Cargo.toml](Cargo.toml) 中的 `license` 字段）。
