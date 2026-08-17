<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
    <img src="assets/logo.png" alt="mixly logo" width="160">
  </picture>
</p>

<h1 align="center">mixly</h1>

<p align="center">用 Rust 写的轻量 <strong>CLI / TUI</strong> 音乐播放器，可以直接用对话驱动。</p>

<p align="center">
  <img alt="language" src="https://img.shields.io/badge/rust-2021-orange?logo=rust">
  <img alt="license" src="https://img.shields.io/badge/license-MIT-blue">
</p>

<p align="center"><strong>简体中文</strong> | <a href="README.en.md">English</a></p>

## 效果展示

| TUI 播放界面 | Claude Code 插件与状态栏 |
|:---:|:---:|
| <img src="assets/screenshots/tui.png" alt="Mixly TUI 播放界面" width="100%"> | <img src="assets/screenshots/claude-plugin.png" alt="Mixly Claude Code 插件与状态栏" width="100%"> |

## 特性

- 网易云音乐 + QQ 音乐 + Bilibili 视频音频 + 本地文件，可以混在同一个歌单里
- 自带 agent skill，装好之后**用大白话让 AI 替你操作播放器**，不用记命令
- Claude Code 状态栏实时显示歌曲、进度与歌词
- 通过外部 **mpv** 播放，音质无损耗
- **API 请求和音频流都真正走代理**（内置本地 HTTP 中继）

## 安装（Windows x86_64）

**和 Claude Code 说：**

> 安装 Mixly 和状态栏：https://github.com/Zyl0812/mixly

它会读本 README 和 [`install.ps1`](install.ps1)，跑安装脚本、装插件、配状态栏，最后提示你 `/reload-plugins`。步骤幂等，重复执行没问题。

**或者手动敲命令**（按顺序执行）：

1. **安装 mixly 本体**（下载 `mixly.exe` 并加入 PATH）

   ```powershell
   irm https://raw.githubusercontent.com/Zyl0812/mixly/main/install.ps1 | iex
   ```

2. **注册插件市场**（第 3 步的 `@mixly` 指的就是这里注册的源）

   ```powershell
   claude plugin marketplace add Zyl0812/mixly --scope user
   ```

3. **安装 Claude Code 插件**（含完整 `mixly` skill 和状态栏用的 `play` skill）

   ```powershell
   claude plugin install mixly@mixly --scope user
   ```

4. **配置状态栏**（往 `~/.claude/settings.json` 写 `statusLine`，只依赖第 1 步）

   ```powershell
   mixly statusline install
   ```

最后回到当前 Claude Code 会话执行 `/reload-plugins`。只想要状态栏、不装插件的话，跑第 1 步和第 4 步就够了。

Codex / Grok 用户装 skill：`mixly skill install codex --global`（或 `grok`），详见 [Agent skill](#agent-skill)。

## 使用

### 登录

先登录再用：QQ 未登录常拿不到播放链接，B站 未登录搜索会被跳过。扫码必须你本人来，AI 只负责把二维码调出来，不会代扫、也不会读你的 token。

**和 Claude Code 说：**

> 帮我登录一下 QQ 音乐

**或者手动敲命令：**

```bash
mixly login --platform qq                    # 一次登录一个平台：netease | qq | bilibili
mixly logout --platform bilibili             # 删除本机凭证
```

### 放歌

**和 Claude Code 说：**

> 放首周杰伦的晴天
>
> 不知道听什么，随便放一首

**或者手动敲命令：**

```bash
mixly search "海阔天空" --platform all
mixly play "晴天"                            # 歌名直接播
mixly play qq <song_id>                      # 平台 + id 精确播
mixly play bilibili BV1xx411c7mD             # BV 号 / av 号 / 视频 URL
```

### 配置歌单

**和 Claude Code 说：**

> 建个叫「test」的歌单，把周杰伦这五首加进去
>
> 把test歌单里的第三首删掉，然后随机播

**或者手动敲命令：**

```bash
mixly playlist create "test"
mixly playlist add "test" qq <id> --name "晴天" --artist "周杰伦"
mixly playlist show "test"                   # 看下标，从 0 起
mixly playlist remove "test" 2
mixly playlist move "test" 2 0
mixly playlist play "test" --random --loop
mixly playlist list                          # 另有 rename / delete
```

### 导入本地音乐

**和 Claude Code 说：**

> 把我 D 盘那个音乐文件夹导进来

**或者手动敲命令：**

```bash
mixly local import D:\Music --playlist "本地"
mixly local list
mixly local remove <路径>                    # 只出库，不删磁盘文件
```

### 搜索优先平台

```bash
mixly config prefer netease                  # 持久化，默认 qq
mixly --prefer qq search "晴天"               # 仅本次覆盖
mixly config show
```

### 播放状态栏

`mixly play` 每 400ms 把当前歌曲、进度、暂停状态和歌词写入 `%APPDATA%\mixly\now-playing.json`，Claude Code 状态栏每秒读一次，不消耗模型 token。

```bash
mixly status --json      # 机器可读状态（无播放时输出空）
mixly status --claude    # 渲染 Claude Code status line
mixly statusline install # 单独安装 / 修复状态栏
```

已有 `statusLine` 时，安装器会把 `mixly status --claude` 放到原命令前面，播放信息在上、原内容在下。原对象备份到 `~/.claude/mixly-statusline-backup.json`，卸载时精确恢复；配置被改过则默认拒绝覆盖，`--force` 也只移除 Mixly 前缀。

### TUI

```bash
mixly tui
```

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

mixly 自带一份 [`skills/mixly/SKILL.md`](skills/mixly/SKILL.md)，已编译进 `mixly.exe`，装进 AI 编码助手后就能用上面那种对话方式操作播放器——背后调的还是同一个 `mixly` 命令，不碰源码。

```powershell
mixly skill install claude --global               # 也可换成 codex / grok；用户目录，所有项目可用
mixly skill install claude --project              # 只装到当前目录
mixly skill path codex --global
mixly skill uninstall codex --global
```

Claude Code 用户走上面[安装](#安装windows-x86_64)里的公开 marketplace 即可（同时装完整 `mixly` skill 和状态栏用的 `play` skill）；`mixly skill install claude --global` 是内嵌插件的本地安装方式。**两者二选一**，不要同时装 `mixly@mixly` 和 `mixly@mixly-local`。

目标文件被改过时安装/卸载会拒绝操作，确认覆盖加 `--force`。装完没识别就重启 Agent。

skill 里写清了规矩，所以 agent 不会瞎搞：

- 只走 CLI，**不开 `mixly tui`**（交互式界面会把 agent 卡死）
- 播放一律**后台运行**，不会前台干等一整首歌，也不会并行开多个 `play`（会抢 mpv 的 IPC 管道）
- 登录必须你本人扫码，不会假装已登录，也不会把 token 内容贴进对话
- 常见故障有对应处理：拿不到播放链接 → 提示 `login` 或换平台；搜不到 → 建议 `--proxy` 或换关键词；mpv 报错 → 提示装 mpv

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

## Bilibili 说明与限制

- 只播放**普通 UGC 视频的音频**（DASH 音频流）：不请求视频轨、不下载、不转码、不导出。
- 必须用你自己的账号扫码登录；凭证只保存在本机，不写入日志。
- 不支持：番剧/影视 PGC、课程、付费内容、直播；大会员/地区/DRM 限制不绕过；Dolby/Hi-Res 特殊音轨；自动续期（过期后重新扫码）。
- 播放时取账号当前允许的最高普通音频带宽，不做音质档位映射；不整段预取，只按需流式播放（Range）。
- 网页接口属实验性能力，可能随平台变化失效；届时该平台会给出明确错误，不影响网易云/QQ/本地。

## 代理

**和 Claude Code 说：**

> 用 socks5://127.0.0.1:7890 这个代理搜一下海阔天空

**或者手动敲命令：**

```bash
mixly --proxy socks5://127.0.0.1:7890 search "海阔天空"
```

想长期生效就写进 `config.toml` 的 `[proxy]`（见[配置](#配置)），也可以直接让 Claude Code 帮你改这个文件。

优先级：`--proxy` > `config.toml [proxy]` > `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` > 直连。

音频固定走 mixly 的本地中继（`127.0.0.1:<随机端口>/current`），所以即使 mpv / FFmpeg 本身不支持 SOCKS，SOCKS5 和 HTTPS CDN 也能正常工作。API 客户端与音频中继读取同一套代理设置。

## 许可

MIT（见 [Cargo.toml](Cargo.toml) 中的 `license` 字段）。
